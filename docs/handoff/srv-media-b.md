# srv-media-b handoff (2026-10-20)

Branch `srv-media-b`, worktree `_wt/srv-media-b` (from main 1be08bc). Task:
port the media SERVING half of the server (public file URLs, public pages,
embeds) to Rust, 1:1 on the same schema, with security fixes 1, 5 and 6
(PORT.md 12). The PROCESSING half (conversions, posters, motion tiles,
timelines, docx rendering, card images) is part A, branch `srv-media-a`;
everything this half needs from it goes through one module,
`server/src/media/part_a.rs`.

## Build and verify

```sh
S=_scratch/srv-media-b
$S/check.sh                      # cargo fmt + clippy -D warnings + cargo test (DB tests included)
$S/commit.sh "subject" ["body"]  # commits server/ and docs/ only when check.sh passes
$S/restart.sh                    # rebuild + run the Rust server on 127.0.0.1:8427 (log: $S/logs/server.log)
$S/stop.sh                       # stops only the process listening on 8427
```

Reference: `modlight-srvmediab-ref` (`$S/ref/docker-compose.yml`, the ORIGINAL
image on 127.0.0.1:8447, its Postgres on 55447); its containers are down
(`docker compose down`, volumes kept): `cd $S/ref && docker compose up -d`
brings it back with its data. The Rust server is stopped too. Both servers share one
AUTH_SECRET (`$S/secrets.env`, never printed), so render tickets, unlock
cookies and view grants minted by either verify on the other. Rust database:
`modlight_srv_media_b` on the dev Postgres 55421.

```sh
cd /path/to/modlight
PARITY_REF=http://localhost:8447 PARITY_RUST=http://localhost:8427 \
  python3 _wt/srv-media-b/server/tests/parity/media_serve.py [--only AREA] [-v]
# 1064 checks passed, 38 counted apart as pending-A (part A not merged)
```

Areas: raw, thumbs, gates, render, pages, bots, tagless, public, zip, oembed,
delete, embed, extras, fix1. The script uploads a corpus
(`$S/corpus`, copied from `_scratch/srv-library` plus generated files) through
ShareX on both servers and aligns ids, codes, delete tokens and dates
(`media_seed.py`); every area compares statuses, the headers that matter and
bodies (hashes for bytes, the reference's RSC props for pages, head tags in
order, the document's skin), and the counters each request left behind.

## What is ported (upstream -> Rust)

| Upstream | Rust |
|---|---|
| `lib/media/serve.ts` | `media/serve.rs`: getMedia (+ `lookup_media`), viewerRelation, isPrivilegedViewer, consumeView, cacheHeadersFor, parseRange, streamMedia; the view grant (fix 1) |
| `lib/media/render-ticket.ts` | `media/render_ticket.rs` |
| `lib/media/embed.ts`, `card.ts`, `bots.ts`, `link-formats.ts`, `download-menu.ts` | `media/{embed,card,bots,link_formats,download_menu}.rs` |
| `lib/crypto/envelope.ts` (server part) | `crypto/envelope.rs` (`isEncryptedUpload`, constants); the rest runs in the browser |
| `app/[userCode]/[mediaCode]/[action]/route.ts` | `routes/media_action.rs`: raw, raw.ext, download, render, timeline.vtt/jpg, convert |
| `app/[userCode]/[mediaCode]/delete/[token]/route.ts` | `routes/media_delete.rs` (fix 5) |
| `app/a/[userCode]/[slug]/zip/route.ts` + archiver 8 | `routes/album_zip.rs` + `archiver.rs` |
| `app/embed/[version]/[card]/route.ts` | `routes/embed_card.rs` |
| `app/oembed/route.ts` | `routes/oembed.rs` (fix 6) |
| `app/actions/unlock.ts` | `actions/unlock.rs` (`/_rpc/unlock/verifyMediaPassword`) |
| `app/[userCode]/[mediaCode]/page.tsx`, `app/[userCode]/page.tsx` | `pages/media.rs` (`load`, `load_tagless`): gate / encrypted / media views, generateMetadata, the direct-media-bot redirect, the tagless action redirect, the owner's skin and wallpaper |
| `app/u/[userCode]/page.tsx`, `app/a/[userCode]/[slug]/page.tsx` | `pages/public_profile.rs`, `pages/public_album.rs` |

Small shared additions: `http/web_response.rs` (Fetch-style `new
Response(text)`), `RequestSession::response_cookies()` (a page can set the
grant cookie), deps `flate2` + `crc32fast` (both were in the local cargo cache).

## Security fixes (SKELETON.md section 9, items 26-28)

**Fix 1, the exact rule** (`media::serve::ViewBudget`): for a file with
`max_views` and a viewer who is neither its owner nor an administrator,
1. what upstream counts is counted (the opening request: no `Range` or one
   from byte 0, no `Sec-Fetch-Dest: image|video`, no `w`/`h`/`motion`; bots
   on capped files) and refused with 404 when the budget is spent;
2. what upstream exempted is served without consuming only with the VIEW
   GRANT of a counted view of that file (cookie `ml_view_<id>`, HMAC over
   `view|<id>|<exp>`, 6 h), even when that view was the last one; without it,
   it consumes a view like 1;
3. this covers everything carrying the file's content: stored bytes (whole or
   ranged, `/raw`, `/raw.<ext>`, `/download`) and pictures made of them
   (resized stills, moving tiles, timeline sheets); refusals and the cue track
   spend nothing;
4. every counted view of a capped file mints the grant: the page's (document
   and `/_page`) and `/raw`/`/download`'s;
5. the page gives a preview bot (uncounted, as upstream) none of a capped
   file's content (no text, no render ticket); the card of a capped text file
   has no excerpt; an album ZIP spends a view (download) per capped item and
   leaves out an item whose budget is refused.

Bypasses tested (parity `fix1`: open on the reference, closed on Rust; and
`tests/media_serving_db.rs`): `Range: bytes=1-` on raw, raw.ext and download;
`Sec-Fetch-Dest: image` and `video` on raw and download; `/download?w=1`;
`?w=64` / `?h=1` falling back to the original; `?w=1200` and `?w=64` with
`Sec-Fetch-Dest: image` on a capped picture; Discordbot UA on a capped file;
a preview bot reading a capped text page (text, ticket, og:description);
the album ZIP. Legitimate flows verified on Rust: page then its `<img>`
(one view), the last view's `<img>` (the reference 404s it), an encrypted
one-time file read in two ranged requests for one view (the reference 404s
the second), a player's seeks after the counted opening request, a grant
bound to its own file.

**Fix 5**: `media_delete::authorized` compares the delete token with
`subtle` (constant time). **Fix 6**: `/oembed` gives a capped file as a `link`
with title and provider only.

## Deliberate differences (besides the fixes)

SKELETON section 9, items 29-32: the album ZIP works (the reference answers
500 on every ZIP: archiver 8 has no default export); metadata always in
`<head>` (Next streams it into the body for browsers); plain-object lookups
match only listed keys; thumbnails carry `Content-Length` (Next: chunked).
Inherited from the skeleton: no `Vary: rsc, ...` (item 11), JSON 500 for a
thrown error (item 24, e.g. `/x/y%25/raw`).

## Waiting for part A (srv-media-a)

`media/part_a.rs` is the only place to change after merging main with A:
`video_poster` (video `?w=` answers 415 now), `motion_preview` (row and probe
refusals now, `no-engine` otherwise), `timeline_track` / `timeline_sheet`
(row refusals now, `no-engine` for videos), `av_convert` (claims nothing:
video/audio `/convert` answers 415 wrong-target), `convert_image_buffer`
(500 "Conversion failed" after the checks; `?check=1` already answers like
upstream for files whose row has dimensions), `docx_to_html` (docx render
500 "Preview failed"), `render_card` (`/embed/v1/*.png` 500). Also move
`docx_support`, `shows_source`/`inline_text_plan`, `can_timeline_preview`,
`diagnose_playback`, `conversion_budget`, `read_derived`/`write_derived` to
A's modules if A ported them (same names), and delete the copies here. The
38 `pending-A` parity checks are exactly these; after wiring they must pass
(rerun `media_serve.py`). The derived pictures of capped files already go
through the fix-1 gate in the route (status 200 only), so A's functions need
no budget logic.

Wiring map against `srv-media-a` as of d4b81ad (not in main yet; main only
moved in docs since 1be08bc):

| bridge here | A's function | adapter |
|---|---|---|
| `video_poster` -> `Result<Option<Bytes>>` | `poster::video_poster(state, media, signal) -> Option<Bytes>` | wrap in `Ok` |
| `motion_preview` -> `MotionResult::{Ok, Refused}` | `motion::motion_preview(state, media, MotionOptions{width, height, fit: MotionFit}, signal) -> Result<Result<(Bytes, u32), MotionRefusal>>` | `inside` -> `MotionFit::Inside`; map the inner Result |
| `timeline_track` / `timeline_sheet` | `timeline::timeline_track/sheet(state, media, signal) -> Result<Result<String / Bytes, TimelineRefusal>>` | map; use A's `TimelineRefusal` |
| `av_convert(state, AvConvertRequest<'_>)` | `av_action::av_convert(state, AvConvertRequest)` with `AdmitHook` (`'static`, `Result<bool>`) | clone the row into the request; the admit closure already owns its clones |
| `convert_image_buffer(input, to, animated, timeout, signal)` | `convert::convert_image_buffer(input, to, ConvertOptions{..})` | build the options |
| `conversion_budget` | `convert::conversion_budget` (same signature) | re-export |
| `read_derived` / `write_derived(state, ..)` | `derived_cache::read_derived/write_derived(&state.storage, ..)` | pass `&state.storage` |
| `render_card` -> `Result<Vec<u8>>` | `card_render::render_card(choice) -> Result<Bytes, String>` | map the error |
| `shows_source(&Upload)`, `inline_text_plan` | `source_preview::shows_source(&SourceSubject)`, `inline_text_plan -> &str` | build the subject |
| `can_timeline_preview(&str, &str, i64)` | `timeline_preview::can_timeline_preview(Option<&str>, Option<&str>, i64)` | wrap in `Some` |
| `diagnose_playback` | `playback_health::diagnose_playback` | re-export |
| `docx_to_html` | not in A yet | keep the stub |

`media/card.rs` and `media/download_menu.rs` exist on both branches (same
upstream files, same names): keep A's, which `card_render` uses; `embed.rs`
uses only `CARD_WIDTH/HEIGHT`, `CardSubject`, `card_choice_for`, `card_url`,
which A's file has with the same signatures; drop this branch's
`download_menu` tests if A's cover the same cases (they port the same test
file).

## Not done / open

- `app/actions/convert.ts` (convert in place) is not in this branch's scope.
- Client-only upstream tests (type-badge, player-contract, player-ssr, the
  client halves of viewer-stage, encrypted-presentation, encrypted-link)
  belong to the SPA's vitest; `server/web` has no `node_modules` here and
  nothing was installed. Their server halves are Rust tests
  (`part_a`, `crypto::envelope`, `media_serving_db`).
- On S3 installs a finished tus upload stays readable at `GET /api/files/<id>`
  whatever its view cap (SKELETON item 23, the uploads port's); only the
  uploader knows that URL, but it is a way around fix 1.
- Owner's preferred domain (`APP_DOMAINS`) was not exercised by the parity
  run (the scratch reference has no extra domains).
- Rows from parity runs remain in both databases (users `ms*`).

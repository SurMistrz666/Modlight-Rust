# Bug hunt 2026-11-11: srv-auth (accounts, sign-in, sessions, permissions, pages)

Worktree `_wt/hunt-srv-auth`. Status: round 2 done (round 1 is staged; round 2 is the unstaged
diff). Work is the unstaged diff (`git diff`) plus this
untracked report; nothing committed. No migration, no snapshot, no web/app change.

## 1. Files read

Docs: `server/docs/PORT.md`, `server/docs/SKELETON.md`, `docs/audit/rust-server.md`,
`docs/audit/rust-delta-2026-10-24.md` (pickup P1-P3), `docs/audit/logic-2026-10-24/01-signin.md`.

Read fully (production code; test modules skimmed):

- [x] `auth/`: mod, api, endpoint, routes/{mod, sign_in, sign_out, sign_up, password, session,
      update_session, update_user, account, email_verification, misc}, origin, ip, rate_limit,
      password, cookies, session, adapter, crypto, email, error, two_factor, admin, server, validate
- [x] `authorize/`: mod, loopback, pickup
- [x] `rpc/mod.rs` (`rpc/tagged.rs` skimmed)
- [x] `actions/`: mod, account, admin, authorize, invite, login_identifier, mail, registration,
      releases, tokens, turnstile (auth checks)
- [x] `pages/`: mod, table, authorize, layout_auth, layout_app, root_layout, root_page, login, register,
      forgot, reset, two_factor, welcome, invite, download, dashboard, dashboard_admin,
      dashboard_admin_user_files, dashboard_profile, dashboard_tokens, dashboard_downloads,
      dashboard_paste, dashboard_upload, public_profile, public_album, shortlink_unlock;
      media (gating parts), meta and shell (escaping), dashboard_{folders,links,trash,duplicates}
      (owner scoping)
- [x] `http.rs`, `http/`: proxy, headers, cookies, static_files; set_cookie (serialisation)
- [x] `i18n/request.rs` (`i18n/format.rs`, `i18n/mod.rs` skimmed); messages en/pl: key sets and ICU
      arguments compared for every namespace (identical)
- [x] `crypto/envelope.rs`, `runtime/limits.rs`
- [x] `data/`: sessions, user_prefs, admin_files; admin and admin_disk (arithmetic and inputs only)
- [x] `routes/`: mod, download_app; api/{me, client_settings, theme, authorize_pickup, health,
      releases_app, background, avatar, admin_disk}
- [x] top-level: auth_helpers, password, registration, invites, secret_box, settings, credentials,
      admin_users, audit, api_token, api_scopes, api_caller, api_json, js_json, js_date, js_value,
      ids, format, client_settings, theme_api, theme_cookies, themes (API), mail.rs,
      mail/address.rs (tokenizer), mail/smtp.rs (command building), mail_settings (verification
      and saving), mail_templates (substitution, escaping), turnstile (entry points and
      verdicts), releases_core (versions, manifest), config (JS number helpers, brand), error,
      main (health probe)

## 2. Findings

Line numbers are those of the base commit (before the fixes).

### AUTH-01 (low): `new Date(text)` panics on non-ASCII text and overflows on a huge offset

`server/src/js_date.rs:54` (`parse_iso_format`), `:139` (`split_offset`), `:213` (`parse_legacy`).

- The ISO parser cuts the input at fixed BYTE offsets (`&r[3..4]`, `&off[2..3]`, ...) after
  checking only the length in bytes. A two-byte character across such an offset panics:
  `GET /dashboard?from=2026-a%C3%A9xy` (the date filter, `pages/dashboard.rs` `parse_day`) or an
  upload expiry sent to the API (`uploads.rs`, `parse_js_date`) panicked the request; the edge
  turned it into a 500. Reproduced: `parse_js_date("2026-a\u{e9}xy")` panicked at
  `js_date.rs:73` ("byte index 3 is not a char boundary").
- The legacy parser multiplied the hours of a numeric offset (`+2147483647`) in `i32`: overflow
  (a panic in a debug build, a wrapped offset in release).

Fix: the ISO format is ASCII only, so `parse_iso_format` returns `None` for non-ASCII input
(the legacy parser, which reads characters, still gets it); the offset is computed with
`checked_mul` / `checked_add` (an absurd offset is an Invalid Date).

Test: `js_date::tests::hostile_text_is_an_invalid_date_not_a_panic` (failed before the fix
with the panic above).

### AUTH-02 (medium): sign-ups racing on a fresh install with registration closed all get accounts

`server/src/auth/routes/sign_up.rs:61` (`registration_gate`), `server/src/registration.rs:179`
(`evaluate_sign_up`), `server/src/auth/adapter.rs:231` (`create_user`).

The registration gate (lib/auth.ts `hooks.before`) allows a sign-up on a closed install only as
the bootstrap exception: "no account exists yet, this one becomes the administrator". It counts
the accounts outside the transaction that creates the account (the same TOCTOU as security fix 3,
which PORT.md 12.3 fixed for the ROLE only). Every sign-up that arrived before the first one
committed passed the gate; `create_user` then serialised them, made the first the administrator
and every other one an ordinary account. On an install the owner deliberately closed, a stranger
racing the owner's first sign-up kept an account. Reproduced: 12 concurrent sign-ups on a fresh
database with registration closed gave 12 accounts.

Fix: the gate now reports whether it allowed the request ONLY as the first account
(`registration::evaluate_sign_up_gate`, `SignUpVerdict::only_as_first_account`: the same request
would be refused once an account exists). The flag rides on `EndpointCtx::sign_up_only_as_first`
into `adapter::create_user_with(.., only_as_first)`, which re-checks the count inside the
transaction holding the advisory lock and refuses (`NotFirstAccount`, nothing written) when the
table is no longer empty. The loser gets the answer the gate gives once an account exists
(`403 REGISTRATION_DISABLED`, or the key refusals). An open install, admin-created accounts and
the first sign-up itself behave exactly as before.

Test: `tests/security_fixes_db.rs` `racing_first_sign_ups_on_a_closed_install_make_exactly_one_account`
(12 concurrent sign-ups, registration closed: 1 account, 11 refused 403, no orphan credential rows).
Failed before the fix: `(12, 0)` instead of `(1, 11)`.

### AUTH-03 (low): an over-long timed ban was silently turned into a ban that expires at once

`server/src/auth/admin.rs:562` (`ban_user`, `POST /api/auth/admin/ban-user`, admin only).

`banExpiresIn` (seconds, any JSON number) was turned into a date with
`JsDate::from_millis(now + (secs * 1000.0) as i64).unwrap_or_else(JsDate::now)`. A value whose
date chrono cannot hold (from about 8.2e15 ms, e.g. `1e13` seconds) or whose product saturates the
cast (`1e300`; in a debug build `now + i64::MAX` overflowed and panicked) fell back to NOW: the
account was answered as banned, its sessions were deleted, and the ban, already expired, was
lifted at its next sign-in (`check_ban_before_session_create`). Upstream's `getDate` makes an
Invalid Date there, the write fails (500) and nobody is banned with a bogus expiry.

Fix: `ban_expiry(now_ms, secs)` computes the time value as JavaScript does (double arithmetic,
truncated) and answers `None` outside the Date range; the endpoint then fails with an internal
error before writing anything, like upstream.

Test: `auth::admin::tests::a_ban_too_long_for_a_date_is_refused_not_lifted` (the old expression
gives `now` for `1e13`, `1e16`, `1e300`, `-1e300`).

### AUTH-04 (medium): unauthenticated memory growth through the `/api/auth/*` rate limiter

`server/src/auth/rate_limit.rs:75-129` (`memory`, `get`, `set`, `key`, `record`).

Better Auth's limiter keys a bucket on `"<ip>|<path below /api/auth>"` and counts every answer,
unknown paths included (upstream mechanics, kept). Its `memory` store forgets an entry only when
the SAME key is read again after it expired, and nothing else ever removed one. The path is the
caller's choice, so one anonymous client (one IP, no rate limit applies across distinct keys)
could send `POST /api/auth/<random>` forever and every request left a permanent entry holding the
whole path; hyper accepts a request line of about 400 KB, so a few thousand requests pin
gigabytes. (The earlier audit's "bounded by the socket address" overlooked the path in the key;
upstream's Map has the same leak, with Node's 16 KB header limit per entry.)

Fix, storage only (every answer unchanged): the store is now a `Memory` struct whose `set`
sweeps expired entries when it holds more than 10 000 at most every 30 s (an expired entry and a
missing one answer alike, so this is invisible to clients); a path longer than 256 bytes is keyed
by its SHA-256 (`ip|#<hex>`, which no pathname can spell), so an entry stays under ~100 bytes and
two long paths are still two buckets. No real route is near 256 bytes.

Tests: `auth::rate_limit::tests::made_up_paths_do_not_pile_up` (10 051 made-up paths, one of
100 000 bytes: the longest key stays small, and a window later the next write sweeps them all;
failed before the fix with a 100 014-byte key and no sweep),
`a_long_path_is_limited_like_any_other` (the digest key still refuses the 121st request in the
window and keeps distinct long paths apart).

### AUTH-05 (medium): the attempt throttle stored caller-typed keys whole (memory DoS)

`server/src/password.rs:342` (`check_rate_limit_at`), caller `server/src/actions/invite.rs:174`
(`invite.resendVerification`, unauthenticated).

The in-process throttle of lib/password.ts keeps up to 20 000 live buckets (then evicts the
oldest), but a bucket key can carry what the caller typed: `resendVerification(address)` counts
`verify-resend:<address>` BEFORE anything checks the address, and an RPC argument may be ~1 MiB.
An anonymous client (the RPC origin check only wants `Origin` to match `Host`, which any non-browser
client sends) could fill the map with 20 000 one-megabyte keys, ~20 GB held for the 15-minute
window. Upstream has the same map.

Fix (storage only, same answers): a key longer than 256 bytes is stored under its SHA-256
(`#<hex>`; real keys are a prefix plus an IP, a UUID or an id, all far shorter). Two distinct long
keys stay two buckets; `reset_rate_limit` maps the key the same way.

Test: `password::tests::a_long_key_is_stored_small_and_counted_alike` (a 500 000-byte address:
counted, refused on the 4th attempt, distinct from a neighbour, reset; failed before the fix
holding a 500 028-byte key).

### AUTH-06 (low): `decodeURIComponent` accepted a signed escape (`%+1`)

`server/src/http/cookies.rs:28` (`decode_uri_component`, used for page and route parameters in
`pages/table.rs` and for cookie values).

`u8::from_str_radix` accepts a leading `+`, so `%+1` decoded to U+0001 and `%+f` to U+000F,
where JavaScript's `decodeURIComponent` throws a URIError (Next answers such a path 400, which
`pages::table::match_path` reproduces as `MalformedPath`). A path parameter or cookie spelled that
way reached the loaders as a control character instead of being refused.

Fix: both characters after `%` must be ASCII hex digits.

Test: `http::cookies::tests::uri_component_round_trip` (`%+1`, `a%+fb` refused).

## 3. Not fixed

- **NF-1 (fixed in round 2, R2-1)** (low, DoS hardening; parity): `/api/auth/*` reads a body of up to 32 MiB
  (`auth/api.rs` `BODY_MAX`) from anybody, and the Better Auth limiter counts a request only once
  it is answered, so concurrent in-flight requests are not limited at all: N slow connections pin
  N x 32 MiB. Upstream buffers without any limit, so this is not a regression; every real auth body
  is a few hundred bytes (sign-up with an `image` data URL is the largest conceivable, and the panel
  never sends one). Recommendation: lower `BODY_MAX` to 1 MiB, the RPC limit (a one-line change;
  left alone because it changes what an absurd request gets, 413 instead of being processed).
- **NF-2 (fixed in round 2, R2-2)** (low, cross-feature): the release channel (`releases_core.rs`
  `publish_release_with` / `current_release_with`, `/api/releases/app`, the panel's
  `releases.publishAppRelease`) accepts an upload whose OWNER has a file vault in state
  `encrypting`/`on` (docs/ENCRYPTION.md): such a file's `/raw` asks for the file password (401/423),
  so every installed client would fail its update check against the manifest. Also a file with a
  view limit is accepted (as upstream) and stops being downloadable after `max_views`. The newer
  Downloads feature (SKELETON 9.45) does not have the problem. Recommendation: treat both as
  `not-public` in `publish_release_with` and `current_release_with` (needs the owner's vault state
  in `ManifestRow`/`PublishRow`), if the old channel is still in use.
- **NF-3 (info, parity with better-auth):** following a still-valid e-mail verification link signs
  the browser in (`autoSignInAfterVerification`) without the second factor, for an account whose
  address is unconfirmed (e.g. 2FA enabled while confirmation was off, confirmation switched on
  later). Needs the mailbox. Upstream does the same; the owner may want verification links to stop
  creating sessions for accounts with `two_factor_enabled`.
- **NF-4 (fixed in round 2, R2-3)** (info, parity): Better Auth's own `/api/auth/admin/update-user` with
  `data.banned = true` (admin only, not used by the panel) bans without deleting the account's
  sessions, so its live sessions keep working until they expire; the panel's ban goes through
  `/admin/ban-user`, which does delete them.
- **NF-5 (info, parity):** a TOTP code stays valid for its whole window (+-1 step) and can be
  replayed within it; `/two-factor/*` is limited to 3 requests per 10 s per address.

## 4. Outside my area

- `server/src/actions/shortlink_unlock.rs`, `vault/unlock.rs`: they key the same in-process
  throttle; covered by the AUTH-05 change in `password.rs` (no change in their files).
- NF-2's fix (round 2) reads `file_vaults.state` and `crate::vault::VaultState` from `releases_core.rs`; no
  file of `vault/` changed.

## 5. Checks run at the end

In `server/` (with `CARGO_BUILD_JOBS=6`, `nice -n 10`):

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test` with `MODLIGHT_TEST_DATABASE_URL` on the dev Postgres (127.0.0.1:55421, each DB test
  makes its own database): 38 suites, 1431 passed, 0 failed, 4 ignored (the existing opt-in ones).
  The database tests ran (checked `security_fixes_db` with `--nocapture`: no "skipped").
- Every new test was run against the code with its fix disabled and failed (AUTH-01 panic,
  AUTH-02 `(12, 0)`, AUTH-04 100 014-byte key, AUTH-05 500 028-byte key, AUTH-06 `%+1` decoded),
  then passed with the fix restored.

Summary: 6 found and fixed (medium: AUTH-02, AUTH-04, AUTH-05; low: AUTH-01, AUTH-03, AUTH-06),
5 not fixed (NF-1..NF-5: parity or owner's decision). Files changed: `auth/adapter.rs`,
`auth/admin.rs`, `auth/endpoint.rs`, `auth/rate_limit.rs` (largest: the store became a struct),
`auth/routes/sign_up.rs`, `http/cookies.rs`, `js_date.rs`, `password.rs`, `registration.rs`,
`tests/security_fixes_db.rs`.

## Round 2 (2026-11-12)

The coordinator asked for NF-1, NF-2 and NF-4 to be fixed; NF-3 and NF-5 stay with the owner.
Each fix has a test that failed with the fix switched off and passes with it.

### R2-1 (was NF-1): `/api/auth/*` reads at most 1 MiB of body

`server/src/auth/api.rs` `BODY_MAX`: 32 MiB -> 1 MiB (the server-action limit). Checked first that
nothing legitimate sends more: the SPA's auth forms send a few fields; avatars and wallpapers go
to `/api/avatar` and `/api/background`; no client (SPA, upstream components, desktop app) calls
`/update-user` with an `image` (and an `image` there is a key or a URL, never a data URL Modlight
would use), so no route needed a limit of its own. A larger body is answered `413` while it is
being read, before any endpoint, session lookup or database work.

Tests: `auth::api::tests::bodies_are_capped_at_one_mebibyte` (1 MiB read, 1 MiB + 1 and 2 MiB
refused 413), `tests/http_edge.rs` `auth_bodies_over_one_mebibyte_are_refused` (a 2 MiB
`/api/auth/update-user` through the whole edge: 413; it was a 401 after reading all of it).

### R2-2 (was NF-2): the old release channel refuses uploads installed copies cannot fetch

`server/src/releases_core.rs` (`undownloadable`, `publish_release_with`, `current_release_with`,
the `publish_row`/`manifest_row` statements), `server/src/routes/api/releases_app.rs`
(`refusal_message`), `server/messages/{en,pl}/admin.json` (`release.errors.encrypted`,
`release.errors.view-limited`), `server/docs/CONTRACT.md` (releases.publishAppRelease).

Publishing (panel action `releases.publishAppRelease` and `POST /api/releases/app`) now refuses,
after upstream's `not-public` check and before anything is written:
- `encrypted`: the upload's stored object is ciphertext (`uploads.encryption` set), or its owner's
  file vault is `encrypting`/`on` (every file of such an account asks for the file password, so
  every updater would get 401/423);
- `view-limited`: the upload has `max_views` (each update check would spend a view until the
  file stops downloading).
`POST /api/releases/app` answers 400 with the reason and a sentence in `message`; the panel's toast
(upstream's `ReleaseChannel`, which reads `release.errors.<reason>`) shows the new en/pl texts. A
pointer published earlier whose file became one of these (the owner turns encryption on later)
makes `currentRelease` answer "no release" (404 manifest, `/download/app` 404), as for a dangling
pointer, instead of a manifest every installed copy fails on. A vault being switched off
(`decrypting`) does not gate the account; its files that are still encrypted are refused by their
own column until the pass has rewritten them.

Note for the merge: two keys were added to upstream's `admin.json` (like `nav.downloads` before);
the SPA's generated stub types still list upstream's reasons only (the toast is a template lookup,
so nothing else in the SPA changes). `npm run verify` passes.

Tests: `releases_core::tests::encrypted_or_view_limited_uploads_cannot_be_the_release` (four
refusals, nothing written; `decrypting` passes; a published pointer whose owner turns the vault on
or whose file gets a limit reads as none), `tests/releases.rs`
`the_channel_refuses_uploads_installed_copies_could_not_fetch` (against Postgres: token route and
panel action refuse `limit1` / `cipher1` / `vault1` with their reasons, nothing published; a plain
upload publishes, and the manifest disappears once its owner's vault is `encrypting`). Before the
fix the token route published the view-limited file (200).

### R2-3 (was NF-4): a ban through `/api/auth/admin/update-user` ends the account's sessions

`server/src/auth/admin.rs` (`admin_update_user`).

When `data.banned` is in the update and the row comes back banned (Postgres reads the value, so
`true`, `"1"`, `"t"` all count), the account's sessions are deleted, as `/admin/ban-user` (the
panel's ban) deletes them. Like `/admin/ban-user`, a ban of the caller's own account is refused
(`400 YOU_CANNOT_BAN_YOURSELF`, nothing written): with the sessions now ending, a self-ban here
would lock the administrator out at once. Other fields and lifting a ban touch no session.

Role changes (`data.role`, also `/admin/set-role` and the panel's role switch) deliberately do NOT
end sessions: a session reads the account row on every request (`auth::session::find_session`
joins the user), so a demotion takes effect on the very next request, and ending sessions would
only sign a promoted or demoted user out for nothing. The panel's role change does not end them
either.

Test: `tests/security_fixes_db.rs` `a_ban_through_admin_update_user_ends_the_sessions` (a name
change keeps the 2 sessions; `banned: true` ends both and no other account's; `"1"` bans and ends
too; lifting keeps a new session; a self-ban is refused and changes nothing). Before the fix the
banned account still had its 2 sessions.

### Round 2 checks

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test` with `MODLIGHT_TEST_DATABASE_URL` (dev Postgres on 127.0.0.1:55421, up again
  after the restart): 38 suites, 1436 passed, 0 failed, 4 ignored (round 1: 1431; +5 new tests).
- `server/web`: `npm run verify` (typecheck, 103 tests, build, bundle audit, sync check: 161 copies
  verbatim, 98 ports current) passes. `node_modules` was installed into this worktree OFFLINE from
  the local pnpm store (`pnpm install --offline --frozen-lockfile`, same lockfile as the other
  worktrees); it and `dist/` are gitignored.


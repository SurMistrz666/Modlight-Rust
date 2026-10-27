# Per-account file encryption ("file vault")

Not upstream: an intended addition on top of the 1:1 port (like Turnstile,
SKELETON.md section 9, item 46). An account holder can switch it on in
Settings -> Security. From then on ALL of that account's files are stored
encrypted, and reading any of them, by the owner in the panel or by anybody
following a link, needs that account's **file password**. A browser that
entered it may remember it for 30 days.

For an account that never switches it on nothing changes for its clients: no
cookie, no header, the same answers as before (a file lookup and an upload
each read the vault table once more; the Settings page carries the card's
status). Every intended difference is listed in section 9.

## 1. Threat model

**Protected**

- A copy of the storage (the `appdata` volume, the S3 bucket, a backup of
  either, a stolen disk): every stored object of an encrypted account is
  ciphertext, and so is everything derived from it (section 5).
- A copy of the database (a dump, a backup): it holds the account's private
  key only wrapped by a key derived from the file password with Argon2id, and
  per-browser copies wrapped by keys derived from secrets that live only in
  those browsers' cookies (and the server's own secret, section 3.4).
- The database AND one browser's cookie, without the server's `.env`: still
  nothing, because the grant key also mixes in a server key (section 3.4).
- Administrators: the admin role opens nothing. The admin panel says so where
  an admin would look at the files, and the user list marks the account.
- Anybody with a link: the file page, the raw and download URLs, previews,
  conversions, galleries, albums and their ZIP all ask for the file password.
  Link unfurlers (Open Graph, oEmbed, Discord, Telegram, Slack) get the same
  card a password-protected file gets: the product name, no file name, no
  picture, no size.

**Not protected** (this is server-side encryption, and these are its limits)

- A server that is running and compromised (root on the host, a modified
  binary, a memory dump): while a file is being read the server holds its key.
  Nothing short of encryption in the browser (the existing end-to-end
  encrypted uploads, `lib/crypto/`) protects against the server itself.
- Metadata in the database stays readable: file names, types, sizes, dates,
  dimensions, durations, view counts, folders, and the SHA-256 of each file's
  contents (the duplicates finder, `/api/uploads/check` and `/api/link-info`
  depend on it; a database reader can confirm that an account holds a file
  the reader already has). The blurhash (a tiny picture of an image) is NOT
  kept for encrypted files.
- Bytes in flight: an upload arrives in plaintext over TLS; tus parts and the
  ShareX temp file are plaintext on the server's disk until the upload is
  finished, and transient workspaces of ffmpeg (posters, timelines,
  conversions) hold a decrypted copy while they run. All are deleted
  afterwards, but deleted is not wiped. On S3 a tus upload is assembled in the
  bucket in plaintext before it is encrypted into a new object and deleted, so
  a bucket with versioning keeps that plaintext version: switch versioning off
  (or expire noncurrent versions) for encrypted accounts.
- A weak file password: the database copy can be attacked offline. Argon2id
  makes each guess cost about 64 MiB and a fraction of a second.
- A stolen cookie works until it expires or is revoked (Settings -> Security
  lists every browser and app that holds one).
- Avatars and wallpapers are public decorations and are not encrypted.

**Lost password = lost files.** There is no recovery key and no admin
override. The panel says so before it lets anybody switch encryption on.

## 2. Keys

| Key | What | Where it lives |
|---|---|---|
| account key pair | X25519 (`x25519-dalek`), made when encryption is switched on | public key in `file_vaults.public_key` (with an HMAC under the server key); private key only wrapped (below) |
| password key | Argon2id v1.3 (file password, 16-byte random salt), 32 bytes; m = 64 MiB, t = 3, p = 1, stored with the record | never stored |
| wrapped private key | XChaCha20-Poly1305 (password key, random 24-byte nonce, AAD = `modlight vault key v1` + user id + public key) | `file_vaults.wrapped_key` |
| file key | 32 random bytes per stored object, fresh on every write | sealed to the public key in `uploads.encryption` |
| grant secret | 32 random bytes per browser (or app) that unlocked | its cookie (or the app's memory); never stored by the server |
| grant key | HKDF-SHA256(grant secret, salt = server key, info = grant id + user id) | never stored |
| grant copy of the private key | XChaCha20-Poly1305 (grant key, random nonce, AAD = grant id + user id) | `vault_grants.wrapped_key` |
| server key | HKDF of `SETTINGS_ENCRYPTION_KEY` or `AUTH_SECRET` (`secret_box`, purpose `vault`) | derived at runtime |

Only audited crates: `chacha20poly1305`, `x25519-dalek`, `argon2`, `hkdf`,
`hmac`/`sha2`, `zeroize`. Secret keys sit in `Zeroizing` buffers (or dalek's
zeroizing `StaticSecret`) and are wiped when dropped. Every check of a secret
is an AEAD tag check, which is constant time; the password itself is never
compared with anything.

**Sealing a file key** (anybody with the public key can do it, so uploads
need no password): an ephemeral X25519 key pair; `shared = X25519(eph, pk)`
(an all-zero result is refused); `wrap = HKDF-SHA256(shared, salt = eph_pk ||
pk, info = "modlight file key v1")`; `sealed = eph_pk || nonce || XChaCha20-
Poly1305(wrap, nonce, file_key, AAD = "modlight file key v1" || storage key)`.
Binding the storage key means a sealed key copied onto another row does not
open.

## 3. Formats

### 3.1 Stored objects (chunked AEAD, the STREAM construction)

The plaintext is cut into chunks of 64 KiB (the last one shorter, an empty
file is one empty chunk). Chunk `i` is stored as
`XChaCha20-Poly1305(file_key, nonce_i, chunk_i)` (16-byte tag each), back to
back, with no header; `nonce_i = prefix (19 random bytes) || i (u32 big
endian) || final (1 byte: 1 for the last chunk, else 0)`. The final flag makes
truncation and extension detectable; the index makes reordering detectable;
the per-object random key and prefix make nonce reuse impossible.

Ciphertext size = plaintext size + 16 x number of chunks. A Range request for
plaintext bytes `a..b` reads only the chunks that cover it (ciphertext offset
`i * (65536 + 16)`), decrypts and verifies each, and cuts the answer out, so
video seeking and resumed downloads work and cost at most two extra chunks.

### 3.2 `uploads.encryption` (text, NULL = plaintext)

`v1.<chunk size>.<plaintext size>.<prefix, base64url>.<sealed key, base64url>`.
`uploads.size` stays the plaintext size (quota, listings and Range work in
plaintext bytes). The column and `uploads.storage_key` always change together,
in one statement.

### 3.3 Tables (migration 0016)

- `file_vaults` (one row per account that has encryption on or is turning it
  on or off): `state` (`encrypting` | `on` | `decrypting`), `public_key`,
  `public_key_mac`, `wrapped_key`, Argon2 salt and parameters, `generation`
  (bumped by a password change), `job_key` (only while `decrypting`: the
  private key in the secret box, because the files are being decrypted on the
  owner's explicit request), `job_error`, timestamps.
- `vault_grants`: `id`, `user_id`, `kind` (`owner` = the account's own
  browser, `visitor` = somebody who opened a link, `app` = a bearer client),
  `generation`, `wrapped_key`, `label` (browser or app name), `ip` (only for
  the owner's own browsers and apps: a visitor's address is not kept),
  `created_at`, `expires_at`, `last_used_at`.
- `vault_cleanup`: storage keys to delete (an encryption pass's own temporary
  object, or the plaintext object a pass replaced), so a crash between "the row
  points at the new object" and "the old one is gone" is finished later.

### 3.4 Grants (what "remember for 30 days" is)

Unlocking (the password is right) creates a grant: a random 32-byte secret
and a row holding the private key encrypted under
`HKDF(secret, salt = server key, info = "modlight vault grant v1|<grant id>|<user id>")`.
The browser gets the cookie `ml_vault_<userId>` = `<grant id>.<secret>`
(httpOnly, Secure on https, SameSite=Lax, Path=/; Max-Age 30 days when
"Remember on this browser for 30 days" is ticked, which it is by default,
otherwise a session cookie and 24 hours on the server). A request carrying the
cookie is checked by opening the row's copy with the key derived from the
cookie: the AEAD tag is the check, the row's `expires_at`, `generation` and
existence are the rest. Neither the database alone, nor the cookie alone, nor
both without the server key can decrypt anything.

Grants are listed in Settings -> Security (kind, browser, address, created,
last used, expiry) and can be revoked one by one or all at once; changing the
file password drops every grant (the browser that changed it gets a new one).
An account keeps at most 20 owner/app grants and 200 visitor grants; the oldest
of the same kind goes first. Expired grants are deleted by the job.

Rotating `AUTH_SECRET` / `SETTINGS_ENCRYPTION_KEY` invalidates every grant
(people type the file password once more) and nothing else: the wrapped private
key does not depend on it.

## 4. Who must unlock, and what they see

Every file of an account whose vault is `encrypting` or `on` (including files
the background pass has not reached yet), for EVERYBODY: the owner, an
administrator, a visitor. A request may read the bytes only when it carries a
grant of that account (cookie `ml_vault_<ownerId>`, or the header
`X-Modlight-Unlock: <grant>` for API clients). While the vault is `decrypting`
(the owner switched it off with the password) nobody is asked any more.

| Surface | Locked answer |
|---|---|
| file page `/{tag}/{code}` (and tagless `/{code}`) | the unlock page ("This file is protected"), owner's skin, no file name |
| `/raw`, `/raw.<ext>`, `/download`, `/convert`, `/timeline.*`, thumbnails (`?w=`), moving tiles (`?motion=1`) | browser document: **401** with a small HTML unlock form; image/video/fetch: **401** `text/plain`; bearer API client: **423** JSON (section 6) |
| `/render` (sandboxed HTML/SVG/docx) | 401, never rendered |
| gallery `/u/{tag}`, album `/a/{tag}/{slug}` | the unlock page for the account; no `og:image` |
| album ZIP | 401 |
| short links `/s/...` | they redirect as before; the target asks |
| pastes | they are files: same as above |
| Open Graph, Twitter, oEmbed, `/api/link-info`, Discord cards | exactly what a password-protected file gets (product name only, oEmbed 404, `password-required`) |
| owner's library | names, sizes and folders as always; thumbnails show the type mark; a banner with the password field |

The unlock page and the HTML form post the password; on success the browser
gets the grant cookie and the page reloads. Wrong password: an error under the
field. Too many attempts: "Too many attempts, try again in N minutes".

**Rate limits** (fixed windows, in memory, like the file-password gate): 10
attempts per 15 minutes per address and account; 50 per 15 minutes per
account from visitors (spread over addresses); the owner's own attempts from
the panel have a bucket of their own (10 per 15 minutes), and so do the app's,
so strangers hammering a link cannot lock the owner out. At most 2 Argon2
derivations run at once in the process; more wait. Every refusal costs the
same as a wrong password, and an unknown link spends an attempt like a real
one. Audit entries: `vault.enabled`, `vault.disabled`,
`vault.password_changed`, `vault.unlocked` (the owner's browsers and apps;
visitors' unlocks live in the grant list instead of flooding the log),
`vault.unlock_failed` (the owner's and the app's wrong passwords; throttled
visitors once per quarter of an hour), `vault.grant_revoked`,
`vault.grants_revoked`.

Passwords: at least 8 characters, at most 1024 bytes, normalised to NFKC
before Argon2 (the same password typed on any keyboard), and never equal to
the account's sign-in password (checked against its hash when it is set).

## 5. Reading, writing, deriving

- **New uploads** (panel tus, ShareX, the desktop app, pastes, "convert" as a
  copy or in place) of an account whose vault is `encrypting` or `on` are
  encrypted with the public key before the row is written: no password
  needed, nothing changes for the clients. Metadata (type, dimensions,
  duration) is read from the plaintext before it is encrypted; the blurhash
  is not kept.
- **Reads** go through a storage view that decrypts exactly the objects the
  request was allowed to read (`vault::view`). Everything above the storage
  layer (Range, thumbnails, posters, timelines, motion tiles, conversions,
  document previews, album ZIPs, ffmpeg workspaces) works unchanged on the
  plaintext the view yields. A request without a grant never gets a view, so a
  forgotten gate fails closed (the bytes would be ciphertext).
- **Derived media is not kept** for these accounts: the derived cache
  (`derived/`) is never written for them, and nothing about them goes into a
  shared or browser cache (`Cache-Control: private, no-store`, `Vary: Cookie`).
- **Avatar from a library file**: needs the files unlocked; the avatar itself
  is public by nature.

## 6. API for bearer clients (the desktop app, scripts)

All with the account's own API token (`Authorization: Bearer` or
`x-api-key`, scope `read` or wider).

- `GET /api/encryption` -> `200 {"state": "off"|"encrypting"|"on"|"decrypting",
  "unlocked": bool, "progress": {"done", "total"}}`; `unlocked` reflects the
  request's `X-Modlight-Unlock` header (or the browser's cookie).
- `POST /api/encryption/unlock`, JSON `{"password": "...", "label"?: "app name"}`
  -> `200 {"grant": "<id>.<secret>", "expiresAt": "<ISO>"}` (30 days). Send it
  as `X-Modlight-Unlock: <grant>` on every request that reads file contents
  (`/raw`, `/download`, `/convert`, thumbnails, album ZIP). Errors:
  `400 {"error": "invalid_request"}`, `401 {"error": "wrong_password"}`,
  `409 {"error": "not_encrypted"}`, `429 {"error": "rate_limited",
  "retryAfter": <seconds>}` (+ `Retry-After`).
- `POST /api/encryption/lock` with `X-Modlight-Unlock` -> `204`: forget that
  grant (revokes it).
- A content request for a locked file with a bearer token answers
  **`423 Locked`**, `{"error": "files_locked", "message": "...", "owner": "<tag>"}`,
  header `X-Modlight-Locked: 1`. Not 401: the app reads 401 as "this token is
  no longer valid, sign in again". On `423` the app should ask for the file
  password, call `/api/encryption/unlock`, keep the grant in the Keychain and
  retry. A grant that expired or was revoked answers `423` again.
- Uploads (`POST /upload`, tus `/api/files`) need nothing new.

The browser equivalents are RPC actions of the module `encryption` (section 8)
and the HTML form `POST /api/encryption/unlock` with
`application/x-www-form-urlencoded` (`password`, `remember`, `next`), used by
the raw-link gate: `next` must be a same-origin path (`/{tag}/{code}/...`,
`/a/{tag}/...`, `/u/{tag}`; anything else is `400`), a cross-origin `Origin` is
`403`, success is `303` back to `next` with the cookie, a wrong password is the
form again (`401`) and a throttled one too (`429`). The form has no script and
answers with its own Content-Security-Policy (`default-src 'none';
style-src 'unsafe-inline'; form-action 'self'; ...`) also on a raw URL, whose
own policy would sandbox it (a sandboxed document cannot submit a form).

## 7. Switching on and off, the background pass

- **On**: the owner sets the file password twice, ticks "I understand that
  files cannot be recovered without this password", and the server refuses a
  password equal to the account's sign-in password or shorter than 8
  characters. The vault is created in state `encrypting` and this browser
  gets an owner grant. The background pass encrypts every existing file
  (trash included), 1 at a time, newest first; the Settings card shows "N of M
  encrypted". When none is left the state becomes `on`.
- **Off** needs the file password. State `decrypting`; the private key is
  kept in the secret box for the pass; the pass decrypts every file back; then
  the vault, its grants and the key are deleted.
- **Change password** needs the current one; the private key is re-wrapped
  under the new password (new salt), the generation goes up, every grant is
  revoked, this browser gets a new one. The files are not touched (the key
  pair stays; to replace it, switch encryption off and on again).

One file per step (rows read in batches of 32, newest first): write the new
object under a fresh key (registered in
`vault_cleanup` first), then in one statement point the row at it (only if the
row still points at the old object and is still in the old state), register
the old object for cleanup, delete it, unregister it. A crash at any point
leaves either the old or the new state of the row, both readable, plus at most
one object in `vault_cleanup`, which the next pass deletes. The pass runs at
boot, when switched on or off (immediately), and every 30 seconds while any
vault is `encrypting` or `decrypting`, holding a PostgreSQL advisory lock per
account so two server processes never work on one account. Files stay readable
throughout: a request reads the row it found, plaintext or not. A failing file
is retried on the next tick and the error is shown on the card.

Uploads that race with switching on or off are handled where the row is
written: the writer re-reads the vault inside its transaction; a plaintext row
written into a vault that is already `on` puts it back to `encrypting`.

## 8. Panel

- Settings -> Security: the "File encryption" card (off; on; encrypting with
  progress; decrypting with progress; locked/unlocked on this browser; change
  password; switch off; the list of browsers and apps that remember the
  password, with revoke buttons).
- Library: a banner while this browser has not unlocked, with the password
  field and "Remember on this browser for 30 days".
- Unlock page: lock icon, "This file is protected" (or "These files are
  protected" for galleries), password field, the remember checkbox (on by
  default), error on a wrong password; the owner's skin; English and Polish.
- Admin -> Users: an "Encrypted" badge; the user's files page: "This account's
  files are encrypted. Administrators cannot open them."
- RPC module `encryption`: `loadEncryptionStatus`, `enableEncryption`,
  `unlockMyFiles`, `lockMyFiles`, `changeFilePassword`, `disableEncryption`,
  `revokeVaultGrant`, `revokeAllVaultGrants`, `unlockOwnerFiles` (visitors:
  by file id or by tag).

## 9. Differences from the behaviour without the feature

Only for accounts with a vault: the gates above, `private, no-store` on
their content, no derived cache, no blurhash, no direct-media redirect for
bots, galleries without `og:image`, `uploads.storage_key` gains a random
segment when the pass rewrites an object (`tag/code-ab12cd34ef56.png`). For
everybody: the column `uploads.encryption` and three tables exist (the
TypeScript server ignores them; **switch encryption off on every account
before going back to the TypeScript server**, which would serve ciphertext).

## 10. What does not work while locked, limits, open questions

- **While a browser is locked** (by design): thumbnails, previews, the player,
  downloads, "Download as", conversions, the avatar picker on a library file
  and album ZIPs. Names, sizes, folders, renaming, moving, publishing,
  deleting, the trash, duplicates (by checksum) and uploads all work.
- In-place conversion and "convert as a copy" of an encrypted file work with
  the owner's browser unlocked (the result is stored encrypted); while locked
  they answer `forbidden` (the upstream dialog has no word for "locked").
- **The desktop app** (until it is adapted): uploads and the library list work
  unchanged; thumbnails and downloads of an encrypted account answer `423`
  (shown as missing pictures, never as a sign-out). It needs a prompt for the
  file password, `POST /api/encryption/unlock`, the grant in the Keychain and
  `X-Modlight-Unlock` on content requests (section 6).
- An account deleted with "keep the files" leaves orphaned encrypted files:
  the vault goes with the account, so nobody can ever read them; delete them
  from Admin -> Orphans.
- No key rotation beyond off and on again; no recovery key; no "forgot the
  file password" path (by design).
- Migration numbering: 0016 assumes the branch with 0015 is merged first or
  together; deploying 0016 alone and 0015 later would skip 0015 (drizzle
  applies only entries newer than the newest applied one). After merging,
  `0016_snapshot.json`'s `prevId` should become 0015's id and include its
  tables.
- Open: should the checksum of an encrypted file be keyed (HMAC under the
  server key) so a database copy cannot confirm known files? It would change
  what `/api/uploads/check`, the duplicates API and `/api/link-info` return
  for those files, so it is left as a decision.

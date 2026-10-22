# Modlight (server) audit - files, media, serving, data exfiltration and supply chain

Scope: `/Users/marcinasdfg/Desktop/modlight/upstream/Modlight` (the Next.js 16 server + the custom `server.ts`).
Method: static analysis only (reading the code, grep, `file`, `xxd`, own Python scripts in `_audyt/narzedzia`). Nothing from the repo was run.
Date: 2026-10-18. Commit: `a59d03c` (a single commit, author Nimfek).

A parallel auditor examines accounts/sign-in/API authorization; below only: files/media/serving (scope A) and sending data outside and the supply chain (scope B).

Short verdict: **no backdoor, no telemetry and no code that sends data to the author or to third parties was found.** The code is unusually carefully written with security in mind (type allow-lists, a sandbox, view gates). One confirmed logic error seriously weakens the "burn after N views" feature; the rest are Medium/Low matters.

---

## Most serious findings (list to fix)

### A1. Bypass of the view limit (maxViews / "burn after viewing") - High, confirmed
File: `app/[userCode]/[mediaCode]/[action]/route.ts:112-114, 150-158, 210-233` and `lib/media/serve.ts:95, 197-229`.

The view counter is incremented ONLY for a request considered "initial and not a preview":

```
const shouldCount = !privileged && isInitialRequest(rangeHeader) && !isPreviewFetch(req, reqUrl) && !botExempt;
```

- `isInitialRequest` returns `true` only when there is no `Range` header or `Range` starts with `bytes=0-` (`/^bytes=0-/`).
- `isPreviewFetch` returns `true` when `Sec-Fetch-Dest: image|video` or when the URL contains `w`/`h`/`motion`.
- `getMedia` (`serve.ts:95`) rejects the file only when `views >= maxViews`.

Effect: anyone who has a link to a file with a view limit can download its entire content without consuming a single view, in two trivial ways:
1. `GET /{tag}/{code}/raw` with the header `Range: bytes=1-` (any start != 0) -> `isInitialRequest=false` -> `shouldCount=false` -> `streamMedia` returns 206 with bytes 1..end, and `views` does not change. Repeatable indefinitely.
2. Embedding `<img src=".../raw.png">` or `<video src=".../raw">` -> the browser sets `Sec-Fetch-Dest: image|video` -> `isPreviewFetch=true` -> `shouldCount=false`. The full image/video can be seen any number of times.

Scenario: the sender sets `maxViews=1` ("for a one-time viewing") and sends the link. An attacker who intercepts the link downloads the whole content any number of times (bytes 1..end), and `views` stays 0, so the intended recipient still sees a working link, and the sender does not learn that the content was read. This also breaks the "burn" guarantee and its tamper-detection property. (Byte 0 can be obtained with a single counted request or with `Range: bytes=0-0`, which matches `^bytes=0-`.)

The same applies to `action === "download"` (the same `shouldCount` gate, lines 216-221).

The intended exclusions (player seek, grid tiles, unfurlers) are described in comments, but the side effect is that the limit cannot be enforced against an attacker. Fix: enforce the view budget as a hard gate on every request that serves the full bytes of a file with `maxViews != null` (e.g. count per (file, session/IP) with deduplication of ranges from the same session, or refuse 206 for files with a limit, or force counting also for `Range: bytes=N-` and `Sec-Fetch-Dest: image/video` when `maxViews != null`). Today the view exemptions are already narrowed to `maxViews == null` only for bots (`botExempt`); the same narrowing has to be applied to `isInitialRequest`/`isPreviewFetch`.

---

## Logic errors (behavior inconsistent with the intent of the code/comments)

### L1. The declared intent "only the first request counts" vs. the reality "no request has to count"
See A1. The comment at `isInitialRequest` says: "Only the opening request counts", but nothing forces the client to send the initial request, so for a file with `maxViews` the content is reachable without being counted. This is a mismatch between intent and implementation, hence also listed here. Severity as in A1 (High).

### L2. `oembed` does not check the view limit - Low, confirmed
File: `app/oembed/route.ts:49-52`. The endpoint refuses for `!published` and `passwordHash`, but does not check `maxViews`. It returns `filename` (as `title`) and `width/height` for a published file with a limit. It is not a real content leak (it returns only the URL/metadata, which are on the preview page anyway for a published file without a password), but it is inconsistent with `lib/media/embed.ts`, which deliberately does not emit `og:image` for files with a limit. The policy is worth aligning. It does not serve bytes and does not consume a view.

---

## Other findings (low / observations)

### N1. The link shortener is a deliberate open redirect (http/https) - Low/by-design
`lib/shortlinks.ts:105-133, 285-302`. `normalizeTarget` correctly rejects `javascript:`, `data:` and everything other than `http/https` (no open-redirect XSS via `javascript:`). The redirect `Response.redirect(link.target, 302)` to any http(s) address is the nature of a shortener; the password gate is honored before the target is read. Left as information, not a vulnerability.

### N2. CSP `script-src/style-src 'unsafe-inline'` across the whole application - Low/by-design
`next.config.ts:63-64`. A deliberate decision (no nonce, because `proxy.ts` cannot buffer bodies). User content goes under separate, very restrictive policies (`rawCsp`, `renderCsp`), so this does not create stored XSS from files. A separate policy `authorizeCsp` allows `connect-src http://127.0.0.1:*` only for `/authorize` (RFC 8252 loopback) and is correctly narrowed to that one path.

### N3. The `X-Real-IP` header as the only source of the IP for rate limiting - Low (operational)
`lib/password.ts:375-377`, `docker/Caddyfile:36`, `docker/nginx.example.conf:25`. If the operator deploys the application without a proxy that overwrites `X-Real-IP`, everyone shares one bucket (throttling too strict, not too lax); this is documented. If, however, the proxy let the client's `X-Real-IP` through, the limits can be bypassed. This is a deployment configuration matter, not a code error; the comments warn about it outright.

### N4. The deletion page is built by hand from HTML - observation, protected
`app/[userCode]/[mediaCode]/delete/[token]/route.ts:43-73`. The file name is injected into HTML, but all values go through `esc()` (`& < > " '`), and the response has a restrictive `Content-Security-Policy: default-src 'none'`. Authorization: the per-file `deleteToken` or an API token with scope `write` belonging to the owner; 404 instead of 403 (no oracle). Looks correct.

### N5. Avatar/background - keys in storage, no arbitrary file read - observation
`app/avatar/[userCode]/route.ts`, `app/bg/[userCode]/route.ts`, `lib/avatar.ts`, `lib/background.ts`. The key is taken from the column `users.image`/`backgroundKey` (not from the URL), validated by `isAvatarKey`/the prefix `backgrounds/`, and served with `nosniff` and a fixed `Content-Type: image/webp`. SVG is rejected when uploading a background (a comment about the SVG loader). No path traversal.

---

## Does the server exfiltrate data - the full list of outgoing connections and the verdict

The entire server and frontend code was searched: `fetch(`, `http/https.request`, `net/tls.connect`, `dns`, `nodemailer`, the S3 client, `WebSocket`, `sendBeacon`, `<script src>`/`<link>`/`<img>` from foreign domains, fonts from a CDN and every hard-coded URL/IP (`grep -rniE "https?://"`). Node_modules does not exist (as assumed).

Outgoing network connections initiated by the server:

| Location | Destination | What data | When | Does the administrator decide? |
|---|---|---|---|---|
| `lib/mail.ts` / `lib/mail-settings.ts` (nodemailer) | the SMTP server from the configuration (`SMTP_HOST`) | verification/reset e-mails to users | sending a mail | YES (configuration, off by default when `SMTP_HOST` is missing) |
| `lib/storage/s3.ts` (`@aws-sdk/client-s3`) | `S3_ENDPOINT` from the configuration | uploading/downloading users' files | when storage=s3 | YES |
| `migrate.ts`, `lib/db` (`pg`) | `DATABASE_URL` | application data | normal operation | YES |

Browser-side connections (frontend): only the own origin / relative paths: `components/profile/*` to `/api/avatar`, `/api/background`; `components/upload/*` to `/api/uploads/check` and tus (`/api/files`); `components/viewer/decrypt-transfer.ts` to its own `/raw` (`cache: no-store`). The only exception: `components/authorize/authorize-prompt.tsx` POSTs the token to `http://127.0.0.1:<port>` (local desktop, RFC 8252); this happens in the user's browser, to their own machine, and is explicitly fenced with a separate CSP.

Fonts: self-hosted (`next/font/local`, `geist`, `public/fonts/Geist-Bold.ttf`), the comment in `app/layout.tsx:2` "no Google Fonts fetch". No Google Fonts, no CDN, no analytics (Plausible/GA/Sentry/PostHog etc.), no beacons, no `NEXT_TELEMETRY` (indeed `NEXT_TELEMETRY_DISABLED=1` in the Dockerfile). Better Auth has a package `@better-auth/telemetry`, but no place was found in the code where it is enabled.

No SSRF: `app/api/link-info/route.ts` and `lib/link-info/parse.ts` do NOT fetch the address being described; they answer solely from a database row (`getMedia`/`SELECT`), and deliberately do not call `consumeView`, `streamMedia`, `redirectShortLink` or anything that touches the network/files. Nowhere in the server is there a `fetch`/`request` to an address coming from a user. No server-side "fetch a link preview".

**Verdict: the server sends no data to the author or to third parties. There is no telemetry, no beacons and no "phone home". The only network outputs are services configured by the administrator (SMTP, S3, Postgres).**

---

## Supply chain

- `package.json` / `pnpm-lock.yaml`: `lockfileVersion 9.0`, all 1149 entries have `resolution: {integrity: sha512-...}` from the npm registry. **No dependencies from git/tarball/URL, no local paths.** The package names were reviewed: the standard ecosystem (Next, React, radix-ui, aws-sdk, @tus/*, sharp, mammoth, pdf-lib, drizzle, better-auth, nodemailer, archiver, busboy). `@shopify/semaphore` and two `@esbuild-kit/*` are marked deprecated (usually informational), nothing suspicious.
- `pnpm-workspace.yaml`: `onlyBuiltDependencies` limited to `sharp` and `esbuild`; the install scripts of dev tooling (`unrs-resolver`, `@parcel/watcher`, `@swc/core`, `msw`) are explicitly on the `ignoredBuiltDependencies` list. An override `kysely: ^0.28.17` with a justification. No `.npmrc`/`.pnpmfile.cjs` (no hidden install hooks).
- `Dockerfile`: `pnpm install --frozen-lockfile`, a multi-stage build, runtime as `node:node` (uid 1000), `cap_drop: ALL`, `no-new-privileges`, cpu/mem/pids limits. ffmpeg from the Debian repositories (`apt-get`, `--no-install-recommends`), **not downloaded from the internet**. `entrypoint.sh` only computes the cgroup limits and starts the migrations + the server; it downloads and sends nothing.
- `docker-compose*.yml`: in production the application is exposed through `expose` (the docker network) or `127.0.0.1:${APP_PORT}` (the variant with your own proxy), not on 0.0.0.0. Postgres without a published port. The dev `docker-compose.yml` publishes Postgres on `5433` with the password `modlight` (only for `pnpm dev`, not production).
- `docker/Caddyfile`, `docker/nginx.example.conf`: proxies, they overwrite `X-Real-IP`, with no body limit (deliberate, for large uploads).
- `setup.sh`: an interactive `.env` generator; it asks from `/dev/tty` (comment: protection against `curl | bash`), generates secrets locally (`openssl rand` / `/dev/urandom`), `chmod 600 .env`. It downloads and sends nothing beyond the local `docker compose up`.
- `scripts/*`: dev/CI tools (Playwright screenshots against `localhost:8420`, a token mint, a backfill). None connects to a foreign address; `publish-release.ts` and `fix-mime-types.ts` work on the local database/storage.

## Obfuscated code / binary files

- A scan for invisible characters (`_audyt/narzedzia/nietypowe_znaki.py`): no BIDI/zero-width characters or Cyrillic in identifiers. The long lines are solely Tailwind classes in `components/ui/*`. Two "control characters" are literals in the code: `0x00` as a separator in `lib/media/convert-verdict.ts:82` (a cache key) and in `app/a/.../zip/route.ts:130,139` (a name-sanitizing regex), and `U+200B` in `components/viewer/TextView.tsx:141` (an empty-line placeholder). All harmless.
- No `eval`, `new Function`, dynamic `require(<variable>)`, `import(<url>)`. `child_process` only in `lib/media/ffmpeg.ts` (spawn of ffmpeg/ffprobe, `shell:false`) and `taskkill` on Windows. `i18n/request.ts` does `import(\`../messages/${locale}/...\`)` where `locale` has been narrowed beforehand to `'pl'|'en'`.
- Binary files (`_audyt/narzedzia/sprawdz_binarne.py` + `file`): `public/brand/*.png`, `app/apple-icon.png`, `public/fonts/Geist-Bold.ttf`, `docs/screenshots/*.webp` - all valid, with no data appended after `IEND`/beyond the declared RIFF/TTF size, no suspicious text chunks (only `Software: Adobe ImageReady` in `sharex.png`). `app/icon.svg`, `docs/logo.svg` - plain SVG with no script.

---

## Things checked and judged correct (scope A)

- **Path traversal / arbitrary read-write**: `LocalDriver.full()` (`lib/storage/local.ts:21-27`) enforces the root directory prefix; `deletePrefix` refuses an empty/root prefix. Storage keys are built from `userCode/code.ext`; `codeFromMediaCode` cuts at the first dot, and the lookup is by the `code` column. Derived cache: `derivedKey = derived/{uuid}/{hex}.{epoch}.{variant}.{ext}` where `variant`/`ext` come from an allow-list of targets (`pdf/png/jpg/webp/avif`). No user-controlled path.
- **Stored XSS (HTML/SVG/PDF/JS from the application's domain)**: `serveDecision` (`lib/media/mime.ts`) is an allow-list: HTML/SVG/XML/JSON go out as `text/plain; charset=utf-8` with `nosniff`. Real rendering of HTML/SVG/docx only through `/render` with: a signed HMAC ticket (`render-ticket.ts`), the requirement `Sec-Fetch-Dest: iframe`, the policy `Content-Security-Policy: sandbox [allow-scripts]` and a `sandbox=""` frame for SVG (intersection of flags). docx is rendered by mammoth in the same sandbox. Content decrypted in the browser goes through the same `serveDecision` (`decrypted-presentation.ts`), and the blob for saving gets `application/octet-stream`.
- **ffmpeg/ffprobe**: `shell:false`, `-protocol_whitelist file,pipe`, a forced `-f <container>` from an allow-list, `-use_absolute_path 0` for mov, an environment reduced to `PATH`, a kill of the whole process group after the deadline. The `?t=`/`?d=` parameters go through `seconds()` (finite 0..86400) and are passed as `toFixed(3)`/`String(<number>)`; `scale=`/`fps=` are constants. No argument injection.
- **Decompression bombs**: `lib/media/zip-guard.ts` actually inflates every entry through `inflateRawSync` with `maxOutputLength`, rejects ZIP64 and unknown methods; checked BEFORE mammoth.
- **sharp limits / transformation DoS**: image input is limited by `MAX_TRANSFORM_BYTES` (32 MB), pixels by `MAX_THUMBNAIL_PIXELS` (64 Mpx) checked from metadata BEFORE decoding, `sharp.concurrency` pinned to the cgroup, everything through a shared `imageTransforms` limiter with a weight and a timeout (`transform-queue`, `transform-cost`). A video poster is a seek, not a decode.
- **Bypass of the space limit / a lying size in tus**: `onUploadCreate` requires `Upload-Length` (rejects `Upload-Defer-Length`), checks `hasRoomFor` and the quota; `maxSize` is dynamic from the remaining quota; the quota reservation is atomic (`reserveQuota` with a predicate in the `WHERE`). ShareX: `ANON_MAX_BYTES` for unauthorized requests, `hasRoomFor` with `Content-Length` before writing.
- **Password-protected/expiring/one-time links**: unlock tokens are HMAC-SHA256 bound to scope+id+exp (`lib/password.ts`); the cookie is signed only; attempt rate limit (5/15 min) per IP+resource; expiry and password are also checked on `/raw`, `/render`, galleries and in ZIP. (Exception: enforcement of `maxViews`; see A1.)
- **Guessability of identifiers**: `lib/ids.ts` and `lib/naming.ts` use `crypto.randomInt`/`randomBytes` (CSPRNG), the minimum code length is enforced (RANDOM_LENGTH_MIN=5 over an alphabet of 56 characters).
- **Trash and restore**: operations are narrowed to the owner (or an admin with an audit entry), `purgeExpiredTrash` returns the quota, retired tags cannot be re-claimed by another account (`user_code_aliases`).
- **E2E encryption**: the key only in the URL fragment (`lib/crypto/link.ts`, `urlLeaksKey` as an assertion), AES-256-GCM, IV = an 8 B prefix (per file) || a 32-bit counter, refusal on counter overflow, AAD binds the frame to the file/position/last-frame flag, metadata in frame 0. The envelope format is consistent (`envelope.ts`).

Auxiliary audit tools: `_audyt/narzedzia/sprawdz_binarne.py`, `_audyt/narzedzia/nietypowe_znaki.py`.

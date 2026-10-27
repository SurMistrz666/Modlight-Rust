# Modlight in Rust: the server

This is the Modlight server rewritten from TypeScript (Next.js 16 + PostgreSQL 17,
the original in `upstream/Modlight`) to Rust, 1:1. Modlight is a self-hosted file
and screenshot hosting service: a web panel (library, folders, trash, duplicates, short
links, pastes, profile, tokens, administration), uploads through ShareX
(`POST /upload`) and through tus (resumable uploads from the panel and from the app),
public file pages, galleries and albums, previews for Discord and other
messengers, and image, audio and video conversions.

The same URLs, the same responses, the same database as the original:
the web panel, ShareX and the Modlight app cannot tell the servers apart. The exceptions are deliberate
and described in `docs/SKELETON.md`, section 9 (the security fixes from the audit
in `docs/audit/` and a few errors of the original that we do not repeat).

Documents for developers: `docs/PORT.md` (the port's rules), `docs/SKELETON.md`
(how the server is built, deviations), `docs/CONTRACT.md` (the contract with the web panel),
`docs/COVERAGE.md` (what from the original is where and how it was checked).

## Local stack in Docker

Everything runs on this Mac only. The panel is at **http://127.0.0.1:8430**,
and the port is open only on the loopback address (127.0.0.1): it cannot be reached
from another computer or from the internet. The database has no
port exposed to the outside.

Docker is Colima (the profile `modlight`). Run all the commands below
from the repository directory (`~/Desktop/modlight`):

```sh
colima start --profile modlight          # if Docker is not running (e.g. after restarting the Mac)
```

### First run

```sh
server/docker/setup-local.sh                                   # creates server/docker/.env (random passwords)
docker compose -f server/docker/docker-compose.yml up -d --build
```

`setup-local.sh` writes `server/docker/.env` with mode 600: the key that
signs sessions (`AUTH_SECRET`), the key for secrets in the database
(`SETTINGS_ENCRYPTION_KEY`), the database password (`POSTGRES_PASSWORD`) and
the token build machines publish the desktop app with (`RELEASE_PUBLISH_TOKEN`,
see "Downloads" below). The file does not go
into git. **Do not change these values on a running installation** (it would sign
everyone out, the SMTP password in the database would become unreadable, and the application would
lose access to the database). The script can be run again: it keeps existing secrets
and saves the old version of the file next to it as a copy.

Then open http://127.0.0.1:8430/register and create an account. **The first account
on an empty installation becomes the administrator** (registration for others is
disabled; you enable it in Admin → Ustawienia (Admin → Settings), and you can also create accounts yourself
in Admin → Użytkownicy (Admin → Users)).

### Day-to-day operation

```sh
F=server/docker/docker-compose.yml
docker compose -f $F ps                 # status (app should be "healthy")
docker compose -f $F logs -f app        # the server's log
docker compose -f $F stop               # stop (the data stays)
docker compose -f $F start              # start again
docker compose -f $F restart app        # restart just the server
docker compose -f $F down               # remove the containers (the data in volumes stays)
```

The containers have `restart: unless-stopped`, so after Colima starts they come up by themselves,
unless you stopped them.

### Updating

After changes in the code (or after merging a new version into `main`):

```sh
docker compose -f server/docker/docker-compose.yml up -d --build
```

This rebuilds the image and replaces the application container. Database migrations run
by themselves on every start of the application (first the configuration check, then
the migrations, then the server). The first build takes about 10 minutes (libvips and its
libraries are compiled from source); later ones are fast thanks to Docker's
cache.

### Where the data is

| What | Where |
|---|---|
| Uploaded files, thumbnails, avatars, wallpapers, files being uploaded, the desktop app's builds (Downloads) | the Docker volume `modlight-rust_appdata` (in the container `/app/data`) |
| The PostgreSQL 17 database (accounts, files, settings) | the Docker volume `modlight-rust_pgdata` |
| Secrets and configuration | `server/docker/.env` (mode 600) |

The volumes live in the Colima virtual machine (the profile `modlight`). `docker compose
down` does not remove them; only `down -v` or `colima delete` would.

### Backups

The database (a consistent dump while the server is running):

```sh
mkdir -p ~/ModlightBackup
docker compose -f server/docker/docker-compose.yml exec -T postgres \
  pg_dump -U modlight -Fc modlight > ~/ModlightBackup/modlight-$(date +%F).dump
```

The files (the `appdata` volume, packed with the application image, which has `tar`):

```sh
docker run --rm --entrypoint tar -v modlight-rust_appdata:/data:ro -v ~/ModlightBackup:/backup \
  modlight-rust:local -czf /backup/appdata-$(date +%F).tar.gz -C /data .
```

Also keep a copy of `server/docker/.env` (without it the encrypted settings, e.g. the
SMTP password, and the sessions cannot be restored). Keep it in a safe place.

Restoring (with the application stopped; in the dump file names `RRRR-MM-DD` is the date as year-month-day, i.e. YYYY-MM-DD):

```sh
F=server/docker/docker-compose.yml
docker compose -f $F stop app
docker compose -f $F exec -T postgres pg_restore -U modlight -d modlight --clean --if-exists < ~/ModlightBackup/modlight-RRRR-MM-DD.dump
docker run --rm --entrypoint tar -v modlight-rust_appdata:/data -v ~/ModlightBackup:/backup \
  modlight-rust:local -xzf /backup/appdata-RRRR-MM-DD.tar.gz -C /data
docker compose -f $F start app
```

The database is compatible with the original in both directions: a dump from the original Modlight
(Next.js) can be restored here and vice versa.

### Ports

| Port | What | Access |
|---|---|---|
| 127.0.0.1:8430 | web panel, API, ShareX, tus | only from this Mac |
| (none) | PostgreSQL | only from the application container |

The port can be changed in `server/docker/.env` (`APP_PORT` and `APP_URL` together), then
`up -d`.

## ShareX

ShareX runs only on Windows, and the server listens only on 127.0.0.1 of this Mac,
so ShareX from another computer will not connect until the server is
made available on the network (that requires a reverse proxy with HTTPS and is not done).
The configuration, when it is needed:

1. Panel → **Integracje** (Integrations) (`/dashboard/tokens`) → create a token.
2. „Pobierz konfigurację ShareX” ("Download ShareX config") (or „Konfiguracja ShareX (.sxcu)” ("ShareX config (.sxcu)") in the window
   of a new token) saves a ready `.sxcu` file; open it by double-clicking in ShareX.
   Manually: upload with `POST <address>/upload`, the file field `upload`, the header
   `x-api-key: <token>`; the file address in the response is `{json:url}`, the deletion
   address `{json:deletionUrl}`.

On this Mac you can check the same contract with the command:

```sh
curl -F upload=@zrzut.png -H "x-api-key: <token>" http://127.0.0.1:8430/upload
```

## The Modlight app for Mac

The app (`app/` in this repository) connects to this server just as it does
to the original:

1. In the sign-in window enter the server address **http://127.0.0.1:8430**.
2. Signing in through the browser opens the panel on the page `/authorize`;
   approve the device there (the app receives the token through a local
   loopback port). Or paste a token created in the panel under **Integracje** (Integrations).
3. The token goes into the macOS Keychain (the service `Modlight`).

## Downloads (Do pobrania): the desktop app's builds

A novelty the original does not have. The server keeps the builds of the
desktop app for **macOS** and **Windows**: every signed-in account downloads
them from the panel (**Do pobrania** / **Downloads** in the left panel), build
machines publish them with a token, and the desktop app checks the same list
for updates. The newest **5** builds of each platform and architecture stay;
publishing a sixth deletes the oldest (row and file).

**The publish token.** `setup-local.sh` writes a random `RELEASE_PUBLISH_TOKEN`
into `server/docker/.env` (an existing one is kept); `up -d` hands it to the
server. Copy it from that file to the build machines; it is never printed.
It opens only the three publish routes below (anywhere else it is an unknown
token: 401). Without it (empty or unset), only administrators publish: with
their session in the panel or with an API token of scope `full`. To replace it,
put a new value in `.env` and `docker compose -f server/docker/docker-compose.yml up -d`;
the build machines then need the new value. The server keeps only its SHA-256
and compares in constant time.

**The API** (errors are JSON `{"error", "code"}`; 401 without valid
credentials, 403 for an account that may not do it):

| Route | Who | What |
|---|---|---|
| `GET /api/downloads` | signed in (session cookie, or the API token the desktop app uses, any scope) | `{"platforms":[{"platform":"macos","builds":[...]},{"platform":"windows","builds":[...]}],"keep":5,"admin":false,"publishToken":null}`, builds newest first; `publishToken` (is the token set) only for administrators |
| `GET /downloads/{id}/{filename}` | signed in | the file: attachment, `Content-Length`, `ETag` = the quoted SHA-256, `Range` |
| `POST /api/downloads/upload` | publish token or administrator | JSON `{"platform","arch","version","build","built","size","sha256","filename","note"}` -> `{"upload":"<id>","chunkSize":52428800}` |
| `PUT /api/downloads/upload/{upload}/{n}` | publish token or administrator | the raw bytes of chunk `n` (0, 1, 2, ...), each at most `chunkSize` |
| `POST /api/downloads/upload/{upload}/finish` | publish token or administrator | the build; a wrong size or SHA-256: 400, and the upload is dropped |
| `DELETE /api/downloads/{id}` | administrator | deletes the build and its file |

A build is `{"id","platform","arch","version","build","built","published","size","sha256","filename","uploader","note","url"}`
with `url` = `/downloads/{id}/{filename}`. `platform` is `macos` or `windows`,
`arch` `arm64` or `x64` (`aarch64`, `x86_64` and `amd64` are accepted; empty
means `arm64` on macOS, `x64` on Windows), `built` an ISO 8601 date-time or
milliseconds since 1970 (empty: now), `size` at most 4 GiB, `filename` letters,
digits, `.`, `_`, `+` and `-` only (the build machines send
`Modlight-<version>-<build>-macos-arm64.zip` and
`Modlight-<version>-<build>-windows-x64.zip`), `note` up to 2000 characters.
Chunks may be smaller than `chunkSize` (50 MiB, half of what Cloudflare lets
through in one request); sending a chunk again replaces it; `finish` joins
them in order and checks the total size and the SHA-256. A repeated `finish`
(its answer got lost) answers the same build. An upload nobody touches for a
day is dropped. Publishing and deleting are written to the admin audit log
(`downloads.published`, `downloads.pruned`, `downloads.deleted`,
`downloads.rejected`).

**Publishing from a build machine** (POSIX shell with curl; the panel's
Downloads page shows the same script with this server's address filled in):

```sh
SERVER=https://files.example.com
TOKEN=...                                   # RELEASE_PUBLISH_TOKEN
FILE=Modlight-1.2.0-45-macos-arm64.zip
SIZE=$(wc -c < "$FILE" | tr -d " ")
SHA=$( (shasum -a 256 "$FILE" 2>/dev/null || sha256sum "$FILE") | cut -d" " -f1)
UPLOAD=$(curl -fsS -X POST "$SERVER/api/downloads/upload" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  --data "{\"platform\":\"macos\",\"arch\":\"arm64\",\"version\":\"1.2.0\",\"build\":\"45\",\"size\":$SIZE,\"sha256\":\"$SHA\",\"filename\":\"$FILE\",\"note\":\"\"}" \
  | sed -n 's/.*"upload":"\([0-9a-f]*\)".*/\1/p')
CHUNK=52428800; N=0
while [ $((N * CHUNK)) -lt "$SIZE" ]; do
  dd if="$FILE" bs=$CHUNK skip=$N count=1 2>/dev/null |
    curl -fsS -X PUT "$SERVER/api/downloads/upload/$UPLOAD/$N" \
      -H "Authorization: Bearer $TOKEN" --data-binary @- > /dev/null
  N=$((N + 1))
done
curl -fsS -X POST "$SERVER/api/downloads/upload/$UPLOAD/finish" -H "Authorization: Bearer $TOKEN"
```

**Where the files are.** In the server's storage under
`releases/<id>/<filename>` (the `appdata` volume on the local driver, the
bucket on S3: both go through the same storage layer), and the list in the
table `releases` (migration 0015). Chunks of an unfinished upload wait in
`UPLOAD_TMP_PATH/releases/` on the server's own disk, which needs room for
twice the build while it is being joined. The account tags `downloads` and
`releases` are reserved from now on (the first is the download path, the
second the storage folder). Details: `docs/SKELETON.md`, section 9, item 45.

## Cloudflare Turnstile (checking that the visitor is a human)

Optional and off by default: without keys everything works exactly as in
the original. Once enabled, the sign-in, registration, password reset request and
invitation acceptance forms show the Cloudflare widget, and the server verifies
every token with Cloudflare before it does anything. When Cloudflare does not respond,
the server refuses (better to let nobody in for a moment than to let bots in).

**Keys from the Cloudflare dashboard**

1. Sign in at https://dash.cloudflare.com and open **Turnstile**
   (in the side menu), then **Add widget**.
2. Any name (e.g. `Modlight`). In **Hostname management** add every
   hostname under which you open the panel: the host from `APP_URL` and each one from
   `APP_DOMAINS` (just the name, without `https://` and without a port, e.g.
   `pliki.example.com`). On a different host the widget will not appear.
3. **Widget Mode**: leave **Managed**. Pre-clearance: no.
4. **Create**: Cloudflare will show the **Site Key** (the site key, public) and the
   **Secret Key** (the secret key). Copy both.

**Where to enter them** (one of two):

- The panel: **Administracja → Ustawienia → Cloudflare Turnstile** (Administration → Settings → Cloudflare Turnstile): the site key,
  the secret key, the switch „Chroń formularze logowania” ("Protect the sign-in forms"), **Zapisz** (Save). Saving
  with the switch on first asks Cloudflare whether it accepts the secret
  key (a wrong key would block every sign-in, including yours); the button
  **Sprawdź** (Test) does the same without saving. The secret key is stored
  encrypted and never goes back to the browser.
- Or `server/docker/.env`: `TURNSTILE_SITE_KEY=...` and
  `TURNSTILE_SECRET_KEY=...` (both, otherwise it stays off), then
  `docker compose -f server/docker/docker-compose.yml up -d`. If the card
  in the panel was ever saved, the panel takes precedence entirely; the button
  „Wróć do ustawień środowiskowych” ("Use the environment settings") hands the decision back to the `.env` file.

**Trying it locally, without a Cloudflare account**: Cloudflare's test keys work
on any host, including 127.0.0.1. The site key `1x00000000000000000000AA`
with the secret key `1x0000000000000000000000000000000AA` lets everyone through,
the secret key `2x0000000000000000000000000000000AA` rejects everyone. The widget
then shows the text "For testing only". For real use, only the keys of
your own widget.

Before you sign out after enabling it, sign in once in a private window. If
signing in does not work (e.g. the host does not match the widget), turn it off from the database:

```sh
docker compose -f server/docker/docker-compose.yml exec postgres \
  psql -U modlight -d modlight -c "UPDATE settings SET value = 'off' WHERE key = 'turnstile_enabled'"
```

The change takes effect within 30 seconds. When Turnstile is enabled, the browser
fetches a script from `challenges.cloudflare.com`, and the server sends there the token
from the form and the visitor's IP address (more: `docs/SKELETON.md`, section 9,
item 43).

## What is different from the original (the most important)

The full list: `docs/SKELETON.md`, section 9. In short:

- **Security fixes from the audit**: the view limit cannot be bypassed
  with the `Range` header or `Sec-Fetch-Dest`; ShareX checks the token before it
  accepts a file; the first administrator account is decided in a transaction;
  the delete token is compared in constant time; `/oembed` does not reveal files
  with a view limit; the `X-Real-IP` header only with `TRUST_PROXY`.
- **Fixes from the integration**: a finished tus upload can be neither downloaded nor
  deleted at its tus address (with S3 this bypassed the file's privacy);
  changing the avatar or wallpaper deletes the old file (the original left it forever).
- The ZIP album works (in the original it was always a 500 error).
- A novelty the original does not have: optional Cloudflare Turnstile on the
  sign-in, registration, password reset and invitation forms (above).
- Another one: Downloads, the desktop app's builds for macOS and Windows,
  published by build machines with `RELEASE_PUBLISH_TOKEN` (above); the tags
  `downloads` and `releases` are reserved for it.
- A bad `%` character in the address: 400 instead of 500.

## Tests

```sh
cd server
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The tests with a database run when `MODLIGHT_TEST_DATABASE_URL` is set
(see `docs/SKELETON.md`, section 1). The comparisons with the original
(`tests/parity`, `tests/parity_media`) and the Playwright tests from the original
(`tests/e2e`) are run on separate, temporary stacks,
never on the one from port 8430; how is described in `docs/handoff/srv-integration.md`
in the repository directory.

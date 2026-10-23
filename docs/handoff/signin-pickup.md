# Handoff: desktop sign-in pickup (branch `signin-pickup`)

Why: the owner's browser (Safari; Chrome behaves the same once its local-network
permission was dismissed) never delivers the /authorize page's POST from
https://files.example.com to the app's `http://127.0.0.1:<port>/` listener, so the
browser sign-in hung until the manual key. The app now ALSO collects the outcome
from the server (PKCE-style). Design and contract: `server/docs/SKELETON.md` 9.44.

## Done (committed, never pushed)

- Server: `authorize/pickup.rs` (store), `routes/api/authorize_pickup.rs`
  (`POST /api/authorize/pickup`), `challenge` in `authorize/loopback.rs` (parsed,
  carried through `/login?next=`), the page loader registers pending entries,
  `approveDesktopClient` stores the token, new `denyDesktopClient` stores the refusal.
- SPA: `src/overrides/components/authorize/authorize-prompt.tsx` (port of the
  upstream prompt), `src/rpc/authorize-pickup.ts`, `messages/*/authorizePickup.json`
  (en, pl), fixture `auth-authorize-pickup`, `tests/authorize-pickup.test.tsx`.
- App: `modlight-api/src/signin.rs` polls the pickup every 2 s beside the loopback;
  first answer wins. UI unchanged (`modlight-app/src/signin.rs` calls the same function).
- Docs: SKELETON 9.44, `app/docs/MACOS.md`, `docs/audit/rust-app.md` item 5,
  `docs/audit/rust-server.md` loopback item.

## Verified

- server: `cargo fmt`, `cargo clippy --all-targets -D warnings`, `cargo test`.
- SPA: `npm run verify` (74 tests, bundle audit, sync:check).
- app: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace`: all green except `tests/reference_server.rs`, which needs
  the reference server on 127.0.0.1:8420 (down: the colima VM `modlight` was stopped).
- App sign-in code headless against a stand-in server speaking the pickup contract
  (`tests/signin_pickup_fake.rs`): signed in through the pickup alone (no loopback
  POST), refusal through the pickup, cancel stops polling, an older server (404)
  still signs in through the loopback.

## Not done: the real-server e2e run

`app/crates/modlight-api/tests/signin_pickup.rs` is written and compiles, but was
not run: Docker (colima profile `modlight`) was stopped, so there was no Postgres
for a server of this branch, and it was not started because the same VM runs the
owner's stacks. To run it:

```sh
colima start modlight                       # the owner's call: also starts his stacks
S=<scratch dir>; DB=modlight_signin_pickup
set -a; source ~/Desktop/modlight/server/docker/.env.devdb; set +a
docker exec modlight-devdb-postgres-1 psql -U modlight -d modlight -c "create database $DB"
cd ~/Desktop/modlight/_wt/signin-pickup/server && (cd web && npm run build)
DATABASE_URL="postgresql://modlight:${POSTGRES_PASSWORD}@127.0.0.1:55421/$DB" \
  PORT=8461 APP_URL=http://127.0.0.1:8461 MODLIGHT_ROOT=$PWD \
  STORAGE_LOCAL_PATH=$S/storage TUS_TMP_PATH=$S/tus UPLOAD_TMP_PATH=$S/tmp cargo run -- serve
# first account on an empty database = admin:
curl -s -H 'origin: http://127.0.0.1:8461' -H 'content-type: application/json' \
  -d '{"email":"e2e@example.com","password":"<pick one>","name":"e2e"}' \
  http://127.0.0.1:8461/api/auth/sign-up/email >/dev/null
printf 'HOST=http://127.0.0.1:8461\nEMAIL=e2e@example.com\nPASSWORD=<same>\n' > $S/e2e.env
cd ../app && MODLIGHT_PICKUP_E2E=$S/e2e.env cargo test -p modlight-api --test signin_pickup -- --nocapture
```

It signs in with a real session, loads the authorize page payload (registers the
pending entry), approves through `/_rpc/authorize/approveDesktopClient` and does
NOT post to the loopback: the app must be signed in within 5 s through the pickup.
It also checks the loopback path and a refusal through `denyDesktopClient`.

## Production

files.example.com needs this server build for the pickup; until then the new app
behaves exactly like the old one there (its polls get 404). An old app against
the new server works as before (no challenge, no pickup).

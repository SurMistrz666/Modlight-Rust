#!/usr/bin/env bash
# Run the Rust server on the SAME database and with the SAME AUTH_SECRET as an
# original Modlight stack, for the cross-compatibility proof
# (tests/parity/crosscompat.py): what one implementation writes, the other
# must accept.
#
#   server/tests/parity/crosscompat-rust.sh <stack-dir> [port] [scratch-dir]
#
# <stack-dir> holds the stack's docker-compose.yml and .env (POSTGRES_PASSWORD,
# AUTH_SECRET; read here, never printed). The stack's Postgres must be
# published on 127.0.0.1:${PG_PORT:-55431} and its container be named
# ${PG_CONTAINER:-modlight-srvauth-x-postgres-1}.
#
# The environment mirrors the stack's app service (production, TRUST_PROXY
# off, registration seed off, MAIL_FROM no-reply@localhost), except APP_URL,
# which is this server's own origin. Storage lives under <scratch-dir>/data.
#
# Before starting, the schema (pg_dump --schema-only) and the drizzle
# bookkeeping are saved to <scratch-dir>/schema-before-rust.sql: `serve`
# runs the migrator first, and on a database the original already migrated it
# must change nothing (crosscompat.py compares the dump again).
set -euo pipefail

STACK="${1:?stack dir (docker-compose.yml + .env)}"
PORT="${2:-8439}"
SCRATCH="${3:-$(dirname "$STACK")}"
PG_PORT="${PG_PORT:-55431}"
PG_CONTAINER="${PG_CONTAINER:-modlight-srvauth-x-postgres-1}"

SERVER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export PATH="/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH"
export DOCKER_CONTEXT="${DOCKER_CONTEXT:-colima-modlight}"

if [[ ! -f "$STACK/.env" ]]; then
  echo "missing $STACK/.env" >&2
  exit 1
fi
POSTGRES_PASSWORD="$(sed -n 's/^POSTGRES_PASSWORD=//p' "$STACK/.env" | tr -d '"'"'"'')"
AUTH_SECRET="$(sed -n 's/^AUTH_SECRET=//p' "$STACK/.env" | tr -d '"'"'"'')"
if [[ -z "$POSTGRES_PASSWORD" || -z "$AUTH_SECRET" ]]; then
  echo "POSTGRES_PASSWORD or AUTH_SECRET missing in $STACK/.env" >&2
  exit 1
fi
export AUTH_SECRET

mkdir -p "$SCRATCH"
docker exec "$PG_CONTAINER" sh -c \
  'pg_dump -U modlight --schema-only modlight; psql -U modlight -d modlight -Atc "select id, hash, created_at from drizzle.__drizzle_migrations order by id"' \
  > "$SCRATCH/schema-before-rust.sql"

export NODE_ENV=production
export TRUST_PROXY=false
export PORT
export BIND_ADDR=127.0.0.1
export APP_URL="http://localhost:$PORT"
export APP_NAME=Modlight
export REGISTRATION_ENABLED=false
export STORAGE_DRIVER=local
export MAX_UPLOAD_SIZE=0
export TUS_CHUNK_SIZE=52428800
export DEFAULT_USER_QUOTA=-1
export MAIL_FROM=no-reply@localhost
export DATABASE_URL="postgresql://modlight:${POSTGRES_PASSWORD}@127.0.0.1:${PG_PORT}/modlight"
unset POSTGRES_PASSWORD
export STORAGE_LOCAL_PATH="$SCRATCH/data/storage"
export TUS_TMP_PATH="$SCRATCH/data/tus"
export UPLOAD_TMP_PATH="$SCRATCH/data/tmp"
export MODLIGHT_ROOT="$SERVER_DIR"
export RUST_LOG="${RUST_LOG:-info}"
mkdir -p "$STORAGE_LOCAL_PATH" "$TUS_TMP_PATH" "$UPLOAD_TMP_PATH"

cd "$SERVER_DIR"
cargo build --quiet
exec "$SERVER_DIR/target/debug/modlight-server" serve

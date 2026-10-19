#!/usr/bin/env bash
# Run the Rust server the way parity checks expect (tests/parity/parity.py):
# production mode, APP_URL = http://localhost:<port>, TRUST_PROXY=true (so
# X-Real-IP counts, as it always did upstream), on its own database of the
# shared dev Postgres (127.0.0.1:55421), storage under <scratch>/data.
#
#   server/scripts/parity-server.sh <port> <database> <scratch-dir> [migrate|serve]
#
# AUTH_SECRET is generated once into <scratch-dir>/rust.env (mode 600) and
# never printed. Extra variables: put them in <scratch-dir>/rust.env too
# (e.g. SMTP_*), or export them before calling.
set -euo pipefail

PORT="${1:?port}"
DB="${2:?database name}"
SCRATCH="${3:?scratch dir}"
MODE="${4:-serve}"

SERVER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMMON_GIT_DIR="$(git -C "$SERVER_DIR" rev-parse --path-format=absolute --git-common-dir)"
MAIN_ROOT="$(dirname "$COMMON_GIT_DIR")"
export PATH="/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH"
export DOCKER_CONTEXT="${DOCKER_CONTEXT:-colima-modlight}"

set -a
# shellcheck disable=SC1091
source "$MAIN_ROOT/server/docker/.env.devdb"
set +a

mkdir -p "$SCRATCH"
if [[ ! -f "$SCRATCH/rust.env" ]]; then
  (umask 077; printf 'AUTH_SECRET=%s\n' "$(openssl rand -hex 32)" > "$SCRATCH/rust.env")
fi
set -a
# shellcheck disable=SC1091
source "$SCRATCH/rust.env"
set +a

if ! docker exec modlight-devdb-postgres-1 psql -U modlight -d postgres -Atc "select 1 from pg_database where datname='$DB'" | grep -q 1; then
  docker exec modlight-devdb-postgres-1 createdb -U modlight "$DB"
fi

export NODE_ENV=production
export TRUST_PROXY=true
export PORT
export BIND_ADDR=127.0.0.1
export APP_URL="${APP_URL:-http://localhost:$PORT}"
export DATABASE_URL="postgresql://modlight:${POSTGRES_PASSWORD}@127.0.0.1:55421/$DB"
export STORAGE_LOCAL_PATH="$SCRATCH/data/storage"
export TUS_TMP_PATH="$SCRATCH/data/tus"
export UPLOAD_TMP_PATH="$SCRATCH/data/tmp"
export MODLIGHT_ROOT="$SERVER_DIR"
export RUST_LOG="${RUST_LOG:-info}"
mkdir -p "$STORAGE_LOCAL_PATH" "$TUS_TMP_PATH" "$UPLOAD_TMP_PATH"

cd "$SERVER_DIR"
cargo build --quiet
exec "$SERVER_DIR/target/debug/modlight-server" "$MODE"

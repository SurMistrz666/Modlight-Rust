#!/usr/bin/env bash
# Start the development database and run the Rust server on 127.0.0.1:8421.
#
#   server/scripts/dev.sh            # migrate, then serve
#   server/scripts/dev.sh migrate    # only migrate
#   server/scripts/dev.sh db         # only start the database and print its URL
#
# The database password is generated once into server/docker/.env.devdb of
# the main checkout (gitignored, shared by every worktree) and never printed. Extra variables for the server can go into
# server/.env (also gitignored); variables already set in the shell win.
set -euo pipefail

SERVER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# One database volume serves every git worktree, so they must all use the same
# password: the file lives in the MAIN checkout's server/docker/ (found through
# the shared git directory), whichever worktree runs this script.
COMMON_GIT_DIR="$(git -C "$SERVER_DIR" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
if [[ -n "$COMMON_GIT_DIR" ]]; then
  MAIN_ROOT="$(dirname "$COMMON_GIT_DIR")"
else
  MAIN_ROOT="$(dirname "$SERVER_DIR")"
fi
ENV_FILE="$MAIN_ROOT/server/docker/.env.devdb"
mkdir -p "$(dirname "$ENV_FILE")"
COMPOSE=(docker compose -f "$SERVER_DIR/docker/dev-db.compose.yml" --env-file "$ENV_FILE")

export PATH="/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH"
export DOCKER_CONTEXT="${DOCKER_CONTEXT:-colima-modlight}"

if [[ ! -f "$ENV_FILE" ]]; then
  umask 077
  printf 'POSTGRES_PASSWORD=%s\n' "$(openssl rand -hex 24)" > "$ENV_FILE"
  echo "generated $ENV_FILE"
fi
# shellcheck disable=SC1090
source "$ENV_FILE"

"${COMPOSE[@]}" up -d --wait >/dev/null
echo "dev database ready on 127.0.0.1:55421"

export DATABASE_URL="postgresql://modlight:${POSTGRES_PASSWORD}@127.0.0.1:55421/modlight"
export PORT="${PORT:-8421}"
export BIND_ADDR="${BIND_ADDR:-127.0.0.1}"
export APP_URL="${APP_URL:-http://127.0.0.1:${PORT}}"
export STORAGE_LOCAL_PATH="${STORAGE_LOCAL_PATH:-$SERVER_DIR/data/storage}"
export TUS_TMP_PATH="${TUS_TMP_PATH:-$SERVER_DIR/data/tus}"
export UPLOAD_TMP_PATH="${UPLOAD_TMP_PATH:-$SERVER_DIR/data/tmp}"
export MODLIGHT_ROOT="${MODLIGHT_ROOT:-$SERVER_DIR}"

cd "$SERVER_DIR"
case "${1:-serve}" in
  db) echo "password file: $ENV_FILE (DATABASE_URL=postgresql://modlight:\$POSTGRES_PASSWORD@127.0.0.1:55421/modlight)" ;;
  migrate) exec cargo run --quiet -- migrate ;;
  serve) exec cargo run --quiet -- serve ;;
  *) echo "usage: $0 [serve|migrate|db]" >&2; exit 2 ;;
esac

#!/bin/sh
# Writes server/docker/.env for the local Docker stack (docker-compose.yml in
# this directory): random secrets, the panel on http://127.0.0.1:8430.
#
# Run again at any time: secrets that already exist are KEPT (changing
# AUTH_SECRET or POSTGRES_PASSWORD on a running install would sign everybody
# out, orphan the stored SMTP password, or lock the app out of its database).
# Prints no secret. The file is mode 600 and ignored by git.
#
#   server/docker/setup-local.sh
#   docker compose -f server/docker/docker-compose.yml up -d --build
set -eu

DIR=$(cd "$(dirname "$0")" && pwd)
ENV_FILE="$DIR/.env"

rand_b64() { openssl rand -base64 "$1" 2>/dev/null || head -c "$1" /dev/urandom | base64 | tr -d '\n'; }
rand_hex() { openssl rand -hex "$1" 2>/dev/null || head -c "$1" /dev/urandom | od -An -tx1 | tr -d ' \n'; }

# The current value of a variable in the existing file (no shell evaluation).
existing() {
  [ -f "$ENV_FILE" ] || return 0
  sed -n "s/^$1=//p" "$ENV_FILE" | tail -n 1
}

AUTH_SECRET=$(existing AUTH_SECRET)
case "$AUTH_SECRET" in ""|change-me*) AUTH_SECRET=$(rand_b64 32); AUTH_NEW=1 ;; *) AUTH_NEW=0 ;; esac
SETTINGS_ENCRYPTION_KEY=$(existing SETTINGS_ENCRYPTION_KEY)
if [ -z "$SETTINGS_ENCRYPTION_KEY" ] && [ "$AUTH_NEW" = 1 ]; then
  SETTINGS_ENCRYPTION_KEY=$(rand_b64 32)
fi
# Hex, not base64: the password goes into a URL, and base64's "/" breaks it.
POSTGRES_PASSWORD=$(existing POSTGRES_PASSWORD)
[ -n "$POSTGRES_PASSWORD" ] || POSTGRES_PASSWORD=$(rand_hex 24)

APP_PORT=$(existing APP_PORT)
[ -n "$APP_PORT" ] || APP_PORT=8430
APP_URL=$(existing APP_URL)
[ -n "$APP_URL" ] || APP_URL="http://127.0.0.1:$APP_PORT"

if [ -f "$ENV_FILE" ]; then
  backup="$ENV_FILE.backup-$(date +%Y%m%d-%H%M%S)"
  cp -p "$ENV_FILE" "$backup"
  chmod 600 "$backup"
  echo "Previous configuration saved as $backup"
fi

umask 077
tmp=$(mktemp "$ENV_FILE.XXXXXX")
{
  cat <<EOF
# Modlight (Rust) local stack, written by setup-local.sh on $(date -u +%Y-%m-%dT%H:%MZ).
# KEEP THIS FILE PRIVATE: it holds the keys that sign sessions and decrypt
# stored secrets, and the database password. Every variable is described in
# upstream's .env.example. After a change: docker compose -f server/docker/docker-compose.yml up -d

# ── Address (loopback only) ──────────────────────────────────────────────────
APP_PORT=$APP_PORT
APP_URL=$APP_URL
APP_NAME=Modlight
# No reverse proxy in front: rate limits key on the connection's own address.
TRUST_PROXY=false

# ── Secrets: never change these on a running install ────────────────────────
AUTH_SECRET=$AUTH_SECRET
EOF
  [ -n "$SETTINGS_ENCRYPTION_KEY" ] && printf 'SETTINGS_ENCRYPTION_KEY=%s\n' "$SETTINGS_ENCRYPTION_KEY"
  cat <<EOF
POSTGRES_PASSWORD=$POSTGRES_PASSWORD

# ── Storage: the Docker volume modlight-rust_appdata ────────────────────────
STORAGE_DRIVER=local

# ── Accounts and limits ─────────────────────────────────────────────────────
# Read once, on the first start; afterwards the switch lives in the admin
# panel. The very first account may always register and becomes the admin.
REGISTRATION_ENABLED=false
MAX_UPLOAD_SIZE=0
DEFAULT_USER_QUOTA=-1
TUS_CHUNK_SIZE=52428800

# ── Mail: configure SMTP in Admin -> Settings when needed ───────────────────
MAIL_FROM=no-reply@localhost
EOF
} > "$tmp"
chmod 600 "$tmp"
mv "$tmp" "$ENV_FILE"

echo "Wrote $ENV_FILE (mode 600; secrets $([ "$AUTH_NEW" = 1 ] && echo "newly generated" || echo "kept"))."
echo "Panel: $APP_URL"
echo "Start: docker compose -f $DIR/docker-compose.yml up -d --build"

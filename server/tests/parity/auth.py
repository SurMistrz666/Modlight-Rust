"""Parity of /api/auth/* (Better Auth) between the reference and Rust.

    python3 server/tests/parity/auth.py --ref http://localhost:8424 --rust http://localhost:8423 \
        --ref-env _scratch/srv-auth/ref/.env --rust-env _scratch/srv-auth/rust/rust.env \
        --ref-db modlight-srvauth-ref-postgres-1/modlight --rust-db modlight-devdb-postgres-1/modlight_srv_auth

DESTRUCTIVE for both databases: every run wipes the accounts and settings
tables and replays the same scenario on both servers (only use scratch
databases). Secrets are read from the env files and never printed.

Ids, tokens and timestamps differ between the servers and are normalised;
everything else (statuses, reason phrases, bodies, cookie names and
attributes, database rows) must match.
"""

import argparse
import base64
import hashlib
import hmac
import json
import os
import random
import struct
import sys
import time
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

ap = argparse.ArgumentParser(allow_abbrev=False)
ap.add_argument("--ref-env", default=os.environ.get("PARITY_REF_ENV", "../_scratch/srv-auth/ref/.env"))
ap.add_argument("--rust-env", default=os.environ.get("PARITY_RUST_ENV", "../_scratch/srv-auth/rust/rust.env"))
ap.add_argument("--ref-db", default="modlight-srvauth-ref-postgres-1/modlight")
ap.add_argument("--rust-db", default="modlight-devdb-postgres-1/modlight_srv_auth")
ap.add_argument("--only", default=None, help="run only scenarios whose name contains this")
args, _ = ap.parse_known_args()

run = parity.Run.from_args()
REF_SECRET = parity.read_env_file(args.ref_env)["AUTH_SECRET"]
RUST_SECRET = parity.read_env_file(args.rust_env)["AUTH_SECRET"]
ref_db = parity.Db(*args.ref_db.split("/"), REF_SECRET)
rust_db = parity.Db(*args.rust_db.split("/"), RUST_SECRET)
DBS = [ref_db, rust_db]

VOLATILE = ("id", "token", "userId", "accountId", "impersonatedBy", "userCode", "createdAt", "updatedAt", "expiresAt", "totpURI")
PASSWORD = "correct-horse-battery"


def both_sql(statement):
    return [db.sql(statement) for db in DBS]


def reset():
    both_sql('TRUNCATE "user", "session", "account", "verification", "two_factor", "settings", "audit_log" CASCADE')
    both_sql("INSERT INTO \"settings\" (\"key\", \"value\") VALUES ('register_enabled', 'on')")


def pair(ip=None, ua="parity/1"):
    return run.pair(user_agent=ua, real_ip=ip or "198.51.100.%d" % (int(time.time() * 1000) % 250 + 1))


_IP_BASE = "10.%d.%d" % (random.randint(0, 255), random.randint(0, 255))


def fresh_ip(n=[0]):
    """A client address no earlier run used (the reference's in-memory rate
    limiter outlives a Rust restart)."""
    n[0] += 1
    return "%s.%d" % (_IP_BASE, n[0] % 250 + 1)


def check(p, name, method, path, **kw):
    kw.setdefault("redact", VOLATILE)
    kw.setdefault("headers_to_compare", ("content-type", "location", "x-retry-after"))
    return p.check(name, method, path, **kw)


def rows(sql_text):
    a, b = both_sql(sql_text)
    return a, b


def compare_rows(name, sql_text):
    a, b = rows(sql_text)
    run.compare(name, parity.normalize_text(a), parity.normalize_text(b))


def b32decode(s):
    s = s.upper()
    s += "=" * (-len(s) % 8)
    return base64.b32decode(s)


def totp(secret_bytes, t=None, step=0):
    counter = int((t or time.time()) // 30) + step
    mac = hmac.new(secret_bytes, struct.pack(">Q", counter), hashlib.sha1).digest()
    off = mac[-1] & 0x0F
    code = (struct.unpack(">I", mac[off:off + 4])[0] & 0x7FFFFFFF) % 1000000
    return "%06d" % code


def totp_secret_from_uri(uri):
    q = urllib.parse.parse_qs(urllib.parse.urlparse(uri).query)
    return b32decode(q["secret"][0])


def jwt(secret, payload):
    def b64(b):
        return base64.urlsafe_b64encode(b).rstrip(b"=").decode()
    header = b64(b'{"alg":"HS256"}')
    body = b64(json.dumps(payload, separators=(",", ":")).encode())
    sig = hmac.new(secret.encode(), (header + "." + body).encode(), hashlib.sha256).digest()
    return header + "." + body + "." + b64(sig)


def enabled(name):
    return args.only is None or args.only in name


# ── 1. Routing, media types, error page ─────────────────────────────────────

def scenario_routing():
    p = pair(fresh_ip())
    check(p, "ok", "GET", "/api/auth/ok")
    check(p, "unknown path 404", "GET", "/api/auth/nope", body="empty")
    check(p, "HEAD ok 404", "HEAD", "/api/auth/ok", body="empty")
    check(p, "POST to a GET route 404", "POST", "/api/auth/ok", json_body={}, body="empty")
    check(p, "GET to a POST route 404", "GET", "/api/auth/sign-in/email", body="empty")
    check(p, "double slash below base 404", "GET", "/api/auth/x/api/auth/ok", body="empty")
    check(p, "error page redirect", "GET", "/api/auth/error?error=a%20b&error_description=%3Cx%3E", body="empty")
    check(p, "error page code kept", "GET", "/api/auth/error?error=INVALID_TOKEN", body="empty")
    check(p, "get-session signed out", "GET", "/api/auth/get-session")
    check(p, "get-session POST 405", "POST", "/api/auth/get-session", json_body={})
    check(p, "get-session garbage cookie", "GET", "/api/auth/get-session",
          headers={"Cookie": "better-auth.session_token=garbage"})
    check(p, "no content type 415", "POST", "/api/auth/sign-out", raw="")
    check(p, "text/plain 415", "POST", "/api/auth/sign-out", raw="{}", headers={"Content-Type": "text/plain"})
    check(p, "sign-in lists two media types", "POST", "/api/auth/sign-in/email", raw="x",
          headers={"Content-Type": "text/xml"})
    check(p, "invalid JSON 500", "POST", "/api/auth/sign-out", raw="{nope",
          headers={"Content-Type": "application/json"}, body="empty")
    check(p, "empty JSON body 500", "POST", "/api/auth/sign-out", raw="",
          headers={"Content-Type": "application/json"}, body="empty")
    check(p, "sign-out signed out", "POST", "/api/auth/sign-out", json_body={})
    check(p, "PUT 405", "PUT", "/api/auth/ok", body="empty", reason=False)


# ── 2. Sign-up ──────────────────────────────────────────────────────────────

def scenario_sign_up():
    reset()
    p = pair(fresh_ip())
    check(p, "sign-up empty body", "POST", "/api/auth/sign-up/email", json_body={})
    check(p, "sign-up array body", "POST", "/api/auth/sign-up/email", json_body=[])
    check(p, "sign-up bad email + empty array password", "POST", "/api/auth/sign-up/email",
          json_body={"name": "n", "email": "bad", "password": []})
    check(p, "sign-up short password", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Ann", "email": "ann@example.com", "password": "short"})
    check(p, "sign-up long password", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Ann", "email": "ann@example.com", "password": "x" * 129})
    check(p, "sign-up role not allowed", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Ann", "email": "ann@example.com", "password": PASSWORD, "role": "admin"})
    check(p, "sign-up bad hideUploads 422", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Ann", "email": "ann@example.com", "password": PASSWORD, "hideUploads": "zz"})
    compare_rows("no user after failed sign-ups", 'SELECT count(*) FROM "user"')
    # The first account: admin, with its own skin and language.
    check(p, "sign-up first account", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Ann Admin", "email": "Ann@Example.com", "password": PASSWORD, "theme": "paper",
                     "locale": "pl", "hideUploads": 1, "usedQuota": 99})
    compare_rows("first account row",
                 'SELECT name, email, email_verified, role, banned, two_factor_enabled, used_quota, max_quota, '
                 'hide_uploads, copy_raw, theme, locale, length(user_code) FROM "user"')
    compare_rows("first account credential",
                 "SELECT provider_id, account_id = user_id, length(password) FROM \"account\"")
    compare_rows("first account session",
                 "SELECT ip_address, user_agent, expires_at - created_at FROM \"session\"")
    check(p, "get-session after sign-up", "GET", "/api/auth/get-session")
    q = pair(fresh_ip())
    check(q, "sign-up duplicate 422", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Ann", "email": "ann@example.com", "password": PASSWORD})
    check(q, "sign-up second account (user role)", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Bob", "email": "bob@example.com", "password": PASSWORD, "rememberMe": False})
    compare_rows("roles", 'SELECT email, role FROM "user" ORDER BY email')
    check(q, "sign-up form encoded", "POST", "/api/auth/sign-up/email",
          form={"name": "Cid", "email": "cid@example.com", "password": PASSWORD})
    both_sql("UPDATE \"settings\" SET \"value\" = 'off' WHERE \"key\" = 'register_enabled'")
    check(pair(fresh_ip()), "sign-up closed 403", "POST", "/api/auth/sign-up/email",
          json_body={"name": "Dan", "email": "dan@example.com", "password": PASSWORD})
    check(pair(fresh_ip()), "sign-up closed beats validation", "POST", "/api/auth/sign-up/email", json_body={})
    both_sql("UPDATE \"settings\" SET \"value\" = 'on' WHERE \"key\" = 'register_enabled'")


# ── 3. Sign-in, origins, cookies ────────────────────────────────────────────

def scenario_sign_in():
    p = pair(fresh_ip())
    check(p, "sign-in missing fields", "POST", "/api/auth/sign-in/email", json_body={})
    check(p, "sign-in bad email", "POST", "/api/auth/sign-in/email",
          json_body={"email": "nope", "password": "x"})
    check(p, "sign-in unknown user", "POST", "/api/auth/sign-in/email",
          json_body={"email": "ghost@example.com", "password": PASSWORD})
    check(p, "sign-in wrong password", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": "wrong-password"})
    check(p, "sign-in rememberMe string", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD, "rememberMe": "false"})
    check(p, "sign-in evil callback 403", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD, "callbackURL": "//evil.com"})
    check(p, "sign-in non-string callback 500", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD, "callbackURL": 5}, body="empty")
    check(p, "sign-in ok with callback", "POST", "/api/auth/sign-in/email",
          json_body={"email": "BOB@example.com", "password": PASSWORD, "callbackURL": "/dashboard"})
    check(p, "get-session after sign-in", "GET", "/api/auth/get-session")
    q = pair(fresh_ip())
    check(q, "sign-in rememberMe false", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD, "rememberMe": False})
    check(q, "sign-in form encoded", "POST", "/api/auth/sign-in/email",
          form={"email": "bob@example.com", "password": PASSWORD})
    r = pair(fresh_ip())
    check(r, "cookie without origin 403", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD},
          headers={"Cookie": "a=b", "Origin": None})
    check(r, "cookie with foreign origin 403", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD},
          headers={"Cookie": "a=b", "Origin": "http://evil.example"})
    check(r, "cross-site navigation 403", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD},
          headers={"Sec-Fetch-Site": "cross-site", "Sec-Fetch-Mode": "navigate", "Origin": None})
    check(r, "fetch metadata, foreign origin 403", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD},
          headers={"Sec-Fetch-Site": "same-origin", "Origin": "http://evil.example"})
    ra = r.ref.request("POST", "/api/auth/sign-out", json_body={},
                       headers={"Cookie": "a=b", "Origin": None, "Referer": r.ref.base + "/x"})
    rb = r.rust.request("POST", "/api/auth/sign-out", json_body={},
                        headers={"Cookie": "a=b", "Origin": None, "Referer": r.rust.base + "/x"})
    r.compare_responses("referer as origin", ra, rb)


# ── 4. Session endpoints and account changes ────────────────────────────────

def signed_in_pair(email="bob@example.com", ip=None):
    p = pair(ip or fresh_ip())
    for b in (p.ref, p.rust):
        b.request("POST", "/api/auth/sign-in/email", json_body={"email": email, "password": PASSWORD})
    return p


def scenario_sessions():
    p = signed_in_pair()
    q = signed_in_pair()
    check(p, "list-sessions", "GET", "/api/auth/list-sessions")
    check(p, "list-accounts", "GET", "/api/auth/list-accounts")
    check(p, "update-user no fields", "POST", "/api/auth/update-user", json_body={})
    check(p, "update-user email refused", "POST", "/api/auth/update-user", json_body={"email": "x@y.zz"})
    check(p, "update-user role refused", "POST", "/api/auth/update-user", json_body={"role": "admin"})
    check(p, "update-user", "POST", "/api/auth/update-user", json_body={"name": "Bobby", "theme": "glass"})
    compare_rows("after update-user", 'SELECT name, theme FROM "user" WHERE email = \'bob@example.com\'')
    check(p, "update-session impersonatedBy", "POST", "/api/auth/update-session",
          json_body={"impersonatedBy": "someone"})
    check(p, "update-session nothing", "POST", "/api/auth/update-session", json_body={"x": 1})
    check(p, "verify-password wrong", "POST", "/api/auth/verify-password", json_body={"password": "nope"})
    check(p, "verify-password", "POST", "/api/auth/verify-password", json_body={"password": PASSWORD})
    check(p, "change-email disabled", "POST", "/api/auth/change-email", json_body={"newEmail": "n@example.com"})
    check(p, "delete-user disabled", "POST", "/api/auth/delete-user", json_body={}, body="empty")
    check(p, "delete-user callback disabled", "GET", "/api/auth/delete-user/callback?token=x")
    check(p, "unlink last account", "POST", "/api/auth/unlink-account", json_body={"providerId": "credential"})
    check(p, "send-verification-email mismatch", "POST", "/api/auth/send-verification-email",
          json_body={"email": "other@example.com"})
    check(p, "send-verification-email signed in", "POST", "/api/auth/send-verification-email",
          json_body={"email": "bob@example.com"})
    check(pair(fresh_ip()), "send-verification-email signed out", "POST", "/api/auth/send-verification-email",
          json_body={"email": "bob@example.com"})
    check(p, "change-password wrong current", "POST", "/api/auth/change-password",
          json_body={"currentPassword": "wrong-one", "newPassword": PASSWORD + "2"})
    check(p, "change-password short", "POST", "/api/auth/change-password",
          json_body={"currentPassword": PASSWORD, "newPassword": "x"})
    check(p, "change-password + revoke others", "POST", "/api/auth/change-password",
          json_body={"currentPassword": PASSWORD, "newPassword": PASSWORD, "revokeOtherSessions": True})
    check(q, "revoked session is gone", "GET", "/api/auth/get-session")
    check(p, "revoke-other-sessions", "POST", "/api/auth/revoke-other-sessions", json_body={})
    check(p, "revoke-session unknown", "POST", "/api/auth/revoke-session", json_body={"token": "nope"})
    check(p, "revoke-sessions", "POST", "/api/auth/revoke-sessions", json_body={})
    check(p, "get-session after revoke-sessions", "GET", "/api/auth/get-session")
    check(p, "list-sessions signed out 401", "GET", "/api/auth/list-sessions")


# ── 5. Two-factor ───────────────────────────────────────────────────────────

def scenario_two_factor():
    p = signed_in_pair("ann@example.com")
    check(p, "2fa enable wrong password", "POST", "/api/auth/two-factor/enable", json_body={"password": "nope"})
    a, b = check(p, "2fa enable", "POST", "/api/auth/two-factor/enable", json_body={"password": PASSWORD},
                 redact=VOLATILE + ("backupCodes",))
    secrets = [totp_secret_from_uri(r.json()["totpURI"]) for r in (a, b)]
    codes = [r.json()["backupCodes"] for r in (a, b)]
    run.compare("2fa uri shape", a.json()["totpURI"].split("?")[0], b.json()["totpURI"].split("?")[0])
    run.compare("backup code shapes", [len(c) for c in codes[0]], [len(c) for c in codes[1]])
    compare_rows("2fa row unverified", "SELECT verified, length(secret), length(backup_codes) FROM \"two_factor\"")
    check(p, "get-totp-uri", "POST", "/api/auth/two-factor/get-totp-uri", json_body={"password": PASSWORD})
    check(p, "verify-totp wrong", "POST", "/api/auth/two-factor/verify-totp", json_body={"code": "000000"})
    ra = p.ref.request("POST", "/api/auth/two-factor/verify-totp", json_body={"code": totp(secrets[0])})
    rb = p.rust.request("POST", "/api/auth/two-factor/verify-totp", json_body={"code": totp(secrets[1])})
    p.compare_responses("verify-totp enables", ra, rb, redact=VOLATILE,
                        headers_to_compare=("content-type",))
    compare_rows("2fa enabled", 'SELECT two_factor_enabled FROM "user" WHERE email = \'ann@example.com\'')
    compare_rows("2fa row verified", "SELECT verified FROM \"two_factor\"")
    # A fresh sign-in now stops at the second factor.
    s = pair(fresh_ip())
    check(s, "sign-in with 2fa", "POST", "/api/auth/sign-in/email",
          json_body={"email": "ann@example.com", "password": PASSWORD})
    compare_rows("pending 2fa verification",
                 "SELECT left(identifier, 4), length(identifier) FROM \"verification\" ORDER BY created_at")
    check(s, "verify-totp wrong (pending)", "POST", "/api/auth/two-factor/verify-totp", json_body={"code": "000000"})
    check(s, "send-otp not configured", "POST", "/api/auth/two-factor/send-otp", json_body={})
    check(s, "verify-otp nothing sent", "POST", "/api/auth/two-factor/verify-otp", json_body={"code": "123456"})
    ra = s.ref.request("POST", "/api/auth/two-factor/verify-totp",
                       json_body={"code": totp(secrets[0]), "trustDevice": True})
    rb = s.rust.request("POST", "/api/auth/two-factor/verify-totp",
                        json_body={"code": totp(secrets[1]), "trustDevice": True})
    s.compare_responses("verify-totp signs in + trusts", ra, rb, redact=VOLATILE,
                        headers_to_compare=("content-type",))
    # Trusted device: sign-in goes straight through.
    check(s, "sign-in on a trusted device", "POST", "/api/auth/sign-in/email",
          json_body={"email": "ann@example.com", "password": PASSWORD})
    t = pair(fresh_ip())
    check(t, "sign-in with 2fa again", "POST", "/api/auth/sign-in/email",
          json_body={"email": "ann@example.com", "password": PASSWORD})
    check(t, "verify-backup-code wrong", "POST", "/api/auth/two-factor/verify-backup-code",
          json_body={"code": "nope-nope"})
    ra = t.ref.request("POST", "/api/auth/two-factor/verify-backup-code", json_body={"code": codes[0][0]})
    rb = t.rust.request("POST", "/api/auth/two-factor/verify-backup-code", json_body={"code": codes[1][0]})
    t.compare_responses("verify-backup-code signs in", ra, rb, redact=VOLATILE,
                        headers_to_compare=("content-type",))
    ra = t.ref.request("POST", "/api/auth/two-factor/verify-backup-code", json_body={"code": codes[0][0]})
    rb = t.rust.request("POST", "/api/auth/two-factor/verify-backup-code", json_body={"code": codes[1][0]})
    t.compare_responses("backup code is single use (signed in: own token)", ra, rb, redact=VOLATILE,
                        headers_to_compare=("content-type",))
    check(t, "generate-backup-codes", "POST", "/api/auth/two-factor/generate-backup-codes",
          json_body={"password": PASSWORD}, redact=VOLATILE + ("backupCodes",))
    check(t, "2fa disable wrong password", "POST", "/api/auth/two-factor/disable", json_body={"password": "x"})
    check(t, "2fa disable", "POST", "/api/auth/two-factor/disable", json_body={"password": PASSWORD})
    compare_rows("2fa disabled", 'SELECT two_factor_enabled, (SELECT count(*) FROM two_factor) FROM "user" '
                 "WHERE email = 'ann@example.com'")


# ── 6. Password reset and e-mail verification links ─────────────────────────

def scenario_links():
    p = pair(fresh_ip())
    check(p, "request reset unknown email", "POST", "/api/auth/request-password-reset",
          json_body={"email": "ghost@example.com", "redirectTo": "/reset"})
    check(p, "request reset bad redirect", "POST", "/api/auth/request-password-reset",
          json_body={"email": "bob@example.com", "redirectTo": "https://evil.example/x"})
    check(p, "request reset, mail failure swallowed", "POST", "/api/auth/request-password-reset",
          json_body={"email": "bob@example.com", "redirectTo": "/reset"})
    for db in DBS:
        db.sql("INSERT INTO verification (id, identifier, value, expires_at, created_at, updated_at) "
               "SELECT 'v1', 'reset-password:tok123', id, now() + interval '1 hour', now(), now() FROM \"user\" "
               "WHERE email = 'bob@example.com'")
        db.sql("INSERT INTO verification (id, identifier, value, expires_at, created_at, updated_at) "
               "SELECT 'v2', 'reset-password:old', id, now() - interval '1 hour', now(), now() FROM \"user\" "
               "WHERE email = 'bob@example.com'")
    check(p, "reset link no callback", "GET", "/api/auth/reset-password/tok123", body="empty")
    check(p, "reset link evil callback", "GET", "/api/auth/reset-password/tok123?callbackURL=https://evil.example")
    check(p, "reset link unknown token", "GET", "/api/auth/reset-password/nope?callbackURL=/reset", body="empty")
    check(p, "reset link ok", "GET", "/api/auth/reset-password/tok123?callbackURL=%2Freset%3Fa%3D1", body="empty")
    compare_rows("expired verification rows cleaned",
                 "SELECT regexp_replace(identifier, '[A-Za-z0-9_-]{20,}$', '<rnd>') FROM verification ORDER BY 1")
    check(p, "reset no token", "POST", "/api/auth/reset-password", json_body={"newPassword": PASSWORD})
    check(p, "reset short", "POST", "/api/auth/reset-password", json_body={"newPassword": "x", "token": "tok123"})
    check(p, "reset unknown token", "POST", "/api/auth/reset-password",
          json_body={"newPassword": PASSWORD, "token": "nope"})
    check(p, "reset ok (token in query)", "POST", "/api/auth/reset-password?token=tok123",
          json_body={"newPassword": PASSWORD})
    compare_rows("reset consumed", "SELECT count(*) FROM verification WHERE identifier = 'reset-password:tok123'")
    # E-mail verification links, signed with each server's own secret.
    both_sql("UPDATE \"user\" SET email_verified = false WHERE email = 'bob@example.com'")
    now = int(time.time())
    v = pair(fresh_ip())
    for label, payload in [("expired", {"email": "bob@example.com", "iat": now - 7200, "exp": now - 3600}),
                           ("unknown user", {"email": "zed@example.com", "iat": now, "exp": now + 3600})]:
        ra = v.ref.request("GET", "/api/auth/verify-email?token=" + jwt(REF_SECRET, payload))
        rb = v.rust.request("GET", "/api/auth/verify-email?token=" + jwt(RUST_SECRET, payload))
        v.compare_responses("verify-email " + label, ra, rb)
        ra = v.ref.request("GET", "/api/auth/verify-email?callbackURL=%2Fx%3Fy%3D1&token=" + jwt(REF_SECRET, payload))
        rb = v.rust.request("GET", "/api/auth/verify-email?callbackURL=%2Fx%3Fy%3D1&token=" + jwt(RUST_SECRET, payload))
        v.compare_responses("verify-email %s with callback" % label, ra, rb, body="empty",
                            headers_to_compare=("location",))
    check(v, "verify-email garbage token", "GET", "/api/auth/verify-email?token=abc")
    check(v, "verify-email missing token", "GET", "/api/auth/verify-email")
    payload = {"email": "bob@example.com", "iat": now, "exp": now + 3600}
    ra = v.ref.request("GET", "/api/auth/verify-email?callbackURL=%2Fdashboard&token=" + jwt(REF_SECRET, payload))
    rb = v.rust.request("GET", "/api/auth/verify-email?callbackURL=%2Fdashboard&token=" + jwt(RUST_SECRET, payload))
    v.compare_responses("verify-email signs in", ra, rb, body="empty", headers_to_compare=("location",))
    compare_rows("verified", 'SELECT email_verified FROM "user" WHERE email = \'bob@example.com\'')
    check(v, "get-session after verify", "GET", "/api/auth/get-session")
    ra = v.ref.request("GET", "/api/auth/verify-email?token=" + jwt(REF_SECRET, payload))
    rb = v.rust.request("GET", "/api/auth/verify-email?token=" + jwt(RUST_SECRET, payload))
    v.compare_responses("verify-email already verified", ra, rb)


# ── 7. Admin plugin ─────────────────────────────────────────────────────────

def ids(email):
    return [db.sql("SELECT id FROM \"user\" WHERE email = '%s'" % email).strip() for db in DBS]


def scenario_admin():
    a = signed_in_pair("ann@example.com")
    u = signed_in_pair("bob@example.com")
    check(u, "list-users as user 403", "GET", "/api/auth/admin/list-users")
    check(pair(fresh_ip()), "list-users signed out 401", "GET", "/api/auth/admin/list-users", body="empty")
    check(a, "list-users", "GET", "/api/auth/admin/list-users?sortBy=email&sortDirection=desc&limit=2")
    check(a, "list-users search", "GET", "/api/auth/admin/list-users?searchValue=ob&searchField=email")
    check(a, "list-users filter", "GET",
          "/api/auth/admin/list-users?filterField=role&filterValue=admin&filterOperator=eq")
    check(a, "list-users unknown field", "GET", "/api/auth/admin/list-users?sortBy=nope")
    check(a, "list-users bad enum", "GET", "/api/auth/admin/list-users?searchField=role")
    bob_ref, bob_rust = ids("bob@example.com")

    def per_side(name, path, body_fn, **kw):
        ra = a.ref.request("POST", path, json_body=body_fn(bob_ref))
        rb = a.rust.request("POST", path, json_body=body_fn(bob_rust))
        a.compare_responses(name, ra, rb, redact=VOLATILE, headers_to_compare=("content-type",), **kw)

    ra = a.ref.request("GET", "/api/auth/admin/get-user?id=" + bob_ref)
    rb = a.rust.request("GET", "/api/auth/admin/get-user?id=" + bob_rust)
    a.compare_responses("get-user", ra, rb, redact=VOLATILE)
    check(a, "get-user unknown", "GET", "/api/auth/admin/get-user?id=nope")
    per_side("set-role", "/api/auth/admin/set-role", lambda i: {"userId": i, "role": ["user", "editor"]})
    per_side("update-user", "/api/auth/admin/update-user", lambda i: {"userId": i, "data": {"name": "Robert", "role": "user"}})
    per_side("update-user bad role", "/api/auth/admin/update-user", lambda i: {"userId": i, "data": {"role": 5}})
    per_side("list-user-sessions", "/api/auth/admin/list-user-sessions", lambda i: {"userId": i})
    per_side("ban-user", "/api/auth/admin/ban-user", lambda i: {"userId": i, "banReason": "spam"})
    check(u, "banned user's session is gone", "GET", "/api/auth/get-session")
    check(pair(fresh_ip()), "banned user cannot sign in", "POST", "/api/auth/sign-in/email",
          json_body={"email": "bob@example.com", "password": PASSWORD})
    per_side("unban-user", "/api/auth/admin/unban-user", lambda i: {"userId": i})
    compare_rows("unbanned", 'SELECT banned, ban_reason, ban_expires FROM "user" WHERE email = \'bob@example.com\'')
    ann_ref, ann_rust = ids("ann@example.com")
    ra = a.ref.request("POST", "/api/auth/admin/ban-user", json_body={"userId": ann_ref})
    rb = a.rust.request("POST", "/api/auth/admin/ban-user", json_body={"userId": ann_rust})
    a.compare_responses("ban self", ra, rb)
    check(a, "create-user bad email", "POST", "/api/auth/admin/create-user",
          json_body={"email": "bad", "name": "x"})
    check(a, "create-user duplicate", "POST", "/api/auth/admin/create-user",
          json_body={"email": "BOB@example.com", "name": "x"})
    check(a, "create-user", "POST", "/api/auth/admin/create-user",
          json_body={"email": "eve@example.com", "name": "Eve", "password": PASSWORD, "role": "admin",
                     "data": {"emailVerified": True, "theme": "mono"}})
    compare_rows("created user", 'SELECT name, role, email_verified, theme FROM "user" WHERE email = \'eve@example.com\'')
    per_side("set-user-password", "/api/auth/admin/set-user-password", lambda i: {"userId": i, "newPassword": PASSWORD})
    per_side("revoke-user-sessions", "/api/auth/admin/revoke-user-sessions", lambda i: {"userId": i})
    check(a, "revoke-user-session unknown", "POST", "/api/auth/admin/revoke-user-session",
          json_body={"sessionToken": "nope"})
    per_side("impersonate-user", "/api/auth/admin/impersonate-user", lambda i: {"userId": i})
    check(a, "session while impersonating", "GET", "/api/auth/get-session")
    check(a, "list-sessions hides impersonation", "GET", "/api/auth/list-sessions")
    check(a, "stop-impersonating", "POST", "/api/auth/admin/stop-impersonating", json_body={})
    check(a, "has-permission", "POST", "/api/auth/admin/has-permission",
          json_body={"permissions": {"user": ["ban"]}})
    check(a, "has-permission both keys", "POST", "/api/auth/admin/has-permission",
          json_body={"permission": {"user": ["ban"]}, "permissions": {"user": ["ban"]}})
    eve_ref, eve_rust = ids("eve@example.com")
    ra = a.ref.request("POST", "/api/auth/admin/remove-user", json_body={"userId": eve_ref})
    rb = a.rust.request("POST", "/api/auth/admin/remove-user", json_body={"userId": eve_rust})
    a.compare_responses("remove-user", ra, rb)
    check(a, "remove-user unknown", "POST", "/api/auth/admin/remove-user", json_body={"userId": "nope"})


# ── 8. Rate limits ──────────────────────────────────────────────────────────

def scenario_rate_limit():
    p = pair(fresh_ip())
    for i in range(3):
        check(p, "change-password %d" % i, "POST", "/api/auth/change-password", json_body={})
    check(p, "change-password 4th: 429", "POST", "/api/auth/change-password", json_body={},
          headers_to_compare=("content-type", "x-retry-after"))
    check(p, "other path unaffected", "GET", "/api/auth/ok")


# ── 9. Social / OAuth endpoints (no provider configured) ───────────────────

def scenario_social():
    p = pair(fresh_ip())
    check(p, "sign-in/social no provider", "POST", "/api/auth/sign-in/social", json_body={"provider": "github"})
    check(p, "sign-in/social bad body", "POST", "/api/auth/sign-in/social", json_body={"provider": 5})
    check(p, "callback no state", "GET", "/api/auth/callback/github?code=x", body="empty")
    check(p, "callback unknown state", "GET", "/api/auth/callback/github?code=x&state=abc", body="empty")
    check(p, "callback POST re-sent as GET", "POST", "/api/auth/callback/github", form={"code": "c", "state": "s"},
          body="empty")
    check(p, "get-access-token signed out", "POST", "/api/auth/get-access-token", json_body={"providerId": "github"},
          body="empty")
    check(p, "refresh-token signed out", "POST", "/api/auth/refresh-token", json_body={"providerId": "github"},
          body="empty")
    check(p, "account-info signed out", "GET", "/api/auth/account-info", body="empty")
    q = signed_in_pair("ann@example.com")
    check(q, "link-social no provider", "POST", "/api/auth/link-social", json_body={"provider": "github"})
    check(q, "get-access-token signed in", "POST", "/api/auth/get-access-token", json_body={"providerId": "github"})
    check(q, "refresh-token signed in", "POST", "/api/auth/refresh-token", json_body={"providerId": "github"})
    check(q, "account-info no id", "GET", "/api/auth/account-info")
    check(q, "unlink unknown provider", "POST", "/api/auth/unlink-account", json_body={"providerId": "github"})


SCENARIOS = [scenario_routing, scenario_sign_up, scenario_sign_in, scenario_sessions, scenario_two_factor,
             scenario_links, scenario_admin, scenario_rate_limit, scenario_social]

for s in SCENARIOS:
    if enabled(s.__name__):
        s()
run.finish()

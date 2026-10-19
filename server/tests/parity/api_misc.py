"""Differential checks for GET/PUT /api/theme, GET/PUT /api/client-settings
and GET /api/me (srv-auth part T) against the original server.

    python3 server/tests/parity/api_misc.py --ref http://localhost:8426 --rust http://localhost:8434

Both servers need their own database (this script writes): it seeds the
same users, sessions and API tokens on both sides (ids `pt_*`, re-created on
every run), then compares every method of the three routes with no
credential, a bad token, each token scope, a revoked token, a banned
account, session cookies (fresh and due for refresh), malformed bodies and
the success paths, re-reading the database after the writes.

Responses are compared on status, reason phrase, content type,
cache-control, allow, Set-Cookie shapes and the RAW body text (key order and
number formatting included) after replacing each server's own origin with
`<origin>` and ISO timestamps / uuids with placeholders.

Where the databases and secrets are (defaults: the srv-auth-t scratch
setup); secrets are read from the env files and never printed:

    --ref-container  modlight-srvauth-t-ref-postgres-1   --ref-db  modlight
    --ref-env        <scratch>/ref/.env (AUTH_SECRET)
    --rust-container modlight-devdb-postgres-1           --rust-db modlight_srv_auth_t
    --rust-env       <scratch>/rust.env (AUTH_SECRET)
"""

import argparse
import http.client
import os
import random
import re
import string
import sys
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

SCRATCH = os.environ.get("PARITY_SCRATCH", "/path/to/modlight/_scratch/srv-auth-t")

ap = argparse.ArgumentParser(allow_abbrev=False)
ap.add_argument("--ref-container", default="modlight-srvauth-t-ref-postgres-1")
ap.add_argument("--ref-db", default="modlight")
ap.add_argument("--ref-env", default=os.path.join(SCRATCH, "ref", ".env"))
ap.add_argument("--rust-container", default="modlight-devdb-postgres-1")
ap.add_argument("--rust-db", default="modlight_srv_auth_t")
ap.add_argument("--rust-env", default=os.path.join(SCRATCH, "rust.env"))
opts, _ = ap.parse_known_args()

run = parity.Run.from_args()
REF_ORIGIN = run.ref.rstrip("/")
RUST_ORIGIN = run.rust.rstrip("/")
ref_db = parity.Db(opts.ref_container, opts.ref_db, parity.read_env_file(opts.ref_env)["AUTH_SECRET"])
rust_db = parity.Db(opts.rust_container, opts.rust_db, parity.read_env_file(opts.rust_env)["AUTH_SECRET"])
SIDES = ((ref_db, REF_ORIGIN), (rust_db, RUST_ORIGIN))

FOLDER = "5f0c3e9a-2b1d-4c8e-9f7a-6d5b4c3a2b10"
SEED_TIME = "2026-01-01 00:00:00.000123"  # microseconds that no JS Date write produces


def secret(n=32):
    return "".join(random.SystemRandom().choice(string.ascii_letters + string.digits) for _ in range(n))


# ── Seeding (identical on both sides) ──────────────────────────────────────


def seed():
    tokens = {}
    for db, origin in SIDES:
        db.sql("DELETE FROM \"user\" WHERE id LIKE 'pt\\_%'")
        # alice: a tag, a preferred domain the server knows (its own origin,
        # spelled differently), an avatar of ours, a quota, a default folder.
        db.create_user("pt_alice", "alice@pt.test", "Alice", user_code="ptalice", theme="terminal", locale="pl",
                       preferred_domain=origin.upper() + "/", image="avatars/pt_alice-tok123.webp",
                       used_quota=1500, max_quota=10000, copy_raw=True)
        db.sql("INSERT INTO folders (id, owner_id, name, slug) VALUES ('%s', 'pt_alice', 'Shots', 'shots')" % FOLDER)
        db.sql("UPDATE \"user\" SET default_folder_id = '%s' WHERE id = 'pt_alice'" % FOLDER)
        # bob: a retired theme, a domain nobody configured, an absolute image,
        # quota numbers past 2^53.
        db.create_user("pt_bob", "bob@pt.test", "Bob \"B\" \u0141\u00f3d\u017a", user_code="ptbob", theme="modern",
                       preferred_domain="https://evil.example.com", image="https://cdn.example.com/b.png",
                       used_quota=9007199254740993, max_quota=9223372036854775807)
        # carol: banned, no tag, a theme that is an Object.prototype name.
        db.create_user("pt_carol", "carol@pt.test", "Carol", theme="constructor", banned=True, ban_reason="spam")
        # dave: an administrator, unlimited, `__proto__` as theme.
        db.create_user("pt_dave", "dave@pt.test", "Dave", role="admin", user_code="ptdave", theme="__proto__")
        # erin: every nullable column NULL.
        db.create_user("pt_erin", "erin@pt.test", "Erin", role=None, user_code="pterin", theme=None, used_quota=None,
                       max_quota=None, copy_raw=None, hide_uploads=None)
        db.sql("UPDATE \"user\" SET created_at = '%s', updated_at = '%s' WHERE id LIKE 'pt\\_%%'"
               % (SEED_TIME, SEED_TIME))
    for owner, name, scope in [("pt_alice", "upload", "upload"), ("pt_alice", "read", "read"),
                               ("pt_alice", "write", "write"), ("pt_alice", "full", "full"),
                               ("pt_alice", "revoked", "full"), ("pt_bob", "read", "read"),
                               ("pt_carol", "full", "full"), ("pt_dave", "full", "full"),
                               ("pt_erin", "write", "write")]:
        value = "pt_" + secret()
        tokens[(owner, name)] = value
        for db, _ in SIDES:
            db.create_api_token(owner, value, scope=scope, name=name)
    for db, _ in SIDES:
        db.sql("UPDATE api_tokens SET revoked_at = now() WHERE user_id = 'pt_alice' AND name = 'revoked'")
    return tokens


TOKENS = seed()


def key(owner, name):
    return {"x-api-key": TOKENS[(owner, name)]}


# ── Comparing ──────────────────────────────────────────────────────────────


def cookie_shapes(res):
    """Set-Cookie shapes; a session cookie's value as the lengths of its token
    and signature (the signature itself depends on each server's secret)."""
    out = []
    for name, value, attrs in parity.cookie_shape(res, volatile_values=False):
        if name.endswith("session_token") and value:
            token, _, sig = urllib.parse.unquote(value).partition(".")
            value = "<token len=%d, signature len=%d>" % (len(token), len(sig))
        elif value:
            value = "<value len=%d>" % len(value)
        out.append((name, value, attrs))
    return out


def cookie_token(res):
    for name, value, _ in parity.cookie_shape(res, volatile_values=False):
        if name.endswith("session_token") and value:
            return urllib.parse.unquote(value).partition(".")[0]
    return None


def shape(res, origin):
    if res is None:
        return {"error": "no response"}
    d = {"status": res.status, "reason": res.reason}
    for h in ("content-type", "cache-control", "allow"):
        d[h] = res.header(h)
    d["set-cookie"] = cookie_shapes(res)
    d["body"] = parity.normalize_text(res.body.decode("utf-8", "replace").replace(origin, "<origin>"))
    return d


def check(pair, name, method, path, headers=None, raw=None):
    a = pair.ref.request(method, path, headers=headers, raw=raw)
    b = pair.rust.request(method, path, headers=headers, raw=raw)
    run.compare(name, shape(a, REF_ORIGIN), shape(b, RUST_ORIGIN))
    return a, b


def send_chunked(browser, method, path, chunks, headers):
    """A request whose body is sent with Transfer-Encoding: chunked (no length)."""
    hdrs = {"User-Agent": browser.user_agent, "Accept": "*/*", "X-Real-IP": browser.real_ip}
    hdrs.update(headers)
    conn = http.client.HTTPConnection(browser.host, browser.port, timeout=60)
    try:
        conn.request(method, path, body=iter(chunks), headers=hdrs, encode_chunked=True)
    except OSError:
        pass  # the server may answer before taking the whole body
    try:
        r = conn.getresponse()
        data = r.read()
        return parity.Response(r.status, r.reason, [(k.lower(), v) for k, v in r.getheaders()], data)
    except (OSError, http.client.HTTPException):
        return None
    finally:
        conn.close()


def send(browser, method, path, body, headers):
    """A request with Content-Length, tolerating an answer before the body is in."""
    hdrs = {"User-Agent": browser.user_agent, "Accept": "*/*", "X-Real-IP": browser.real_ip}
    hdrs.update(headers)
    conn = http.client.HTTPConnection(browser.host, browser.port, timeout=60)
    try:
        conn.request(method, path, body=body, headers=hdrs)
    except OSError:
        pass
    try:
        r = conn.getresponse()
        data = r.read()
        return parity.Response(r.status, r.reason, [(k.lower(), v) for k, v in r.getheaders()], data)
    except (OSError, http.client.HTTPException):
        return None
    finally:
        conn.close()


def check_sent(pair, name, method, path, body, headers):
    a = send(pair.ref, method, path, body, headers)
    b = send(pair.rust, method, path, body, headers)
    run.compare(name, shape(a, REF_ORIGIN), shape(b, RUST_ORIGIN))


def check_chunked(pair, name, method, path, chunks, headers):
    a = send_chunked(pair.ref, method, path, chunks, headers)
    b = send_chunked(pair.rust, method, path, chunks, headers)
    run.compare(name, shape(a, REF_ORIGIN), shape(b, RUST_ORIGIN))


TS_RE = re.compile(r"\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?(?:[+-]\d{2}(?::?\d{2})?|Z)?")


def rows(name, sql):
    """The same SQL on both databases, compared after normalising timestamps."""
    out = []
    for db, origin in SIDES:
        text = db.sql(sql).replace(origin, "<origin>")
        out.append(TS_RE.sub("<ts>", text).splitlines())
    run.compare("db: " + name, out[0], out[1])


def user_rows(name):
    rows(name, "SELECT id, coalesce(theme, '<null>'), updated_at <> created_at AS touched, "
               "(extract(microseconds from updated_at)::bigint % 1000) = 0 AS ms "
               "FROM \"user\" WHERE id LIKE 'pt\\_%' ORDER BY id")


def token_rows(name):
    rows(name, "SELECT user_id, name, scope, last_used_at IS NOT NULL, revoked_at IS NOT NULL, "
               "coalesce((extract(microseconds from last_used_at)::bigint % 1000) = 0, true) "
               "FROM api_tokens WHERE user_id LIKE 'pt\\_%' ORDER BY user_id, name")


def settings_rows(name):
    rows(name, "SELECT owner_id, client, version, settings::text, created_at IS NOT NULL, "
               "(extract(microseconds from updated_at)::bigint % 1000) = 0 AS ms "
               "FROM client_settings WHERE owner_id LIKE 'pt\\_%' ORDER BY owner_id, client")


def json_headers(extra=None):
    h = {"Content-Type": "application/json"}
    h.update(extra or {})
    return h


anon = run.pair()


def session_pair(user_id, aged=False):
    p = run.pair()
    token = parity.random_id(32)
    for (db, _), browser in zip(SIDES, (p.ref, p.rust)):
        browser.cookies[parity.SESSION_COOKIE] = db.mint_session(user_id, ip=browser.real_ip or "", token=token)
        if aged:
            # Due for Better Auth's refresh (expires in 5 of its 7 days).
            db.sql("UPDATE session SET expires_at = now() + interval '5 days' WHERE token = '%s'" % token)
    return p, token


ROUTES = ["/api/me", "/api/theme", "/api/client-settings"]
ALICE, ADMIN, BANNED, ERIN = (session_pair(u)[0] for u in ("pt_alice", "pt_dave", "pt_carol", "pt_erin"))

# ── Every route: methods ───────────────────────────────────────────────────

for path in ROUTES:
    for method in ("OPTIONS", "POST", "DELETE", "PATCH"):
        check(anon, "%s %s" % (method, path), method, path)
    check(anon, "HEAD %s (anonymous)" % path, "HEAD", path)
    check(anon, "HEAD %s (full token)" % path, "HEAD", path, headers=key("pt_alice", "full"))
    if path != "/api/me":
        check(anon, "PUT %s without body (anonymous)" % path, "PUT", path)

# ── Every route and method: who may call ───────────────────────────────────

CALLS = [("GET", "/api/me", None), ("GET", "/api/theme", None), ("GET", "/api/client-settings", None),
         ("PUT", "/api/theme", '{"theme":"paper"}'), ("PUT", "/api/client-settings", '{"settings":{"probe":1}}')]
CREDENTIALS = [
    ("no credential", {}),
    ("bad x-api-key", {"x-api-key": "pt_not-a-token"}),
    ("bad bearer", {"Authorization": "Bearer pt_not-a-token"}),
    ("empty bearer", {"Authorization": "Bearer "}),
    ("basic auth", {"Authorization": "Basic cHQ6cHQ="}),
    ("empty x-api-key", {"x-api-key": ""}),
    ("revoked token", key("pt_alice", "revoked")),
    ("banned account's token", key("pt_carol", "full")),
    ("upload token", key("pt_alice", "upload")),
    ("read token", key("pt_alice", "read")),
    ("write token", key("pt_alice", "write")),
    ("full token", key("pt_alice", "full")),
    ("full token as lower-case bearer", {"Authorization": "bearer   %s  " % TOKENS[("pt_alice", "full")]}),
    ("empty x-api-key falls through to bearer", {"x-api-key": "  ", "Authorization": "Bearer " + TOKENS[("pt_alice", "read")]}),
]
for method, path, body in CALLS:
    for label, headers in CREDENTIALS:
        h = json_headers(headers) if body else headers
        check(anon, "%s %s: %s" % (method, path, label), method, path, headers=h, raw=body)
    for label, pair in (("alice's session", ALICE), ("admin's session", ADMIN), ("banned account's session", BANNED),
                        ("erin's session", ERIN)):
        h = json_headers() if body else None
        check(pair, "%s %s: %s" % (method, path, label), method, path, headers=h, raw=body)
    # A token that is present and wrong never falls through to the cookie.
    h = json_headers({"x-api-key": "pt_not-a-token"}) if body else {"x-api-key": "pt_not-a-token"}
    check(ALICE, "%s %s: bad token beside a valid session" % (method, path), method, path, headers=h, raw=body)
    h = json_headers(key("pt_alice", "upload")) if body else key("pt_alice", "upload")
    check(ADMIN, "%s %s: upload token beside an admin session" % (method, path), method, path, headers=h, raw=body)

user_rows("after the authentication matrix")
token_rows("after the authentication matrix")
settings_rows("after the authentication matrix")

# A session due for refresh: the route handler re-issues the cookie.
for method, path, body in CALLS:
    pair, token = session_pair("pt_erin", aged=True)
    a, b = check(pair, "%s %s: session due for refresh" % (method, path), method, path,
                 headers=json_headers() if body else None, raw=body)
    run.compare("%s %s: refreshed cookie carries the same token" % (method, path),
                cookie_token(a) == token, cookie_token(b) == token)
    rows("%s %s: refreshed session row" % (method, path),
         "SELECT user_id, expires_at > now() + interval '6 days 23 hours' FROM session WHERE token = '%s'" % token)

# ── GET /api/me per account ────────────────────────────────────────────────

for owner, name in (("pt_alice", "full"), ("pt_bob", "read"), ("pt_dave", "full"), ("pt_erin", "write")):
    check(anon, "GET /api/me as %s" % owner, "GET", "/api/me", headers=key(owner, name))

# ── GET /api/theme per stored value ────────────────────────────────────────

for owner, name in (("pt_bob", "read"), ("pt_dave", "full"), ("pt_erin", "write")):
    check(anon, "GET /api/theme as %s" % owner, "GET", "/api/theme", headers=key(owner, name))
for stored in ("toString", "__proto__", "hasOwnProperty", "valueOf", "MODERN", "", " paper", "nope", "glass"):
    for db, _ in SIDES:
        db.sql("UPDATE \"user\" SET theme = %s WHERE id = 'pt_erin'" % parity.sql_quote(stored))
    check(anon, "GET /api/theme with %r stored" % stored, "GET", "/api/theme", headers=key("pt_erin", "write"))
for db, _ in SIDES:
    db.sql("UPDATE \"user\" SET theme = NULL WHERE id = 'pt_erin'")

# ── PUT /api/theme bodies ──────────────────────────────────────────────────

W = key("pt_alice", "write")
THEME_BODIES = [
    ("valid", '{"theme":"synthwave"}'),
    ("retired id", '{"theme":"modern"}'),
    ("padded id", '{"theme":" \\u00a0terminal\\n "}'),
    ("unknown id", '{"theme":"hunter2"}'),
    ("inherited name", '{"theme":"constructor"}'),
    ("__proto__", '{"theme":"__proto__"}'),
    ("case differs", '{"theme":"Paper"}'),
    ("number", '{"theme":0}'),
    ("boolean", '{"theme":true}'),
    ("null", '{"theme":null}'),
    ("object", '{"theme":{}}'),
    ("array", '{"theme":["terminal"]}'),
    ("missing", '{"locale":"pl"}'),
    ("extra fields", '{"theme":"paper","locale":"pl","x":[1,2]}'),
    ("duplicate keys", '{"theme":"x","theme":"win98"}'),
    ("array body", '["terminal"]'),
    ("string body", '"terminal"'),
    ("null body", "null"),
    ("number body", "42"),
    ("empty", ""),
    ("whitespace", "  \n\t "),
    ("nbsp only", "\u00a0"),
    ("nbsp before JSON", "\u00a0{\"theme\":\"paper\"}"),
    ("byte-order mark", "\ufeff{\"theme\":\"mono\"}"),
    ("two byte-order marks", "\ufeff\ufeff{\"theme\":\"mono\"}"),
    ("unterminated", '{"theme":"paper"'),
    ("single quotes", "{'theme':'paper'}"),
    ("trailing comma", '{"theme":"paper",}'),
    ("trailing garbage", '{"theme":"paper"} x'),
    ("lone surrogate", '{"theme":"\\ud800"}'),
    ("escaped id", '{"theme":"\\u0067lass"}'),
    ("deep nesting", '{"theme":' + "[" * 2000 + "]" * 2000 + "}"),
    ("big number", '{"theme":1e400}'),
]
for label, body in THEME_BODIES:
    check(anon, "PUT /api/theme body: %s" % label, "PUT", "/api/theme", headers=json_headers(W), raw=body)
    user_rows("PUT /api/theme body: %s" % label)
for ctype in ("text/plain", "application/x-www-form-urlencoded", None):
    h = dict(W)
    if ctype:
        h["Content-Type"] = ctype
    check(anon, "PUT /api/theme content type %s" % ctype, "PUT", "/api/theme", headers=h, raw='{"theme":"riso"}')
check(anon, "PUT /api/theme invalid UTF-8", "PUT", "/api/theme", headers=json_headers(W),
      raw=b'{"theme":"pa\xffper"}')
check(anon, "PUT /api/theme invalid UTF-8 inside a valid id", "PUT", "/api/theme", headers=json_headers(W),
      raw=b'{"theme":"deco"}\xff')
# Size: declared length first, then UTF-16 units of what arrived.
pad = 4096 - len('{"theme":"deco","x":""}')
check(anon, "PUT /api/theme exactly 4096 bytes", "PUT", "/api/theme", headers=json_headers(W),
      raw='{"theme":"deco","x":"%s"}' % ("x" * pad))
check(anon, "PUT /api/theme 4097 bytes", "PUT", "/api/theme", headers=json_headers(W),
      raw='{"theme":"deco","x":"%s"}' % ("x" * (pad + 1)))
check(anon, "PUT /api/theme 5000 bytes", "PUT", "/api/theme", headers=json_headers(W),
      raw='{"theme":"deco","x":"%s"}' % ("x" * (5000 - len('{"theme":"deco","x":""}'))))
two_byte = '{"theme":"nebula","x":"%s"}' % ("\u0105" * 3000)
check(anon, "PUT /api/theme 3000 two-byte characters with a length", "PUT", "/api/theme", headers=json_headers(W),
      raw=two_byte)
check_chunked(anon, "PUT /api/theme 3000 two-byte characters, chunked", "PUT", "/api/theme",
              [two_byte.encode("utf-8")[i:i + 1000] for i in range(0, len(two_byte.encode("utf-8")), 1000)],
              json_headers(W))
over = '{"theme":"nebula","x":"%s"}' % ("x" * 4100)
check_chunked(anon, "PUT /api/theme 4123 bytes, chunked", "PUT", "/api/theme",
              [over[:2000].encode(), over[2000:].encode()], json_headers(W))
three_byte = '{"theme":"nebula","x":"%s"}' % ("\u20ac" * 4090)
check_chunked(anon, "PUT /api/theme 4090 three-byte characters, chunked", "PUT", "/api/theme",
              [three_byte.encode("utf-8")], json_headers(W))
user_rows("after the PUT /api/theme size checks")
# Writes through every credential that may write, and what the row holds.
for label, headers, theme in (("full token", key("pt_alice", "full"), "glass"),
                              ("erin's write token", key("pt_erin", "write"), "cartoon"),
                              ("dave's full token", key("pt_dave", "full"), "spotify")):
    check(anon, "PUT /api/theme via %s" % label, "PUT", "/api/theme", headers=json_headers(headers),
          raw='{"theme":"%s"}' % theme)
check(BANNED, "PUT /api/theme via the banned account's session", "PUT", "/api/theme", headers=json_headers(),
      raw='{"theme":"win98"}')
user_rows("after the PUT /api/theme writes")
check(anon, "GET /api/theme after the writes", "GET", "/api/theme", headers=key("pt_alice", "read"))

# ── /api/client-settings ───────────────────────────────────────────────────

CS = "/api/client-settings"
for query in ("?client=mobile", "?client=DESKTOP", "?client=+Desktop+", "?client=%20desktop%09", "?client=",
              "?client", "?client=desktop&client=mobile", "?client=mobile&client=desktop", "?client=%E2%84%AAx",
              "?client=DES%E2%84%AATOP", "?client=%20%20", "?Client=mobile", "?client=desk%74op", "?client=%ZZ"):
    check(anon, "GET %s%s" % (CS, query), "GET", CS + query, headers=key("pt_alice", "read"))
    check(anon, "PUT %s%s" % (CS, query), "PUT", CS + query, headers=json_headers(W), raw='{"settings":{"q":1}}')
settings_rows("after the client discriminator checks")

SETTINGS_BODIES = [
    ("empty", ""),
    ("whitespace", " \r\n"),
    ("invalid JSON", "{"),
    ("not JSON", "not json"),
    ("single quotes", "{'settings': {}}"),
    ("array body", "[]"),
    ("null body", "null"),
    ("string body", '"x"'),
    ("bare settings", '{"hotkeys":{}}'),
    ("settings array", '{"settings":[]}'),
    ("settings string", '{"settings":"a string"}'),
    ("settings number", '{"settings":42}'),
    ("settings null", '{"settings":null}'),
    ("settings true", '{"settings":true}'),
    ("version -1", '{"settings":{},"version":-1}'),
    ("version 1.5", '{"settings":{},"version":1.5}'),
    ("version string", '{"settings":{},"version":"3"}'),
    ("version true", '{"settings":{},"version":true}'),
    ("version array", '{"settings":{},"version":[]}'),
    ("version object", '{"settings":{},"version":{}}'),
    ("version 2147483647", '{"settings":{},"version":2147483647}'),
    ("version 2^53", '{"settings":{},"version":9007199254740992}'),
    ("version 1e400", '{"settings":{},"version":1e400}'),
    ("NUL in a value", '{"settings":{"k":"a\\u0000b"}}'),
    ("NUL in a key", '{"settings":{"a\\u0000":1}}'),
    ("literal backslash-u0000", '{"settings":{"k":"\\\\u0000"}}'),
    ("lone surrogate", '{"settings":{"k":"\\ud800"}}'),
    ("lone surrogate, stale version", '{"settings":{"k":"\\udc00"},"version":99}'),
    ("windows path", '{"settings":{"dir":"C:\\\\ud800\\\\shots"}}'),
]
for label, body in SETTINGS_BODIES:
    check(anon, "PUT %s body: %s" % (CS, label), "PUT", CS, headers=json_headers(W), raw=body)
    settings_rows("PUT %s body: %s" % (CS, label))

# Version flow on a fresh account (erin): 0 inserts, 0 again conflicts, the
# current version moves it on, a stale one conflicts, no version replaces.
E = key("pt_erin", "write")
for db, _ in SIDES:
    db.sql("DELETE FROM client_settings WHERE owner_id = 'pt_erin'")
FLOW = [
    ("v0 on nothing stored", '{"settings":{"hotkeys":{"capture":"PrintScreen"}},"version":0}'),
    ("v0 again", '{"settings":{"x":1},"version":0}'),
    ("v1 (current)", '{"settings":{"b":2,"a":1,"10":3,"2":4},"version":1}'),
    ("v1 again (stale)", '{"settings":{"stale":true},"version":1}'),
    ("v5 (ahead)", '{"settings":{"ahead":true},"version":5}'),
    ("blind replace", '{"settings":{"blind":[1.0,-0,1e21,1E-7,12345678901234567890,1e400]}}'),
    ("null version", '{"settings":{"nulled":true},"version":null}'),
    ("v4 as 4.0", '{"settings":{"float":true},"version":4.0}'),
    ("v5 as 5e0", '{"settings":{"exp":"\\u00e9\\ud83d\\ude00\\u2028"},"version":5e0}'),
    ("-0 on a stored row", '{"settings":{},"version":-0}'),
    ("pretty", '{\n  "settings": {\n    "a": [\n      1,\n      2\n    ]\n  }\n}'),
    ("duplicate keys", '{"settings":{"a":1,"b":2,"a":3},"settings":{"z":1,"y":2,"z":0}}'),
    ("integer-like keys", '{"settings":{"b":1,"4294967295":2,"4294967294":3,"1":4,"01":5,"-1":6,"1.5":7}}'),
    ("__proto__ key", '{"settings":{"__proto__":{"x":1},"toJSON":2,"constructor":3}}'),
]
for label, body in FLOW:
    check(anon, "PUT %s flow: %s" % (CS, label), "PUT", CS, headers=json_headers(E), raw=body)
    settings_rows("PUT %s flow: %s" % (CS, label))
    check(anon, "GET %s flow: after %s" % (CS, label), "GET", CS, headers=key("pt_erin", "write"))

# Sizes: the stored cap measures the canonical JSON in bytes; the transport
# cap the body as it arrived.
over_head = len('{"settings":{"blob":""}}')
check(anon, "PUT %s canonical exactly 262144 bytes" % CS, "PUT", CS, headers=json_headers(E),
      raw='{"settings":{"blob":"%s"}}' % ("x" * (262144 - len('{"blob":""}'))))
check(anon, "PUT %s canonical 262145 bytes" % CS, "PUT", CS, headers=json_headers(E),
      raw='{"settings":{"blob":"%s"}}' % ("x" * (262145 - len('{"blob":""}'))))
check(anon, "PUT %s canonical over, counted in UTF-8" % CS, "PUT", CS, headers=json_headers(E),
      raw='{"settings":{"blob":"%s"}}' % ("\u0105" * 131072))
check(anon, "PUT %s pretty body that fits once canonical" % CS, "PUT", CS, headers=json_headers(E),
      raw='{"settings":{"blob":"%s"}}%s' % ("x" * 200000, " " * 300000))
check(anon, "PUT %s body exactly 524288 bytes" % CS, "PUT", CS, headers=json_headers(E),
      raw='{"settings":{"blob":"%s"}}' % ("x" * (524288 - over_head)))
big = ('{"settings":{"blob":"%s"}}' % ("x" * (524289 - over_head))).encode()
check_sent(anon, "PUT %s 524289 bytes with a length" % CS, "PUT", CS, big, json_headers(E))
check_chunked(anon, "PUT %s 524289 bytes, chunked" % CS, "PUT", CS,
              [big[i:i + 65536] for i in range(0, len(big), 65536)], json_headers(E))
fits = ('{"settings":{"blob":"%s"}}' % ("x" * 1000)).encode()
check_chunked(anon, "PUT %s small, chunked" % CS, "PUT", CS, [fits[:7], fits[7:]], json_headers(E))
settings_rows("after the size checks")

# Nesting: V8's JSON.stringify throws past a stack-bound depth (a bare 500,
# nothing stored); the edge is modelled in js_json::STRINGIFY_STACK.
for label, depth in (("arrays 4346 deep", 4346), ("arrays 4347 deep", 4347), ("arrays 6000 deep", 6000)):
    check(anon, "PUT %s %s" % (CS, label), "PUT", CS, headers=json_headers(E),
          raw='{"settings":{"a":%s%s}}' % ("[" * depth, "]" * depth))
    settings_rows("PUT %s %s" % (CS, label))
for label, depth in (("objects 4000 deep", 4000), ("objects 4100 deep", 4100)):
    check(anon, "PUT %s %s" % (CS, label), "PUT", CS, headers=json_headers(E),
          raw='{"settings":%s1%s}' % ('{"a":' * depth, "}" * depth))
    settings_rows("PUT %s %s" % (CS, label))
check(anon, "GET %s after the nesting checks" % CS, "GET", CS, headers=key("pt_erin", "write"))

# A session reads and writes the same row; the banned account's session too.
check(ERIN, "GET %s via erin's session" % CS, "GET", CS)
check(ERIN, "PUT %s via erin's session" % CS, "PUT", CS, headers=json_headers(), raw='{"settings":{"s":1},"version":0}')
check(BANNED, "PUT %s via the banned account's session" % CS, "PUT", CS, headers=json_headers(),
      raw='{"settings":{"banned":true}}')
check(BANNED, "GET %s via the banned account's session" % CS, "GET", CS)
settings_rows("after the session writes")
token_rows("at the end")
user_rows("at the end")

run.finish()

"""Cross-compatibility proof: the original TypeScript server and the Rust port
on ONE database, with ONE AUTH_SECRET. What one writes, the other accepts, in
both directions (PORT.md 1, "Same database").

    python3 server/tests/parity/crosscompat.py --ts http://localhost:8431 --rust http://localhost:8439 \
        --container modlight-srvauth-x-postgres-1

Setup (see tests/parity/crosscompat-rust.sh): an original stack (its app on
--ts, its Postgres published on 127.0.0.1) restored from a copy of a database
the TS server wrote, and the Rust server started by crosscompat-rust.sh on the
same database and secret. Every scenario runs twice: writer TS -> reader Rust,
then writer Rust -> reader TS, through the real /api/auth endpoints (each
server with its own Origin header):

1. existing data: session rows the TS server wrote before this run, signed
   with the shared secret, accepted by both servers' get-session and pages;
   sessions the writer issues for those accounts (impersonation) accepted by
   the reader;
2. sign-up on the writer, sign-in with the same password on the reader,
   sign-out on one ends the session on both;
3. sessions: get-session, list-sessions, update-user across servers,
   revoke-other-sessions, remember-me off (the signed dont_remember cookie);
4. change-password on the writer, sign-in with the new password on the reader;
5. two-factor: enable on the writer, finish with a TOTP on the reader, sign in
   on both servers with TOTP codes and backup codes, a sign-in started on one
   finished on the other, trusted devices, regenerated backup codes, disable;
6. mail links: SMTP settings saved on the writer (password encrypted with the
   shared secret) used by the reader; the verification and reset links mailed
   by the writer followed on the reader. Mail goes to smtp_sink.mjs, which
   runs on the host for the Rust server and inside the app container for the
   TS server, both on 127.0.0.1:<--sink-port>;
7. admin: set-role, ban (and an expired ban), unban, impersonate and stop,
   set-user-password, create-user, remove-user.

Before the scenarios: the schema is what it was before the Rust server's
`serve` ran its migrator (crosscompat-rust.sh saved it).

Rate limits: the Rust server runs with TRUST_PROXY off, so it keys its
limiter on the socket address (127.0.0.1 for every request here); a 429 is
waited out (X-Retry-After) and retried. The TS server, with no X-Real-IP,
skips rate limiting. Neither is a compatibility question.

Nothing secret is printed: cookies, passwords, TOTP secrets and codes, backup
codes and mailed tokens stay inside the script. Writes go only to the stack's
own database. Only the Python standard library; Python 3.9.
"""

import argparse
import base64
import email
import email.policy
import hashlib
import hmac
import json
import os
import random
import re
import struct
import subprocess
import sys
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import parity  # noqa: E402

DEFAULT_SCRATCH = "/path/to/modlight/_scratch/srv-auth-x"

ap = argparse.ArgumentParser(allow_abbrev=False)
ap.add_argument("--ts", default="http://localhost:8431")
ap.add_argument("--rust", default="http://localhost:8439")
ap.add_argument("--container", default="modlight-srvauth-x-postgres-1", help="the shared Postgres container")
ap.add_argument("--database", default="modlight")
ap.add_argument("--app-container", default="modlight-srvauth-x-app-1", help="the TS app container")
ap.add_argument("--env", default=os.path.join(DEFAULT_SCRATCH, "stack", ".env"), help="AUTH_SECRET (never printed)")
ap.add_argument("--scratch", default=DEFAULT_SCRATCH, help="schema snapshot and sink logs")
ap.add_argument("--sink-port", type=int, default=2630)
ap.add_argument("--only", default=None, help="run only scenarios whose name contains this")
ap.add_argument("-v", "--verbose", action="store_true")
ARGS = ap.parse_args()

SECRET = parity.read_env_file(ARGS.env)["AUTH_SECRET"]
DB = parity.Db(ARGS.container, ARGS.database, SECRET)
SESSION = parity.SESSION_COOKIE
DONT_REMEMBER = "better-auth.dont_remember"
TWO_FACTOR = "better-auth.two_factor"
TRUST_DEVICE = "better-auth.trust_device"
ADMIN_SESSION = "better-auth.admin_session"

SERVERS = {"ts": ARGS.ts.rstrip("/"), "rust": ARGS.rust.rstrip("/")}
LABEL = {"ts": "TS", "rust": "Rust"}
DIRECTIONS = [("ts", "rust"), ("rust", "ts")]
parity.ORIGINS[:] = list(SERVERS.values())

TAG = "".join(random.SystemRandom().choice("abcdefghijklmnopqrstuvwxyz0123456789") for _ in range(6))
RUN_STARTED = time.time()


def new_password():
    return "pw-" + "".join(random.SystemRandom().choice("abcdefghijkmnpqrstuvwxyz23456789") for _ in range(20))


# ── results ─────────────────────────────────────────────────────────────────

PASSED = []
FAILED = []
CONTEXT = [""]


def check(name, ok, reason=""):
    full = ("[%s] %s" % (CONTEXT[0], name)) if CONTEXT[0] else name
    line = "%s %s%s" % ("PASS" if ok else "FAIL", full, (": " + reason) if reason else "")
    print(line, flush=True)
    (PASSED if ok else FAILED).append(full)
    return ok


def direction(writer, reader):
    CONTEXT[0] = "%s -> %s" % (LABEL[writer], LABEL[reader])


def describe(res):
    """Status plus the error code, never cookies or tokens."""
    body = res.json()
    code = body.get("code") if isinstance(body, dict) else None
    return "%d%s" % (res.status, (" " + code) if code else "")


# ── one browser for both servers ────────────────────────────────────────────


class Client(object):
    """One browser. Cookies are not port-scoped, so a real browser sends the
    same localhost cookies to both servers: one jar, two origins."""

    def __init__(self, ua="crosscompat/1"):
        self.jar = {}
        self.b = {}
        for side, base in SERVERS.items():
            b = parity.Browser(base, real_ip=None, user_agent=ua)
            b.cookies = self.jar
            self.b[side] = b

    def req(self, side, method, path, **kw):
        res = None
        for _ in range(8):
            res = self.b[side].request(method, path, **kw)
            if res.status != 429:
                return res
            wait = res.header("x-retry-after")
            time.sleep((int(wait) if wait and wait.isdigit() else 5) + 0.5)
        return res

    def post(self, side, path, body=None, **kw):
        return self.req(side, "POST", path, json_body={} if body is None else body, **kw)

    def get(self, side, path, **kw):
        return self.req(side, "GET", path, **kw)

    def session(self, side):
        res = self.get(side, "/api/auth/get-session")
        body = res.json() if res.status == 200 else None
        return res, body if isinstance(body, dict) else None

    def copy(self, ua="crosscompat/1"):
        c = Client(ua)
        c.jar.update(self.jar)
        return c

    def only(self, *names):
        c = Client()
        c.jar.update(dict((k, v) for k, v in self.jar.items() if k in names))
        return c


def sign_up(side, email_, password, name="Cross User", **extra):
    c = Client()
    body = {"name": name, "email": email_, "password": password}
    body.update(extra)
    return c, c.post(side, "/api/auth/sign-up/email", body)


def sign_in(side, email_, password, client=None, **extra):
    c = client or Client()
    body = {"email": email_, "password": password}
    body.update(extra)
    return c, c.post(side, "/api/auth/sign-in/email", body)


def expect_session(title, client, side, email_, **fields):
    """get-session on `side` answers this user (and these user fields)."""
    res, body = client.session(side)
    user = (body or {}).get("user") or {}
    ok = res.status == 200 and user.get("email") == email_
    bad = [k for k, v in fields.items() if user.get(k) != v]
    reason = "get-session %d, user %s" % (res.status, "matches" if ok else "missing or other")
    if bad:
        reason += ", differs in " + ", ".join(bad)
    return check(title, ok and not bad, reason), body


def expect_no_session(name, client, side):
    res, body = client.session(side)
    return check(name, res.status == 200 and body is None, "get-session %d, %s" % (
        res.status, "null" if body is None else "a session"))


def user_row(email_, columns):
    out = DB.sql('SELECT %s FROM "user" WHERE email = %s' % (columns, parity.sql_quote(email_.lower()))).strip()
    return out.split("\t") if out else None


def user_id(email_):
    row = user_row(email_, "id")
    return row[0] if row else None


def fresh_email(label):
    return "%s-%s@cross.test" % (label, TAG)


# ── pages ───────────────────────────────────────────────────────────────────

FLIGHT_RE = re.compile(r'self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)')


def flight_object(flight, prefix):
    """The first JSON object in the RSC payload that starts with `prefix`."""
    i = flight.find(prefix)
    if i < 0:
        return None
    try:
        return json.JSONDecoder().raw_decode(flight, i)[0]
    except ValueError:
        return None


def profile_page(client, side):
    """The signed-in user's email, 2FA flag and the current session id as
    /dashboard/profile shows them: the client components' props in the
    reference's RSC payload, the page data from /_page on Rust. None when
    the page redirects (signed out)."""
    if side == "ts":
        res = client.get(side, "/dashboard/profile")
        if res.status != 200:
            return {"status": res.status, "location": res.header("location")}
        flight = "".join(json.loads('"%s"' % c) for c in FLIGHT_RE.findall(res.text()))
        layout = flight_object(flight, '{"user":{"name"') or {}
        security = flight_object(flight, '{"twoFactorEnabled"') or {}
        sessions = (flight_object(flight, '{"sessions":[') or {}).get("sessions") or []
        user = layout.get("user") or {}
    else:
        res = client.get(side, "/_page?path=/dashboard/profile")
        data = res.json() or {}
        if "redirect" in data:
            return {"status": data.get("status"), "location": data.get("redirect")}
        user = ((data.get("layouts") or {}).get("(app)") or {}).get("user") or {}
        props = data.get("props") or {}
        security = props.get("u") or {}
        sessions = props.get("sessions") or []
    current = [s.get("id") for s in sessions if s.get("current")]
    return {"status": 200, "email": user.get("email"), "role": user.get("role"),
            "twoFactorEnabled": security.get("twoFactorEnabled"), "current": current}


def expect_page(title, client, side, email_, session_id=None, **fields):
    page = profile_page(client, side)
    ok = page.get("status") == 200 and page.get("email") == email_
    if session_id is not None:
        ok = ok and page.get("current") == [session_id]
    bad = [k for k, v in fields.items() if page.get(k) != v]
    reason = "page %s, user %s" % (page.get("status"), "matches" if page.get("email") == email_ else "missing or other")
    if session_id is not None:
        reason += ", current session %s" % ("matches" if page.get("current") == [session_id] else "differs")
    if bad:
        reason += ", differs in " + ", ".join(bad)
    return check(title, ok and not bad, reason)


def expect_page_signed_out(name, client, side):
    page = profile_page(client, side)
    ok = page.get("status") == 307 and (page.get("location") or "").startswith("/login")
    return check(name, ok, "page %s -> %s" % (page.get("status"), page.get("location")))


# ── TOTP ────────────────────────────────────────────────────────────────────


def totp_key(uri):
    """The HMAC key: the secret STRING's bytes, base32 in the otpauth URI."""
    q = urllib.parse.parse_qs(urllib.parse.urlparse(uri).query)
    s = q["secret"][0].upper()
    return base64.b32decode(s + "=" * (-len(s) % 8))


def totp(key, step=0):
    counter = int(time.time() // 30) + step
    mac = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    off = mac[-1] & 0x0F
    return "%06d" % ((struct.unpack(">I", mac[off:off + 4])[0] & 0x7FFFFFFF) % 1000000)


# ── server actions (mail settings) ──────────────────────────────────────────

_ACTION_IDS = []


def call_action(client, side, module, export, args):
    """A server action through each server's own protocol: Next's on TS (the
    id from its server-reference manifest), /_rpc on Rust. The envelope."""
    if side == "ts":
        if not _ACTION_IDS:
            _ACTION_IDS.append(parity.ActionIds(ARGS.app_container))
        action_id, page = _ACTION_IDS[0].get(module, export)
        res = client.req("ts", "POST", page, raw=json.dumps(parity._encode_reply(list(args))), headers={
            "Next-Action": action_id, "Content-Type": "text/plain;charset=UTF-8", "Accept": "text/x-component"})
        return parity.next_action_envelope(res)
    return parity.rpc_envelope(client.b["rust"].rpc(module, export, parity._encode_tagged(list(args))))


# ── scenario 0: the schema the Rust migrator left ───────────────────────────


def strip_restrict(text):
    # pg_dump 17 writes a random \restrict key into every dump.
    return "\n".join(line for line in text.splitlines() if not line.startswith(("\\restrict", "\\unrestrict")))


def scenario_schema():
    CONTEXT[0] = "Rust serve on the TS database"
    path = os.path.join(ARGS.scratch, "schema-before-rust.sql")
    if not os.path.exists(path):
        check("schema snapshot exists", False, "%s missing (start Rust with crosscompat-rust.sh)" % path)
        return
    out = subprocess.run(["docker", "exec", ARGS.container, "sh", "-c",
                          "pg_dump -U modlight --schema-only %s; psql -U modlight -d %s -Atc "
                          "\"select id, hash, created_at from drizzle.__drizzle_migrations order by id\""
                          % (ARGS.database, ARGS.database)],
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout.decode("utf-8")
    with open(path) as f:
        before = f.read()
    same = strip_restrict(before) == strip_restrict(out)
    rows = len([line for line in out.splitlines() if re.match(r"^\d+\|[0-9a-f]{64}\|\d+$", line)])
    check("schema and drizzle bookkeeping unchanged by the migrator", same and rows > 0,
          "%d migration rows, dump %s" % (rows, "identical" if same else "DIFFERS"))
    for side in SERVERS:
        res = Client().get(side, "/api/auth/ok")
        check("%s answers /api/auth/ok" % LABEL[side], res.status == 200 and res.json() == {"ok": True},
              describe(res))
    reg = DB.sql("SELECT value FROM settings WHERE key = 'register_enabled'").strip()
    check("registration open in the copied settings", reg == "on", "register_enabled=%s" % (reg or "<unset>"))


# ── scenario 1: data the TS server wrote before this run ────────────────────


def copied_sessions():
    """The newest live session per account, from rows written before this run
    (accounts this script creates, @cross.test, are left out)."""
    out = DB.sql(
        'SELECT DISTINCT ON (s.user_id) s.id, s.token, u.email, u.role FROM "session" s JOIN "user" u ON u.id = s.user_id '
        "WHERE s.expires_at > now() AND s.impersonated_by IS NULL AND s.created_at < to_timestamp(%d) "
        "AND u.email NOT LIKE '%%@cross.test' ORDER BY s.user_id, s.created_at DESC" % int(RUN_STARTED))
    rows = []
    for line in out.splitlines():
        sid, token, email_, role = line.split("\t")
        rows.append({"id": sid, "token": token, "email": email_, "role": role})
    return rows


def cookie_for(token):
    sig = base64.b64encode(hmac.new(SECRET.encode(), token.encode(), hashlib.sha256).digest()).decode()
    return urllib.parse.quote(token + "." + sig, safe="")


ADMIN = {}


def admin_client():
    c = Client()
    c.jar[SESSION] = ADMIN["cookie"]
    return c


def scenario_existing():
    CONTEXT[0] = "copied data"
    rows = copied_sessions()
    check("the copied database has live sessions the TS server wrote", len(rows) >= 2,
          "%d accounts with a live session" % len(rows))
    for row in rows:
        if row["role"] == "admin" and not ADMIN:
            ADMIN.update({"cookie": cookie_for(row["token"]), "email": row["email"], "id": user_id(row["email"])})
    for row in rows:
        c = Client()
        c.jar[SESSION] = cookie_for(row["token"])
        for side in ("rust", "ts"):
            _, body = expect_session("%s get-session accepts the TS-issued session of %s" % (LABEL[side], row["role"]),
                                     c, side, row["email"])
            sid = ((body or {}).get("session") or {}).get("id")
            check("%s get-session names the same session row (%s)" % (LABEL[side], row["role"]), sid == row["id"],
                  "session id %s" % ("matches" if sid == row["id"] else "differs"))
            expect_page("%s /dashboard/profile for the TS-issued session (%s)" % (LABEL[side], row["role"]),
                        c, side, row["email"], session_id=row["id"])
    if not ADMIN:
        check("an admin among the copied accounts", False, "no live admin session")
        return
    others = [r for r in rows if r["role"] != "admin"]
    if not others:
        check("a non-admin among the copied accounts", False, "none")
        return
    target = others[0]
    for writer, reader in DIRECTIONS:
        direction(writer, reader)
        a = admin_client()
        res = a.post(writer, "/api/auth/admin/impersonate-user", {"userId": user_id(target["email"])})
        check("writer issues a session for the copied account (impersonate)",
              res.status == 200 and SESSION in a.jar and ADMIN_SESSION in a.jar, describe(res))
        _, body = expect_session("reader get-session accepts the writer's session for the copied account",
                                 a, reader, target["email"])
        imp = ((body or {}).get("session") or {}).get("impersonatedBy")
        check("reader sees who impersonates", imp == ADMIN["id"], "impersonatedBy %s" % (
            "is the admin" if imp == ADMIN["id"] else repr(imp)))
        expect_page("reader /dashboard/profile for the writer's session", a, reader, target["email"])
        res = a.post(reader, "/api/auth/admin/stop-impersonating")
        check("reader stops the writer's impersonation (admin_session cookie)", res.status == 200, describe(res))
        expect_session("writer sees the admin's own session restored", a, writer, ADMIN["email"])
        left = DB.sql('SELECT count(*) FROM "session" WHERE impersonated_by IS NOT NULL').strip()
        check("impersonation session removed", left == "0", "%s impersonation rows" % left)


# ── scenario 2: sign-up, sign-in, sign-out ──────────────────────────────────

ACCOUNTS = {}


def scenario_sign_up_in_out():
    for writer, reader in DIRECTIONS:
        direction(writer, reader)
        em = fresh_email("signup-" + writer)
        pw = new_password()
        c1, res = sign_up(writer, em, pw, name="Cross %s" % LABEL[writer])
        check("sign-up on the writer", res.status == 200 and SESSION in c1.jar, describe(res))
        fmt = DB.sql("SELECT password ~ '^[0-9a-f]{32}:[0-9a-f]{128}$' FROM account a JOIN \"user\" u ON u.id = a.user_id "
                     "WHERE u.email = %s AND a.provider_id = 'credential'" % parity.sql_quote(em)).strip()
        check("writer stored a Better Auth scrypt hash (salt:key, hex)", fmt == "t", "format %s" % (
            "ok" if fmt == "t" else "other"))
        expect_session("reader accepts the sign-up session cookie", c1, reader, em)
        # The row the writer created (user code, quota, theme defaults) as the reader's page shows it.
        code, role = user_row(em, "user_code, role")
        page = profile_page(c1, reader)
        check("reader /dashboard/profile renders the account the writer created",
              page.get("status") == 200 and page.get("email") == em and page.get("role") == role == "user"
              and page.get("twoFactorEnabled") is False and len(code) == 6,
              "page %s, %s" % (page.get("status"), "fields match" if page.get("email") == em else "user missing"))
        _, res = sign_in(reader, em, "wrong-" + pw)
        check("reader refuses a wrong password", res.status == 401, describe(res))
        c2, res = sign_in(reader, em, pw)
        body = res.json() or {}
        check("reader signs in with the password set on the writer",
              res.status == 200 and (body.get("user") or {}).get("email") == em and SESSION in c2.jar, describe(res))
        expect_session("writer accepts the reader's sign-in session", c2, writer, em)
        expect_page("writer /dashboard/profile for the reader's session", c2, writer, em)
        kept = c2.copy()
        res = c2.post(writer, "/api/auth/sign-out")
        check("sign-out on the writer", res.status == 200 and (res.json() or {}).get("success") is True, describe(res))
        expect_no_session("reader: signed-out session is gone", kept, reader)
        expect_no_session("writer: signed-out session is gone", kept, writer)
        kept1 = c1.copy()
        res = c1.post(reader, "/api/auth/sign-out")
        check("sign-out on the reader of the writer's sign-up session", res.status == 200, describe(res))
        expect_no_session("writer: the sign-up session is gone", kept1, writer)
        expect_page_signed_out("writer page sends the signed-out cookie to /login", kept1, writer)
        ACCOUNTS[writer] = {"email": em, "password": pw}


# ── scenario 3: sessions across servers ─────────────────────────────────────


def session_row(token_or_id, by="token"):
    out = DB.sql('SELECT id, extract(epoch FROM expires_at - now())::bigint FROM "session" WHERE %s = %s'
                 % (by, parity.sql_quote(token_or_id))).strip()
    if not out:
        return None
    sid, secs = out.split("\t")
    return {"id": sid, "expires_in": int(secs)}


def session_token(client):
    v = client.jar.get(SESSION)
    return urllib.parse.unquote(v).rsplit(".", 1)[0] if v else None


def scenario_sessions():
    for writer, reader in DIRECTIONS:
        direction(writer, reader)
        acc = ACCOUNTS[writer]
        a, res = sign_in(writer, acc["email"], acc["password"])
        check("sign-in on the writer", res.status == 200, describe(res))
        _, body = expect_session("reader get-session", a, reader, acc["email"])
        res = a.get(reader, "/api/auth/list-sessions")
        listed = res.json() if res.status == 200 else None
        mine = session_token(a)
        count = DB.sql('SELECT count(*) FROM "session" s JOIN "user" u ON u.id = s.user_id WHERE u.email = %s '
                       "AND s.expires_at > now()" % parity.sql_quote(acc["email"])).strip()
        check("reader list-sessions lists the writer's session",
              isinstance(listed, list) and any(s.get("token") == mine for s in listed) and str(len(listed)) == count,
              "%s, %s listed, %s in the database" % (describe(res), len(listed) if isinstance(listed, list) else "-", count))
        new_name = "Renamed by %s %s" % (LABEL[reader], TAG)
        res = a.post(reader, "/api/auth/update-user", {"name": new_name, "theme": "paper"})
        check("reader update-user with the writer's session", res.status == 200 and (res.json() or {}).get("status") is True,
              describe(res))
        expect_session("writer sees the reader's update", a, writer, acc["email"], name=new_name, theme="paper")

        # revoke-other-sessions on the writer ends the reader's sessions.
        b, res = sign_in(reader, acc["email"], acc["password"])
        check("second sign-in on the reader", res.status == 200, describe(res))
        res = a.post(writer, "/api/auth/revoke-other-sessions")
        check("writer revoke-other-sessions", res.status == 200 and (res.json() or {}).get("status") is True,
              describe(res))
        expect_no_session("reader: the revoked session is gone", b, reader)
        expect_session("reader: the revoking session lives on", a, reader, acc["email"])

        # Remember me off: the writer's signed dont_remember cookie stops the
        # reader from refreshing (and so from extending) the session.
        d, res = sign_in(writer, acc["email"], acc["password"], rememberMe=False)
        cookies = dict((n, attrs) for n, _, attrs in res.set_cookies())
        check("writer: rememberMe false sets a session cookie and dont_remember",
              res.status == 200 and SESSION in cookies and "max-age" not in cookies[SESSION] and DONT_REMEMBER in cookies,
              "%s, cookies %s" % (describe(res), sorted(cookies)))
        row = session_row(session_token(d))
        check("writer: the session row lives one day", row is not None and 86000 < row["expires_in"] <= 86400,
              "expires in %ss" % (row or {}).get("expires_in"))
        res = d.get(reader, "/api/auth/get-session")
        after = session_row(session_token(d))
        refreshed = [n for n, _, _ in res.set_cookies() if n == SESSION]
        check("reader honours dont_remember: no refresh, no new cookie",
              res.status == 200 and (res.json() or {}).get("user") is not None and not refreshed
              and after is not None and abs(after["expires_in"] - row["expires_in"]) < 30,
              "%s, %d session cookies set, expires in %ss" % (describe(res), len(refreshed), (after or {}).get("expires_in")))
        res = d.only(SESSION).get(reader, "/api/auth/get-session")
        after = session_row(session_token(d))
        set_ = [attrs for n, v, attrs in res.set_cookies() if n == SESSION and v]
        check("control: without dont_remember the reader refreshes the same row to seven days",
              res.status == 200 and len(set_) == 1 and set_[0].get("max-age") in ("604799", "604800")
              and after is not None and after["expires_in"] > 604000,
              "%s, cookie max-age %s, expires in %ss" % (describe(res), set_[0].get("max-age") if set_ else None,
                                                         (after or {}).get("expires_in")))
        for c in (a, d):
            c.post(writer, "/api/auth/sign-out")


# ── scenario 4: change-password ─────────────────────────────────────────────


def scenario_change_password():
    for writer, reader in DIRECTIONS:
        direction(writer, reader)
        acc = ACCOUNTS[writer]
        a, res = sign_in(writer, acc["email"], acc["password"])
        b, _ = sign_in(reader, acc["email"], acc["password"])
        new_pw = new_password()
        res = a.post(writer, "/api/auth/change-password", {
            "currentPassword": acc["password"], "newPassword": new_pw, "revokeOtherSessions": True})
        body = res.json() or {}
        check("writer change-password (revoking other sessions)",
              res.status == 200 and isinstance(body.get("token"), str), describe(res))
        _, res = sign_in(reader, acc["email"], acc["password"])
        check("reader refuses the old password", res.status == 401, describe(res))
        c, res = sign_in(reader, acc["email"], new_pw)
        check("reader signs in with the new password", res.status == 200, describe(res))
        expect_no_session("reader: the other session was revoked by the writer", b, reader)
        expect_session("reader accepts the session change-password issued", a, reader, acc["email"])
        acc["password"] = new_pw
        for x in (a, c):
            x.post(writer, "/api/auth/sign-out")


# ── scenario 5: two-factor ──────────────────────────────────────────────────


def second_factor(side, acc, kind, value, client=None, **extra):
    """Sign in on `side` (stops at the second factor), then pass it there."""
    c, res = sign_in(side, acc["email"], acc["password"], client=client)
    pending = res.status == 200 and (res.json() or {}).get("twoFactorRedirect") is True and TWO_FACTOR in c.jar
    path = "/api/auth/two-factor/verify-totp" if kind == "totp" else "/api/auth/two-factor/verify-backup-code"
    body = {"code": value}
    body.update(extra)
    res2 = c.post(side, path, body)
    return c, pending, res2


def scenario_two_factor():
    for writer, reader in DIRECTIONS:
        direction(writer, reader)
        em = fresh_email("2fa-" + writer)
        acc = {"email": em, "password": new_password()}
        a, res = sign_up(writer, em, acc["password"])
        check("sign-up for two-factor", res.status == 200, describe(res))
        res = a.post(writer, "/api/auth/two-factor/enable", {"password": acc["password"]})
        body = res.json() or {}
        uri, codes = body.get("totpURI"), body.get("backupCodes") or []
        check("writer enable returns a totpURI and ten backup codes",
              res.status == 200 and isinstance(uri, str) and uri.startswith("otpauth://totp/") and len(codes) == 10,
              describe(res))
        if not uri:
            continue
        key = totp_key(uri)
        row = DB.sql("SELECT t.verified, u.two_factor_enabled, t.secret ~ '^[0-9a-f]+$', t.backup_codes ~ '^[0-9a-f]+$' "
                     'FROM two_factor t JOIN "user" u ON u.id = t.user_id WHERE u.email = %s' % parity.sql_quote(em)).strip()
        check("writer stored an unverified, encrypted secret", row == "f\tf\tt\tt", "row %s" % row.replace("\t", ","))
        res = a.post(reader, "/api/auth/two-factor/verify-totp", {"code": totp(key)})
        check("reader verifies the first TOTP against the writer's secret", res.status == 200, describe(res))
        row = DB.sql('SELECT t.verified, u.two_factor_enabled FROM two_factor t JOIN "user" u ON u.id = t.user_id '
                     "WHERE u.email = %s" % parity.sql_quote(em)).strip()
        check("two-factor is now on", row == "t\tt", "row %s" % row.replace("\t", ","))
        expect_page("writer /dashboard/profile shows two-factor on", a, writer, em, twoFactorEnabled=True)

        # TOTP on each server.
        for side in (reader, writer):
            c, pending, res = second_factor(side, acc, "totp", totp(key))
            check("%s sign-in stops at the second factor" % LABEL[side], pending, "pending" if pending else "no")
            check("%s accepts a TOTP from the %s secret" % (LABEL[side], LABEL[writer]),
                  res.status == 200 and SESSION in c.jar, describe(res))
            expect_session("%s session accepted by %s" % (LABEL[side], LABEL[writer if side == reader else reader]),
                           c, writer if side == reader else reader, em)
        # Backup codes generated by the writer.
        c, pending, res = second_factor(reader, acc, "backup", codes[0])
        check("reader accepts a backup code the writer generated", pending and res.status == 200 and SESSION in c.jar,
              describe(res))
        c, pending, res = second_factor(writer, acc, "backup", codes[0])
        check("writer refuses the backup code the reader consumed", pending and res.status == 401, describe(res))
        c2, pending, res = second_factor(writer, acc, "backup", codes[1], client=c)
        check("writer accepts the next code from the list the reader re-encrypted", res.status == 200, describe(res))
        c, pending, res = second_factor(reader, acc, "backup", codes[1])
        check("reader refuses the code the writer consumed", pending and res.status == 401, describe(res))

        # A sign-in started on the writer, finished on the reader.
        c, res = sign_in(writer, em, acc["password"])
        res = c.post(reader, "/api/auth/two-factor/verify-totp", {"code": totp(key), "trustDevice": True})
        check("reader finishes the writer's pending sign-in (two_factor cookie, verification row)",
              res.status == 200 and SESSION in c.jar and TRUST_DEVICE in c.jar, describe(res))
        expect_session("writer accepts that session", c, writer, em)
        # The reader's trust_device cookie lets the writer skip the second factor.
        t = c.only(TRUST_DEVICE)
        _, res = sign_in(writer, em, acc["password"], client=t)
        body = res.json() or {}
        check("writer honours the reader's trusted device", res.status == 200 and not body.get("twoFactorRedirect")
              and (body.get("user") or {}).get("email") == em, describe(res))

        # Fresh backup codes on the reader, used on the writer.
        res = a.post(reader, "/api/auth/two-factor/generate-backup-codes", {"password": acc["password"]})
        fresh = (res.json() or {}).get("backupCodes") or []
        check("reader generate-backup-codes", res.status == 200 and len(fresh) == 10, describe(res))
        c, pending, res = second_factor(writer, acc, "backup", codes[2])
        check("writer refuses an old code after the reader regenerated", pending and res.status == 401, describe(res))
        if fresh:
            c, pending, res = second_factor(writer, acc, "backup", fresh[0])
            check("writer accepts a code the reader generated", pending and res.status == 200, describe(res))

        # Disable on the writer: the reader asks for no second factor.
        res = a.post(writer, "/api/auth/two-factor/disable", {"password": acc["password"]})
        check("writer disables two-factor", res.status == 200, describe(res))
        _, res = sign_in(reader, em, acc["password"])
        body = res.json() or {}
        check("reader signs in without a second factor", res.status == 200 and not body.get("twoFactorRedirect")
              and (body.get("user") or {}).get("email") == em, describe(res))
        expect_page("reader /dashboard/profile shows two-factor off", a, reader, em, twoFactorEnabled=False)


# ── scenario 6: mail settings and links ─────────────────────────────────────


class Sinks(object):
    """smtp_sink.mjs next to each server, both on 127.0.0.1:<port>."""

    def __init__(self):
        self.host_log = os.path.join(ARGS.scratch, "sink", "host-sink.jsonl")
        self.ts_log = "/tmp/crosscompat-sink.jsonl"
        self.seen = {"ts": 0, "rust": 0}
        self.proc = None

    def start(self):
        os.makedirs(os.path.dirname(self.host_log), exist_ok=True)
        self.proc = parity.start_rust_sink(os.path.join(HERE, "smtp_sink.mjs"), self.host_log, ARGS.sink_port, 1800)
        subprocess.run(["docker", "cp", os.path.join(HERE, "smtp_sink.mjs"), ARGS.app_container + ":/tmp/smtp_sink.mjs"],
                       check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["docker", "exec", "-d", ARGS.app_container, "node", "/tmp/smtp_sink.mjs", self.ts_log,
                        str(ARGS.sink_port), "1800"], check=True)
        time.sleep(1.5)

    def stop(self):
        if self.proc:
            self.proc.terminate()
            self.proc.wait()
        subprocess.run(["docker", "exec", ARGS.app_container, "sh", "-c", "kill $(cat %s.pid)" % self.ts_log],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def records(self, side):
        if side == "rust":
            try:
                with open(self.host_log) as f:
                    text = f.read()
            except OSError:
                text = ""
        else:
            text = subprocess.run(["docker", "exec", ARGS.app_container, "cat", self.ts_log], stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL).stdout.decode("utf-8")
        return [json.loads(line) for line in text.splitlines() if line.strip()]

    def wait(self, side, rcpt, timeout=20):
        """The next message `side`'s sink received for `rcpt`."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            recs = self.records(side)
            for i in range(self.seen[side], len(recs)):
                if any(rcpt in t for t in recs[i]["to"]):
                    self.seen[side] = i + 1
                    return recs[i]
            time.sleep(0.5)
        return None


SINKS = Sinks()


def mail_link(record, marker):
    msg = email.message_from_string(record["data"], policy=email.policy.default)
    part = msg.get_body(preferencelist=("plain", "html"))
    text = part.get_content() if part is not None else ""
    for url in re.findall(r"https?://[^\s\"'<>]+", text):
        if marker in url:
            return url.replace("&amp;", "&"), msg
    return None, msg


def follow_on(url, side):
    """The path and query of a link, to request from `side`."""
    u = urllib.parse.urlsplit(url)
    return u.path + ("?" + u.query if u.query else "")


def smtp_settings(from_name):
    return {"host": "127.0.0.1", "port": ARGS.sink_port, "security": "none", "user": "good-user",
            "password": "good-pass", "from": "no-reply@cross.test", "fromName": from_name}


def clear_mail(admin):
    for side in SERVERS:
        call_action(admin, side, "mail", "clearSmtpSettings", [])


def scenario_mail_links():
    if not ADMIN:
        check("mail links need the copied admin", False, "no admin session")
        return
    admin = admin_client()
    SINKS.start()
    try:
        for writer, reader in DIRECTIONS:
            direction(writer, reader)
            from_name = "Cross %s %s" % (LABEL[writer], TAG)
            env = call_action(admin, writer, "mail", "saveSmtpSettings", [smtp_settings(from_name)])
            ok = env.get("ok") if isinstance(env.get("ok"), dict) else {}
            check("writer saves SMTP settings with a password", ok.get("ok") is True, "envelope %s" % (
                "ok" if ok.get("ok") is True else json.dumps(env)[:200]))
            box = DB.sql("SELECT value ~ '^v1\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+$' FROM settings "
                         "WHERE key = 'smtp_password_enc'").strip()
            check("the SMTP password is stored as a v1 secret box", box == "t", "format %s" % ("ok" if box == "t" else box))
            # The reader may hold a 30 s snapshot of the old mail settings
            # (lib/mail-settings.ts TTL_MS), as the original would.
            time.sleep(31)

            # The verification link the writer mails at sign-up, followed on the reader.
            em = fresh_email("mail-" + writer)
            acc = {"email": em, "password": new_password()}
            c, res = sign_up(writer, em, acc["password"], callbackURL="/dashboard")
            check("sign-up on the writer", res.status == 200, describe(res))
            rec = SINKS.wait(writer, em)
            url, msg = mail_link(rec, "/api/auth/verify-email") if rec else (None, None)
            check("writer mails a verification link", url is not None and url.startswith(SERVERS[writer] + "/"),
                  "%s, link %s" % ("delivered" if rec else "nothing delivered",
                                   "points at the writer" if url and url.startswith(SERVERS[writer]) else "missing"))
            if url:
                f = Client()
                res = f.get(reader, follow_on(url, reader))
                check("reader follows the writer's verification link",
                      res.status == 302 and res.header("location") == "/dashboard" and SESSION in f.jar,
                      "%s -> %s" % (describe(res), res.header("location")))
                row = user_row(em, "email_verified")
                check("address confirmed", row == ["t"], "email_verified %s" % (row[0] if row else None))
                expect_session("writer accepts the reader's sign-in after verification", f, writer, em, emailVerified=True)

            # The reset link the writer mails, followed and used on the reader.
            res = Client().post(writer, "/api/auth/request-password-reset", {"email": em, "redirectTo": "/reset"})
            check("writer request-password-reset", res.status == 200 and (res.json() or {}).get("status") is True,
                  describe(res))
            rec = SINKS.wait(writer, em)
            url, _ = mail_link(rec, "/api/auth/reset-password/") if rec else (None, None)
            check("writer mails a reset link", url is not None and url.startswith(SERVERS[writer] + "/"),
                  "delivered" if rec else "nothing delivered")
            if url:
                f = Client()
                res = f.get(reader, follow_on(url, reader))
                loc = res.header("location") or ""
                token = urllib.parse.parse_qs(urllib.parse.urlsplit(loc).query).get("token", [None])[0]
                # better-auth resolves the callback against its own base URL.
                check("reader's reset link callback redirects to its /reset with the token",
                      res.status == 302 and loc.startswith(SERVERS[reader] + "/reset?") and bool(token),
                      "%s -> %s" % (describe(res), parity.normalize_text(re.sub(r"token=[^&]+", "token=<token>", loc))))
                new_pw = new_password()
                res = f.post(reader, "/api/auth/reset-password", {"newPassword": new_pw, "token": token})
                check("reader resets the password with the writer's token",
                      res.status == 200 and (res.json() or {}).get("status") is True, describe(res))
                _, res = sign_in(writer, em, acc["password"])
                check("writer refuses the old password", res.status == 401, describe(res))
                _, res = sign_in(writer, em, new_pw)
                check("writer signs in with the password set on the reader", res.status == 200, describe(res))
                acc["password"] = new_pw

            # The reader sends with the settings (and the encrypted password) the writer saved.
            res = Client().post(reader, "/api/auth/request-password-reset", {"email": em, "redirectTo": "/reset"})
            rec = SINKS.wait(reader, em)
            auth = (rec or {}).get("auth") or {}
            msg = email.message_from_string(rec["data"], policy=email.policy.default) if rec else None
            sender = str(msg.get("From")) if msg else ""
            check("reader mails with the writer's SMTP settings, password decrypted",
                  res.status == 200 and auth.get("ok") is True and auth.get("user") == "good-user" and from_name in sender
                  and "no-reply@cross.test" in (rec or {}).get("from", ""),
                  "%s, %s, AUTH %s, From %s" % (describe(res), "delivered" if rec else "nothing delivered",
                                                "ok" if auth.get("ok") else "missing or failed",
                                                "matches" if from_name in sender else "differs"))
        clear_mail(admin)
    finally:
        SINKS.stop()


# ── scenario 7: admin ───────────────────────────────────────────────────────


def scenario_admin():
    if not ADMIN:
        check("admin checks need the copied admin", False, "no admin session")
        return
    for writer, reader in DIRECTIONS:
        direction(writer, reader)
        admin = admin_client()
        em = fresh_email("admin-" + writer)
        acc = {"email": em, "password": new_password()}
        _, res = sign_up(writer, em, acc["password"])
        uid = user_id(em)
        t, res = sign_in(reader, em, acc["password"])
        check("target signed in on the reader", res.status == 200, describe(res))
        res = t.get(reader, "/api/auth/admin/list-users")
        check("reader: a user may not list users", res.status == 403, describe(res))

        res = admin.post(writer, "/api/auth/admin/set-role", {"userId": uid, "role": "admin"})
        check("writer set-role admin", res.status == 200, describe(res))
        expect_session("reader sees the new role", t, reader, em, role="admin")
        res = t.get(reader, "/api/auth/admin/list-users")
        check("reader lets the promoted user list users", res.status == 200, describe(res))
        res = admin.post(writer, "/api/auth/admin/set-role", {"userId": uid, "role": "user"})
        res = t.get(reader, "/api/auth/admin/list-users")
        check("reader refuses again after the writer demotes", res.status == 403, describe(res))

        # Impersonation by the writer, ended on the reader.
        res = admin.post(writer, "/api/auth/admin/impersonate-user", {"userId": uid})
        check("writer impersonate-user", res.status == 200, describe(res))
        _, body = expect_session("reader accepts the impersonation session", admin, reader, em)
        imp = ((body or {}).get("session") or {}).get("impersonatedBy")
        check("reader reports impersonatedBy", imp == ADMIN["id"], "impersonatedBy %s" % (
            "is the admin" if imp == ADMIN["id"] else repr(imp)))
        res = admin.post(reader, "/api/auth/admin/stop-impersonating")
        check("reader stop-impersonating", res.status == 200, describe(res))
        expect_session("writer: admin session restored", admin, writer, ADMIN["email"])

        # Ban on the writer: the reader drops the sessions and refuses sign-in.
        res = admin.post(writer, "/api/auth/admin/ban-user", {"userId": uid, "banReason": "cross " + TAG})
        check("writer ban-user", res.status == 200, describe(res))
        expect_no_session("reader: the banned user's session is gone", t, reader)
        _, res = sign_in(reader, em, acc["password"])
        check("reader refuses the banned user", res.status == 403 and (res.json() or {}).get("code") == "BANNED_USER",
              describe(res))
        res = admin.post(writer, "/api/auth/admin/unban-user", {"userId": uid})
        check("writer unban-user", res.status == 200, describe(res))
        t, res = sign_in(reader, em, acc["password"])
        check("reader lets the unbanned user in", res.status == 200, describe(res))
        # A ban that has run out: the reader lifts it at sign-in.
        res = admin.post(writer, "/api/auth/admin/ban-user", {"userId": uid, "banReason": "short", "banExpiresIn": 1})
        check("writer ban-user for one second", res.status == 200, describe(res))
        time.sleep(2.5)
        _, res = sign_in(reader, em, acc["password"])
        row = user_row(em, "banned")
        check("reader lets the user in after the ban ran out (and lifts it)", res.status == 200 and row == ["f"],
              "%s, banned %s" % (describe(res), row[0] if row else None))

        # Passwords an admin sets, accounts an admin creates and removes.
        new_pw = new_password()
        res = admin.post(writer, "/api/auth/admin/set-user-password", {"userId": uid, "newPassword": new_pw})
        check("writer set-user-password", res.status == 200, describe(res))
        _, res = sign_in(reader, em, acc["password"])
        check("reader refuses the password the admin replaced", res.status == 401, describe(res))
        t, res = sign_in(reader, em, new_pw)
        check("reader signs in with the password the admin set", res.status == 200, describe(res))
        made = fresh_email("created-" + writer)
        made_pw = new_password()
        res = admin.post(writer, "/api/auth/admin/create-user",
                         {"email": made, "name": "Created by %s" % LABEL[writer], "password": made_pw, "role": "user"})
        check("writer admin create-user", res.status == 200, describe(res))
        m, res = sign_in(reader, made, made_pw)
        check("reader signs in to the account the admin created", res.status == 200, describe(res))
        expect_page("reader /dashboard/profile for the created account", m, reader, made, role="user")
        res = admin.post(writer, "/api/auth/admin/remove-user", {"userId": user_id(made)})
        check("writer remove-user", res.status == 200, describe(res))
        expect_no_session("reader: the removed account's session is gone", m, reader)
        _, res = sign_in(reader, made, made_pw)
        check("reader refuses the removed account", res.status == 401, describe(res))


SCENARIOS = [scenario_schema, scenario_existing, scenario_sign_up_in_out, scenario_sessions, scenario_change_password,
             scenario_two_factor, scenario_mail_links, scenario_admin]


def main():
    print("cross-compatibility run %s: TS %s, Rust %s" % (TAG, SERVERS["ts"], SERVERS["rust"]), flush=True)
    needed = {"scenario_sessions": "scenario_sign_up_in_out", "scenario_change_password": "scenario_sign_up_in_out"}
    selected = [s for s in SCENARIOS if ARGS.only is None or ARGS.only in s.__name__]
    names = [s.__name__ for s in selected]
    for s in list(selected):
        dep = needed.get(s.__name__)
        if dep and dep not in names:
            selected.insert(selected.index(s), globals()[dep])
            names.append(dep)
    if any(s.__name__ in ("scenario_mail_links", "scenario_admin") for s in selected) and \
            "scenario_existing" not in names:
        selected.insert(0, scenario_existing)
    for s in selected:
        CONTEXT[0] = ""
        s()
    total = len(PASSED) + len(FAILED)
    print("\n%d/%d cross-compatibility checks passed" % (len(PASSED), total))
    if FAILED:
        print("failed:\n  " + "\n  ".join(FAILED))
        sys.exit(1)


main()

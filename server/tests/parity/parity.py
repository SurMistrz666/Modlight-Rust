"""Differential checks: the same requests against the reference (the original
TypeScript server) and the Rust server, responses compared after normalising
ids, tokens and timestamps (PORT.md section 13).

Every area has its own script next to this file (`auth.py`, `releases.py`,
...). They share this module:

    import parity
    run = parity.Run.from_args()             # --ref URL --rust URL (or PARITY_REF / PARITY_RUST)
    a = run.pair()                           # one browser per server (own cookie jar)
    a.check("get-session signed out", "GET", "/api/auth/get-session")
    ...
    run.finish()                             # prints a summary, exit code 1 on any mismatch

Conventions:
- Both servers run in production mode (NODE_ENV=production) on http, with
  APP_URL = their own origin; the Rust server with TRUST_PROXY=true, because
  the reference honours X-Real-IP unconditionally (PORT.md 12.7 keeps that
  only behind the flag). Every request sends `X-Real-IP` (a per-run value, so
  in-memory rate limits start fresh) and, for cookie-carrying writes, an
  `Origin` equal to the target's own APP_URL.
- State-changing scripts must not run against a shared reference
  (127.0.0.1:8420): use a scratch reference stack of your own.
- Database helpers go through `docker exec <container> psql` (no psql on the
  host). Never print secrets: the scripts read passwords from env files and
  only ever show names.

Only the Python standard library; Python 3.9.
"""

import argparse
import difflib
import http.client
import json
import os
import random
import re
import subprocess
import sys
import urllib.parse

ISO_RE = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?Z")
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
HTTP_DATE_RE = re.compile(r"[A-Z][a-z]{2}, \d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} GMT")


class Response(object):
    def __init__(self, status, reason, headers, body):
        self.status = status
        self.reason = reason
        self.headers = headers  # list of (lower-case name, value)
        self.body = body

    def header(self, name):
        name = name.lower()
        for k, v in self.headers:
            if k == name:
                return v
        return None

    def header_all(self, name):
        name = name.lower()
        return [v for k, v in self.headers if k == name]

    def json(self):
        if not self.body:
            return None
        try:
            return json.loads(self.body.decode("utf-8"))
        except ValueError:
            return None

    def text(self):
        return self.body.decode("utf-8", "replace")

    def set_cookies(self):
        """[(name, value, attrs-dict)] in order."""
        out = []
        for raw in self.header_all("set-cookie"):
            parts = [p.strip() for p in raw.split(";")]
            name, _, value = parts[0].partition("=")
            attrs = {}
            for p in parts[1:]:
                k, _, v = p.partition("=")
                attrs[k.lower()] = v
            out.append((name, value, attrs))
        return out


class Browser(object):
    """One client of one server: base URL, cookie jar, default headers."""

    def __init__(self, base, origin=None, real_ip=None, user_agent="parity/1"):
        self.base = base.rstrip("/")
        u = urllib.parse.urlparse(self.base)
        self.host = u.hostname
        self.port = u.port or 80
        self.origin = origin or self.base
        self.real_ip = real_ip
        self.user_agent = user_agent
        self.cookies = {}

    def request(self, method, path, headers=None, json_body=None, form=None, raw=None, cookies=True, origin=True):
        hdrs = {"User-Agent": self.user_agent, "Accept": "*/*"}
        if self.real_ip:
            hdrs["X-Real-IP"] = self.real_ip
        if origin and self.origin:
            hdrs["Origin"] = self.origin
        body = None
        if json_body is not None:
            body = json.dumps(json_body).encode("utf-8")
            hdrs["Content-Type"] = "application/json"
        elif form is not None:
            body = urllib.parse.urlencode(form).encode("utf-8")
            hdrs["Content-Type"] = "application/x-www-form-urlencoded"
        elif raw is not None:
            body = raw if isinstance(raw, bytes) else raw.encode("utf-8")
        if cookies and self.cookies:
            hdrs["Cookie"] = "; ".join("%s=%s" % (k, v) for k, v in self.cookies.items())
        for k, v in (headers or {}).items():
            if v is None:
                hdrs.pop(k, None)
                for existing in list(hdrs):
                    if existing.lower() == k.lower():
                        hdrs.pop(existing)
            else:
                hdrs[k] = v
        conn = http.client.HTTPConnection(self.host, self.port, timeout=60)
        conn.request(method, path, body=body, headers=hdrs)
        r = conn.getresponse()
        data = r.read()
        res = Response(r.status, r.reason, [(k.lower(), v) for k, v in r.getheaders()], data)
        conn.close()
        for name, value, attrs in res.set_cookies():
            max_age = attrs.get("max-age")
            if value == "" or max_age == "0":
                self.cookies.pop(name, None)
            else:
                self.cookies[name] = value
        return res

    def rpc(self, module, export, args, headers=None):
        h = {"X-Modlight-Rpc": "1", "Content-Type": "application/json"}
        h.update(headers or {})
        return self.request("POST", "/_rpc/%s/%s" % (module, export), headers=h, raw=json.dumps(args))


def normalize_text(s, extra=None):
    s = ISO_RE.sub("<iso>", s)
    s = UUID_RE.sub("<uuid>", s)
    s = HTTP_DATE_RE.sub("<http-date>", s)
    for pattern, repl in extra or []:
        s = re.sub(pattern, repl, s)
    return s


def normalize_json(v, ignore=(), redact=(), path=""):
    """Replace volatile values: ISO dates, uuids; `ignore` keys are dropped,
    `redact` keys keep their presence and type but lose their value."""
    if isinstance(v, dict):
        out = {}
        for k, x in v.items():
            if k in ignore:
                continue
            if k in redact:
                out[k] = "<%s>" % type(x).__name__ if x is not None else None
                continue
            out[k] = normalize_json(x, ignore, redact, path + "." + k)
        return out
    if isinstance(v, list):
        return [normalize_json(x, ignore, redact, path) for x in v]
    if isinstance(v, str):
        return normalize_text(v)
    return v


def cookie_shape(res, volatile_values=True):
    """Set-Cookie headers as comparable tuples: name, value (or a placeholder),
    attributes without volatile dates, attribute ORDER kept."""
    out = []
    for raw in res.header_all("set-cookie"):
        parts = [p.strip() for p in raw.split(";")]
        name, _, value = parts[0].partition("=")
        if volatile_values and value:
            value = "<value len=%d>" % len(value)
        attrs = []
        for p in parts[1:]:
            k, _, v = p.partition("=")
            if k.lower() == "expires":
                v = "<date>"
            if k.lower() == "max-age" and v not in ("0",):
                n = int(v) if v.lstrip("-").isdigit() else v
                # allow a second of drift between the two servers
                v = "<%d>" % (round(n / 10.0) * 10) if isinstance(n, int) else v
            attrs.append("%s=%s" % (k, v) if "=" in p else k)
        out.append((name, value, attrs))
    return out


class Run(object):
    def __init__(self, ref, rust, real_ip=None, verbose=False):
        self.ref = ref
        self.rust = rust
        self.real_ip = real_ip or "198.51.100.%d" % random.randint(1, 254)
        self.verbose = verbose
        self.failures = []
        self.passed = 0

    @staticmethod
    def from_args(argv=None):
        p = argparse.ArgumentParser()
        p.add_argument("--ref", default=os.environ.get("PARITY_REF", "http://localhost:8424"))
        p.add_argument("--rust", default=os.environ.get("PARITY_RUST", "http://localhost:8423"))
        p.add_argument("--ip", default=None, help="X-Real-IP to send (default: random per run)")
        p.add_argument("-v", "--verbose", action="store_true")
        a, _ = p.parse_known_args(argv)
        return Run(a.ref, a.rust, a.ip, a.verbose)

    def pair(self, user_agent="parity/1", real_ip=None):
        ip = real_ip or self.real_ip
        return Pair(self, Browser(self.ref, real_ip=ip, user_agent=user_agent),
                    Browser(self.rust, real_ip=ip, user_agent=user_agent))

    def compare(self, name, a, b):
        if a == b:
            self.passed += 1
            if self.verbose:
                print("PASS %s" % name)
            return True
        sa = json.dumps(a, indent=2, sort_keys=False, ensure_ascii=False, default=str).splitlines()
        sb = json.dumps(b, indent=2, sort_keys=False, ensure_ascii=False, default=str).splitlines()
        diff = "\n".join(difflib.unified_diff(sa, sb, "reference", "rust", lineterm=""))
        self.failures.append((name, diff))
        print("FAIL %s\n%s" % (name, diff))
        return False

    def finish(self):
        total = self.passed + len(self.failures)
        print("\n%d/%d checks passed" % (self.passed, total))
        if self.failures:
            print("failed: " + ", ".join(n for n, _ in self.failures))
            sys.exit(1)


class Pair(object):
    """The same browser on both servers."""

    def __init__(self, run, ref, rust):
        self.run = run
        self.ref = ref
        self.rust = rust

    def both(self, method, path, **kw):
        return self.ref.request(method, path, **kw), self.rust.request(method, path, **kw)

    def check(self, name, method, path, body="json", headers_to_compare=("content-type",), ignore=(), redact=(),
              cookies=True, reason=True, text_extra=None, **kw):
        a, b = self.both(method, path, **kw)
        self.compare_responses(name, a, b, body=body, headers_to_compare=headers_to_compare, ignore=ignore,
                               redact=redact, cookies=cookies, reason=reason, text_extra=text_extra)
        return a, b

    def rpc(self, name, module, export, args, ignore=(), redact=(), **kw):
        a = self.ref.rpc(module, export, args, **kw)
        b = self.rust.rpc(module, export, args, **kw)
        self.compare_responses(name, a, b, ignore=ignore, redact=redact, headers_to_compare=("content-type",))
        return a, b

    def compare_responses(self, name, a, b, body="json", headers_to_compare=("content-type",), ignore=(), redact=(),
                          cookies=True, reason=True, text_extra=None):
        def shape(r):
            d = {"status": r.status}
            if reason:
                d["reason"] = r.reason
            for h in headers_to_compare:
                v = r.header(h)
                d[h] = normalize_text(v) if v is not None else None
            if cookies:
                d["set-cookie"] = cookie_shape(r)
            if body == "json":
                d["body"] = normalize_json(r.json(), ignore, redact) if r.body else "<empty>"
                if r.body and r.json() is None:
                    d["body"] = normalize_text(r.text(), text_extra)
            elif body == "text":
                d["body"] = normalize_text(r.text(), text_extra)
            elif body == "empty":
                d["body"] = "<empty>" if not r.body else "<%d bytes>" % len(r.body)
            return d

        return self.run.compare(name, shape(a), shape(b))


def psql(container, database, sql, user="modlight"):
    """Run SQL in a Postgres container; returns stdout (tab-separated rows)."""
    out = subprocess.run(
        ["docker", "exec", "-i", container, "psql", "-U", user, "-d", database, "-v", "ON_ERROR_STOP=1", "-AtF", "\t",
         "-c", sql],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if out.returncode != 0:
        raise RuntimeError("psql failed: %s" % out.stderr.decode("utf-8", "replace"))
    return out.stdout.decode("utf-8")


def read_env_file(path):
    """KEY=VALUE lines (values are never printed by these scripts)."""
    values = {}
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, _, v = line.partition("=")
            values[k.strip()] = v.strip().strip('"').strip("'")
    return values


# ── Seeding identical state on both servers ────────────────────────────────


def sql_quote(v):
    if v is None:
        return "NULL"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if isinstance(v, (int, float)):
        return repr(v)
    return "'" + str(v).replace("'", "''") + "'"


def random_id(n=32):
    """Better Auth's generateId alphabet."""
    alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
    return "".join(random.SystemRandom().choice(alphabet) for _ in range(n))


class Db(object):
    """One server's database, through `docker exec <container> psql`."""

    def __init__(self, container, database, auth_secret):
        self.container = container
        self.database = database
        self.auth_secret = auth_secret  # never printed

    def sql(self, statement):
        return psql(self.container, self.database, statement)

    def create_user(self, user_id, email, name, role="user", user_code=None, email_verified=True, **columns):
        """Insert a `user` row the way Better Auth would (theme 'modern',
        quota defaults). Extra columns by their SQL names."""
        cols = {
            "id": user_id, "name": name, "email": email.lower(), "email_verified": email_verified,
            "role": role, "banned": False, "two_factor_enabled": False, "user_code": user_code,
            "used_quota": 0, "max_quota": -1, "hide_uploads": False, "copy_raw": False, "theme": "modern",
        }
        cols.update(columns)
        names = ", ".join('"%s"' % k for k in cols)
        values = ", ".join(sql_quote(v) for v in cols.values())
        self.sql('INSERT INTO "user" (%s, "created_at", "updated_at") VALUES (%s, now(), now())' % (names, values))

    def mint_session(self, user_id, ip="", user_agent="parity/1", token=None):
        """Insert a session row and return the signed cookie VALUE for
        `better-auth.session_token` (URI-encoded, as a browser sends it)."""
        import base64
        import hashlib
        import hmac
        token = token or random_id(32)
        self.sql(
            'INSERT INTO "session" ("id", "token", "user_id", "expires_at", "created_at", "updated_at", '
            '"ip_address", "user_agent") VALUES (%s, %s, %s, now() + interval \'7 days\', now(), now(), %s, %s)'
            % (sql_quote(random_id(32)), sql_quote(token), sql_quote(user_id), sql_quote(ip), sql_quote(user_agent)))
        sig = base64.b64encode(hmac.new(self.auth_secret.encode(), token.encode(), hashlib.sha256).digest()).decode()
        return urllib.parse.quote(token + "." + sig, safe="")

    def create_api_token(self, user_id, token, scope="upload", name="parity"):
        import hashlib
        digest = hashlib.sha256(token.encode()).hexdigest()
        self.sql(
            'INSERT INTO "api_tokens" ("user_id", "name", "scope", "token_hash", "preview") VALUES (%s, %s, %s, %s, %s)'
            % (sql_quote(user_id), sql_quote(name), sql_quote(scope), sql_quote(digest), sql_quote(token[-6:])))


SESSION_COOKIE = "better-auth.session_token"


def signed_in(pair, ref_db, rust_db, user_id):
    """Give both browsers of `pair` a fresh session for the same user id."""
    pair.ref.cookies[SESSION_COOKIE] = ref_db.mint_session(user_id, ip=pair.ref.real_ip or "")
    pair.rust.cookies[SESSION_COOKIE] = rust_db.mint_session(user_id, ip=pair.rust.real_ip or "")

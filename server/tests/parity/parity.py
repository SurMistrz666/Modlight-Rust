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


# ── Server actions: Next's protocol on the reference, /_rpc on Rust ────────


class Undefined(object):
    """JavaScript `undefined` as an action argument or result."""

    def __repr__(self):
        return "undefined"

    def __eq__(self, other):
        return isinstance(other, Undefined)


UNDEFINED = Undefined()


class JsDate(object):
    def __init__(self, iso):
        self.iso = iso

    def __eq__(self, other):
        return isinstance(other, JsDate) and other.iso == self.iso


class ActionIds(object):
    """`(module, export) -> (action id, a page path whose bundle includes it)`
    from the reference's `.next/server/server-reference-manifest.json`."""

    def __init__(self, app_container):
        out = subprocess.run(["docker", "exec", app_container, "cat",
                              "/app/.next/server/server-reference-manifest.json"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
        manifest = json.loads(out.stdout.decode("utf-8"))
        self.ids = {}
        for action_id, entry in manifest["node"].items():
            workers = entry["workers"]
            first = list(workers.values())[0]
            module = first["filename"].rsplit("/", 1)[-1][:-3]
            self.ids[(module, first["exportedName"])] = (action_id, self._page_path(list(workers.keys())[0]))

    @staticmethod
    def _page_path(worker):
        parts = []
        for seg in worker.split("/")[1:-1]:
            if seg.startswith("(") and seg.endswith(")"):
                continue
            if seg.startswith("["):
                seg = "x"
            parts.append(seg)
        return "/" + "/".join(parts)

    def get(self, module, export):
        return self.ids[(module, export)]


def _encode_reply(v):
    """React `encodeReply` for the JSON-like values actions take."""
    if isinstance(v, Undefined):
        return "$undefined"
    if isinstance(v, JsDate):
        return "$D" + v.iso
    if isinstance(v, float) and v != v:
        return "$NaN"
    if v == float("inf"):
        return "$Infinity"
    if v == float("-inf"):
        return "$-Infinity"
    if isinstance(v, str):
        return "$" + v if v.startswith("$") else v
    if isinstance(v, list):
        return [_encode_reply(x) for x in v]
    if isinstance(v, dict):
        return dict((k, _encode_reply(x)) for k, x in v.items())
    return v


def _encode_tagged(v):
    """The SPA's tagged JSON (CONTRACT 3)."""
    if isinstance(v, Undefined):
        return {"$undefined": 1}
    if isinstance(v, JsDate):
        return {"$date": v.iso}
    if isinstance(v, float) and v != v:
        return {"$num": "NaN"}
    if v == float("inf"):
        return {"$num": "Infinity"}
    if v == float("-inf"):
        return {"$num": "-Infinity"}
    if isinstance(v, list):
        return [_encode_tagged(x) for x in v]
    if isinstance(v, dict):
        return dict((k, _encode_tagged(x)) for k, x in v.items())
    return v


def _decode_flight(v, rows):
    if isinstance(v, str) and v.startswith("$"):
        if v.startswith("$$"):
            return v[1:]
        if v == "$undefined":
            return "<undefined>"
        if v.startswith("$D"):
            return "<date:%s>" % v[2:]
        if v in ("$NaN", "$Infinity", "$-Infinity"):
            return "<num:%s>" % v[1:]
        if v.startswith("$n"):
            return "<bigint:%s>" % v[2:]
        if v.startswith("$@") or re.match(r"^\$[0-9a-f]+$", v):
            key = v[2:] if v.startswith("$@") else v[1:]
            return _decode_flight(rows.get(key), rows)
        return v
    if isinstance(v, list):
        return [_decode_flight(x, rows) for x in v]
    if isinstance(v, dict):
        return dict((k, _decode_flight(x, rows)) for k, x in v.items())
    return v


def _decode_tagged(v):
    if isinstance(v, dict) and len(v) == 1:
        (k, x), = v.items()
        if k == "$undefined":
            return "<undefined>"
        if k == "$date":
            return "<date:%s>" % x
        if k == "$num":
            return "<num:%s>" % x
        if k == "$bigint":
            return "<bigint:%s>" % x
    if isinstance(v, list):
        return [_decode_tagged(x) for x in v]
    if isinstance(v, dict):
        return dict((k, _decode_tagged(x)) for k, x in v.items())
    return v


MASKED = "<masked error>"


def next_action_envelope(res):
    """A Next server-action response as the /_rpc envelope it corresponds to,
    with dates as `<date:iso>`, undefined as `<undefined>`."""
    env = {}
    redirect = res.header("x-action-redirect")
    if redirect:
        env["redirect"] = redirect.split(";")[0]
    elif res.header("content-type", ) and "text/x-component" in res.header("content-type"):
        rows = {}
        for line in res.text().split("\n"):
            if ":" not in line:
                continue
            key, _, payload = line.partition(":")
            if payload.startswith("E"):
                rows[key] = ("error", json.loads(payload[1:]))
            else:
                try:
                    rows[key] = json.loads(payload)
                except ValueError:
                    rows[key] = payload
        head = rows.get("0", {})
        ref = head.get("a") if isinstance(head, dict) else None
        key = ref[2:] if isinstance(ref, str) and ref.startswith("$@") else None
        value = rows.get(key) if key else None
        if isinstance(value, tuple) and value[0] == "error":
            env["error"] = MASKED
        else:
            plain = dict((k, v) for k, v in rows.items() if not isinstance(v, tuple))
            env["ok"] = _decode_flight(value, plain)
    else:
        env["http"] = res.status
    revalidated = res.header("x-action-revalidated")
    if revalidated and revalidated not in ("[[],0,0]", "0"):
        env["revalidate"] = True
    return env


def rpc_envelope(res):
    """A Rust /_rpc response in the same normalised form."""
    body = res.json()
    if not isinstance(body, dict):
        return {"http": res.status}
    env = {}
    if "redirect" in body:
        env["redirect"] = body["redirect"]
    elif "error" in body:
        env["error"] = MASKED
    elif "notFound" in body:
        env["notFound"] = True
    elif "ok" in body:
        env["ok"] = _decode_tagged(body["ok"])
    else:
        env["http"] = res.status
    if body.get("revalidate"):
        env["revalidate"] = True
    return env


class Actions(object):
    """Call the same server action on both servers and compare."""

    def __init__(self, run, ref_app_container):
        self.run = run
        self.ids = ActionIds(ref_app_container)

    def call_ref(self, browser, module, export, args):
        action_id, page = self.ids.get(module, export)
        body = json.dumps(_encode_reply(list(args)))
        return browser.request("POST", page, raw=body, headers={
            "Next-Action": action_id, "Content-Type": "text/plain;charset=UTF-8", "Accept": "text/x-component"})

    def call_rust(self, browser, module, export, args):
        return browser.rpc(module, export, _encode_tagged(list(args)))

    def check(self, pair, name, module, export, args, ignore=(), redact=(), cookies=True):
        a = self.call_ref(pair.ref, module, export, args)
        b = self.call_rust(pair.rust, module, export, args)
        ea = normalize_json(next_action_envelope(a), ignore, redact)
        eb = normalize_json(rpc_envelope(b), ignore, redact)
        if cookies:
            ea["set-cookie"] = [(n, v, [x for x in attrs if not x.lower().startswith("expires")])
                                for n, v, attrs in cookie_shape(a)]
            eb["set-cookie"] = [(n, v, [x for x in attrs if not x.lower().startswith("expires")])
                                for n, v, attrs in cookie_shape(b)]
        self.run.compare(name, ea, eb)
        return a, b

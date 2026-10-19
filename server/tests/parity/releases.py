#!/usr/bin/env python3
"""Parity checks for the app release channel and the Integrations page:

    GET/POST /api/releases/app, GET /download/app,
    releases.loadRelease / publishAppRelease / unpublishAppRelease,
    the tokens actions' argument edges,
    the /download and /dashboard/tokens documents and page data.

    python3 server/tests/parity/releases.py --ref http://localhost:8425 --rust http://localhost:8433

The reference is a scratch stack of the ORIGINAL server (never the shared one
on 8420): every run seeds its own users, tokens and uploads (fresh ids, the
same on both sides) and moves the global `release_app` setting. Flags or
environment (defaults are part R's environment):

    --ref-app / PARITY_REF_APP     reference app container (action ids)
    --ref-pg / PARITY_REF_PG       reference Postgres container
    --ref-db / PARITY_REF_DB       reference database
    --ref-env / PARITY_REF_ENV     reference .env (AUTH_SECRET; never printed)
    --rust-pg / PARITY_RUST_PG     dev Postgres container of the Rust server
    --rust-db / PARITY_RUST_DB     Rust database
    --rust-env / PARITY_RUST_ENV   Rust env file (AUTH_SECRET; never printed)

Values that legitimately differ are normalised before comparing: each server's
own APP_URL (`http://localhost:<port>`) becomes `<app-url>`, plus ISO dates,
uuids and HTTP dates (parity.py). Security fix 4 (PORT.md 12) makes
`POST /api/releases/app` refuse bodies the reference accepted; those requests
are checked against the documented outcome of EACH side instead of against
each other (`[fix 4]` checks).

Checks that need the `(app)` layout loader (another helper's work) are
reported as SKIP while the Rust server answers 501 for them.
"""

import argparse
import html as htmlmod
import json
import os
import random
import re
import string
import sys
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402
from parity import UNDEFINED  # noqa: E402

SCRATCH = os.environ.get("PARITY_SCRATCH", os.path.expanduser("~/Desktop/modlight/_scratch/srv-auth-r"))

NOT_ADMIN = {"message": "Administrator credentials required."}
MASKED = parity.MASKED


def options():
    # allow_abbrev=False: `--ref` / `--rust` belong to parity.Run, not to us.
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--ref-app", default=os.environ.get("PARITY_REF_APP", "modlight-srvauth-r-ref-app-1"))
    p.add_argument("--ref-pg", default=os.environ.get("PARITY_REF_PG", "modlight-srvauth-r-ref-postgres-1"))
    p.add_argument("--ref-db", default=os.environ.get("PARITY_REF_DB", "modlight"))
    p.add_argument("--ref-env", default=os.environ.get("PARITY_REF_ENV", os.path.join(SCRATCH, "ref", ".env")))
    p.add_argument("--rust-pg", default=os.environ.get("PARITY_RUST_PG", "modlight-devdb-postgres-1"))
    p.add_argument("--rust-db", default=os.environ.get("PARITY_RUST_DB", "modlight_srv_auth_r"))
    p.add_argument("--rust-env", default=os.environ.get("PARITY_RUST_ENV", os.path.join(SCRATCH, "rust.env")))
    p.add_argument("--dump", default=None, help="write every compared pair (normalised) to this JSON file")
    a, _ = p.parse_known_args()
    return a


def app_url_of(base):
    """APP_URL of a parity server: `http://localhost:<port>` (parity-server.sh, the compose file)."""
    return "http://localhost:%d" % (urllib.parse.urlparse(base).port or 80)


class Checker(object):
    """Comparisons with each side's APP_URL normalised, and the expected
    differences of security fix 4."""

    def __init__(self, run, ref_db, rust_db, actions, dump=None):
        self.run = run
        self.dumped = [] if dump else None
        if dump:
            compare = run.compare

            def recording(name, a, b):
                self.dumped.append({"name": name, "reference": a, "rust": b})
                return compare(name, a, b)

            run.compare = recording
        self.db = {"ref": ref_db, "rust": rust_db}
        self.urls = {"ref": app_url_of(run.ref), "rust": app_url_of(run.rust)}
        self.actions = actions
        self.skipped = []

    # ── normalisation ──

    def text(self, s, side):
        if s is None:
            return None
        return parity.normalize_text(s.replace(self.urls[side], "<app-url>"))

    def value(self, v, side, ignore=(), redact=()):
        raw = json.dumps(v, ensure_ascii=False).replace(self.urls[side], "<app-url>")
        return parity.normalize_json(json.loads(raw), ignore, redact)

    def shape(self, res, side, body="json", headers=("content-type", "cache-control"), ignore=(), redact=()):
        d = {"status": res.status, "reason": res.reason}
        for h in headers:
            d[h] = self.text(res.header(h), side)
        d["set-cookie"] = parity.cookie_shape(res)
        if body == "json":
            parsed = res.json()
            if not res.body:
                d["body"] = "<empty>"
            elif parsed is None:
                d["body"] = self.text(res.text(), side)
            else:
                d["body"] = self.value(parsed, side, ignore, redact)
        elif body == "text":
            d["body"] = self.text(res.text(), side)
        elif body == "empty":
            d["body"] = "<empty>" if not res.body else "<%d bytes>" % len(res.body)
        return d

    # ── HTTP ──

    def check(self, pair, name, method, path, body="json", compare=("content-type", "cache-control"), ignore=(),
              redact=(), **kw):
        """`compare`: the response headers to compare; `**kw` goes to the
        request (`headers`, `raw`, ...)."""
        a, b = pair.both(method, path, **kw)
        self.run.compare(name, self.shape(a, "ref", body, compare, ignore, redact),
                         self.shape(b, "rust", body, compare, ignore, redact))
        return a, b

    def check_fix4(self, pair, name, method, path, ref_expected, rust_expected, **kw):
        """Security fix 4: the reference answers what upstream answered, Rust
        what the shared body rules answer. Both are pinned."""
        a, b = pair.both(method, path, **kw)

        def pick(res, side):
            d = {"status": res.status, "cache-control": res.header("cache-control")}
            parsed = res.json()
            d["body"] = self.value(parsed, side) if parsed is not None else (
                "<empty>" if not res.body else self.text(res.text(), side))
            return d

        self.run.compare(name + " [fix 4: reference]", pick(a, "ref"), ref_expected)
        self.run.compare(name + " [fix 4: rust]", pick(b, "rust"), rust_expected)
        return a, b

    # ── server actions ──

    def action(self, pair, name, module, export, args, ignore=(), redact=()):
        a = self.actions.call_ref(pair.ref, module, export, args)
        b = self.actions.call_rust(pair.rust, module, export, args)
        ea = self.value(parity.next_action_envelope(a), "ref", ignore, redact)
        eb = self.value(parity.rpc_envelope(b), "rust", ignore, redact)
        for env, res in ((ea, a), (eb, b)):
            env["set-cookie"] = [(n, v, [x for x in attrs if not x.lower().startswith("expires")])
                                 for n, v, attrs in parity.cookie_shape(res)]
        self.run.compare(name, ea, eb)
        return ea, eb

    # ── database ──

    def sql_both(self, statement):
        return self.db["ref"].sql(statement), self.db["rust"].sql(statement)

    def check_sql(self, name, statement):
        a, b = self.sql_both(statement)
        self.run.compare(name, self.text(a, "ref"), self.text(b, "rust"))
        return a, b

    def skip(self, name, why):
        self.skipped.append((name, why))
        print("SKIP %s: %s" % (name, why))


# ── documents ──

def head_of(text):
    """The `<title>` and the named metas a link preview reads."""
    title = re.search(r"<title>(.*?)</title>", text, re.S)
    metas = {}
    for m in re.finditer(r'<meta\s+name="([^"]+)"\s+content="([^"]*)"\s*/?>', text):
        metas.setdefault(m.group(1), htmlmod.unescape(m.group(2)))
    return {
        "title": htmlmod.unescape(title.group(1)) if title else None,
        "description": metas.get("description"),
        "application-name": metas.get("application-name"),
        "robots": metas.get("robots"),
    }


def flight_of(text):
    """The RSC payload the reference inlines (`self.__next_f.push([1,"..."])`)."""
    chunks = re.findall(r'self\.__next_f\.push\(\[1,("(?:[^"\\]|\\.)*")\]\)', text)
    return "".join(json.loads(c) for c in chunks)


def props_with(flight, first_key, must_have):
    """The first client-component props object in `flight` that starts with
    `first_key` and has every key of `must_have`."""
    decoder = json.JSONDecoder()
    for m in re.finditer(r'\{"%s":' % re.escape(first_key), flight):
        try:
            obj, _ = decoder.raw_decode(flight, m.start())
        except ValueError:
            continue
        if all(k in obj for k in must_have):
            return obj
    return None


def page_data(browser, path):
    return browser.request("GET", "/_page?path=" + urllib.parse.quote(path, safe=""))


def document(browser, path):
    return browser.request("GET", path, headers={"Accept": "text/html"})


def doc_shape(c, res, side):
    return {
        "status": res.status,
        "content-type": res.header("content-type"),
        "cache-control": res.header("cache-control"),
        "location": c.text(res.header("location"), side),
        "head": head_of(res.text()) if res.status == 200 else None,
    }


# ── seeding ──

def rand(n=6):
    return "".join(random.SystemRandom().choice(string.ascii_lowercase + string.digits) for _ in range(n))


def insert_upload(db, row):
    cols = ["id", "owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "checksum", "published",
            "delete_token", "password_hash"]
    values = [row.get(c) for c in cols]
    extra = []
    if row.get("expires"):
        extra.append(("expires_at", "now() + interval '30 days'"))
    if row.get("deleted"):
        extra.append(("deleted_at", "now()"))
    names = ", ".join('"%s"' % c for c in cols + [e[0] for e in extra])
    vals = ", ".join([parity.sql_quote(v) for v in values] + [e[1] for e in extra])
    db.sql('INSERT INTO "uploads" (%s) VALUES (%s)' % (names, vals))


def seed(c):
    """Users, tokens and uploads with the same ids on both sides."""
    r = rand()
    s = {"r": r}
    s["admin"] = "prl-adm-" + r
    s["admin2"] = "prl-ad2-" + r  # skip_tag_in_url: tagless links
    s["admin3"] = "prl-ad3-" + r  # no tag: the "u" fallback
    s["banned"] = "prl-ban-" + r
    s["user"] = "prl-usr-" + r
    for side in ("ref", "rust"):
        db = c.db[side]
        db.create_user(s["admin"], s["admin"] + "@parity.test", "Release Admin", role="admin", user_code="pa" + r)
        db.create_user(s["admin2"], s["admin2"] + "@parity.test", "Second Admin", role="admin", user_code="pb" + r,
                       skip_tag_in_url=True)
        db.create_user(s["admin3"], s["admin3"] + "@parity.test", "Third Admin", role="admin", user_code=None,
                       preferred_domain="https://unknown.example")
        db.create_user(s["banned"], s["banned"] + "@parity.test", "Banned Admin", role="admin", user_code="pc" + r,
                       banned=True)
        # An unknown preferred domain is never echoed back: links use APP_URL.
        db.create_user(s["user"], s["user"] + "@parity.test", "Parity User Name", role="user", user_code="pu" + r,
                       preferred_domain="https://unknown.example/")
    s["tok_full"] = "prlfull" + rand(25)
    s["tok_write"] = "prlwrite" + rand(24)
    s["tok_user_full"] = "prlufull" + rand(24)
    s["tok_banned"] = "prlban" + rand(26)
    s["tok_user_upload"] = "prluup" + rand(26)
    for side in ("ref", "rust"):
        db = c.db[side]
        db.create_api_token(s["admin"], s["tok_full"], "full", name="release script")
        db.create_api_token(s["admin"], s["tok_write"], "write", name="desktop app")
        db.create_api_token(s["user"], s["tok_user_full"], "full", name="everything")
        db.create_api_token(s["banned"], s["tok_banned"], "full")
        db.create_api_token(s["user"], s["tok_user_upload"], "upload", name="ShareX")
    digest = "%064x" % random.SystemRandom().getrandbits(256)
    s["digest"] = digest
    u = {}

    def up(key, owner, filename, ext, **kw):
        code = "pr" + key + r
        u[key] = code
        row = {
            "id": "00000000-0000-4000-8000-%012x" % random.SystemRandom().getrandbits(48),
            "owner_id": owner, "code": code, "filename": filename, "ext": ext,
            "mime_type": "application/octet-stream", "size": kw.get("size", 82317446),
            "storage_key": "x/%s.%s" % (code, ext), "checksum": kw.get("checksum"),
            "published": kw.get("published", True), "delete_token": "dt" + code + rand(8),
            "password_hash": kw.get("password_hash"), "expires": kw.get("expires"), "deleted": kw.get("deleted"),
        }
        for side in ("ref", "rust"):
            insert_upload(c.db[side], row)

    up("ok", s["admin"], "Modlight-0.7.0-setup-x64.exe", "exe", checksum=digest)
    up("nn", s["admin"], "installer.exe", "exe", checksum=digest, size=1234)
    up("un", s["admin"], "Modlight-0.7.0-setup-x64.exe", "exe", published=False)
    up("pw", s["admin"], "Modlight-0.7.0-setup-x64.exe", "exe", password_hash="scrypt$AAAA$BBBB")
    up("ex", s["admin"], "Modlight-0.7.0-setup-x64.exe", "exe", expires=True)
    up("or", None, "Modlight-0.7.0-setup-x64.exe", "exe")
    up("tr", s["admin"], "Modlight-0.7.0-setup-x64.exe", "exe", deleted=True)
    up("tl", s["admin2"], "Modlight 1.2.3 setup", "", size=5)
    up("nt", s["admin3"], "Modlight_v2.0_x64.zip", "zip", checksum=digest)
    s["uploads"] = u
    return s


def reset_release(c):
    c.sql_both("INSERT INTO \"settings\" (\"key\", \"value\") VALUES ('release_app', NULL) "
               "ON CONFLICT (\"key\") DO UPDATE SET \"value\" = NULL")


def bearer(token):
    return {"Authorization": "Bearer " + token}


def post_json(body, extra=None):
    h = {"Content-Type": "application/json"}
    h.update(extra or {})
    return {"raw": json.dumps(body), "headers": h}


# ── the checks ──

def manifest_checks(c, anon, label):
    c.check(anon, "%s: GET /api/releases/app" % label, "GET", "/api/releases/app")
    c.check(anon, "%s: HEAD /api/releases/app" % label, "HEAD", "/api/releases/app", body="empty")
    c.check(anon, "%s: GET /download/app" % label, "GET", "/download/app", body="text",
            compare=("content-type", "cache-control", "location", "x-release-version", "x-release-sha256"))
    c.check(anon, "%s: HEAD /download/app" % label, "HEAD", "/download/app", body="empty",
            compare=("content-type", "cache-control", "location", "x-release-version", "x-release-sha256"))


def download_page_checks(c, anon, label):
    a, b = document(anon.ref, "/download"), document(anon.rust, "/download")
    c.run.compare("%s: /download document" % label, doc_shape(c, a, "ref"), doc_shape(c, b, "rust"))
    # The props: `currentRelease()`, the same call GET /api/releases/app answers
    # on the reference, whose rendered page must show that release.
    manifest = anon.ref.request("GET", "/api/releases/app")
    expected = manifest.json() if manifest.status == 200 else None
    data = page_data(anon.rust, "/download").json() or {}
    c.run.compare("%s: /download props" % label, c.value({"release": expected}, "ref"),
                  c.value(data.get("props"), "rust"))
    c.run.compare("%s: /download meta" % label, head_of(a.text()), {
        "title": data.get("meta", {}).get("title"),
        "description": data.get("meta", {}).get("description"),
        "application-name": data.get("meta", {}).get("applicationName"),
        "robots": data.get("meta", {}).get("robots"),
    })
    if expected:
        page = a.text()
        shown = {
            "download link": 'href="/download/app"' in page,
            "sha256": expected["sha256"] is None or expected["sha256"] in page,
            "publishedAt": not expected["publishedAt"] or ('dateTime="%s"' % expected["publishedAt"]) in page,
            "notes": not expected["notes"] or htmlmod.escape(expected["notes"], quote=False) in page,
        }
        c.run.compare("%s: reference /download renders that release" % label, shown,
                      dict((k, True) for k in shown))


def post_refusals(c, s, anon, user_pair, admin_pair):
    u = s["uploads"]
    body = {"version": "0.7.0", "code": u["ok"]}
    path = "/api/releases/app"
    c.check(anon, "POST without credentials", "POST", path, **post_json(body))
    c.check(anon, "POST with an unknown token", "POST", path, **post_json(body, bearer("nope-" + s["r"])))
    c.check(anon, "POST with an empty x-api-key falls through to the session", "POST", path,
            **post_json(body, {"x-api-key": ""}))
    c.check(anon, "POST with a user's full token (role gate)", "POST", path,
            **post_json(body, {"x-api-key": s["tok_user_full"]}))
    c.check(anon, "POST with an admin's write token (scope gate)", "POST", path,
            **post_json(body, bearer(s["tok_write"])))
    c.check(anon, "POST with a user's upload token", "POST", path, **post_json(body, bearer(s["tok_user_upload"])))
    c.check(anon, "POST with a banned admin's full token", "POST", path, **post_json(body, bearer(s["tok_banned"])))
    c.check(user_pair, "POST with a user session", "POST", path, **post_json(body))
    # A token wins over the session and is never a fall-through to it.
    c.check(admin_pair, "POST with an admin session and a bad token", "POST", path,
            **post_json(body, bearer("nope-" + s["r"])))

    full = bearer(s["tok_full"])
    for name, b in [
        ("{}", {}),
        ("version not a string", {"version": 7, "code": u["ok"]}),
        ("code not a string", {"version": "0.7.0", "code": None}),
        ("blank version", {"version": "  ", "code": u["ok"]}),
        ("blank code", {"version": "0.7.0", "code": " "}),
        ("bad version 8", {"version": "8", "code": u["ok"]}),
        ("bad version 0.7.1-beta", {"version": "0.7.1-beta", "code": u["ok"]}),
        ("unknown code", {"version": "0.7.0", "code": "nope" + s["r"]}),
        ("trashed upload", {"version": "0.7.0", "code": u["tr"]}),
        ("unpublished upload", {"version": "0.7.0", "code": u["un"]}),
        ("password-protected upload", {"version": "0.7.0", "code": u["pw"]}),
        ("the name says 0.7.0", {"version": "0.7.1", "code": u["ok"]}),
        ("allowVersionMismatch must be the literal true", {"version": "0.7.1", "code": u["ok"],
                                                           "allowVersionMismatch": "true"}),
        ("a link is not a code for the API", {"version": "0.7.0",
                                              "code": "http://x/t/%s.exe" % u["ok"]}),
    ]:
        c.check(anon, "POST refused: " + name, "POST", path, **post_json(b, full))

    # Security fix 4: bodies the reference read with req.json().
    missing = {"version": "0.7.0", "code": "nope" + s["r"]}
    c.check_fix4(anon, "POST text/plain body", "POST", path,
                 {"status": 404, "cache-control": "no-store",
                  "body": {"message": "No such upload.", "reason": "not-found"}},
                 {"status": 415, "cache-control": "no-store",
                  "body": {"message": "Content-Type must be application/json."}},
                 raw=json.dumps(missing), headers=dict(full, **{"Content-Type": "text/plain"}))
    c.check_fix4(anon, "POST without a content type", "POST", path,
                 {"status": 404, "cache-control": "no-store",
                  "body": {"message": "No such upload.", "reason": "not-found"}},
                 {"status": 415, "cache-control": "no-store",
                  "body": {"message": "Content-Type must be application/json."}},
                 raw=json.dumps(missing), headers=full)
    c.check_fix4(anon, "POST invalid JSON", "POST", path,
                 {"status": 400, "cache-control": "no-store", "body": {"message": "Expected a JSON body."}},
                 {"status": 400, "cache-control": "no-store", "body": {"message": "Body must be a JSON object."}},
                 raw="{not json", headers=dict(full, **{"Content-Type": "application/json"}))
    c.check_fix4(anon, "POST JSON array", "POST", path,
                 {"status": 400, "cache-control": "no-store", "body": {"message": "version and code are required."}},
                 {"status": 400, "cache-control": "no-store", "body": {"message": "Body must be a JSON object."}},
                 raw="[]", headers=dict(full, **{"Content-Type": "application/json"}))
    c.check_fix4(anon, "POST JSON null", "POST", path,
                 {"status": 500, "cache-control": None, "body": "<empty>"},
                 {"status": 400, "cache-control": "no-store", "body": {"message": "Body must be a JSON object."}},
                 raw="null", headers=dict(full, **{"Content-Type": "application/json"}))
    big = dict(missing, notes="n" * 20000)
    c.check_fix4(anon, "POST over 16 KiB", "POST", path,
                 {"status": 404, "cache-control": "no-store",
                  "body": {"message": "No such upload.", "reason": "not-found"}},
                 {"status": 413, "cache-control": "no-store",
                  "body": {"message": "Body must be at most 16384 bytes."}},
                 **post_json(big, full))
    # The same rules for a request that is refused anyway: authentication first.
    c.check(anon, "POST text/plain without credentials (401 before the body)", "POST", path,
            raw="{not json", headers={"Content-Type": "text/plain"})


def audit_rows(c, s):
    return ("SELECT \"user_id\", \"action\", \"detail\"::text, \"ip\" FROM \"audit_log\" "
            "WHERE \"action\" = 'release.published' AND \"user_id\" IN (%s, %s) ORDER BY \"created_at\""
            % (parity.sql_quote(s["admin"]), parity.sql_quote(s["admin2"])))


SETTING = "SELECT \"value\" FROM \"settings\" WHERE \"key\" = 'release_app'"


def post_success(c, s, anon, admin_pair):
    u = s["uploads"]
    path = "/api/releases/app"
    c.check(anon, "POST publish by token (v prefix, padded code, notes)", "POST", path,
            **post_json({"version": " v0.7.0 ", "code": "  %s " % u["ok"], "notes": "  First build.\nEnjoy.  "},
                        bearer(s["tok_full"])))
    c.check_sql("release_app after a token publish", SETTING)
    manifest_checks(c, anon, "published 0.7.0")
    download_page_checks(c, anon, "published 0.7.0")
    c.check(admin_pair, "POST publish by session, mismatch confirmed", "POST", path,
            **post_json({"version": "0.7.1", "code": u["ok"], "allowVersionMismatch": True, "notes": 5}))
    c.check_sql("release_app after a session publish (non-string notes dropped)", SETTING)
    c.check(anon, "POST publish by x-api-key, no version in the name", "POST", path,
            **post_json({"version": "1.2", "code": u["nn"], "notes": "   "}, {"x-api-key": s["tok_full"]}))
    c.check_sql("release_app with blank notes", SETTING)
    manifest_checks(c, anon, "published 1.2")
    # Accepted by publishRelease, refused by currentRelease: the pointer stays
    # written and the channel reads as empty.
    c.check(anon, "POST an expiring upload: not-public, pointer written", "POST", path,
            **post_json({"version": "0.7.0", "code": u["ex"]}, bearer(s["tok_full"])))
    c.check_sql("release_app names the expiring upload", SETTING)
    manifest_checks(c, anon, "dangling (expiring)")
    c.check(anon, "POST an orphaned upload: not-public", "POST", path,
            **post_json({"version": "0.7.0", "code": u["or"]}, bearer(s["tok_full"])))
    manifest_checks(c, anon, "dangling (orphaned)")
    c.check_sql("release.published audit rows", audit_rows(c, s))


def release_actions(c, s, admin_pair, user_pair, nobody_pair):
    u = s["uploads"]
    for export, args in (("loadRelease", []), ("publishAppRelease", [{"version": "1.0", "code": u["ok"]}]),
                         ("unpublishAppRelease", [])):
        c.action(nobody_pair, "releases.%s without a session" % export, "releases", export, args)
        c.action(user_pair, "releases.%s as a user" % export, "releases", export, args)
    reset_release(c)
    c.action(admin_pair, "releases.loadRelease, nothing published", "releases", "loadRelease", [])
    link = "  http://localhost/pa%s/%s.exe?dl=1#x " % (s["r"], u["ok"])
    for name, args in [
        ("blank version", [{"version": "", "code": u["ok"]}]),
        ("blank code", [{"version": "0.7.0", "code": "   "}]),
        ("bad version", [{"version": "8", "code": u["ok"]}]),
        ("unknown code", [{"version": "0.7.0", "code": "nope" + s["r"]}]),
        ("trashed", [{"version": "0.7.0", "code": u["tr"]}]),
        ("unpublished", [{"version": "0.7.0", "code": u["un"]}]),
        ("password", [{"version": "0.7.0", "code": u["pw"]}]),
        ("version mismatch from a pasted link", [{"version": "0.7.1", "code": link}]),
        ("mismatch, allowVersionMismatch 1 is not true", [{"version": "0.7.1", "code": link,
                                                          "allowVersionMismatch": 1}]),
        ("non-string notes on an unknown code: the refusal wins", [{"version": "0.7.0", "code": "nope", "notes": 5}]),
        ("non-string notes on a mismatch: the refusal wins", [{"version": "0.7.1", "code": u["ok"], "notes": []}]),
        ("non-string notes reaching the write: TypeError", [{"version": "0.7.0", "code": u["ok"], "notes": 5}]),
        ("missing version: TypeError", [{"code": u["ok"]}]),
        ("non-string code: TypeError", [{"version": "0.7.0", "code": 5}]),
        ("date code: TypeError", [{"version": "0.7.0", "code": parity.JsDate("2026-10-18T12:00:00.000Z")}]),
        ("no input: TypeError", []),
        ("undefined input: TypeError", [UNDEFINED]),
        ("null input: TypeError", [None]),
        ("string input: TypeError", ["0.7.0"]),
    ]:
        c.action(admin_pair, "releases.publishAppRelease " + name, "releases", "publishAppRelease", args)
    c.check_sql("release_app untouched by refusals", SETTING)
    c.action(admin_pair, "releases.publishAppRelease mismatch confirmed (link, notes)", "releases",
             "publishAppRelease", [{"version": "v0.7.1", "code": link, "notes": " Hotfix ",
                                    "allowVersionMismatch": True}])
    c.check_sql("release_app after the action", SETTING)
    c.action(admin_pair, "releases.loadRelease, published", "releases", "loadRelease", [])
    # "Modlight 1.2.3 setup": only the LAST dot starts the extension, so the
    # name says 1.2 (".3 setup" is cut), and 1.2.3 is a mismatch.
    c.action(admin_pair, "releases.publishAppRelease name read up to its last dot", "releases",
             "publishAppRelease", [{"version": "1.2.3", "code": u["tl"]}])
    c.action(admin_pair, "releases.publishAppRelease tagless owner, no extension, no checksum", "releases",
             "publishAppRelease", [{"version": "1.2", "code": u["tl"]}])
    manifest_checks(c, admin_pair, "tagless 1.2")
    c.action(admin_pair, "releases.publishAppRelease owner without a tag", "releases", "publishAppRelease",
             [{"version": "2.0.0.0", "code": u["nt"], "notes": "zip"}])
    manifest_checks(c, admin_pair, "untagged 2.0")
    download_page_checks(c, admin_pair, "untagged 2.0")
    c.action(admin_pair, "releases.publishAppRelease expiring: not-public", "releases", "publishAppRelease",
             [{"version": "0.7.0", "code": u["ex"]}])
    manifest_checks(c, admin_pair, "dangling after the action")
    c.action(admin_pair, "releases.publishAppRelease 0.7 for the 0.7.0 build", "releases", "publishAppRelease",
             [{"version": "0.7", "code": u["ok"]}])
    c.action(admin_pair, "releases.unpublishAppRelease", "releases", "unpublishAppRelease", [])
    c.check_sql("release_app after unpublish", SETTING)
    manifest_checks(c, admin_pair, "unpublished")
    download_page_checks(c, admin_pair, "unpublished")
    c.action(admin_pair, "releases.unpublishAppRelease again", "releases", "unpublishAppRelease", [])


def token_actions(c, s, user_pair):
    """The edges of app/actions/tokens.ts that the Rust port now follows."""
    c.action(user_pair, "tokens.myTokens", "tokens", "myTokens", [])
    for name, args in [
        ("revokeUploadToken(undefined)", ("revokeUploadToken", [UNDEFINED])),
        ("revokeUploadToken(null)", ("revokeUploadToken", [None])),
        ("revokeUploadToken()", ("revokeUploadToken", [])),
        ("revokeUploadToken(5)", ("revokeUploadToken", [5])),
        ("revokeUploadToken(not a uuid)", ("revokeUploadToken", ["nope"])),
        ("revokeUploadToken(unknown uuid)", ("revokeUploadToken", ["00000000-0000-4000-8000-000000000000"])),
        ("narrowTokenScope(5, bogus)", ("narrowTokenScope", [5, "bogus"])),
        ("narrowTokenScope(null, read)", ("narrowTokenScope", [None, "read"])),
        ("narrowTokenScope(undefined, read)", ("narrowTokenScope", [UNDEFINED, "read"])),
        ("narrowTokenScope(5, read)", ("narrowTokenScope", [5, "read"])),
        ("narrowTokenScope(not a uuid, read)", ("narrowTokenScope", ["nope", "read"])),
        ("narrowTokenScope(unknown uuid, read)", ("narrowTokenScope", ["00000000-0000-4000-8000-000000000000",
                                                                       "read"])),
    ]:
        export, args = args
        c.action(user_pair, "tokens." + name, "tokens", export, args)
    c.action(user_pair, "tokens.createUploadToken(undefined)", "tokens", "createUploadToken", [UNDEFINED],
             redact=("token",))
    c.action(user_pair, "tokens.createUploadToken(5)", "tokens", "createUploadToken", [5])
    c.action(user_pair, "tokens.myTokens after creating one", "tokens", "myTokens", [],
             redact=("tokenHash", "preview"))
    # Key order: drizzle's select() order, not the table's.
    a = c.actions.call_ref(user_pair.ref, "tokens", "myTokens", [])
    b = c.actions.call_rust(user_pair.rust, "tokens", "myTokens", [])
    ka = [list(r.keys()) for r in (parity.next_action_envelope(a).get("ok") or [])]
    kb = [list(r.keys()) for r in (parity.rpc_envelope(b).get("ok") or [])]
    c.run.compare("tokens.myTokens key order", ka, kb)


def tokens_page_checks(c, s, user_pair, label, redact=()):
    a, b = document(user_pair.ref, "/dashboard/tokens"), document(user_pair.rust, "/dashboard/tokens")
    if b.status == 501:
        c.skip("%s: /dashboard/tokens document" % label, "the Rust server has no (app) layout loader yet (501)")
    else:
        c.run.compare("%s: /dashboard/tokens document" % label, doc_shape(c, a, "ref"), doc_shape(c, b, "rust"))
    data = page_data(user_pair.rust, "/dashboard/tokens")
    payload = data.json() or {}
    if data.status == 501:
        c.skip("%s: /dashboard/tokens page data" % label, "the Rust server has no (app) layout loader yet (501)")
        return
    props = payload.get("props") or {}
    flight = flight_of(a.text())
    manager = props_with(flight, "tokens", ("appUrl", "userCode", "userName", "userEmail")) or {}
    sharex = props_with(flight, "appUrl", ("username", "showDomainNote")) or {}
    ref_view = {
        "tokens": manager.get("tokens"),
        "appUrl": manager.get("appUrl"),
        "username": sharex.get("username"),
        "user": {"userCode": manager.get("userCode"), "name": manager.get("userName"),
                 "email": manager.get("userEmail")},
    }
    rust_view = dict((k, props.get(k)) for k in ("tokens", "appUrl", "username", "user"))
    c.run.compare("%s: /dashboard/tokens props" % label, c.value(ref_view, "ref", redact=redact),
                  c.value(rust_view, "rust", redact=redact))
    # <AppCard> is a server component upstream: its release is the manifest.
    manifest = user_pair.ref.request("GET", "/api/releases/app")
    expected = manifest.json() if manifest.status == 200 else None
    c.run.compare("%s: /dashboard/tokens release" % label, c.value(expected, "ref"),
                  c.value(props.get("release"), "rust"))
    has_download = 'href="/download/app"' in a.text()
    c.run.compare("%s: reference AppCard offers the download" % label, has_download, expected is not None)
    meta = payload.get("meta") or {}
    c.run.compare("%s: /dashboard/tokens meta" % label, head_of(a.text()), {
        "title": meta.get("title"), "description": meta.get("description"),
        "application-name": meta.get("applicationName"), "robots": meta.get("robots")})


def main():
    opts = options()
    run = parity.Run.from_args()
    ref_secret = parity.read_env_file(opts.ref_env)["AUTH_SECRET"]
    rust_secret = parity.read_env_file(opts.rust_env)["AUTH_SECRET"]
    ref_db = parity.Db(opts.ref_pg, opts.ref_db, ref_secret)
    rust_db = parity.Db(opts.rust_pg, opts.rust_db, rust_secret)
    c = Checker(run, ref_db, rust_db, parity.Actions(run, opts.ref_app), dump=opts.dump)

    s = seed(c)
    reset_release(c)

    anon = run.pair()
    admin_pair = run.pair()
    parity.signed_in(admin_pair, ref_db, rust_db, s["admin"])
    user_pair = run.pair()
    parity.signed_in(user_pair, ref_db, rust_db, s["user"])
    # A cookie that passes proxy.ts's optimistic gate but names no session, so
    # the reference runs the action and requireAdmin() redirects to /login.
    nobody = run.pair()
    for b in (nobody.ref, nobody.rust):
        b.cookies[parity.SESSION_COOKIE] = "garbage"

    manifest_checks(c, anon, "nothing published")
    download_page_checks(c, anon, "nothing published")
    c.check(anon, "POST /download/app", "POST", "/download/app", body="empty", compare=("content-type", "allow"))
    c.check(anon, "OPTIONS /api/releases/app", "OPTIONS", "/api/releases/app", body="empty",
            compare=("content-type", "allow"))
    c.check(anon, "PUT /api/releases/app", "PUT", "/api/releases/app", body="empty", compare=("allow",))
    c.action(admin_pair, "releases.loadRelease as admin, nothing published", "releases", "loadRelease", [])
    tokens_page_checks(c, s, user_pair, "nothing published")

    post_refusals(c, s, anon, user_pair, admin_pair)
    post_success(c, s, anon, admin_pair)
    release_actions(c, s, admin_pair, user_pair, nobody)

    c.action(admin_pair, "releases.publishAppRelease for the Integrations page", "releases", "publishAppRelease",
             [{"version": "0.7.0", "code": s["uploads"]["ok"], "notes": "For the page"}])
    tokens_page_checks(c, s, user_pair, "published")
    token_actions(c, s, user_pair)
    # createUploadToken minted a different random token on each side.
    tokens_page_checks(c, s, user_pair, "after the token actions", redact=("preview",))

    reset_release(c)
    if opts.dump:
        with open(opts.dump, "w") as f:
            json.dump(c.dumped, f, indent=1, ensure_ascii=False, default=str)
    if c.skipped:
        print("\n%d skipped: %s" % (len(c.skipped), ", ".join(n for n, _ in c.skipped)))
    run.finish()


if __name__ == "__main__":
    main()

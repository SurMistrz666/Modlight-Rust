"""Parity: the desktop-app authorize flow (CONTRACT 6.10 and 7.4 `authorize`).

    python3 server/tests/parity/authorize.py --ref http://localhost:8427 --rust http://localhost:8435

- The `/authorize` document for invalid and valid queries, signed out (the
  307 to `/login?next=...` and its exact encoding) and signed in: status,
  `Location`, head (title, description, application-name, robots), the
  security headers with the `/authorize` CSP (origins normalised: the
  reference's CSP carries its BUILD-time APP_URL), and the props. The
  reference has no `/_page`, so its props are read from the RSC payload in the
  HTML (the `AuthorizePrompt` props, or the "invalid" card); Rust's from the
  inlined `__MODLIGHT_PAGE__` payload, and `/_page` is checked to agree.
- `approveDesktopClient` through both protocols: signed out, invalid inputs,
  successes (the `api_tokens` row and the audit row re-read from both
  databases), the cut-surrogate name (token issued, no audit row), and the
  rate limit after 10 approvals.

Seeds its own users (same ids on both sides, new ones every run). Database
access goes through `docker exec ... psql`; secrets are read from the env
files and never printed.
"""

import argparse
import hashlib
import json
import os
import random
import re
import sys
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

SCRATCH = os.environ.get("PARITY_SCRATCH", "/path/to/modlight/_scratch/srv-auth-m")


def env_args():
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--ref-app", default="modlight-srvauth-m-ref-app-1")
    p.add_argument("--ref-db-container", default="modlight-srvauth-m-ref-postgres-1")
    p.add_argument("--ref-db", default="modlight")
    p.add_argument("--ref-env", default=os.path.join(SCRATCH, "ref", ".env"))
    p.add_argument("--rust-db-container", default="modlight-devdb-postgres-1")
    p.add_argument("--rust-db", default="modlight_srv_auth_m")
    p.add_argument("--rust-env", default=os.path.join(SCRATCH, "rust.env"))
    a, _ = p.parse_known_args()
    return a


ARGS = env_args()
run = parity.Run.from_args()
REF_DB = parity.Db(ARGS.ref_db_container, ARGS.ref_db, parity.read_env_file(ARGS.ref_env)["AUTH_SECRET"])
RUST_DB = parity.Db(ARGS.rust_db_container, ARGS.rust_db, parity.read_env_file(ARGS.rust_env)["AUTH_SECRET"])
TAG = "".join(random.choice("abcdefghijklmnopqrstuvwxyz") for _ in range(6))

ORIGIN_RE = re.compile(r"http://localhost:\d+")
SECURITY_HEADERS = ("content-type", "cache-control", "x-content-type-options", "referrer-policy",
                    "strict-transport-security", "x-frame-options", "permissions-policy",
                    "content-security-policy")
# The invalid card's heading as rendered (the dictionaries themselves are in
# every RSC payload, so the bare text proves nothing).
INVALID_CARDS = tuple('"children":"%s"' % t for t in ("This link is not a valid request",
                                                      "Ten link nie jest prawidłową prośbą"))


def both_db(fn):
    return fn(REF_DB), fn(RUST_DB)


def seed_user(key, role="user", email=None):
    user_id = "pm%s%s" % (TAG, key)
    email = email or "%s.%s@authorize.test" % (key, TAG)
    for db in (REF_DB, RUST_DB):
        db.create_user(user_id, email, "Parity %s" % key, role=role)
    return user_id, email


# ── documents ───────────────────────────────────────────────────────────────


def rsc_text(html):
    chunks = re.findall(r'self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)', html)
    return "".join(json.loads('"' + c + '"') for c in chunks)


def head_of(html):
    out = {}
    m = re.search(r"<title>(.*?)</title>", html, re.S)
    out["title"] = m.group(1) if m else None
    for name in ("description", "application-name", "robots"):
        m = re.search(r'<meta name="%s" content="([^"]*)"' % name, html)
        out[name] = m.group(1) if m else None
    return out


def ref_props(html):
    """The props as the reference rendered them: the invalid card, or the
    AuthorizePrompt's `account` and `request` (mapped to the port's shape)."""
    text = rsc_text(html)
    if any(t in text for t in INVALID_CARDS):
        return {"parsed": {"ok": False}, "session": None}
    i = text.find('"account":{"email":')
    if i < 0:
        return {"unrecognised": True}
    start = text.rfind('{"appName":', 0, i)
    obj, _ = json.JSONDecoder().raw_decode(text, start)
    req = obj["request"]
    return {
        "parsed": {"ok": True, "request": {"port": int(req["port"]), "state": req["state"],
                                           "clientName": req["clientName"], "callback": req["callback"]}},
        "session": {"user": {"email": obj["account"]["email"]}},
    }


def rust_payload(html):
    m = re.search(r'<script id="__MODLIGHT_PAGE__" type="application/json">(.*?)</script>', html, re.S)
    return json.loads(m.group(1)) if m else None


def doc_shape(res, side):
    d = {"status": res.status, "location": res.header("location")}
    for h in SECURITY_HEADERS:
        if h == "content-type" and res.status == 307:
            # Next sends its HTML error shell with a page's redirect; the
            # skeleton's document redirect has no body (pages/mod.rs).
            continue
        v = res.header(h)
        d[h] = ORIGIN_RE.sub("<origin>", v) if v else v
    if res.status == 200:
        html = res.text()
        d["head"] = head_of(html)
        if side == "ref":
            d["props"] = ref_props(html)
        else:
            payload = rust_payload(html) or {}
            props = json.loads(json.dumps(payload.get("props")))
            if props and props.get("parsed", {}).get("ok") is False:
                d["reason"] = props["parsed"].pop("reason", None)
            d["props"] = props
            d["route"] = payload.get("route")
    return d


def check_document(pair, name, query, expect_reason=None, headers=None, ref_unicode=None):
    path = "/authorize" + (("?" + query) if query else "")
    a = pair.ref.request("GET", path, headers=dict(headers or {}, Accept="text/html"), origin=False)
    b = pair.rust.request("GET", path, headers=dict(headers or {}, Accept="text/html"), origin=False)
    sa, sb = doc_shape(a, "ref"), doc_shape(b, "rust")
    reason = sb.pop("reason", None)
    route = sb.pop("route", None)
    if ref_unicode and "props" in sa:
        # The reference's name ends in a lone surrogate the port shows as U+FFFD.
        req = sa["props"].get("parsed", {}).get("request")
        if req:
            req["clientName"] = ref_unicode(req["clientName"])
    run.compare("document " + name, sa, sb)
    if b.status == 200:
        run.compare("document route " + name, "/authorize", route)
    if expect_reason is not None:
        run.compare("document reason " + name, expect_reason, reason)
    # /_page agrees with the document on the Rust side.
    page = pair.rust.request("GET", "/_page?path=%2Fauthorize" + (("&" + query) if query else ""),
                             headers=headers, origin=False)
    pj = page.json() or {}
    if b.status == 307:
        run.compare("/_page redirect " + name, {"redirect": b.header("location"), "status": 307}, pj)
    else:
        doc = rust_payload(b.text()) or {}
        run.compare("/_page props " + name, doc.get("props"), pj.get("props"))
    return a, b


def documents():
    pair = run.pair()
    # Invalid requests: the invalid card, no session lookup, no redirect.
    for name, query, reason in [
        ("no query", "", "invalid-port"),
        ("privileged port", "port=80&state=abcdefgh", "invalid-port"),
        ("leading zero", "port=08080&state=abcdefgh", "invalid-port"),
        ("hex port", "port=0x1f90&state=abcdefgh", "invalid-port"),
        ("port 65536", "port=65536&state=abcdefgh", "invalid-port"),
        ("repeated port", "port=8080&port=9090&state=abcdefgh", "invalid-port"),
        ("short state", "port=8080&state=short", "invalid-state"),
        ("repeated state", "port=8080&state=abcdefgh&state=ijklmnop", "invalid-state"),
        ("state with a space", "port=8080&state=abcd+efgh", "invalid-state"),
        ("state with a quote", "port=8080&state=abcdefgh%22", "invalid-state"),
        ("state 129", "port=8080&state=" + "a" * 129, "invalid-state"),
    ]:
        check_document(pair, "invalid: " + name, query, expect_reason=reason)
    check_document(pair, "invalid (pl)", "port=1&state=x", expect_reason="invalid-port",
                   headers={"Accept-Language": "pl-PL,pl;q=0.9"})

    # Valid requests, signed out: 307 to /login?next=<encodeURIComponent(authorizePath)>.
    surrogate_cut = "a" * 63 + "\U0001F600"
    for name, query in [
        ("plain", "port=8080&state=abcdefgh"),
        ("name with a bidi override", "port=8080&state=abcdefgh&name=My%20App%E2%80%AE!"),
        ("state with a tilde", "port=8080&state=abc~def.gh_-"),
        ("padded port", "port=%208080&state=abcdefgh"),
        ("padded state", "port=49732&state=%20s7Kd93jf0aQ2%0A"),
        ("unicode name", "port=49732&state=s7Kd93jf0aQ2&name=" + urllib.parse.quote("Zażółć gęślą jaźń")),
        ("name of spaces", "port=49732&state=s7Kd93jf0aQ2&name=%20%20%20"),
        ("name with newlines", "port=49732&state=s7Kd93jf0aQ2&name=Modlight%0A%0A%0ADesktop"),
        ("name with symbols", "port=49732&state=s7Kd93jf0aQ2&name=" + urllib.parse.quote("a+b*c'd(e)~f!g")),
        ("long name", "port=49732&state=s7Kd93jf0aQ2&name=" + "x" * 300),
        ("name cut in a surrogate pair", "port=49732&state=s7Kd93jf0aQ2&name=" + urllib.parse.quote(surrogate_cut)),
        ("repeated name", "port=49732&state=s7Kd93jf0aQ2&name=a&name=b"),
        ("max port", "port=65535&state=" + "Z" * 128),
    ]:
        check_document(pair, "signed out: " + name, query)

    # Signed in: the prompt with the account and the parsed request.
    user_id, email = seed_user("doc")
    parity.signed_in(pair, REF_DB, RUST_DB, user_id)
    fffd = lambda s: s[:-1] + "�" if s and "\ud800" <= s[-1] <= "\udbff" else s  # noqa: E731
    for name, query in [
        ("plain", "port=8080&state=abcdefgh"),
        ("named", "port=49732&state=s7Kd93jf0aQ2&name=Modlight%20Desktop"),
        ("bidi name", "port=49732&state=s7Kd93jf0aQ2&name=evil%E2%80%AE%20moc.live"),
        ("long name", "port=49732&state=s7Kd93jf0aQ2&name=" + "y" * 300),
        ("name cut in a surrogate pair", "port=49732&state=s7Kd93jf0aQ2&name=" + urllib.parse.quote(surrogate_cut)),
    ]:
        check_document(pair, "signed in: " + name, query, ref_unicode=fffd)
    check_document(pair, "signed in (pl)", "port=8080&state=abcdefgh", headers={"Accept-Language": "pl"})
    check_document(pair, "signed in, invalid", "port=8080", expect_reason="invalid-state")

    # Paths next to /authorize: their CSP (the blanket rule must not match
    # /authorize/...). Only the header: both are media pages, not ported here.
    for path in ("/authorize/x", "/authorized"):
        a, b = pair.both("GET", path, headers={"Accept": "text/html"}, origin=False)
        sa, sb = [ORIGIN_RE.sub("<origin>", r.header("content-security-policy") or "") for r in (a, b)]
        run.compare("neighbour CSP " + path, sa, sb)


# ── approveDesktopClient ────────────────────────────────────────────────────


def ok_value(env):
    return env.get("ok") if isinstance(env.get("ok"), dict) else None


def tokens_of(a, b):
    ea, eb = parity.next_action_envelope(a), parity.rpc_envelope(b)
    return (ok_value(ea) or {}).get("token"), (ok_value(eb) or {}).get("token")


def last_token_row(db, user_id):
    out = db.sql("SELECT name, scope, token_hash, preview, last_used_at IS NULL, revoked_at IS NULL "
                 "FROM api_tokens WHERE user_id = %s ORDER BY created_at DESC, id LIMIT 1" % parity.sql_quote(user_id))
    return out.rstrip("\n").split("\t") if out.strip() else None


def audit_rows(db, user_id, action):
    out = db.sql("SELECT detail::text, ip FROM audit_log WHERE user_id = %s AND action = %s ORDER BY created_at, id"
                 % (parity.sql_quote(user_id), parity.sql_quote(action)))
    return [line.split("\t") for line in out.splitlines() if line]


def check_issue(acts, pair, name, user_id, payload, expect_audit=True):
    before = both_db(lambda db: len(audit_rows(db, user_id, "api_token.authorized")))
    a, b = acts.check(pair, "approve " + name, "authorize", "approveDesktopClient", [payload], redact=("token",))
    ta, tb = tokens_of(a, b)
    if not (ta and tb):
        return
    rows = []
    for token, db in ((ta, REF_DB), (tb, RUST_DB)):
        row = last_token_row(db, user_id)
        name_, scope, token_hash, preview, unused, live = row
        sane = token_hash == hashlib.sha256(token.encode()).hexdigest() and preview == token[-6:] and len(token) == 32
        rows.append({"name": name_, "scope": scope, "hash/preview/len ok": sane, "never used": unused, "live": live})
    run.compare("api_tokens row " + name, rows[0], rows[1])
    audits = []
    for token, db, n in ((ta, REF_DB, before[0]), (tb, RUST_DB, before[1])):
        found = audit_rows(db, user_id, "api_token.authorized")[n:]
        shaped = []
        for detail, ip in found:
            d = json.loads(detail)
            d["preview"] = "<last 6 of token>" if d.get("preview") == token[-6:] else d.get("preview")
            shaped.append({"detail": d, "ip": ip})
        audits.append(shaped)
    run.compare("audit " + name, audits[0], audits[1])
    run.compare("audit written " + name, expect_audit, bool(audits[1]))


def actions():
    acts = parity.Actions(run, ARGS.ref_app)
    pair = run.pair()
    good = {"port": "49732", "state": "s7Kd93jf0aQ2", "name": "Modlight Desktop"}

    acts.check(pair, "approve signed out", "authorize", "approveDesktopClient", [good])
    user_id, _ = seed_user("act")
    parity.signed_in(pair, REF_DB, RUST_DB, user_id)

    for name, arg in [
        ("null input", None),
        ("undefined input", parity.UNDEFINED),
        ("string input", "49732"),
        ("array input", ["49732", "s7Kd93jf0aQ2"]),
        ("numeric port", {"port": 49732, "state": "s7Kd93jf0aQ2"}),
        ("array port", {"port": ["49732"], "state": "s7Kd93jf0aQ2"}),
        ("privileged port", {"port": "443", "state": "s7Kd93jf0aQ2"}),
        ("missing state", {"port": "49732"}),
        ("bad state", {"port": "49732", "state": "s7Kd93jf0aQ2\""}),
        ("undefined state", {"port": "49732", "state": parity.UNDEFINED}),
    ]:
        acts.check(pair, "approve invalid: " + name, "authorize", "approveDesktopClient", [arg])
    acts.check(pair, "approve no argument", "authorize", "approveDesktopClient", [])

    check_issue(acts, pair, "named", user_id, good)
    check_issue(acts, pair, "unnamed", user_id, {"port": "1024", "state": "a" * 128})
    check_issue(acts, pair, "padded", user_id, {"port": " 65535 ", "state": " KZ9-x_2Q.tilde~ok ", "name": "  "})
    check_issue(acts, pair, "bidi name", user_id, {"port": "49732", "state": "s7Kd93jf0aQ2", "name": "She‮ll"})
    check_issue(acts, pair, "long name", user_id, {"port": "49732", "state": "s7Kd93jf0aQ2", "name": "y" * 400})
    check_issue(acts, pair, "unicode name", user_id,
                {"port": "49732", "state": "s7Kd93jf0aQ2", "name": "Zażółć​gęślą\n\njaźń"})
    check_issue(acts, pair, "non-string name", user_id, {"port": "49732", "state": "s7Kd93jf0aQ2", "name": 5})
    # slice(0, 64) splits the emoji: the token is issued, and the audit insert
    # fails upstream (a lone surrogate in jsonb), so neither side logs it.
    check_issue(acts, pair, "name cut in a surrogate pair", user_id,
                {"port": "49732", "state": "s7Kd93jf0aQ2", "name": "a" * 63 + "\U0001F600"}, expect_audit=False)

    # The rate limit: 10 approvals per caller and account in 15 minutes.
    rl_user, _ = seed_user("rl")
    parity.signed_in(pair, REF_DB, RUST_DB, rl_user)
    for i in range(10):
        acts.check(pair, "approve #%d" % (i + 1), "authorize", "approveDesktopClient", [good], redact=("token",))
    acts.check(pair, "approve #11 (rate limited)", "authorize", "approveDesktopClient", [good])
    acts.check(pair, "approve invalid after the limit", "authorize", "approveDesktopClient", [{"port": "1"}])
    counts = both_db(lambda db: db.sql("SELECT count(*) FROM api_tokens WHERE user_id = %s"
                                       % parity.sql_quote(rl_user)).strip())
    run.compare("tokens issued under the limit", counts[0], counts[1])


documents()
actions()
run.finish()

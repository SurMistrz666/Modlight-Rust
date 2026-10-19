"""Parity of the public auth pages and `/`, `/welcome` between the reference
and Rust (CONTRACT 6.1, 6.2, 6.4-6.9).

    python3 server/tests/parity/auth_pages.py --ref http://localhost:8424 --rust http://localhost:8423 \
        --ref-env ../_scratch/srv-auth/ref/.env --rust-env ../_scratch/srv-auth/rust/rust.env

For each page and state: the document's status, `Location`, title and meta
tags on both servers; the props the reference's client components received
(read from its RSC payload) against the props in the Rust document payload;
and the Rust `/_page` answer against its own document. Database side effects
(the invitation deletes) are compared too.

Changes settings on both servers (registration switch and key, mail) and
puts them back to "registration on, no key, no mail" at the end. It never
deletes users, so `bootstrap` (no account at all) is only checked when both
databases happen to be empty. Secrets are never printed.
"""

import argparse
import hashlib
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser(allow_abbrev=False)
ap.add_argument("--ref-env", default="../_scratch/srv-auth/ref/.env")
ap.add_argument("--rust-env", default="../_scratch/srv-auth/rust/rust.env")
ap.add_argument("--ref-db", default="modlight-srvauth-ref-postgres-1/modlight")
ap.add_argument("--rust-db", default="modlight-devdb-postgres-1/modlight_srv_auth")
ap.add_argument("--ref-app", default="modlight-srvauth-ref-app-1")
args, _ = ap.parse_known_args()

run = parity.Run.from_args()
REF_DB = parity.Db(*args.ref_db.split("/"), parity.read_env_file(args.ref_env)["AUTH_SECRET"])
RUST_DB = parity.Db(*args.rust_db.split("/"), parity.read_env_file(args.rust_env)["AUTH_SECRET"])
DBS = [REF_DB, RUST_DB]
ACTS = parity.Actions(run, args.ref_app)
TAG = parity.random_id(6).lower()
q = parity.sql_quote

with open(os.path.join(HERE, "..", "..", "messages", "en", "auth.json")) as f:
    AUTH = json.load(f)


def msg(key):
    node = AUTH
    for part in key.split("."):
        node = node[part]
    return node


def both(sql_text):
    return [db.sql(sql_text) for db in DBS]


def compare_rows(name, sql_text):
    a, b = both(sql_text)
    run.compare(name, parity.normalize_text(a), parity.normalize_text(b))


def seed_user(key, **columns):
    user_id = "pp%s%s" % (TAG, key)
    email = "%s.%s@pages.test" % (key, TAG)
    for db in DBS:
        db.create_user(user_id, email, "Page %s" % key, **columns)
    return user_id, email


def seed_invite(user_id, token, expired=False):
    digest = hashlib.sha256(token.encode()).hexdigest()
    when = "now() - interval '1 minute'" if expired else "now() + interval '7 days'"
    for db in DBS:
        db.sql('INSERT INTO "verification" ("id", "identifier", "value", "expires_at", "created_at", "updated_at") '
               "VALUES (%s, %s, %s, %s, now(), now())" % (q(parity.random_id(32)), q("invite:" + digest), q(user_id),
                                                         when))


# ── reading the two documents ───────────────────────────────────────────────


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


def rust_payload(html):
    m = re.search(r'<script id="__MODLIGHT_PAGE__" type="application/json">(.*?)</script>', html, re.S)
    return json.loads(m.group(1)) if m else None


def first_object_with(text, key, kind=bool, where=lambda obj: True):
    """The first JSON object in the RSC text whose `key` holds a value of
    `kind` (the innermost `{` enclosing `"key":`, decoded) and that `where`
    accepts."""
    for m in re.finditer(re.escape('"%s":' % key), text):
        start = m.start()
        while start > 0:
            start = text.rfind("{", 0, start)
            if start < 0:
                break
            try:
                obj, _ = json.JSONDecoder().raw_decode(text, start)
            except ValueError:
                continue
            if isinstance(obj, dict) and key in obj:
                if isinstance(obj[key], kind) and where(obj):
                    return obj
                break
    return None


# What the reference's tree shows, mapped onto the port's props.

def rendered(text, key):
    """Whether the message `auth.<key>` is rendered as a text node (the
    messages blob of the root layout holds every string, so a bare search
    would always succeed)."""
    return ('"children":%s' % json.dumps(msg(key), ensure_ascii=False)) in text


def ref_login(text):
    form = first_object_with(text, "registrationEnabled")
    return {"registration": {"open": form["registrationEnabled"]}} if form else {"unrecognised": True}


def ref_register(text, key_required_when_closed):
    if rendered(text, "register.closedBody"):
        return {"state": {"bootstrap": False, "open": False, "keyRequired": key_required_when_closed}}
    form = first_object_with(text, "keyRequired")
    if not form:
        return {"unrecognised": True}
    return {"state": {"bootstrap": form["bootstrap"], "open": True, "keyRequired": form["keyRequired"]}}


def ref_forgot(text):
    if rendered(text, "forgot.unavailableBody"):
        return {"mailReady": False}
    return {"mailReady": True}


def ref_invite(text, token, name):
    if rendered(text, "invite.expiredBody"):
        return {"token": token, "preview": None}
    form = first_object_with(text, "email", str, lambda obj: obj.get("token") == token)
    if not form:
        return {"unrecognised": True}
    # Upstream sent only the email; the port's type carries the name too
    # (open question 12.10): it is checked against the database instead.
    return {"token": token, "preview": {"email": form["email"], "name": name}}


def ref_welcome(text):
    # SiteNav, Hero and Cta are server components: what is rendered is a link
    # to the panel for a signed-in visitor, to /login and /register otherwise.
    return {"isAuthed": '{"href":"/login"' not in text}


def doc(browser, path):
    return browser.request("GET", path, headers={"Accept": "text/html"}, origin=False)


def check_page(p, name, path, ref_props_of=None, route=None):
    a, b = doc(p.ref, path), doc(p.rust, path)
    sa = {"status": a.status, "location": a.header("location")}
    sb = {"status": b.status, "location": b.header("location")}
    if a.status == 200:
        html = a.text()
        sa["head"] = head_of(html)
        if ref_props_of:
            sa["props"] = ref_props_of(rsc_text(html))
    if b.status == 200:
        html = b.text()
        sb["head"] = head_of(html)
        payload = rust_payload(html) or {}
        if ref_props_of:
            sb["props"] = payload.get("props")
        run.compare("route of " + name, route or path.split("?")[0], payload.get("route"))
    run.compare("document " + name, sa, sb)
    if run.verbose:
        print("     ref: %s" % json.dumps({k: v for k, v in sa.items() if k != "head"}, ensure_ascii=False))
        print("     head: %s" % json.dumps(sa.get("head"), ensure_ascii=False))
    # /_page agrees with the Rust document.
    page = p.rust.request("GET", "/_page?path=" + parity.urllib.parse.quote(path.split("?")[0], safe=""),
                          origin=False)
    pj = page.json() or {}
    if b.status in (307, 308):
        run.compare("/_page " + name, {"redirect": b.header("location"), "status": b.status}, pj)
    else:
        run.compare("/_page " + name, (rust_payload(b.text()) or {}).get("props"), pj.get("props"))
    return sa, sb


# ── scenarios ───────────────────────────────────────────────────────────────


def set_registration(admin, enabled, key=None):
    ACTS.check(admin, "registration %s" % ("on" if enabled else "off"), "registration", "setRegistrationEnabled",
               [enabled])
    if key is not None:
        ACTS.check(admin, "registration key %s" % ("set" if key else "cleared"), "registration",
                   "saveRegistrationKey", [key])


def main():
    admin_id, _ = seed_user("admin", role="admin")
    member_id, _ = seed_user("member")
    admin = run.pair()
    parity.signed_in(admin, REF_DB, RUST_DB, admin_id)
    member = run.pair()
    parity.signed_in(member, REF_DB, RUST_DB, member_id)
    anon = run.pair()

    # `/` and /welcome, signed out and in.
    check_page(anon, "/ signed out", "/")
    check_page(member, "/ signed in", "/")
    anon.ref.cookies[parity.SESSION_COOKIE] = "garbage"
    anon.rust.cookies[parity.SESSION_COOKIE] = "garbage"
    check_page(anon, "/ with a cookie that does not verify", "/")
    anon.ref.cookies.clear()
    anon.rust.cookies.clear()
    check_page(anon, "/welcome signed out", "/welcome", ref_welcome)
    check_page(member, "/welcome signed in", "/welcome", ref_welcome)

    # /login and /register in every registration state.
    empty = all(db.sql('SELECT count(*) FROM "user"').strip() == "0" for db in DBS)
    key_set = [False]
    for enabled, key in ((True, ""), (False, None), (True, "a-key-%s" % TAG), (False, None)):
        set_registration(admin, enabled, key)
        if key is not None:
            key_set[0] = bool(key)
        label = "registration %s%s" % ("on" if enabled else "off", ", key" if key else "")
        check_page(anon, "/login " + label, "/login", ref_login)
        check_page(member, "/login signed in, " + label, "/login?next=/dashboard/profile", ref_login,
                   route="/login")
        check_page(anon, "/register " + label, "/register",
                   lambda text, k=key_set[0]: ref_register(text, k))
    set_registration(admin, True, "")
    if empty:
        print("note: both databases are empty, bootstrap is reachable")

    # /forgot without and with mail.
    check_page(anon, "/forgot without mail", "/forgot", ref_forgot)
    ACTS.check(admin, "mail on", "mail", "saveSmtpSettings", [{
        "host": "127.0.0.1", "port": 2799, "security": "none", "user": "", "from": "no-reply@sink.test",
        "fromName": "Parity"}])
    check_page(anon, "/forgot with mail", "/forgot", ref_forgot)
    ACTS.check(admin, "mail off", "mail", "clearSmtpSettings", [])
    check_page(anon, "/forgot mail cleared", "/forgot", ref_forgot)

    # /reset and /2fa read nothing on the server.
    check_page(anon, "/reset", "/reset?token=abc", lambda text: {}, route="/reset")
    check_page(anon, "/reset error", "/reset?error=INVALID_TOKEN", lambda text: {}, route="/reset")
    check_page(anon, "/2fa", "/2fa?next=//evil.example", lambda text: {}, route="/2fa")

    # /invite/[token]: valid, unknown, expired, account gone.
    invited, invited_email = seed_user("invited", email_verified=False)
    token = parity.random_id(43)
    seed_invite(invited, token)
    check_page(anon, "/invite valid", "/invite/" + token,
               lambda text: ref_invite(text, token, "Page invited"), route="/invite/[token]")
    compare_rows("valid invitation kept", 'SELECT count(*) FROM "verification" WHERE value = %s' % q(invited))
    check_page(member, "/invite valid, signed in", "/invite/" + token,
               lambda text: ref_invite(text, token, "Page invited"), route="/invite/[token]")
    check_page(anon, "/invite unknown", "/invite/x", lambda text: ref_invite(text, "x", None),
               route="/invite/[token]")

    stale, _ = seed_user("stale", email_verified=False)
    stale_token = parity.random_id(43)
    seed_invite(stale, stale_token, expired=True)
    compare_rows("expired invitation present", 'SELECT count(*) FROM "verification" WHERE value = %s' % q(stale))
    check_page(anon, "/invite expired", "/invite/" + stale_token,
               lambda text: ref_invite(text, stale_token, None), route="/invite/[token]")
    compare_rows("expired invitation deleted by the page",
                 'SELECT count(*) FROM "verification" WHERE value = %s' % q(stale))

    orphan_token = parity.random_id(43)
    seed_invite("pp%sgone" % TAG, orphan_token)
    check_page(anon, "/invite account gone", "/invite/" + orphan_token,
               lambda text: ref_invite(text, orphan_token, None), route="/invite/[token]")
    compare_rows("orphan invitation deleted by the page",
                 'SELECT count(*) FROM "verification" WHERE value = %s' % q("pp%sgone" % TAG))

    # Page spelling of an encoded token.
    odd = "a%20b~c"
    check_page(anon, "/invite encoded token", "/invite/" + odd,
               lambda text: ref_invite(text, "a%20b~c", None), route="/invite/[token]")

    # Polish titles.
    pl = run.pair()
    for b in (pl.ref, pl.rust):
        b.cookies["ml_locale"] = "pl"
    check_page(pl, "/login pl", "/login", ref_login)
    check_page(pl, "/2fa pl", "/2fa", lambda text: {})

    run.finish()


if __name__ == "__main__":
    main()

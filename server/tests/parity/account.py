"""Parity checks for app/actions/account.ts (all 20 actions), lib/data/sessions.ts
and the page /dashboard/profile: the reference (the original server) against the
Rust port, on identical state seeded into both databases.

    python3 server/tests/parity/account.py --ref http://localhost:8429 --rust http://localhost:8437

Options (defaults in brackets):
    --scratch DIR    [<main checkout>/_scratch/srv-auth-p] holds ref/.env and rust.env,
                     the two AUTH_SECRETs (read, never printed)
    --ref-app NAME   [modlight-srvauth-p-ref-app-1]       reference app container
    --ref-pg NAME    [modlight-srvauth-p-ref-postgres-1]  its Postgres container
    --ref-db NAME    [modlight]
    --rust-pg NAME   [modlight-devdb-postgres-1]          the dev Postgres of the Rust server
    --rust-db NAME   [modlight_srv_auth_p]

Both servers run in production mode with the SAME extra domains:
APP_DOMAINS="https://H.example.com, https://s.example.com/" (the reference in its
compose file, the Rust server through <scratch>/rust.env, which
scripts/parity-server.sh loads). Each run seeds its own accounts (ids and tags
carry a random run tag), so the script can be repeated against the same
databases; only this run's rows are compared.

What is compared:
- every account action through Next's own protocol on the reference and /_rpc
  on Rust: the envelope (ok / redirect / masked error / revalidate) and every
  Set-Cookie (exact values for ml_skin / ml_locale; the session cookie only as
  set or cleared, its value is per-secret), valid input, refusal paths, wrong
  argument types, signed out, bad / unknown / expired / refresh-due session
  cookies, admin actions as a non-admin;
- the database rows each call leaves behind (`user`, `user_code_aliases`,
  `uploads.password_hash/password_version`, `session`), including whether the
  user row was written at all (`updated_at`);
- /dashboard/profile: the props of every client component in the reference's
  RSC payload against the same props derived from the Rust page data (the SPA
  page does that derivation; mirrored below), plus the title. QuotaCard is a
  server component: its inputs (`stats`, quota columns) are checked against
  SQL on both databases. The Rust page needs the (app) layout loader: until it
  is registered, /_page answers 501 and those checks are reported as skipped.
"""

import argparse
import base64
import hashlib
import hmac
import json
import os
import random
import re
import string
import subprocess
import sys
import urllib.parse
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

UNDEFINED = parity.UNDEFINED
SESSION = parity.SESSION_COOKIE
NAN = float("nan")
INF = float("inf")
NOW = "(now() AT TIME ZONE 'UTC')"


def default_scratch():
    here = os.path.dirname(os.path.abspath(__file__))
    out = subprocess.run(["git", "-C", here, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    main = os.path.dirname(out.stdout.decode().strip()) if out.returncode == 0 else os.getcwd()
    return os.path.join(main, "_scratch", "srv-auth-p")


def options():
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--scratch", default=os.environ.get("PARITY_SCRATCH") or default_scratch())
    p.add_argument("--ref-app", default="modlight-srvauth-p-ref-app-1")
    p.add_argument("--ref-pg", default="modlight-srvauth-p-ref-postgres-1")
    p.add_argument("--ref-db", default="modlight")
    p.add_argument("--rust-pg", default="modlight-devdb-postgres-1")
    p.add_argument("--rust-db", default="modlight_srv_auth_p")
    a, _ = p.parse_known_args()
    return a


OPT = options()
run = parity.Run.from_args()
REF = parity.Db(OPT.ref_pg, OPT.ref_db, parity.read_env_file(os.path.join(OPT.scratch, "ref", ".env"))["AUTH_SECRET"])
RUST = parity.Db(OPT.rust_pg, OPT.rust_db, parity.read_env_file(os.path.join(OPT.scratch, "rust.env"))["AUTH_SECRET"])
acts = parity.Actions(run, OPT.ref_app)
R = "".join(random.SystemRandom().choice(string.ascii_lowercase) for _ in range(6))
q = parity.sql_quote
skipped = []


# ── helpers ──────────────────────────────────────────────────────────────────


def both(sql):
    return REF.sql(sql), RUST.sql(sql)


def rows_of(text):
    return [line.split("\t") for line in text.strip("\n").split("\n")] if text.strip() else []


def same(name, sql):
    a, b = both(sql)
    run.compare(name, rows_of(a), rows_of(b))
    return rows_of(a)


def unorigin(v, origin):
    if isinstance(v, str):
        return v.replace(origin, "<origin>")
    if isinstance(v, list):
        return [unorigin(x, origin) for x in v]
    if isinstance(v, tuple):
        return tuple(unorigin(x, origin) for x in v)
    if isinstance(v, dict):
        return dict((k, unorigin(x, origin)) for k, x in v.items())
    return v


def signed(db, token):
    sig = base64.b64encode(hmac.new(db.auth_secret.encode(), token.encode(), hashlib.sha256).digest()).decode()
    return urllib.parse.quote(token + "." + sig, safe="")


def pair_with(token=None, raw_cookie=None):
    p = run.pair(user_agent="Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0")
    if token:
        p.ref.cookies[SESSION] = signed(REF, token)
        p.rust.cookies[SESSION] = signed(RUST, token)
    if raw_cookie is not None:
        p.ref.cookies[SESSION] = raw_cookie
        p.rust.cookies[SESSION] = raw_cookie
    return p


def cookie_list(res):
    """Set-Cookie headers, attribute order kept; the session cookies only as
    set or cleared (their value is signed with each server's own secret)."""
    out = []
    for raw in res.header_all("set-cookie"):
        parts = [x.strip() for x in raw.split(";")]
        name, _, value = parts[0].partition("=")
        if "better-auth" in name:
            value = "<set>" if value else ""
        attrs = []
        for x in parts[1:]:
            k, eq, v = x.partition("=")
            if k.lower() == "expires":
                v = "<epoch>" if v.startswith("Thu, 01 Jan 1970") else "<date>"
            elif k.lower() == "max-age" and v.lstrip("-").isdigit() and v != "0":
                v = "<%d>" % (round(int(v) / 10.0) * 10)
            attrs.append(k + (eq + v if eq else ""))
        out.append([name, value, attrs])
    return out


def rejects_nested(res):
    """Does the action's result reference an error row? React serialises a
    value it cannot encode (a function, say) as its own `E` row and the
    result row points at it; the client then rejects the whole call (its
    `initializeModelChunk` rethrows the referenced chunk's error), which is
    what the caller sees. parity.next_action_envelope only looks at the root
    row, so it would report `ok` with a null in place of the bad value."""
    if "text/x-component" not in (res.header("content-type") or ""):
        return False
    rows, errors = {}, set()
    for line in res.text().split("\n"):
        key, sep, payload = line.partition(":")
        if not sep:
            continue
        if payload.startswith("E"):
            errors.add(key)
            continue
        try:
            rows[key] = json.loads(payload)
        except ValueError:
            pass

    def refers(v, seen):
        if isinstance(v, str):
            m = re.match(r"^\$@?([0-9a-f]+)$", v)
            if not m:
                return False
            k = m.group(1)
            if k in errors:
                return True
            if k in rows and k not in seen:
                seen.add(k)
                return refers(rows[k], seen)
            return False
        if isinstance(v, list):
            return any(refers(x, seen) for x in v)
        if isinstance(v, dict):
            return any(refers(x, seen) for x in v.values())
        return False

    head = rows.get("0")
    return isinstance(head, dict) and refers(head.get("a"), set())


def ref_envelope(res):
    env = parity.next_action_envelope(res)
    if "ok" in env and rejects_nested(res):
        env = dict([("error", parity.MASKED)] + [(k, v) for k, v in env.items() if k == "revalidate"])
    return env


def call(pair, export, *args):
    a = acts.call_ref(pair.ref, "account", export, list(args))
    b = acts.call_rust(pair.rust, "account", export, list(args))
    ea = unorigin(parity.normalize_json(ref_envelope(a)), run.ref)
    eb = unorigin(parity.normalize_json(parity.rpc_envelope(b)), run.rust)
    ea["set-cookie"] = cookie_list(a)
    eb["set-cookie"] = cookie_list(b)
    return ea, eb


def check(pair, name, export, *args):
    ea, eb = call(pair, export, *args)
    run.compare(name, ea, eb)
    return ea


def mark(uid):
    """Backdate updated_at so `written(uid)` tells whether the call wrote the row."""
    both('UPDATE "user" SET "updated_at" = \'2000-01-01\' WHERE "id" = %s' % q(uid))


USER_COLUMNS = ('"name", "hide_uploads", "copy_raw", "theme", "locale", "user_code", "skip_tag_in_url", '
                '"allow_root_short_links", "default_folder_id", "naming_scheme", "naming_pattern", '
                '"naming_word_count", "naming_random_length", "preferred_domain", "public_profile", '
                '"background_dim", "used_quota", "max_quota", "default_password_hash" IS NOT NULL, '
                '"default_password_version", "updated_at" > \'2001-01-01\' AS written')


def user_state(name, uid):
    return same(name + " [user row]", 'SELECT %s FROM "user" WHERE "id" = %s' % (USER_COLUMNS, q(uid)))


NODE_SCRYPT = (
    "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{"
    "const c=require('crypto');const {p,h}=JSON.parse(d);"
    "if(h===null){const s=c.randomBytes(16);"
    "process.stdout.write('scrypt$'+s.toString('base64url')+'$'+c.scryptSync(p,s,32).toString('base64url'));return;}"
    "const [k,s,dk]=h.split('$');"
    "process.stdout.write(String(k==='scrypt'&&c.scryptSync(p,Buffer.from(s,'base64url'),32).toString('base64url')===dk));"
    "})"
)


def node_scrypt(password, stored=None):
    """lib/password.ts's hashPassword / verifyPassword (Node's scrypt defaults,
    N=16384 r=8 p=1), computed by the reference container's own Node: the host
    Python (LibreSSL) has no hashlib.scrypt. Inputs go through stdin."""
    out = subprocess.run(["docker", "exec", "-i", OPT.ref_app, "node", "-e", NODE_SCRYPT],
                         input=json.dumps({"p": password, "h": stored}).encode(), stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, check=True)
    return out.stdout.decode()


def scrypt_verifies(password, stored):
    return node_scrypt(password, stored) == "true"


# ── seed ─────────────────────────────────────────────────────────────────────

A_ID, U_ID, V_ID, W_ID, X_ID = ("acp_%s_%s" % (k, R) for k in ("admin", "alicja", "bob", "wanda", "xavier"))
A_TAG, U_TAG, V_TAG, X_TAG = "a" + R, "u" + R, "v" + R, "x" + R
RETIRED = "r" + R  # bob's retired tag
OWN_ALIAS = "o" + R  # alicja's retired tag (hers to reclaim)
F1, F2, F3, F4 = (str(uuid.uuid4()) for _ in range(4))
DEFAULT_HASH = node_scrypt("old-default")


def seed_user(uid, email, name, role, user_code, offset, **cols):
    for db in (REF, RUST):
        db.create_user(uid, email, name, role=role, user_code=user_code, **cols)
    both('UPDATE "user" SET "created_at" = %s - interval \'1 hour\' + interval \'%d seconds\', '
         '"updated_at" = %s - interval \'1 hour\' WHERE "id" = %s' % (NOW, offset, NOW, q(uid)))


def seed():
    seed_user(A_ID, "admin-%s@acp.test" % R, "Ada Admin", "admin", A_TAG, 1)
    seed_user(U_ID, "alicja-%s@acp.test" % R, "Alicja Kowalska", "user", U_TAG, 2,
              copy_raw=True, image="avatars/%s-tok123.webp" % U_ID,
              background_key="backgrounds/%s-bgtok.webp" % U_ID, background_dim=55, naming_scheme="words",
              naming_word_count=3, default_password_hash=DEFAULT_HASH, default_password_version=2,
              used_quota=1234, max_quota=1073741824)
    seed_user(V_ID, "bob-%s@acp.test" % R, "Bob", "user", V_TAG, 3)
    seed_user(W_ID, "wanda-%s@acp.test" % R, "Wanda", None, None, 4, banned=None, email_verified=False,
              used_quota=None, max_quota=None, hide_uploads=None, copy_raw=None, theme=None)
    seed_user(X_ID, "xavier-%s@acp.test" % R, "Xavier", "user", X_TAG, 5, banned=True,
              image="https://cdn.example.com/face.png", allow_root_short_links=True)
    both('INSERT INTO "user_code_aliases" ("code", "user_id") VALUES (%s, %s), (%s, %s)'
         % (q(RETIRED), q(V_ID), q(OWN_ALIAS), q(U_ID)))

    folders = [(F1, U_ID, None, "Screenshots", 0), (F2, U_ID, F1, "2026", 0), (F3, U_ID, None, "Docs", 1),
               (F4, V_ID, None, "Bob's", 0)]
    for fid, owner, parent, name, order in folders:
        both('INSERT INTO "folders" ("id", "owner_id", "parent_id", "name", "slug", "sort_order") '
             'VALUES (%s, %s, %s, %s, %s, %d)' % (q(fid), q(owner), q(parent), q(name), q(name.lower()), order))
    both('UPDATE "user" SET "default_folder_id" = %s WHERE "id" = %s' % (q(F1), q(U_ID)))

    # (code, owner, folder, size, views, password_hash, password_version, trashed)
    uploads = [("1", U_ID, F1, 1000, 5, "scrypt$x$1", 2, False), ("2", U_ID, F2, 2000, 7, "scrypt$x$2", 1, False),
               ("3", U_ID, None, 3000, 0, "scrypt$x$3", None, False), ("4", U_ID, None, 500, 1, None, None, False),
               ("5", U_ID, None, 4000, 9, "scrypt$x$5", 2, True), ("6", V_ID, None, 100, 0, None, None, False)]
    for n, owner, folder, size, views, ph, pv, trashed in uploads:
        both('INSERT INTO "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", '
             '"delete_token", "views", "folder_id", "password_hash", "password_version", "deleted_at") '
             'VALUES (%s, %s, %s, \'png\', \'image/png\', %d, %s, %s, %d, %s, %s, %s, %s)'
             % (q(owner), q("c%s%s" % (R, n)), q("file-%s.png" % n), size, q("k/%s/%s" % (R, n)),
                q("dt%s%s" % (R, n)), views, q(folder), q(ph), "NULL" if pv is None else pv,
                "now()" if trashed else "NULL"))

    # A pending invitation for Wanda, an expired one for Xavier.
    both('INSERT INTO "verification" ("id", "identifier", "value", "expires_at", "created_at", "updated_at") VALUES '
         "(%s, %s, %s, %s + interval '7 days', %s, %s), (%s, %s, %s, %s - interval '1 day', %s, %s)"
         % (q("inv1" + R), q("invite:w" + R), q(W_ID), NOW, NOW, NOW,
            q("inv2" + R), q("invite:x" + R), q(X_ID), NOW, NOW, NOW))


CHROME_WIN = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) "
              "Chrome/141.0.0.0 Safari/537.36")
SESSIONS = {
    # key: (user, created offset, expires, ip, user agent, impersonated by)
    "U0": (U_ID, "1 hour", "7 days - 1 hour", "203.0.113.7", CHROME_WIN, None),
    "U1": (U_ID, "2 days", "3 days", "   ", "Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) "
           "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Mobile/15E148 Safari/604.1", None),
    "U2": (U_ID, "3 days", "3 days", None, "Mozilla/5.0 (Linux; Android 15; SM-X710) AppleWebKit/537.36 "
           "Chrome/141.0.0.0 Safari/537.36", None),
    "U3": (U_ID, "4 days", "3 days", "2001:db8::1", "ShareX/16.1.0", None),
    "U4": (U_ID, "5 days", "3 days", "  198.51.100.4  ", "curl/8.9.1", None),
    "U5": (U_ID, "6 days", "3 days", "x" * 100, "A" * 500, None),
    "U6": (U_ID, "7 days", "3 days", "", "", None),
    "U7": (U_ID, "10 days", "-1 days", "203.0.113.8", "Wget/1.21", None),  # expired: never listed or revoked
    "U8": (U_ID, "1 day 1 hour", "6 days", "203.0.113.9", CHROME_WIN + " Edg/141.0.0.0", A_ID),  # impersonated
    "U9": (U_ID, "8 days", "3 days", "198.51.100.10", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:131.0) "
           "Gecko/20100101 Firefox/131.0", None),
    "U10": (U_ID, "9 days", "3 days", None, "Mozilla/5.0 (iPad; CPU OS 17_1 like Mac OS X) Version/17.1 "
            "Mobile/15E148 Safari/604.1", None),
    "U11": (U_ID, "6 days 1 hour", "1 day", "203.0.113.11", "Modlight/1.4.2 (Windows NT 10.0)", None),  # refresh due
    "U12": (U_ID, "12 days", "-2 days", None, "okhttp/4.12.0", None),  # expired, used as a cookie
    # More user agents for describeDevice (listed on the profile page).
    "D1": (U_ID, "11 days", "3 days", None, "Mozilla/5.0 (Linux; Android 14; SAMSUNG SM-S918B) AppleWebKit/537.36 "
           "(KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36", None),
    "D2": (U_ID, "11 days 1 hour", "3 days", None, "Mozilla/5.0 (Windows NT 6.1; Win64; x64; rv:115.0) "
           "Gecko/20100101 Firefox/115.0", None),
    "D3": (U_ID, "11 days 2 hours", "3 days", None, "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 "
           "(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36", None),
    "D4": (U_ID, "11 days 3 hours", "3 days", None, "python-requests/2.32.3", None),
    "D5": (U_ID, "11 days 4 hours", "3 days", None, "Go-http-client/1.1", None),
    "D6": (U_ID, "11 days 5 hours", "3 days", None, "Mozilla/5.0 (Windows NT 10.0; Microsoft Windows 10.0.22631; "
           "en-US) PowerShell/7.4.5", None),
    "D7": (U_ID, "11 days 6 hours", "3 days", None, "insomnia/2023.5.8", None),
    "D8": (U_ID, "11 days 7 hours", "3 days", None, "Mozilla/5.0 (Windows NT 6.3; Win64; x64) AppleWebKit/537.36 "
           "(KHTML, like Gecko) Chrome/109.0.0.0 Safari/537.36 OPR/95.0.0.0", None),
    "D9": (U_ID, "11 days 8 hours", "3 days", None, "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
           "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Vivaldi/6.5", None),
    "D10": (U_ID, "11 days 9 hours", "3 days", None, "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) "
            "AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/120.0.6099.119 Mobile/15E148 Safari/604.1", None),
    "D11": (U_ID, "11 days 10 hours", "3 days", None, "Mozilla/5.0 (Android 14; Mobile; rv:121.0) Gecko/121.0 "
            "Firefox/121.0", None),
    "D12": (U_ID, "11 days 11 hours", "3 days", None, "Mozilla/5.0 (Windows NT 6.2; WOW64) Opera/9.80", None),
    "D13": (U_ID, "11 days 12 hours", "3 days", None, "MODLIGHT/2.0", None),
    "D14": (U_ID, "11 days 13 hours", "3 days", None, "sharex", None),
    "D15": (U_ID, "11 days 14 hours", "3 days", None, "   ", None),
    "D16": (U_ID, "11 days 15 hours", "3 days", None, "Android 9\u2028Mobile", None),
    "D17": (U_ID, "11 days 16 hours", "3 days", None, "Mozilla/5.0 (Linux; Android 13; Tablet) okhttp/4.9 "
            "Mobile", None),
    "D18": (U_ID, "11 days 17 hours", "3 days", None, "Mozilla/5.0 (iPod touch; CPU iPhone OS 12_5 like Mac OS X) "
            "Version/12.1.2 Safari/605.1", None),
    "D19": (U_ID, "11 days 18 hours", "3 days", None, "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:131.0) "
            "Gecko/20100101 Firefox/131.0 EdgA/131.0", None),
    "A0": (A_ID, "1 hour", "7 days - 1 hour", "203.0.113.20", CHROME_WIN, None),
    "V0": (V_ID, "1 hour", "7 days - 1 hour", "203.0.113.30", CHROME_WIN, None),
    "V1": (V_ID, "2 hours", "6 days", "203.0.113.31", "PostmanRuntime/7.39.0", None),
}
SID = dict((k, "acp%s%s" % (k.lower(), R) + "s" * (26 - len(k) - len(R))) for k in SESSIONS)
TOKEN = dict((k, parity.random_id(32)) for k in SESSIONS)


def seed_sessions():
    values = []
    for k, (user, created, expires, ip, ua, imp) in SESSIONS.items():
        values.append("(%s, %s, %s, %s - interval '%s', %s - interval '%s', %s + interval '%s', %s, %s, %s)" % (
            q(SID[k]), q(TOKEN[k]), q(user), NOW, created, NOW, created, NOW, expires, q(ip), q(ua), q(imp)))
    both('INSERT INTO "session" ("id", "token", "user_id", "created_at", "updated_at", "expires_at", '
         '"ip_address", "user_agent", "impersonated_by") VALUES ' + ", ".join(values))


def session_rows(name):
    return same(name + " [sessions]",
                'SELECT "id", "expires_at" > %s AS live FROM "session" WHERE "user_id" IN (%s, %s) ORDER BY "id"'
                % (NOW, q(U_ID), q(V_ID)))


# ── /dashboard/profile ──────────────────────────────────────────────────────


def flight_rows(html):
    chunks = re.findall(r'self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)', html)
    text = "".join(json.loads('"' + c + '"') for c in chunks)
    rows, names = {}, {}
    for line in text.split("\n"):
        m = re.match(r"^([0-9a-f]+):(.*)$", line, re.S)
        if not m:
            continue
        key, payload = m.group(1), m.group(2)
        if payload.startswith("I["):
            module = json.loads(payload[1:])
            names["$L" + key] = module[2] if len(module) > 2 else str(module[0])
            continue
        try:
            rows[key] = json.loads(payload)
        except ValueError:
            pass
    return rows, names


def client_props(html):
    """{component export name: props} of every client component the page rendered."""
    rows, names = flight_rows(html)
    found = {}

    def walk(node):
        if isinstance(node, list):
            if (len(node) == 4 and node[0] == "$" and isinstance(node[1], str) and node[1] in names
                    and isinstance(node[3], dict)):
                found.setdefault(names[node[1]], parity._decode_flight(node[3], rows))
            for x in node:
                walk(x)
        elif isinstance(node, dict):
            for x in node.values():
                walk(x)

    for value in rows.values():
        walk(value)
    return found


def components_from_page(props, config):
    """What upstream's page.tsx handed each client component, from the SPA
    page's props (the derivations of web/src/pages/(app)/dashboard/profile/page.tsx)."""
    u, prefs, url = props["u"], props["prefs"], props["urlPrefs"]
    tag = prefs.get("userCode") if prefs and prefs.get("userCode") is not None else u.get("userCode")
    link_tag = None if (prefs and prefs.get("skipTagInUrl")) else tag
    name_or_email = u["name"] or u["email"]
    return {
        "ProfileForm": {"defaults": {"name": u["name"] if u["name"] is not None else "", "email": u["email"],
                                     "userCode": u["userCode"], "hideUploads": bool(u["hideUploads"]),
                                     "copyRaw": bool(u["copyRaw"])}},
        "DuplicatesSection": {},
        "TagSection": {"appUrl": url["baseUrl"], "tag": tag, "isAdmin": u["role"] == "admin",
                       "skipTagInUrl": prefs["skipTagInUrl"] if prefs else False},
        "NamingSection": {"baseUrl": url["baseUrl"], "tag": link_tag, "scheme": url["scheme"],
                          "pattern": url["pattern"], "wordCount": url["wordCount"],
                          "randomLength": url["randomLength"]},
        "DomainSection": {"domains": config["appDomains"], "defaultDomain": config["appUrl"],
                          "preferredDomain": url["preferredDomain"], "tag": link_tag},
        "DefaultFolderSection": {"folders": props["folders"],
                                 "defaultFolderId": prefs["defaultFolderId"] if prefs else None},
        "AvatarSection": {"url": props["avatarUrl"], "initials": name_or_email[:2].upper()},
        "AppearanceSection": {"theme": u["theme"] if u["theme"] is not None else "discord", "locale": u["locale"]},
        "BackgroundSection": {"url": props["backgroundUrl"], "dim": prefs["backgroundDim"] if prefs else 70},
        "SecuritySection": {"twoFactorEnabled": bool(u["twoFactorEnabled"]), "emailVerified": bool(u["emailVerified"])},
        "SessionList": {"sessions": props["sessions"]},
    }


def check_profile_page(label, token, uid, sessions):
    p = pair_with(token)
    ref = p.ref.request("GET", "/dashboard/profile", headers={"Accept": "text/html"}, origin=False)
    rust = p.rust.request("GET", "/_page?path=%2Fdashboard%2Fprofile", headers={"Accept": "application/json"},
                          origin=False)
    if rust.status == 501:
        skipped.append("%s: /_page answered 501 (no (app) layout loader registered yet)" % label)
        print("SKIP %s (no (app) layout loader on the Rust server yet)" % label)
        return
    data = rust.json()
    run.compare(label + " status", ref.status, 200 if data and "props" in data else rust.status)
    title = re.search(r"<title>([^<]*)</title>", ref.text())
    run.compare(label + " title", title.group(1) if title else None, data["meta"]["title"])
    expected = client_props(ref.text())
    got = components_from_page(data["props"], data["layouts"]["root"]["config"])
    run.compare(label + " sessions listed (live, not impersonated)", sessions,
                len((expected.get("SessionList") or {}).get("sessions") or []))
    for name in sorted(got):
        run.compare("%s %s props" % (label, name),
                    unorigin(parity.normalize_json(expected.get(name)), run.ref),
                    unorigin(parity.normalize_json(got[name]), run.rust))
    # Nothing a server-only column holds may appear in the page data.
    text = json.dumps(data["props"])
    leaks = [k for k in ("token", "backgroundKey", "defaultPasswordHash", "image") if '"%s"' % k in text]
    leaks += [t for t in TOKEN.values() if t in text]
    run.compare(label + " no secrets in the props", [], leaks)
    # QuotaCard is a server component: its inputs against SQL on each side.
    stats_sql = ('SELECT count(*), coalesce(sum("size"), 0), coalesce(sum("views"), 0) FROM "uploads" '
                 'WHERE "owner_id" = %s AND "deleted_at" IS NULL' % q(uid))
    quota_sql = 'SELECT "used_quota", "max_quota" FROM "user" WHERE "id" = %s' % q(uid)
    ref_stats, rust_stats = rows_of(REF.sql(stats_sql)), rows_of(RUST.sql(stats_sql))
    run.compare(label + " stats (SQL on both)", ref_stats, rust_stats)
    s = data["props"]["stats"]
    run.compare(label + " stats prop = Rust SQL", rust_stats, [[str(s["count"]), str(s["totalSize"]),
                                                               str(s["totalViews"])]])
    ref_quota = rows_of(REF.sql(quota_sql))
    run.compare(label + " quota columns (ref SQL vs Rust prop)", ref_quota,
                [["" if data["props"]["u"]["usedQuota"] is None else str(data["props"]["u"]["usedQuota"]),
                  "" if data["props"]["u"]["maxQuota"] is None else str(data["props"]["u"]["maxQuota"])]])
    doc = p.rust.request("GET", "/dashboard/profile", headers={"Accept": "text/html"}, origin=False)
    rust_title = re.search(r"<title>([^<]*)</title>", doc.text())
    run.compare(label + " document", (ref.status, title.group(1) if title else None),
                (doc.status, rust_title.group(1) if rust_title else None))


# ── the checks ───────────────────────────────────────────────────────────────


def check_sessions_and_gates():
    # A garbage cookie passes the proxy gate but is no session: every action
    # redirects to /login, nothing is written, no cookie is touched.
    anon = pair_with(raw_cookie="garbage")
    samples = {
        "updatePreferences": [{"name": "x"}], "checkTagAvailability": ["abc"], "updateTag": ["abc"],
        "updateSkipTagInUrl": [True], "setAllowRootShortLinks": [U_ID, True], "updateDefaultFolder": [None],
        "myUrlPrefs": [], "updateNaming": [{"scheme": "random"}], "updatePreferredDomain": [None],
        "myDefaultFilePassword": [], "updateDefaultFilePassword": [""], "myBackground": [],
        "updateBackgroundDim": [10], "updatePublicProfile": [True], "listAdminUsers": [],
        "setUserQuota": [U_ID, 5], "setUserTag": [U_ID, "abc"], "recalculateUserQuota": [U_ID],
        "revokeMySession": ["x"], "revokeMyOtherSessions": [],
    }
    for export, args in samples.items():
        check(anon, "signed out (bad signature): " + export, export, *args)
    # Not an administrator: the admin-only actions send to /dashboard.
    user = pair_with(TOKEN["U0"])
    for export in ("updateSkipTagInUrl", "setAllowRootShortLinks", "listAdminUsers", "setUserQuota",
                   "setUserTag", "recalculateUserQuota"):
        mark(V_ID)
        check(user, "non-admin: " + export, export, *samples[export])
    user_state("non-admin calls write nothing", V_ID)
    # A validly signed token without a row: Better Auth clears the cookies.
    ghost = pair_with(parity.random_id(32))
    check(ghost, "signed token without a session row", "myBackground")
    # An expired session: cookies cleared, the row deleted.
    check(pair_with(TOKEN["U12"]), "expired session", "myUrlPrefs")
    same("expired session row deleted",
         'SELECT count(*) FROM "session" WHERE "id" = %s' % q(SID["U12"]))
    # A session in its last 6 days is refreshed: row + cookie (revalidate).
    old = pair_with(TOKEN["U11"])
    check(old, "refresh-due session", "myBackground")
    same("refreshed session row",
         'SELECT "expires_at" > %s + interval \'6 days\', "updated_at" > %s - interval \'1 minute\' '
         'FROM "session" WHERE "id" = %s' % (NOW, NOW, q(SID["U11"])))
    check(old, "refreshed session works", "myBackground")


def check_reads():
    user, admin = pair_with(TOKEN["U0"]), pair_with(TOKEN["A0"])
    check(user, "myUrlPrefs", "myUrlPrefs")
    check(user, "myUrlPrefs ignores arguments", "myUrlPrefs", "junk", 5)
    check(user, "myDefaultFilePassword", "myDefaultFilePassword")
    check(user, "myBackground", "myBackground")
    check(pair_with(TOKEN["V0"]), "myBackground without a wallpaper", "myBackground")
    # listAdminUsers lists every account in the database: compare this run's.
    a = acts.call_ref(admin.ref, "account", "listAdminUsers", [])
    b = acts.call_rust(admin.rust, "account", "listAdminUsers", [])
    ea, eb = ref_envelope(a), parity.rpc_envelope(b)
    mine = lambda env: [r for r in (env.get("ok") or []) if r["id"].endswith("_" + R)]  # noqa: E731
    run.compare("listAdminUsers (this run's rows, in order)", parity.normalize_json(mine(ea)),
                parity.normalize_json(mine(eb)))
    run.compare("listAdminUsers envelope", dict((k, v) for k, v in ea.items() if k != "ok"),
                dict((k, v) for k, v in eb.items() if k != "ok"))
    run.compare("listAdminUsers has 5 rows of this run", 5, len(mine(eb)))


def check_preferences():
    user = pair_with(TOKEN["U0"])
    cases = [
        ("full patch", {"name": "  Ala  ", "hideUploads": True, "copyRaw": "x", "theme": "glass", "locale": "pl"}),
        ("alias is not a theme id", {"theme": "modern"}),
        ("locale null", {"locale": None}),
        ("locale undefined is dropped", {"locale": UNDEFINED}),
        ("locale en", {"locale": "en"}),
        ("locale de", {"locale": "de"}),
        ("long name", {"name": "N" * 200}),
        ("blank name only", {"name": "   "}),
        ("empty object", {}),
        ("array", []),
        ("wrong types", {"name": 5, "hideUploads": "true", "copyRaw": 1, "theme": "nope"}),
        ("theme default", {"theme": "default", "copyRaw": False}),
        ("date argument", parity.JsDate("2026-10-18T12:00:00.000Z")),
        ("null", None),
        ("string", "abc"),
        ("number", 5),
        ("undefined", UNDEFINED),
    ]
    for label, arg in cases:
        mark(U_ID)
        check(user, "updatePreferences: " + label, "updatePreferences", arg)
        user_state("updatePreferences: " + label, U_ID)
    mark(U_ID)
    check(user, "updatePreferences: no argument", "updatePreferences")
    # A theme column naming an Object.prototype member: resolveTheme returns a
    # function, which the action result cannot carry.
    bob = pair_with(TOKEN["V0"])
    both('UPDATE "user" SET "theme" = \'toString\' WHERE "id" = %s' % q(V_ID))
    mark(V_ID)
    check(bob, "updatePreferences: theme column 'toString', nothing to write", "updatePreferences", {})
    check(bob, "updatePreferences: theme column 'toString', with a write", "updatePreferences", {"name": "Bobby"})
    user_state("updatePreferences: theme column 'toString'", V_ID)
    both('UPDATE "user" SET "theme" = \'__proto__\' WHERE "id" = %s' % q(V_ID))
    check(bob, "updatePreferences: theme column '__proto__'", "updatePreferences", {"hideUploads": True})
    both('UPDATE "user" SET "theme" = NULL WHERE "id" = %s' % q(V_ID))
    check(bob, "updatePreferences: theme column NULL", "updatePreferences", {})


def aliases(name):
    return same(name + " [tags]",
                'SELECT u."id", u."user_code", (SELECT string_agg(a."code", \',\' ORDER BY a."code") '
                'FROM "user_code_aliases" a WHERE a."user_id" = u."id") FROM "user" u '
                'WHERE u."id" IN (%s, %s, %s) ORDER BY u."id"' % (q(U_ID), q(V_ID), q(W_ID)))


def check_tags():
    user = pair_with(TOKEN["U0"])
    for label, arg in [("too short", "a"), ("too long", "x" * 33), ("charset", "-ab"), ("reserved", "Login"),
                       ("current", U_TAG), ("current, padded", "  %s  " % U_TAG), ("taken", V_TAG),
                       ("retired by another", RETIRED), ("own retired alias", OWN_ALIAS), ("free", "n" + R),
                       ("case differs", U_TAG.upper()), ("unicode", "éé"),
                       ("number", 5), ("null", None), ("undefined", UNDEFINED), ("array", ["ab"])]:
        check(user, "checkTagAvailability: " + label, "checkTagAvailability", arg)
    check(user, "checkTagAvailability: no argument", "checkTagAvailability")

    for label, arg in [("reserved", "dashboard"), ("taken", V_TAG), ("retired", RETIRED), ("current", U_TAG),
                       ("number", 42), ("undefined", UNDEFINED)]:
        mark(U_ID)
        check(user, "updateTag: " + label, "updateTag", arg)
        aliases("updateTag: " + label)
        user_state("updateTag: " + label, U_ID)
    mark(U_ID)
    check(user, "updateTag: new tag", "updateTag", "  n%s  " % R)
    aliases("updateTag: new tag")
    user_state("updateTag: new tag", U_ID)
    check(user, "updateTag: reclaim own alias", "updateTag", OWN_ALIAS)
    aliases("updateTag: reclaim own alias")
    check(user, "checkTagAvailability: the tag just left", "checkTagAvailability", "n" + R)

    admin = pair_with(TOKEN["A0"])
    for label, args in [("reserved", [V_ID, "Login"]), ("taken", [V_ID, OWN_ALIAS]),
                        ("unknown account", ["nobody-" + R, "z" + R]), ("non-string tag", [V_ID, 5]),
                        ("null id", [None, "z" + R]), ("numeric id", [5, "z" + R]), ("current", [V_ID, V_TAG]),
                        ("own retired alias", [V_ID, RETIRED]), ("account without a tag", [W_ID, "w" + R]),
                        ("padded", [V_ID, "  t%s " % R])]:
        check(admin, "setUserTag: " + label, "setUserTag", *args)
        aliases("setUserTag: " + label)


def check_flags():
    admin = pair_with(TOKEN["A0"])
    for label, arg in [("true", True), ("false", False), ("'yes'", "yes"), ("'OF'", "OF"), ("1", 1), ("0", 0),
                       ("null", None), ("2", 2), ("'maybe'", "maybe"), ("object", {}), ("array", [True]),
                       ("1.5", 1.5), ("undefined", UNDEFINED)]:
        mark(A_ID)
        check(admin, "updateSkipTagInUrl: " + label, "updateSkipTagInUrl", arg)
        user_state("updateSkipTagInUrl: " + label, A_ID)
    mark(A_ID)
    check(admin, "updateSkipTagInUrl: no argument", "updateSkipTagInUrl")
    user_state("updateSkipTagInUrl: no argument", A_ID)

    for label, args in [("allow", [V_ID, True]), ("'off'", [V_ID, "off"]), ("unknown id", ["nobody", True]),
                        ("numeric id", [5, True]), ("undefined value", [V_ID, UNDEFINED]),
                        ("bad value", [V_ID, "maybe"]), ("null value", [V_ID, None]), ("'t'", [V_ID, "t"])]:
        mark(V_ID)
        check(admin, "setAllowRootShortLinks: " + label, "setAllowRootShortLinks", *args)
        user_state("setAllowRootShortLinks: " + label, V_ID)

    user = pair_with(TOKEN["U0"])
    for label, arg in [("true", True), ("'no'", "no"), ("null", None), ("2", 2), ("undefined", UNDEFINED),
                       ("' TRUE '", " TRUE ")]:
        mark(U_ID)
        check(user, "updatePublicProfile: " + label, "updatePublicProfile", arg)
        user_state("updatePublicProfile: " + label, U_ID)


def check_default_folder():
    user = pair_with(TOKEN["U0"])
    for label, arg in [("own folder", F3), ("another account's folder", F4), ("unknown uuid", str(uuid.uuid4())),
                       ("null", None), ("upper case", F2.upper()), ("braces", "{%s}" % F1),
                       ("no hyphens", F3.replace("-", "")), ("empty string", ""), ("not a uuid", "not-a-uuid"),
                       ("number", 5), ("zero", 0), ("false", False), ("undefined", UNDEFINED),
                       ("object", {"id": F1}), ("padded", " %s" % F1)]:
        mark(U_ID)
        check(user, "updateDefaultFolder: " + label, "updateDefaultFolder", arg)
        user_state("updateDefaultFolder: " + label, U_ID)
    mark(U_ID)
    check(user, "updateDefaultFolder: no argument", "updateDefaultFolder")
    check(user, "updateDefaultFolder: back to F1", "updateDefaultFolder", F1)


def check_naming():
    user = pair_with(TOKEN["U0"])
    cases = [
        ("custom", {"scheme": "custom", "pattern": "  {year}-{name}  "}),
        ("words with junk numbers", {"scheme": "words", "wordCount": 3.5, "randomLength": "9", "pattern": "x"}),
        ("custom without pattern", {"scheme": "custom", "pattern": "   "}),
        ("unknown scheme", {"scheme": "nonsense"}),
        ("random-chars with nulls", {"scheme": "random-chars", "wordCount": None, "randomLength": 1}),
        ("non-string pattern", {"scheme": "random", "pattern": 5}),
        ("long custom pattern", {"scheme": "custom", "pattern": "p" * 200}),
        ("array and boolean numbers", {"scheme": "words", "wordCount": [3], "randomLength": True}),
        ("undefined fields", {"scheme": "filename", "wordCount": UNDEFINED, "pattern": UNDEFINED}),
        ("date number", {"scheme": "date-filename", "wordCount": parity.JsDate("2026-10-18T12:00:00.000Z")}),
        ("NaN and Infinity", {"scheme": "filename-random", "wordCount": NAN, "randomLength": INF}),
        ("pattern of an unusable expansion", {"scheme": "custom", "pattern": "%%%"}),
        ("scheme case", {"scheme": "Custom", "pattern": "x"}),
        ("null", None),
        ("string", "custom"),
        ("undefined", UNDEFINED),
    ]
    for label, arg in cases:
        mark(U_ID)
        check(user, "updateNaming: " + label, "updateNaming", arg)
        user_state("updateNaming: " + label, U_ID)
        check(user, "myUrlPrefs after updateNaming: " + label, "myUrlPrefs")


def check_domain():
    user = pair_with(TOKEN["U0"])
    for label, arg in [("configured, typed with slashes", "https://h.example.com//"),
                       ("configured second", " https://s.example.com "), ("unknown", "https://evil.example"),
                       ("null", None), ("blank", "   "), ("only slashes", "///"), ("number", 5),
                       ("undefined", UNDEFINED), ("upper case", "HTTPS://S.EXAMPLE.COM")]:
        mark(U_ID)
        check(user, "updatePreferredDomain: " + label, "updatePreferredDomain", arg)
        user_state("updatePreferredDomain: " + label, U_ID)
        check(user, "myUrlPrefs after updatePreferredDomain: " + label, "myUrlPrefs")
    # The app's own origin differs per server: send each its own.
    a = acts.call_ref(user.ref, "account", "updatePreferredDomain", [run.ref + "/"])
    b = acts.call_rust(user.rust, "account", "updatePreferredDomain", [run.rust + "/"])
    run.compare("updatePreferredDomain: the app's own origin",
                unorigin(parity.normalize_json(ref_envelope(a)), run.ref),
                unorigin(parity.normalize_json(parity.rpc_envelope(b)), run.rust))
    check(user, "updatePreferredDomain: back to h", "updatePreferredDomain", "https://H.example.com")


def password_rows(name):
    rows = same(name + " [uploads]",
                'SELECT up."code", up."password_hash" IS NOT NULL, up."password_hash" = u."default_password_hash", '
                'up."password_version", up."password_hash" ~ \'^scrypt\\$x\\$\' FROM "uploads" up '
                'JOIN "user" u ON u."id" = up."owner_id" WHERE up."owner_id" = %s ORDER BY up."code"' % q(U_ID))
    same(name + " [default password]",
         'SELECT "default_password_hash" IS NOT NULL, "default_password_version", '
         '"default_password_hash" ~ \'^scrypt\\$[A-Za-z0-9_-]{22}\\$[A-Za-z0-9_-]{43}$\' '
         'FROM "user" WHERE "id" = %s' % q(U_ID))
    return rows


def check_default_password():
    user = pair_with(TOKEN["U0"])
    cases = [
        ("too short", ["abc"]),
        ("too short, astral", ["\U0001F600"]),
        ("clear", [""]),
        ("set and apply", ["hunter22", True]),
        ("non-string clears", [5]),
        ("clear and apply", [None, True]),
        ("set, truthy string apply", ["pässwörd", "yes"]),
        ("set, falsy apply", ["second-pass", 0]),
        ("four UTF-16 units", ["p1\U0001F600"]),
        ("apply with an empty array", ["third-pass", []]),
        ("undefined", [UNDEFINED]),
        ("no argument", []),
        ("set again and apply", ["final-pass", True]),
    ]
    for label, args in cases:
        mark(U_ID)
        check(user, "updateDefaultFilePassword: " + label, "updateDefaultFilePassword", *args)
        password_rows("updateDefaultFilePassword: " + label)
        user_state("updateDefaultFilePassword: " + label, U_ID)
        check(user, "myDefaultFilePassword after: " + label, "myDefaultFilePassword")
    # The stored hash is the file-password format and verifies the password.
    sql = 'SELECT "default_password_hash" FROM "user" WHERE "id" = %s' % q(U_ID)
    run.compare("default password verifies with scrypt (N=16384, r=8, p=1)", [True, True],
                [scrypt_verifies("final-pass", REF.sql(sql).strip()),
                 scrypt_verifies("final-pass", RUST.sql(sql).strip())])


def check_background_dim():
    user = pair_with(TOKEN["U0"])
    for label, arg in [("33", 33), ("'12'", "12"), ("42.5", 42.5), ("-5", -5), ("1000", 1000), ("null", None),
                       ("''", ""), ("true", True), ("[50]", [50]), ("{}", {}), ("NaN", NAN), ("Infinity", INF),
                       ("'abc'", "abc"), ("undefined", UNDEFINED), ("' 0x10 '", " 0x10 "), ("[[7]]", [[7]]),
                       ("date", parity.JsDate("1970-01-01T00:00:00.050Z")), ("'1e1'", "1e1")]:
        mark(U_ID)
        check(user, "updateBackgroundDim: " + label, "updateBackgroundDim", arg)
        user_state("updateBackgroundDim: " + label, U_ID)
    check(user, "myBackground after the dims", "myBackground")


def check_admin_quota():
    admin = pair_with(TOKEN["A0"])
    for label, args in [("1 GiB", [V_ID, 1073741824]), ("negative", [V_ID, -5]), ("fraction", [V_ID, 2.5]),
                        ("numeric string", [V_ID, "100"]), ("NaN", [V_ID, NAN]), ("Infinity", [V_ID, INF]),
                        ("past 2^53", [V_ID, 9007199254740993]), ("1e300", [V_ID, 1e300]),
                        ("unknown id", ["nobody", 5]), ("minus zero", [V_ID, -0.0]), ("-0.4", [V_ID, -0.4]),
                        ("null", [V_ID, None]), ("numeric id", [5, 10]), ("undefined bytes", [V_ID, UNDEFINED])]:
        mark(V_ID)
        check(admin, "setUserQuota: " + label, "setUserQuota", *args)
        user_state("setUserQuota: " + label, V_ID)
    for label, args in [("alicja", [U_ID]), ("unknown", ["nobody"]), ("number", [5]), ("null", [None]),
                        ("undefined", [UNDEFINED]), ("no argument", [])]:
        mark(U_ID)
        check(admin, "recalculateUserQuota: " + label, "recalculateUserQuota", *args)
        user_state("recalculateUserQuota: " + label, U_ID)


def check_session_actions():
    user = pair_with(TOKEN["U0"])
    session_rows("before revokeMySession")
    for label, arg in [("current", SID["U0"]), ("empty", ""), ("blank", "   "), ("number", 5), ("null", None),
                       ("undefined", UNDEFINED), ("another account's", SID["V1"]), ("expired", SID["U7"]),
                       ("impersonated", SID["U8"]), ("unknown", "nope" + R), ("padded", " %s " % SID["U1"]),
                       ("a device", SID["U1"]), ("the same again", SID["U1"])]:
        check(user, "revokeMySession: " + label, "revokeMySession", arg)
        session_rows("revokeMySession: " + label)
    check(user, "revokeMySession: no argument", "revokeMySession")
    check(user, "revokeMyOtherSessions", "revokeMyOtherSessions")
    session_rows("revokeMyOtherSessions")
    check(user, "revokeMyOtherSessions again (nothing left)", "revokeMyOtherSessions")
    check(pair_with(TOKEN["V0"]), "revokeMyOtherSessions: bob", "revokeMyOtherSessions")
    session_rows("revokeMyOtherSessions: bob")


def main():
    seed()
    seed_sessions()
    print("seeded run %s" % R)
    live = [k for k, v in SESSIONS.items() if v[0] == U_ID and not v[2].startswith("-") and not v[5]]
    check_profile_page("profile page (seeded)", TOKEN["U0"], U_ID, len(live))
    check_profile_page("profile page (admin)", TOKEN["A0"], A_ID, 1)
    check_sessions_and_gates()
    check_reads()
    check_preferences()
    check_tags()
    check_flags()
    check_default_folder()
    check_naming()
    check_domain()
    check_default_password()
    check_background_dim()
    check_admin_quota()
    check_profile_page("profile page (after the actions)", TOKEN["U0"], U_ID, len(live))
    check_session_actions()
    check_profile_page("profile page (one session left)", TOKEN["U0"], U_ID, 1)
    if skipped:
        print("\nskipped:\n  " + "\n  ".join(skipped))
    run.finish()


if __name__ == "__main__":
    main()

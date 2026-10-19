"""Parity of the account-administration actions of app/actions/admin.ts
(loadUserDetail, updateUserAccount, setUserRole, setUserBanned,
createAccount, resendInvite, setUserEmailVerified, sendVerificationLink)
between the reference and Rust.

    python3 server/tests/parity/admin_users.py --ref http://localhost:8430 --rust http://localhost:8438

Every export signed out, as a non-admin and as an admin: valid input, every
refusal and issue code, wrongly-typed arguments, self-targeting. After each
call the rows of this run's accounts are re-read from both databases (`user`,
`account`, `session`, `verification`, `audit_log` with its detail) and
compared, with created accounts' random ids and tags replaced by labels, and
the mails both servers handed to their SMTP sink (one on the host for Rust,
one inside the reference app container, both on 127.0.0.1) are compared with
origins and tokens normalised.

Not destructive: every run seeds its own users with the same ids on both
sides. It changes the mail settings of both servers and sets them back to
"not configured" at the end. Secrets come from the env files and are never
printed; cookie values and tokens never appear in the output.
"""

import argparse
import hashlib
import json
import os
import random
import re
import subprocess
import sys
import time
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
SCRATCH_DEFAULT = "/path/to/modlight/_scratch/srv-auth-d"

ap = argparse.ArgumentParser(allow_abbrev=False)
ap.add_argument("--scratch", default=SCRATCH_DEFAULT)
ap.add_argument("--ref-app", default="modlight-srvauth-d-ref-app-1")
ap.add_argument("--ref-db", default="modlight-srvauth-d-ref-postgres-1/modlight")
ap.add_argument("--rust-db", default="modlight-devdb-postgres-1/modlight_srv_auth_d")
ap.add_argument("--ref-env", default=None)
ap.add_argument("--rust-env", default=None)
ap.add_argument("--sink-port", type=int, default=2760)
args, _ = ap.parse_known_args()

run = parity.Run.from_args()
REF_SECRET = parity.read_env_file(args.ref_env or os.path.join(args.scratch, "ref", ".env"))["AUTH_SECRET"]
RUST_SECRET = parity.read_env_file(args.rust_env or os.path.join(args.scratch, "rust.env"))["AUTH_SECRET"]
REF_DB = parity.Db(*args.ref_db.split("/"), REF_SECRET)
RUST_DB = parity.Db(*args.rust_db.split("/"), RUST_SECRET)
DBS = [REF_DB, RUST_DB]
ACTS = parity.Actions(run, args.ref_app)
TAG = "".join(random.choice("abcdefghijklmnopqrstuvwxyz") for _ in range(6))
q = parity.sql_quote

# A Better Auth hash shape (the content is never used to sign in here).
FAKE_HASH = "0" * 32 + ":" + "1" * 128
SEEDED = []
ACTORS = []
NUMERIC_ID = str(random.randint(10 ** 8, 10 ** 9 - 1))

_IP = [0]
_IP_BASE = "10.%d.%d" % (random.randint(0, 255), random.randint(0, 255))


def pair():
    _IP[0] += 1
    return run.pair(real_ip="%s.%d" % (_IP_BASE, _IP[0] % 250 + 1))


# ── values JSON cannot carry ────────────────────────────────────────────────


class BigInt(object):
    def __init__(self, digits):
        self.digits = str(digits)


def enc_ref(v):
    """React `encodeReply`."""
    if isinstance(v, BigInt):
        return "$n" + v.digits
    if isinstance(v, list):
        return [enc_ref(x) for x in v]
    if isinstance(v, dict):
        return dict((k, enc_ref(x)) for k, x in v.items())
    return parity._encode_reply(v)


def enc_rust(v):
    """The SPA's tagged JSON."""
    if isinstance(v, BigInt):
        return {"$bigint": v.digits}
    if isinstance(v, list):
        return [enc_rust(x) for x in v]
    if isinstance(v, dict):
        return dict((k, enc_rust(x)) for k, x in v.items())
    return parity._encode_tagged(v)


# ── seeding ─────────────────────────────────────────────────────────────────


def uid(key):
    return "pd%s%s" % (TAG, key)


def addr(key):
    return "%s.%s@admin.test" % (key, TAG)


def seed_user(key, role="user", password=FAKE_HASH, user_id=None, **columns):
    """`password`: a hash, `"NULL"` for a credential row without one, or None
    for no credential row at all."""
    user_id = user_id or uid(key)
    email = columns.pop("email", addr(key))
    name = columns.pop("name", "Parity %s" % key)
    columns.setdefault("user_code", "t%s%s" % (TAG, key.replace("_", "")))
    for db in DBS:
        db.create_user(user_id, email, name, role=role, **columns)
        if password is not None:
            db.sql('INSERT INTO "account" ("id", "account_id", "provider_id", "user_id", "password", "created_at", '
                   '"updated_at") VALUES (%s, %s, \'credential\', %s, %s, now(), now())'
                   % (q(parity.random_id(32)), q(user_id), q(user_id), "NULL" if password == "NULL" else q(password)))
    SEEDED.append(user_id)
    return user_id, email


def seed_sessions(user_id, n=2):
    for db in DBS:
        for _ in range(n):
            db.mint_session(user_id)


def seed_invite(user_id, expired=False):
    token = parity.random_id(43)
    digest = hashlib.sha256(token.encode()).hexdigest()
    when = "now() - interval '1 minute'" if expired else "now() + interval '7 days'"
    for db in DBS:
        db.sql('INSERT INTO "verification" ("id", "identifier", "value", "expires_at", "created_at", "updated_at") '
               "VALUES (%s, %s, %s, %s, now(), now())" % (q(parity.random_id(32)), q("invite:" + digest), q(user_id),
                                                         when))


def seed_upload(user_id, trashed=False):
    for db in DBS:
        db.sql('INSERT INTO "uploads" ("owner_id", "code", "filename", "mime_type", "storage_key", "delete_token", '
               '"deleted_at") VALUES (%s, %s, \'a.txt\', \'text/plain\', %s, %s, %s)'
               % (q(user_id), q(parity.random_id(8)), q("k/" + parity.random_id(12)), q(parity.random_id(16)),
                  "now()" if trashed else "NULL"))


# ── state snapshots ─────────────────────────────────────────────────────────

MINE = "(u.email LIKE %s OR u.id LIKE %s OR u.id = %s)" % (q("%" + TAG + "%"), q("pd" + TAG + "%"), q(NUMERIC_ID))
LABELS = [{}, {}]  # per side: generated id -> label


def label_new_users(i, db):
    """Created accounts get random ids and tags: name them by their address."""
    out = db.sql('SELECT u.id, u.email FROM "user" u WHERE %s ORDER BY u.email' % MINE)
    for line in out.splitlines():
        user_id, email = line.split("\t")
        if user_id not in SEEDED and user_id not in LABELS[i]:
            LABELS[i][user_id] = "<new:%s>" % email


LONE_SURROGATE = re.compile("[\ud800-\udfff]")
SURROGATES_SEEN = []


def no_lone_surrogates(v, where):
    """A JavaScript string may end in half a surrogate pair (`slice` through
    an emoji) and Flight sends it as such; a Rust string cannot hold one and
    carries U+FFFD instead (what Postgres stores on both sides). Compared as
    U+FFFD, and reported."""
    if isinstance(v, str):
        if LONE_SURROGATE.search(v):
            SURROGATES_SEEN.append(where)
            return LONE_SURROGATE.sub("\ufffd", v)
        return v
    if isinstance(v, list):
        return [no_lone_surrogates(x, where) for x in v]
    if isinstance(v, dict):
        return dict((k, no_lone_surrogates(x, where)) for k, x in v.items())
    return v


def relabel(v, i):
    if isinstance(v, str):
        for user_id, label in LABELS[i].items():
            v = v.replace(user_id, label)
        return v
    if isinstance(v, list):
        return [relabel(x, i) for x in v]
    if isinstance(v, tuple):
        return tuple(relabel(x, i) for x in v)
    if isinstance(v, dict):
        return dict((k, relabel(x, i)) for k, x in v.items())
    return v


def rows(db, sql_text):
    return [line.split("\t") for line in db.sql(sql_text).splitlines()]


def snapshot(i, db):
    label_new_users(i, db)
    users = rows(db, 'SELECT u.id, u.name, u.email, u.email_verified, u.role, u.banned, u.ban_reason, '
                     'u.ban_expires IS NOT NULL, u.user_code, u.username, u.max_quota, u.used_quota, '
                     'u.allow_root_short_links, u.public_profile, u.skip_tag_in_url, u.two_factor_enabled, u.locale, '
                     'u.theme, u.updated_at > \'2001-01-01\' FROM "user" u WHERE %s ORDER BY u.email' % MINE)
    for r in users:
        if r[0] in LABELS[i] and re.match(r"^[A-Za-z]{6}$", r[8] or ""):
            r[8] = "<generated:%d>" % len(r[8])
        if r[0] in LABELS[i]:
            r[0] = "<id len=%d>" % len(r[0]) + LABELS[i][r[0]]
    accounts = rows(db, 'SELECT u.email, a.provider_id, a.account_id = a.user_id, a.password IS NULL, '
                        'a.password ~ \'^[0-9a-f]{32}:[0-9a-f]{128}$\', a.password = %s, '
                        'a.updated_at > \'2001-01-01\' FROM "account" a JOIN "user" u ON u.id = a.user_id '
                        'WHERE %s ORDER BY u.email, a.provider_id' % (q(FAKE_HASH), MINE))
    sessions = rows(db, 'SELECT u.email, count(s.id) FROM "user" u LEFT JOIN "session" s ON s.user_id = u.id '
                        'WHERE %s GROUP BY u.email ORDER BY u.email' % MINE)
    invites = rows(db, 'SELECT u.email, v.identifier ~ \'^invite:[0-9a-f]{64}$\', '
                       'round(extract(epoch FROM v.expires_at - now()) / 3600), v.updated_at = v.created_at '
                       'FROM "verification" v JOIN "user" u ON u.id = v.value WHERE %s ORDER BY u.email, v.expires_at'
                       % MINE)
    # Mark everything as untouched again, so the next snapshot shows what moved.
    db.sql('UPDATE "user" u SET updated_at = \'2000-01-01\' WHERE %s' % MINE)
    db.sql('UPDATE "account" SET updated_at = \'2000-01-01\' WHERE user_id IN (SELECT u.id FROM "user" u WHERE %s)'
           % MINE)
    return relabel({"user": users, "account": accounts, "session": sessions, "invite": invites}, i)


class AuditCursor(object):
    def __init__(self):
        self.seen = [0, 0]

    def check(self, name):
        if not ACTORS:
            return
        new = []
        for i, db in enumerate(DBS):
            out = db.sql("SELECT action, detail::text, ip, user_id FROM audit_log WHERE user_id IN (%s) "
                         "ORDER BY created_at, id" % ", ".join(q(a) for a in ACTORS))
            found = []
            for line in out.splitlines():
                action, detail, ip, user_id = line.split("\t")
                found.append({"action": action, "detail": json.loads(detail), "ip": ip, "user": user_id})
            new.append(relabel(found[self.seen[i]:], i))
            self.seen[i] = len(found)
        run.compare("audit after " + name, new[0], new[1])


AUDIT = AuditCursor()


def check_state(name):
    a, b = [snapshot(i, db) for i, db in enumerate(DBS)]
    run.compare("rows after " + name, a, b)
    AUDIT.check(name)


# ── the SMTP sinks ──────────────────────────────────────────────────────────


class Sinks(object):
    def __init__(self):
        self.host_log = os.path.join(args.scratch, "sink", "admin-users-sink.jsonl")
        self.ref_log = "/tmp/admin-users-sink.jsonl"
        os.makedirs(os.path.dirname(self.host_log), exist_ok=True)
        self.seen = [0, 0]
        self.proc = None

    def start(self):
        open(self.host_log, "w").close()
        self.proc = subprocess.Popen(["node", os.path.join(HERE, "smtp_sink.mjs"), self.host_log,
                                      str(args.sink_port), "1800"], stdout=subprocess.DEVNULL,
                                     stderr=subprocess.DEVNULL)
        subprocess.run(["docker", "cp", os.path.join(HERE, "smtp_sink.mjs"), args.ref_app + ":/tmp/smtp_sink.mjs"],
                       check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["docker", "exec", args.ref_app, "sh", "-c", ": > " + self.ref_log], check=True)
        subprocess.run(["docker", "exec", "-d", args.ref_app, "node", "/tmp/smtp_sink.mjs", self.ref_log,
                        str(args.sink_port), "1800"], check=True)
        time.sleep(1.5)

    def stop(self):
        if self.proc:
            self.proc.terminate()
        subprocess.run(["docker", "exec", args.ref_app, "sh", "-c", "kill $(cat %s.pid)" % self.ref_log],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def records(self):
        with open(self.host_log) as f:
            rust = [json.loads(line) for line in f if line.strip()]
        out = subprocess.run(["docker", "exec", args.ref_app, "cat", self.ref_log], stdout=subprocess.PIPE,
                             check=True).stdout.decode("utf-8")
        ref = [json.loads(line) for line in out.splitlines() if line.strip()]
        return ref, rust

    def check(self, name, settle=0.8):
        if not self.proc:
            return
        time.sleep(settle)
        ref, rust = self.records()
        new = [ref[self.seen[0]:], rust[self.seen[1]:]]
        self.seen = [len(ref), len(rust)]
        run.compare("mail of " + name, [mail_shape(r) for r in new[0]], [mail_shape(r) for r in new[1]])


def mail_shape(record):
    import email
    import email.policy
    msg = email.message_from_string(record["data"], policy=email.policy.default)
    texts = []
    for part in msg.walk():
        if part.get_content_type() in ("text/plain", "text/html"):
            body = part.get_content()
            body = re.sub(r"http://localhost:\d+", "<origin>", body)
            body = re.sub(r"token=[A-Za-z0-9._\-]+", "token=<token>", body)
            body = re.sub(r"/invite/[A-Za-z0-9_\-]+", "/invite/<token>", body)
            texts.append((part.get_content_type(), body.replace("\r\n", "\n").strip()))
    return {"from": record["from"], "rcpt": record["to"], "rejected": record["rejected"],
            "subject": str(msg.get("Subject")), "to": str(msg.get("To")), "texts": texts}


SINKS = Sinks()


# ── calling the actions ─────────────────────────────────────────────────────


def call_ref(browser, module, export, argv, headers):
    action_id, page = ACTS.ids.get(module, export)
    h = {"Next-Action": action_id, "Content-Type": "text/plain;charset=UTF-8", "Accept": "text/x-component"}
    h.update(headers or {})
    return browser.request("POST", page, raw=json.dumps(enc_ref(list(argv))), headers=h)


def call_rust(browser, module, export, argv, headers):
    return browser.rpc(module, export, enc_rust(list(argv)), headers=headers)


def cookies(res):
    out = []
    for name, value, attrs in parity.cookie_shape(res):
        attrs = [x for x in attrs if not x.lower().startswith("expires")]
        out.append((name, "<set>" if value else "", attrs))
    return out


def act(p, name, export, argv, module="admin", headers=None, state=True, mail=False):
    a = call_ref(p.ref, module, export, argv, headers)
    b = call_rust(p.rust, module, export, argv, headers)
    if state:
        for i, db in enumerate(DBS):
            label_new_users(i, db)
    ea = no_lone_surrogates(relabel(parity.normalize_json(parity.next_action_envelope(a)), 0), "reference " + name)
    eb = no_lone_surrogates(relabel(parity.normalize_json(parity.rpc_envelope(b)), 1), "rust " + name)
    ea["set-cookie"] = cookies(a)
    eb["set-cookie"] = cookies(b)
    run.compare(name, ea, eb)
    if run.verbose:
        print("     ref: %s" % json.dumps(ea, default=str))
    for browser, res in ((p.ref, a), (p.rust, b)):
        for cname, cvalue, _attrs in res.set_cookies():
            if cvalue:
                browser.cookies[cname] = cvalue
            else:
                browser.cookies.pop(cname, None)
    if state:
        check_state(name)
    if mail:
        SINKS.check(name)
    return ea


def as_user(user_id):
    p = pair()
    parity.signed_in(p, REF_DB, RUST_DB, user_id)
    return p


def signed_out():
    """A session cookie that does not verify: the reference's proxy.ts
    answers a cookie-less POST to /dashboard/admin itself (307), so the
    action is reached only with SOME cookie, and both servers then run the
    action's own requireAdmin()."""
    p = pair()
    p.ref.cookies[parity.SESSION_COOKIE] = "garbage"
    p.rust.cookies[parity.SESSION_COOKIE] = "garbage"
    return p


def mail_on(admin_pair, name):
    data = {"host": "127.0.0.1", "port": args.sink_port, "security": "none", "user": "",
            "from": "no-reply@sink.test", "fromName": "Parity"}
    act(admin_pair, "mail on " + name, "saveSmtpSettings", [data], module="mail", state=False)


def mail_off(admin_pair, name):
    act(admin_pair, "mail off " + name, "clearSmtpSettings", [], module="mail", state=False)


# ── scenarios ───────────────────────────────────────────────────────────────

EXPORT_ARGS = [
    ("loadUserDetail", [uid("t1")]),
    ("updateUserAccount", [uid("t1"), {"name": "x"}]),
    ("setUserRole", [uid("t1"), "admin"]),
    ("setUserBanned", [uid("t1"), True]),
    ("createAccount", [{"email": addr("gate"), "mode": "password", "password": "long-enough-1"}]),
    ("resendInvite", [uid("t1")]),
    ("setUserEmailVerified", [uid("t1"), True]),
    ("sendVerificationLink", [uid("t1")]),
]


def scenario_gates(user):
    for who, p in (("signed out", signed_out()), ("non-admin", as_user(user))):
        for export, argv in EXPORT_ARGS:
            act(p, "%s %s" % (who, export), export, argv)


def scenario_load(admin_pair):
    full, _ = seed_user("full", role="admin", banned=True, username="Full_%s" % TAG, max_quota=1 << 40,
                        used_quota=12345, allow_root_short_links=True, public_profile=True, skip_tag_in_url=True,
                        two_factor_enabled=True, image="avatars/x-tok123.webp", email_verified=False)
    seed_upload(full)
    seed_upload(full)
    seed_upload(full, trashed=True)
    seed_invite(full)
    nopw, _ = seed_user("nopw", password=None, image="https://cdn.example/a.png", banned=None, role=None,
                        two_factor_enabled=None, max_quota=None, used_quota=None, user_code=None)
    invited, _ = seed_user("invited", password="NULL")
    seed_invite(invited)
    stale, _ = seed_user("stale", password="")
    seed_invite(stale, expired=True)
    seed_user("numeric", user_id=NUMERIC_ID)
    for name, argv in [
        ("no argument", []), ("undefined", [parity.UNDEFINED]), ("null", [None]), ("empty", [""]), ("zero", [0]),
        ("false", [False]), ("NaN", [float("nan")]), ("unknown id", ["nobody"]), ("object", [{"id": full}]),
        ("array", [[full]]), ("full account", [full]), ("no credential row", [nopw]),
        ("invited, no password", [invited]), ("empty password, expired invite", [stale]),
        ("numeric id", [int(NUMERIC_ID)]), ("bigint id", [BigInt(NUMERIC_ID)]), ("true", [True]),
    ]:
        act(admin_pair, "loadUserDetail " + name, "loadUserDetail", argv, state=False)


def scenario_verified(admin_pair):
    t, _ = seed_user("ver", email_verified=False)
    s = "setUserEmailVerified"
    for name, argv in [
        ("falsy id", ["", True]), ("unknown id", ["nobody", True]), ("verify", [t, True]),
        ("unverify", [t, False]), ("truthy string", [t, "yes"]), ("number one", [t, 1]),
        ("unparsable string throws", [t, "maybe"]), ("null throws (NOT NULL)", [t, None]),
        ("undefined throws (no values)", [t]), ("NaN throws", [t, float("nan")]),
        ("date throws", [t, parity.JsDate("2026-10-18T12:00:00.000Z")]), ("numeric id", [int(NUMERIC_ID), True]),
        ("bigint id: written, audit not", [BigInt(NUMERIC_ID), False]), ("bigint flag", [t, BigInt(1)]),
        ("object id", [{"a": 1}, True]),
    ]:
        act(admin_pair, "%s %s" % (s, name), s, argv)


def scenario_role_ban(admin_pair, admin):
    t, _ = seed_user("rb")
    seed_sessions(t)
    other_admin, _ = seed_user("rbadmin", role="admin")
    nullrole, _ = seed_user("rbnull", role=None, banned=None)
    seed_sessions(nullrole)
    r, b = "setUserRole", "setUserBanned"
    for name, argv in [
        ("falsy id", [None, "admin"]), ("unknown", ["nobody", "admin"]), ("self demote", [admin, "user"]),
        ("self same role", [admin, "admin"]), ("self role missing", [admin]), ("promote", [t, "admin"]),
        ("same again", [t, "admin"]), ("demote", [t, "user"]), ("unvalidated role", [t, "superuser"]),
        ("role array", [t, ["admin", "user"]]), ("role null: changed but not applied", [t, None]),
        ("role empty string", [t, ""]), ("role number throws", [t, 5]), ("demote other admin", [other_admin, "user"]),
        ("null stored role vs user", [nullrole, "user"]), ("null stored role promote", [nullrole, "admin"]),
        ("role date throws", [t, parity.JsDate("2026-10-18T12:00:00.000Z")]), ("back to user", [t, "user"]),
    ]:
        act(admin_pair, "%s %s" % (r, name), r, argv)
    for name, argv in [
        ("self ban", [admin, True]), ("self unban (no change)", [admin, False]), ("ban", [t, True]),
        ("ban again", [t, True]), ("unban", [t, False]), ("truthy string bans", [t, "yes"]),
        ("zero unbans", [t, 0]), ("null on an unbanned account", [t, None]), ("missing", [t]),
        ("null stored ban, false", [nullrole, False]), ("null stored ban, true", [nullrole, True]),
        ("ban other admin", [other_admin, True]), ("numeric id", [int(NUMERIC_ID), True]),
        ("numeric id unban", [int(NUMERIC_ID), False]),
    ]:
        act(admin_pair, "%s %s" % (b, name), b, argv)


def scenario_update(admin_pair, admin, admin_email):
    u = "updateUserAccount"
    t, t_email = seed_user("up", username="up_%s" % TAG, locale="pl")
    seed_sessions(t)
    seed_invite(t)
    holder, _ = seed_user("holder", username="Holder_%s" % TAG, user_code="h%s" % TAG)
    retired, _ = seed_user("retired")
    for db in DBS:
        db.sql('INSERT INTO "user_code_aliases" ("code", "user_id") VALUES (%s, %s)' % (q("old%s" % TAG), q(retired)))
    bare, _ = seed_user("bare", password=None)
    seed_sessions(bare, 1)

    cases = [
        ("falsy id", [0, {"name": "x"}]),
        ("unknown id", ["nobody", {"name": "x"}]),
        ("unknown id, undefined patch", ["nobody"]),
        ("undefined patch throws", [t]),
        ("null patch throws", [t, None]),
        ("string patch is empty", [t, "name"]),
        ("empty patch", [t, {}]),
        ("undefined keys are absent", [t, {"name": parity.UNDEFINED, "role": parity.UNDEFINED}]),
        ("name trimmed", [t, {"name": "  New Name  "}]),
        ("same name", [t, {"name": "New Name"}]),
        ("long name cut", [t, {"name": "N" * 130}]),
        ("emoji name cut through a pair", [t, {"name": "a" * 119 + "\U0001F600"}]),
        ("blank name", [t, {"name": "   "}]),
        ("numeric name throws", [t, {"name": 5}]),
        ("username", [t, {"username": "  New_%s  " % TAG}]),
        ("username taken in another case", [t, {"username": ("holder_%s" % TAG).upper()}]),
        ("username own in another case", [t, {"username": ("NEW_%s" % TAG)}]),
        ("username charset", [t, {"username": "-bad"}]),
        ("username too short", [t, {"username": "a"}]),
        ("username cleared by empty", [t, {"username": ""}]),
        ("username null (already clear)", [t, {"username": None}]),
        ("username number throws", [t, {"username": 7}]),
        ("email invalid", [t, {"email": "nope"}]),
        ("email too long", [t, {"email": "a" * 250 + "@x.test"}]),
        ("email number throws", [t, {"email": 5}]),
        ("email same in another case", [t, {"email": "  " + t_email.upper()}]),
        ("email taken", [t, {"email": addr("holder").upper()}]),
        ("password too short", [t, {"newPassword": "short"}]),
        ("password too long", [t, {"newPassword": "x" * 129}]),
        ("empty password ignored", [t, {"newPassword": ""}]),
        ("numeric password ignored", [t, {"newPassword": 12345678}]),
        ("quota string", [t, {"maxQuota": "1024"}]),
        ("quota NaN", [t, {"maxQuota": float("nan")}]),
        ("quota Infinity", [t, {"maxQuota": float("inf")}]),
        ("quota null", [t, {"maxQuota": None}]),
        ("quota bigint", [t, {"maxQuota": BigInt(5)}]),
        ("quota huge", [t, {"maxQuota": 1e300}]),
        ("quota rounded", [t, {"maxQuota": 1536.5}]),
        ("quota negative = unlimited", [t, {"maxQuota": -3}]),
        ("quota same unlimited", [t, {"maxQuota": -1}]),
        ("permissions", [t, {"allowRootShortLinks": True, "publicProfile": True, "skipTagInUrl": False}]),
        ("permissions null", [t, {"publicProfile": None}]),
        ("permission string yes", [t, {"publicProfile": "yes"}]),
        ("permission number one", [t, {"skipTagInUrl": 1}]),
        ("tag", [t, {"tag": "  up%s  " % TAG}]),
        ("same tag", [t, {"tag": "up%s" % TAG}]),
        ("tag taken", [t, {"tag": "h%s" % TAG}]),
        ("tag retired", [t, {"tag": "old%s" % TAG}]),
        ("tag reserved", [t, {"tag": "dashboard"}]),
        ("tag charset", [t, {"tag": "-x"}]),
        ("tag too short", [t, {"tag": "a"}]),
        ("tag number throws", [t, {"tag": 5}]),
        ("tag number with an issue returns the issue", [t, {"tag": 5, "name": ""}]),
        ("tag null ignored", [t, {"tag": None}]),
        ("bad permission fails the update after the tag", [t, {"tag": "up2%s" % TAG, "publicProfile": "maybe",
                                                              "name": "Never Written"}]),
        ("every issue at once", [t, {"name": "", "username": "!", "email": "x", "newPassword": "a",
                                     "maxQuota": "x", "role": "admin", "banned": True}]),
        ("self: role and ban refused", [admin, {"role": "user", "banned": True}]),
        ("self: same role and ban", [admin, {"role": "admin", "banned": False, "name": "Admin %s" % TAG}]),
        ("role and ban of another", [t, {"role": "admin", "banned": True}]),
        ("unban and demote", [t, {"role": "user", "banned": False}]),
        ("numeric id", [int(NUMERIC_ID), {"name": "Numeric", "username": "num_%s" % TAG}]),
    ]
    for name, argv in cases:
        act(admin_pair, "%s %s" % (u, name), u, argv)

    # Wrongly-typed passwords pass validation and throw at the hash, after the
    # columns and the role were written.
    act(admin_pair, u + " array password throws after writing", u,
        [t, {"newPassword": ["a"] * 8, "name": "Array Pw", "role": "admin"}])
    act(admin_pair, u + " object password throws", u, [t, {"newPassword": {"length": 10}, "name": "Object Pw"}])

    # Mail off: password and email changes still go through, nothing is sent.
    act(admin_pair, u + " password, mail off", u, [t, {"newPassword": "long-enough-1"}])
    act(admin_pair, u + " no credential row: inserted", u, [bare, {"newPassword": "long-enough-1"}])

    mail_on(admin_pair, "(update)")
    act(admin_pair, u + " email change (invites revoked, old address told)", u,
        [t, {"email": "  Moved.%s@Admin.TEST " % TAG}], mail=True)
    act(admin_pair, u + " password change (Polish target)", u, [t, {"newPassword": "another-long-1"}], mail=True)
    for db in DBS:
        db.sql('UPDATE "user" SET locale = NULL WHERE id = %s' % q(t))
    act(admin_pair, u + " both, browser locale pl", u,
        [t, {"email": addr("moved2"), "newPassword": "third-long-pw", "name": "Both"}],
        headers={"Accept-Language": "de, pl;q=0.5"}, mail=True)
    act(admin_pair, u + " both, browser locale en", u,
        [t, {"email": addr("moved3"), "newPassword": "fourth-long-pw"}], headers={"Accept-Language": "en-US"},
        mail=True)
    act(admin_pair, u + " rejected recipient still succeeds", u,
        [t, {"email": "reject.up.%s@admin.test" % TAG, "newPassword": "fifth-long-pw1"}], mail=True)
    act(admin_pair, u + " everything at once", u,
        [t, {"name": "  All  ", "username": "all_%s" % TAG, "email": addr("all"), "tag": "all%s" % TAG,
             "newPassword": "sixth-long-pw", "maxQuota": 4096, "role": "admin", "banned": True,
             "allowRootShortLinks": False, "publicProfile": False, "skipTagInUrl": True}], mail=True)

    # An admin resetting their own password ends their own session too.
    me = as_user(admin)
    act(me, u + " own password", u, [admin, {"newPassword": "admin-long-pw"}], mail=True)
    act(me, u + " after revoking own sessions", u, [admin, {"name": "x"}])

    # A session due for a refresh: the cookie comes back and the call revalidates.
    p = as_user(admin)
    for browser, db in ((p.ref, REF_DB), (p.rust, RUST_DB)):
        token = urllib.parse.unquote(browser.cookies[parity.SESSION_COOKIE]).split(".")[0]
        db.sql('UPDATE "session" SET expires_at = now() + interval \'5 days\' WHERE token = %s' % q(token))
    act(p, u + " with a session due for refresh", u, [t, {"role": "user", "banned": False}])
    return admin_email


def scenario_create(admin_pair):
    c = "createAccount"
    seed_user("existing")
    cases = [
        ("no input throws", []), ("null input throws", [None]), ("string input throws", ["x"]),
        ("numeric email throws", [{"email": 5, "mode": "password", "password": "long-enough-1"}]),
        ("invalid email", [{"email": "nope", "mode": "password"}]),
        ("too long email", [{"email": "a" * 250 + "@x.test", "mode": "password"}]),
        ("missing password", [{"email": addr("c1"), "mode": "password"}]),
        ("short password", [{"email": addr("c1"), "mode": "password", "password": "short"}]),
        ("empty password", [{"email": addr("c1"), "password": ""}]),
        ("NaN password", [{"email": addr("c1"), "password": float("nan")}]),
        ("taken, any case", [{"email": "  " + addr("existing").upper(), "mode": "password",
                              "password": "long-enough-1"}]),
        ("invite without mail", [{"email": addr("c1"), "mode": "invite"}]),
        ("numeric password: zod refuses", [{"email": addr("c2"), "password": 12345678}]),
        ("array password: zod refuses", [{"email": addr("c2"), "password": ["a"] * 8}]),
        ("object password: zod refuses", [{"email": addr("c2"), "password": {"length": 9}}]),
        ("date password: zod refuses", [{"email": addr("c2"), "password": parity.JsDate("2026-10-18T12:00:00.000Z")}]),
        ("bigint password: zod refuses", [{"email": addr("c2"), "password": BigInt(123)}]),
        ("Infinity password: zod refuses", [{"email": addr("c2"), "password": float("inf")}]),
        ("numeric role: zod refuses", [{"email": addr("c2"), "password": "long-enough-1", "role": 5}]),
        ("numeric name fails as data", [{"email": addr("c2"), "password": "long-enough-1", "name": 5}]),
        ("zod stricter than the pattern", [{"email": "a..b.%s@admin.test" % TAG, "password": "long-enough-1"}]),
        ("password mode", [{"email": "  C3.%s@Admin.TEST " % TAG, "mode": "password", "password": "long-enough-1",
                            "name": "  Third  "}]),
        ("unknown mode is password mode", [{"email": addr("c4"), "mode": "invite ", "password": "long-enough-1"}]),
        ("no mode, admin role, blank name", [{"email": addr("c5"), "password": "long-enough-1", "role": "admin",
                                              "name": "   "}]),
        ("null role", [{"email": addr("c6"), "password": "long-enough-1", "role": None}]),
        ("unvalidated role", [{"email": addr("c7"), "password": "long-enough-1", "role": "superuser"}]),
        ("empty role", [{"email": addr("c8"), "password": "long-enough-1", "role": ""}]),
        ("role array", [{"email": addr("c9"), "password": "long-enough-1", "role": ["admin", "user"]}]),
        ("date mode", [{"email": addr("c10"), "password": "long-enough-1",
                        "mode": parity.JsDate("2026-10-18T12:00:00.000Z")}]),
        ("bigint mode: created, audit not", [{"email": addr("c11"), "password": "long-enough-1",
                                              "mode": BigInt(7)}]),
    ]
    for name, argv in cases:
        act(admin_pair, "%s %s" % (c, name), c, argv)

    mail_on(admin_pair, "(create)")
    act(admin_pair, c + " invite", c, [{"email": addr("inv1"), "mode": "invite", "name": "Invitee"}], mail=True)
    act(admin_pair, c + " invite, Polish browser", c, [{"email": addr("inv2"), "mode": "invite"}],
        headers={"Accept-Language": "pl-PL,pl;q=0.9"}, mail=True)
    act(admin_pair, c + " invite, recipient refused", c, [{"email": "reject.%s@admin.test" % TAG, "mode": "invite"}],
        mail=True)
    act(admin_pair, c + " invite ignores the password", c,
        [{"email": addr("inv3"), "mode": "invite", "password": 5, "role": "admin"}], mail=True)
    act(admin_pair, c + " password mode sends nothing", c,
        [{"email": addr("c12"), "mode": "password", "password": "long-enough-1"}], mail=True)


def scenario_invites(admin_pair):
    r = "resendInvite"
    t, _ = seed_user("ri", password="NULL", locale="pl")
    seed_invite(t)
    seed_invite(t, expired=True)
    other, _ = seed_user("ri2")
    refused, _ = seed_user("ri3", email="reject.ri.%s@admin.test" % TAG)
    seed_invite(refused)
    mail_off(admin_pair, "(invites)")
    act(admin_pair, r + " falsy id", r, [""])
    act(admin_pair, r + " mail off", r, [t])
    mail_on(admin_pair, "(invites)")
    for name, argv, headers in [
        ("falsy id", [None], None), ("unknown id", ["nobody"], None), ("pending invitation, Polish account", [t], None),
        ("any account, browser locale", [other], {"Accept-Language": "pl"}), ("numeric id", [int(NUMERIC_ID)], None),
        ("bigint id: sent, audit not", [BigInt(NUMERIC_ID)], None), ("send failure", [refused], None),
    ]:
        act(admin_pair, "%s %s" % (r, name), r, argv, headers=headers, mail=True)


def scenario_verification_links(admin_pair):
    s = "sendVerificationLink"
    unverified, _ = seed_user("vl", email_verified=False, locale="pl")
    verified, _ = seed_user("vl2")
    plain, _ = seed_user("vl3", email_verified=False)
    refused, _ = seed_user("vl4", email="reject.vl.%s@admin.test" % TAG, email_verified=False)
    mail_off(admin_pair, "(verification links)")
    act(admin_pair, s + " falsy id", s, [False])
    act(admin_pair, s + " mail off", s, [unverified])
    mail_on(admin_pair, "(verification links)")
    for name, argv, headers in [
        ("unknown id", ["nobody"], None), ("unverified, Polish account", [unverified], None),
        ("verified: nothing sent", [verified], None), ("unverified, browser pl ignored", [plain],
                                                       {"Accept-Language": "pl"}),
        ("recipient refused", [refused], None), ("object id", [{"a": 1}], None),
    ]:
        act(admin_pair, "%s %s" % (s, name), s, argv, headers=headers, mail=True)


def main():
    admin, admin_email = seed_user("admin", role="admin")
    ACTORS.append(admin)
    user, _ = seed_user("user")
    seed_user("t1")
    admin_pair = as_user(admin)
    mail_off(admin_pair, "(start)")
    SINKS.start()
    try:
        scenario_gates(user)
        scenario_load(admin_pair)
        scenario_verified(admin_pair)
        scenario_role_ban(admin_pair, admin)
        scenario_create(as_user(admin))
        mail_off(admin_pair, "(before update)")
        scenario_update(as_user(admin), admin, admin_email)
        scenario_invites(as_user(admin))
        scenario_verification_links(as_user(admin))
    finally:
        try:
            mail_off(as_user(admin), "(end)")
        finally:
            SINKS.stop()
    for where in SURROGATES_SEEN:
        print("NOTE lone surrogate compared as U+FFFD: %s" % where)
    run.finish()


if __name__ == "__main__":
    main()

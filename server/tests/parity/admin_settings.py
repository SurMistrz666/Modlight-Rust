"""Parity: the mail and registration admin actions (CONTRACT 7.4 `mail`,
`registration`).

    python3 server/tests/parity/admin_settings.py --ref http://localhost:8427 --rust http://localhost:8435

Every action through both protocols (signed out, non-admin, admin), the
`settings` rows and the `audit_log` rows re-read from both databases after
each write (encrypted values compared by shape: each server has its own
secret), and `sendTestMail` against nothing, a refused port, an unknown host,
and a scriptable SMTP sink (`smtp_sink.mjs`) that runs once on the host (for
the Rust server) and once inside the reference app container, both on
127.0.0.1, so one configuration means the same server to both. The messages
the sinks received are compared too (envelope, headers, MIME tree, decoded
parts, the inline logo).

`loadMailStatus` and `loadRegistrationSettings` are not callable on the
reference (no client bundle imports them): they are checked on Rust against
the status the reference's last action returned for the same state, and
against the database.

Each admin may send five test mails per 15 minutes (per process), so the
script rotates through fresh admins; every run seeds new users.
"""

import argparse
import email
import email.policy
import hashlib
import json
import os
import random
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
SCRATCH = os.environ.get("PARITY_SCRATCH", "/Users/marcinasdfg/Desktop/modlight/_scratch/srv-auth-m")
SINK_BASE = 2525


def env_args():
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--ref-app", default="modlight-srvauth-m-ref-app-1")
    p.add_argument("--ref-db-container", default="modlight-srvauth-m-ref-postgres-1")
    p.add_argument("--ref-db", default="modlight")
    p.add_argument("--ref-env", default=os.path.join(SCRATCH, "ref", ".env"))
    p.add_argument("--rust-db-container", default="modlight-devdb-postgres-1")
    p.add_argument("--rust-db", default="modlight_srv_auth_m")
    p.add_argument("--rust-env", default=os.path.join(SCRATCH, "rust.env"))
    p.add_argument("--skip-slow", action="store_true", help="skip the 10 s greeting timeout case")
    a, _ = p.parse_known_args()
    return a


ARGS = env_args()
run = parity.Run.from_args()
REF_DB = parity.Db(ARGS.ref_db_container, ARGS.ref_db, parity.read_env_file(ARGS.ref_env)["AUTH_SECRET"])
RUST_DB = parity.Db(ARGS.rust_db_container, ARGS.rust_db, parity.read_env_file(ARGS.rust_env)["AUTH_SECRET"])
ACTS = parity.Actions(run, ARGS.ref_app)
TAG = "".join(random.choice("abcdefghijklmnopqrstuvwxyz") for _ in range(6))

MAIL_KEYS = ("smtp_host", "smtp_port", "smtp_security", "smtp_user", "smtp_password_enc", "mail_from",
             "mail_from_name", "email_verification_required")
REG_KEYS = ("register_enabled", "registration_key_enc")
USERS = []


def both_db(fn):
    return fn(REF_DB), fn(RUST_DB)


def seed_user(key, role="user"):
    user_id = "pm%s%s" % (TAG, key)
    email_ = "%s.%s@settings.test" % (key, TAG)
    for db in (REF_DB, RUST_DB):
        db.create_user(user_id, email_, "Parity %s" % key, role=role)
    USERS.append(user_id)
    return user_id, email_


# ── database snapshots ──────────────────────────────────────────────────────


def settings_rows(db, keys):
    out = db.sql("SELECT key, value FROM settings WHERE key IN (%s) ORDER BY key"
                 % ", ".join(parity.sql_quote(k) for k in keys))
    rows = {}
    for line in out.splitlines():
        key, _, value = line.partition("\t")
        if key.endswith("_enc") and value:
            value = "<v1 box>" if value.startswith("v1.") and value.count(".") == 3 else "<other box>"
        rows[key] = value if value != "" else None
    return rows


class AuditCursor(object):
    """The audit rows written since the last look, for this run's users."""

    def __init__(self):
        self.seen = [0, 0]

    def rows(self, db):
        out = db.sql("SELECT action, detail::text, ip, user_id FROM audit_log WHERE user_id IN (%s) "
                     "ORDER BY created_at, id" % ", ".join(parity.sql_quote(u) for u in USERS))
        rows = []
        for line in out.splitlines():
            action, detail, ip, user_id = line.split("\t")
            rows.append({"action": action, "detail": json.loads(detail), "ip": ip, "user": user_id})
        return rows

    def check(self, name):
        if not USERS:
            return
        new = []
        for i, db in enumerate((REF_DB, RUST_DB)):
            rows = self.rows(db)
            new.append(rows[self.seen[i]:])
            self.seen[i] = len(rows)
        run.compare("audit after " + name, new[0], new[1])


AUDIT = AuditCursor()


def check_state(name, keys=MAIL_KEYS + REG_KEYS):
    a, b = both_db(lambda db: settings_rows(db, keys))
    run.compare("settings after " + name, a, b)
    AUDIT.check(name)


def act(pair, name, module, export, args, ignore=(), keys=MAIL_KEYS + REG_KEYS):
    a, b = ACTS.check(pair, name, module, export, args, ignore=ignore)
    check_state(name, keys)
    return parity.next_action_envelope(a), parity.rpc_envelope(b)


def rust_rpc(pair, module, export, args):
    return parity.rpc_envelope(pair.rust.rpc(module, export, args))


# ── the SMTP sinks ──────────────────────────────────────────────────────────


class Sinks(object):
    def __init__(self):
        self.host_log = os.path.join(SCRATCH, "sink", "rust-sink.jsonl")
        self.ref_log = "/tmp/parity-sink.jsonl"
        os.makedirs(os.path.dirname(self.host_log), exist_ok=True)

    def start(self):
        self.proc = parity.start_rust_sink(os.path.join(HERE, "smtp_sink.mjs"), self.host_log, SINK_BASE, 3600)
        subprocess.run(["docker", "cp", os.path.join(HERE, "smtp_sink.mjs"), ARGS.ref_app + ":/tmp/smtp_sink.mjs"],
                       check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["docker", "exec", "-d", ARGS.ref_app, "node", "/tmp/smtp_sink.mjs", self.ref_log,
                        str(SINK_BASE), "3600"], check=True)
        time.sleep(1.5)

    def stop(self):
        self.proc.terminate()
        subprocess.run(["docker", "exec", ARGS.ref_app, "sh", "-c", "kill $(cat %s.pid)" % self.ref_log],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def records(self):
        with open(self.host_log) as f:
            rust = [json.loads(line) for line in f if line.strip()]
        out = subprocess.run(["docker", "exec", ARGS.ref_app, "cat", self.ref_log], stdout=subprocess.PIPE,
                             check=True).stdout.decode("utf-8")
        ref = [json.loads(line) for line in out.splitlines() if line.strip()]
        return ref, rust


SINKS = Sinks()


def mime_tree(part):
    node = {"type": part.get_content_type()}
    if part.is_multipart():
        node["parts"] = [mime_tree(p) for p in part.iter_parts()]
        return node
    payload = part.get_payload(decode=True) or b""
    if node["type"].startswith("text/"):
        node["content"] = payload.decode(part.get_content_charset() or "utf-8").replace("\r\n", "\n").rstrip("\n")
    else:
        node["sha256"] = hashlib.sha256(payload).hexdigest()
        node["content-id"] = part.get("Content-ID")
        node["filename"] = part.get_filename()
        node["disposition"] = part.get_content_disposition()
    return node


def message_shape(record):
    msg = email.message_from_string(record["data"], policy=email.policy.default)
    mid = str(msg.get("Message-ID") or "")
    return {
        "envelope from": record["from"],
        "rcpt": record["to"],
        "rejected": record["rejected"],
        "auth": record["auth"],
        "from": str(msg.get("From")),
        "to": str(msg.get("To")),
        "subject": str(msg.get("Subject")),
        "mime-version": str(msg.get("MIME-Version")),
        "date present": bool(msg.get("Date")),
        "message-id domain": mid.rsplit("@", 1)[-1] if "@" in mid else None,
        "body": mime_tree(msg),
    }


class SinkCursor(object):
    def __init__(self):
        self.seen = [0, 0]

    def check(self, name):
        ref, rust = SINKS.records()
        new = [ref[self.seen[0]:], rust[self.seen[1]:]]
        self.seen = [len(ref), len(rust)]
        run.compare("delivered by " + name, [message_shape(r) for r in new[0]], [message_shape(r) for r in new[1]])


DELIVERED = SinkCursor()


# ── test-mail admins (5 sends each per 15 minutes) ──────────────────────────


class Senders(object):
    """Signs the pair in as an admin with test mails left in its budget."""

    def __init__(self, pair):
        self.pair = pair
        self.used = 5
        self.count = 0
        self.current = None

    def next(self):
        if self.used >= 5:
            self.count += 1
            self.current, _ = seed_user("mailer%d" % self.count, role="admin")
            self.used = 0
        self.used += 1
        parity.signed_in(self.pair, REF_DB, RUST_DB, self.current)


def configure(pair, name, **fields):
    data = {"host": "127.0.0.1", "port": SINK_BASE, "security": "none", "user": "", "from": "no-reply@sink.test",
            "fromName": "Parity"}
    data.update(fields)
    return act(pair, "save " + name, "mail", "saveSmtpSettings", [data])


def test_mail(senders, name, to=parity.UNDEFINED, delivered=False, ignore_server_says=False, expect=None):
    """sendTestMail as a fresh-enough admin; `expect` is the outcome the
    scenario is meant to produce (`"ok"`, a failure kind, or `"error"`), so a
    scenario that silently stopped exercising its path fails."""
    senders.next()
    ignore = ("tookMs", "serverSays") if ignore_server_says else ("tookMs",)
    args = [] if to is parity.UNDEFINED else [to]
    _, rust = act(senders.pair, "test mail: " + name, "mail", "sendTestMail", args, ignore=ignore)
    ok = rust.get("ok")
    got = "error" if "error" in rust else ("ok" if isinstance(ok, dict) and ok.get("ok") else (ok or {}).get("kind"))
    if expect is not None:
        run.compare("test mail outcome: " + name, expect, got)
    if delivered:
        DELIVERED.check(name)


def signed_out_or(user_id):
    """A pair signed in as `user_id`, or signed out.

    Signed out means a session cookie with a bad signature: these actions are
    bundled with /dashboard/admin, so the harness posts them there on the
    reference, and without ANY session cookie proxy.ts answers that path
    first (307 to /login?next=%2Fdashboard%2Fadmin, verified) and the action
    never runs. The SPA posts to /_rpc, which has no such gate; with a cookie
    that does not verify, both servers reach the action's own requireAdmin().
    """
    p = run.pair()
    if user_id:
        parity.signed_in(p, REF_DB, RUST_DB, user_id)
    else:
        p.ref.cookies[parity.SESSION_COOKIE] = "garbage"
        p.rust.cookies[parity.SESSION_COOKIE] = "garbage"
    return p


# ── registration ────────────────────────────────────────────────────────────


def registration(pair, admin, user):
    for who, setup in (("signed out", None), ("non-admin", user)):
        p = signed_out_or(setup)
        for export, args in (("setRegistrationEnabled", [True]), ("saveRegistrationKey", ["k"]),
                             ("revealRegistrationKey", [])):
            act(p, "%s %s" % (who, export), "registration", export, args)
        run.compare("%s loadRegistrationSettings (rust)" % who,
                    {"redirect": "/login" if not setup else "/dashboard"},
                    rust_rpc(p, "registration", "loadRegistrationSettings", []))

    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    last = None
    for name, value in [("on", True), ("off", False), ("truthy string", "yes"), ("zero", 0),
                        ("undefined", parity.UNDEFINED), ("empty object", {}), ("NaN", float("nan")),
                        ("date", parity.JsDate("2026-10-18T12:00:00.000Z")), ("on again", True)]:
        ref, _ = act(pair, "setRegistrationEnabled " + name, "registration", "setRegistrationEnabled", [value])
        last = ref.get("ok")
    act(pair, "setRegistrationEnabled no argument", "registration", "setRegistrationEnabled", [])
    run.compare("loadRegistrationSettings (rust) = reference state", {"enabled": False, "keyState": "unset"},
                rust_rpc(pair, "registration", "loadRegistrationSettings", []).get("ok"))
    act(pair, "setRegistrationEnabled back on", "registration", "setRegistrationEnabled", [True])
    del last

    for name, key in [("padded", "  team-key  "), ("exactly 200", "k" * 200), ("201", "k" * 201),
                      ("200 code units of emoji", "\U0001F600" * 100), ("202 code units", "\U0001F600" * 101),
                      ("number", 5), ("object", {"trim": 1}), ("unicode", "Zażółć-klucz")]:
        act(pair, "saveRegistrationKey " + name, "registration", "saveRegistrationKey", [key])
    act(pair, "revealRegistrationKey (set)", "registration", "revealRegistrationKey", [])
    status = both_db(lambda db: settings_rows(db, REG_KEYS))
    run.compare("loadRegistrationSettings (rust) with a key", {"enabled": True, "keyState": "set"},
                rust_rpc(pair, "registration", "loadRegistrationSettings", []).get("ok"))
    del status

    # A key the server can no longer read: the panel is told, the reveal gets null.
    for db in (REF_DB, RUST_DB):
        db.sql("UPDATE settings SET value = 'v1.AAAAAAAAAAAAAAAA.AAAAAAAAAAAAAAAAAAAAAA.AAAA' "
               "WHERE key = 'registration_key_enc'")
    act(pair, "setRegistrationEnabled (undecryptable key)", "registration", "setRegistrationEnabled", [True])
    act(pair, "revealRegistrationKey (undecryptable)", "registration", "revealRegistrationKey", [])
    run.compare("loadRegistrationSettings (rust) undecryptable", {"enabled": True, "keyState": "undecryptable"},
                rust_rpc(pair, "registration", "loadRegistrationSettings", []).get("ok"))

    for name, key in [("null (clear)", None), ("empty", ""), ("undefined", parity.UNDEFINED), ("spaces", "   ")]:
        act(pair, "saveRegistrationKey " + name, "registration", "saveRegistrationKey", [key])
    act(pair, "saveRegistrationKey no argument", "registration", "saveRegistrationKey", [])
    act(pair, "revealRegistrationKey (unset)", "registration", "revealRegistrationKey", [])
    act(pair, "setRegistrationEnabled finally off", "registration", "setRegistrationEnabled", [False])


# ── mail ────────────────────────────────────────────────────────────────────


def status_matches(pair, name, ref_env):
    """loadMailStatus on Rust against the status the reference just returned."""
    ok = ref_env.get("ok")
    status = ok.get("status") if isinstance(ok, dict) and "status" in ok else ok
    run.compare("loadMailStatus (rust) after " + name, status, rust_rpc(pair, "mail", "loadMailStatus", []).get("ok"))


def mail(pair, admin, user):
    good = {"host": "127.0.0.1", "port": SINK_BASE, "security": "none", "user": "", "from": "no-reply@sink.test",
            "fromName": "Parity"}
    for who, setup in (("signed out", None), ("non-admin", user)):
        p = signed_out_or(setup)
        for export, args in (("saveSmtpSettings", [good]), ("clearSmtpSettings", []), ("clearSmtpPassword", []),
                             ("setVerificationRequired", [True]), ("sendTestMail", [])):
            act(p, "%s %s" % (who, export), "mail", export, args)
        run.compare("%s loadMailStatus (rust)" % who, {"redirect": "/login" if not setup else "/dashboard"},
                    rust_rpc(p, "mail", "loadMailStatus", []))

    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    ref, _ = act(pair, "clearSmtpSettings (start)", "mail", "clearSmtpSettings", [])
    status_matches(pair, "clearSmtpSettings", ref)
    senders = Senders(pair)
    test_mail(senders, "nothing configured", expect="not-configured")
    parity.signed_in(pair, REF_DB, RUST_DB, admin)

    # Validation, in upstream's order: host, from, port, then the password rule.
    for name, fields in [
        ("empty host", {"host": "  "}), ("host with a space", {"host": "smtp example"}),
        ("host with a slash", {"host": "smtp/x"}), ("bad from", {"from": "nope"}), ("from without a dot", {"from": "a@b"}),
        ("bad host and from", {"host": "", "from": "nope"}), ("port 0", {"port": 0}), ("port 65536", {"port": 65536}),
        ("port as a string", {"port": "587"}), ("port NaN", {"port": float("nan")}),
        ("port Infinity", {"port": float("inf")}), ("port missing", {"port": parity.UNDEFINED}),
        ("user without a password", {"user": "someone"}),
    ]:
        data = dict(good)
        data.update(fields)
        act(pair, "save invalid: " + name, "mail", "saveSmtpSettings", [data])
    for name, arg in [("null input", None), ("no input", parity.UNDEFINED), ("numeric host", dict(good, host=5)),
                      ("missing from", dict(good, **{"from": parity.UNDEFINED})),
                      ("numeric user", dict(good, user=7)), ("missing fromName", dict(good, fromName=parity.UNDEFINED))]:
        act(pair, "save throws: " + name, "mail", "saveSmtpSettings", [arg])

    ref, _ = configure(pair, "relay (no auth)", port=SINK_BASE + 0.9, fromName="  Parity  ", security="bogus")
    status_matches(pair, "save relay", ref)
    ref, _ = configure(pair, "relay again (nothing changed)", security="none")
    test_mail(senders, "own address", delivered=True, expect="ok")
    test_mail(senders, "named and upper-case domain", "  Some One <someone@Example.TEST>  ", delivered=True, expect="ok")
    test_mail(senders, "two recipients", "a@x.test, b@x.test", delivered=True, expect="ok")
    test_mail(senders, "one accepted, one rejected", "a@x.test; reject@x.test", delivered=True, expect="ok")
    test_mail(senders, "no address at all", "not-an-address", expect="recipient")
    test_mail(senders, "recipient rejected", "reject@x.test", expect="recipient")
    test_mail(senders, "message refused", "reject-data@x.test", delivered=True, expect="recipient")
    test_mail(senders, "non-string recipient", 5, expect="error")
    test_mail(senders, "unicode name", "Zażółć <zz@x.test>", delivered=True, expect="ok")

    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    configure(pair, "authenticated", user="good-user", password="good-pass")
    test_mail(senders, "AUTH PLAIN", delivered=True, expect="ok")
    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    ref, _ = configure(pair, "same identity, no password (kept)", user="  good-user ")
    status_matches(pair, "save kept password", ref)
    configure(pair, "password as a number (kept, truthy)", user="good-user", password=5)
    configure(pair, "password as an array (throws after writing)", user="good-user", password=["x"])
    configure(pair, "wrong password", user="good-user", password="bad-pass")
    test_mail(senders, "AUTH failure", expect="auth")
    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    configure(pair, "AUTH LOGIN only", port=SINK_BASE + 6, user="good-user", password="good-pass")
    test_mail(senders, "AUTH LOGIN", delivered=True, expect="ok")
    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    configure(pair, "AUTH CRAM-MD5 only", port=SINK_BASE + 7, user="good-user", password="good-pass")
    test_mail(senders, "AUTH CRAM-MD5", delivered=True, expect="ok")
    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    configure(pair, "EHLO refused", port=SINK_BASE + 5, user="good-user", password="good-pass")
    test_mail(senders, "HELO fallback", delivered=True, expect="ok")
    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    configure(pair, "host change forgets the password", host="localhost", user="good-user")
    ref, _ = act(pair, "clearSmtpPassword", "mail", "clearSmtpPassword", [])
    status_matches(pair, "clearSmtpPassword", ref)
    test_mail(senders, "user without a password (not ready)", expect="not-configured")

    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    for name, port, extra, server_says, expect in [
        ("greeting 554", SINK_BASE + 1, {}, True, "recipient"),
        ("closed at once", SINK_BASE + 3, {}, True, "unknown"),
        ("STARTTLS then hang-up", SINK_BASE + 4, {}, True, "tls"),
        ("STARTTLS required, refused", SINK_BASE, {"security": "starttls"}, True, "tls"),
        ("implicit TLS against plain text", SINK_BASE, {"security": "tls"}, False, "tls"),
        ("connection refused", SINK_BASE + 74, {}, True, "tls"),
        ("sender rejected", SINK_BASE, {"from": "reject-from@sink.test"}, True, "recipient"),
    ]:
        configure(pair, name, port=port, **extra)
        test_mail(senders, name, ignore_server_says=not server_says, expect=expect)
        parity.signed_in(pair, REF_DB, RUST_DB, admin)
    configure(pair, "unknown host", host="nonexistent.invalid")
    test_mail(senders, "unknown host", expect="connection")
    if not ARGS.skip_slow:
        parity.signed_in(pair, REF_DB, RUST_DB, admin)
        configure(pair, "silent server", port=SINK_BASE + 2)
        test_mail(senders, "greeting never received", expect="connection")

    # The throttle: a sixth test within 15 minutes by the same admin.
    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    configure(pair, "back to the sink")
    rate_admin, _ = seed_user("ratelimit", role="admin")
    parity.signed_in(pair, REF_DB, RUST_DB, rate_admin)
    for i in range(5):
        act(pair, "test mail #%d" % (i + 1), "mail", "sendTestMail", ["n%d@x.test" % i], ignore=("tookMs",))
    act(pair, "test mail #6 (rate limited)", "mail", "sendTestMail", [], ignore=("tookMs",))
    DELIVERED.check("the throttled admin")

    parity.signed_in(pair, REF_DB, RUST_DB, admin)
    for name, value in [("on", True), ("truthy string", "x"), ("off", False), ("undefined", parity.UNDEFINED),
                        ("null", None), ("array", [1]), ("on again", True)]:
        ref, _ = act(pair, "setVerificationRequired " + name, "mail", "setVerificationRequired", [value])
        status_matches(pair, "setVerificationRequired " + name, ref)
    ref, _ = act(pair, "clearSmtpSettings (end)", "mail", "clearSmtpSettings", [])
    status_matches(pair, "clearSmtpSettings (end)", ref)
    act(pair, "setVerificationRequired off (end)", "mail", "setVerificationRequired", [False])


def main():
    for db in (REF_DB, RUST_DB):
        db.sql("DELETE FROM settings WHERE key IN (%s)" % ", ".join(parity.sql_quote(k) for k in MAIL_KEYS))
        db.sql("INSERT INTO settings (key, value) VALUES ('register_enabled', 'off'), ('registration_key_enc', NULL) "
               "ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
    admin, _ = seed_user("admin", role="admin")
    user, _ = seed_user("user")
    pair = run.pair()
    registration(pair, admin, user)
    SINKS.start()
    try:
        mail(pair, admin, user)
    finally:
        SINKS.stop()
    run.finish()


main()

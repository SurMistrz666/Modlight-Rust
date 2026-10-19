"""Parity of the unauthenticated auth actions (app/actions/invite.ts,
app/actions/login-identifier.ts) between the reference and Rust.

    python3 server/tests/parity/auth_actions.py --ref http://localhost:8424 --rust http://localhost:8423 \
        --ref-env ../_scratch/srv-auth/ref/.env --rust-env ../_scratch/srv-auth/rust/rust.env

Not destructive: every run seeds its own users (same ids on both sides) and
only adds rows. It does change the mail settings of both servers (pointed at
an SMTP sink that runs on the host for Rust and inside the reference app
container for the original, both on 127.0.0.1) and sets them back to
"not configured" at the end. Secrets are read from the env files and never
printed; cookie values, passwords and tokens never appear in the output.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser(allow_abbrev=False)
ap.add_argument("--ref-env", default="../_scratch/srv-auth/ref/.env")
ap.add_argument("--rust-env", default="../_scratch/srv-auth/rust/rust.env")
ap.add_argument("--ref-db", default="modlight-srvauth-ref-postgres-1/modlight")
ap.add_argument("--rust-db", default="modlight-devdb-postgres-1/modlight_srv_auth")
ap.add_argument("--ref-app", default="modlight-srvauth-ref-app-1")
ap.add_argument("--scratch", default="../_scratch/srv-auth")
ap.add_argument("--sink-port", type=int, default=2711)
args, _ = ap.parse_known_args()

run = parity.Run.from_args()
REF_SECRET = parity.read_env_file(args.ref_env)["AUTH_SECRET"]
RUST_SECRET = parity.read_env_file(args.rust_env)["AUTH_SECRET"]
REF_DB = parity.Db(*args.ref_db.split("/"), REF_SECRET)
RUST_DB = parity.Db(*args.rust_db.split("/"), RUST_SECRET)
DBS = [REF_DB, RUST_DB]
ACTS = parity.Actions(run, args.ref_app)
TAG = parity.random_id(6).lower()
PASSWORD = "correct-horse-battery"
q = parity.sql_quote

_IP = [0]
_IP_BASE = "10.%d.%d" % (int(TAG[:2], 36) % 256, int(TAG[2:4], 36) % 256)


def pair():
    """A browser pair with a client address no earlier run used (the
    in-memory rate limiters outlive a Rust restart)."""
    _IP[0] += 1
    return run.pair(real_ip="%s.%d" % (_IP_BASE, _IP[0] % 250 + 1))


def both(sql_text):
    return [db.sql(sql_text) for db in DBS]


def compare_rows(name, sql_text, normalize=None):
    a, b = both(sql_text)
    if normalize:
        a, b = normalize(a), normalize(b)
    run.compare(name, parity.normalize_text(a), parity.normalize_text(b))


_HASHES = {}


def scrypt_hash(password):
    """Better Auth's password hash (`salt:key`, scrypt N=16384 r=16 p=1),
    computed by node:crypto on the host; the password goes through stdin."""
    if password not in _HASHES:
        js = ("let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{const c=require('crypto');"
              "const s=c.randomBytes(16).toString('hex');"
              "const k=c.scryptSync(d.normalize('NFKC'),s,64,{N:16384,r:16,p:1,maxmem:128*16384*16*2});"
              "process.stdout.write(s+':'+k.toString('hex'))})")
        out = subprocess.run(["node", "-e", js], input=password.encode(), stdout=subprocess.PIPE, check=True)
        _HASHES[password] = out.stdout.decode()
    return _HASHES[password]


def seed_user(key, password=PASSWORD, role="user", **columns):
    user_id = "pa%s%s" % (TAG, key)
    email = columns.pop("email", "%s.%s@actions.test" % (key, TAG))
    name = columns.pop("name", "Parity %s" % key)
    for db in DBS:
        db.create_user(user_id, email, name, role=role, **columns)
        if password is not None:
            db.sql('INSERT INTO "account" ("id", "account_id", "provider_id", "user_id", "password", "created_at", '
                   '"updated_at") VALUES (%s, %s, \'credential\', %s, %s, now(), now())'
                   % (q(parity.random_id(32)), q(user_id), q(user_id), q(scrypt_hash(password))))
    return user_id, email


def seed_invite(user_id, token, expired=False):
    digest = hashlib.sha256(token.encode()).hexdigest()
    when = "now() - interval '1 minute'" if expired else "now() + interval '7 days'"
    for db in DBS:
        db.sql('INSERT INTO "verification" ("id", "identifier", "value", "expires_at", "created_at", "updated_at") '
               "VALUES (%s, %s, %s, %s, now(), now())" % (q(parity.random_id(32)), q("invite:" + digest), q(user_id),
                                                         when))


VOLATILE = ("id", "token")


def act(p, name, module, export, argv):
    """Compare the envelopes (cookie values are compared by shape only)."""
    a = ACTS.call_ref(p.ref, module, export, argv)
    b = ACTS.call_rust(p.rust, module, export, argv)
    ea = parity.normalize_json(parity.next_action_envelope(a))
    eb = parity.normalize_json(parity.rpc_envelope(b))
    ea["set-cookie"] = cookies(a)
    eb["set-cookie"] = cookies(b)
    run.compare(name, ea, eb)
    if run.verbose:
        print("     ref: %s" % json.dumps(ea, default=str))
    # The browsers keep what they were given, as a real one would.
    for browser, res in ((p.ref, a), (p.rust, b)):
        for cname, cvalue, _attrs in res.set_cookies():
            if cvalue:
                browser.cookies[cname] = cvalue
            else:
                browser.cookies.pop(cname, None)
    return ea


def cookies(res):
    out = []
    for name, value, attrs in parity.cookie_shape(res):
        attrs = [x for x in attrs if not x.lower().startswith("expires")]
        out.append((name, "<set>" if value else "", attrs))
    return out


# ── the SMTP sinks ──────────────────────────────────────────────────────────


class Sinks(object):
    def __init__(self):
        self.host_log = os.path.join(args.scratch, "sink", "auth-actions-sink.jsonl")
        self.ref_log = "/tmp/auth-actions-sink.jsonl"
        os.makedirs(os.path.dirname(self.host_log), exist_ok=True)
        self.seen = [0, 0]

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

    def check(self, name, settle=1.5):
        time.sleep(settle)
        ref, rust = self.records()
        new = [ref[self.seen[0]:], rust[self.seen[1]:]]
        self.seen = [len(ref), len(rust)]
        shapes = [[mail_shape(r) for r in new[0]], [mail_shape(r) for r in new[1]]]
        run.compare("delivered by " + name, shapes[0], shapes[1])
        if run.verbose:
            print("     ref mails: %s" % [(m["rcpt"], m["subject"]) for m in shapes[0]])


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
            texts.append((part.get_content_type(), body.replace("\r\n", "\n").strip()))
    return {"rcpt": record["to"], "subject": str(msg.get("Subject")), "to": str(msg.get("To")), "texts": texts}


SINKS = Sinks()


# ── login-identifier ────────────────────────────────────────────────────────


def scenario_sign_in():
    user, email = seed_user("signin", username="pa_%s" % TAG)
    seed_user("named", name="Named %s" % TAG)
    seed_user("twin1", name="Twin %s" % TAG)
    seed_user("twin2", name="Twin %s" % TAG)
    seed_user("banned", banned=True)
    seed_user("tfa", two_factor_enabled=True)
    seed_user("nopw", password=None)

    p = pair()
    s = "signInWithIdentifier"
    act(p, "identifier undefined throws", "login-identifier", s, [parity.UNDEFINED, PASSWORD])
    act(p, "identifier number throws", "login-identifier", s, [5, PASSWORD])
    act(p, "identifier null throws", "login-identifier", s, [None, PASSWORD])
    act(p, "empty identifier", "login-identifier", s, ["", PASSWORD])
    act(p, "blank identifier", "login-identifier", s, ["  \t", PASSWORD])
    act(p, "empty password", "login-identifier", s, [email, ""])
    act(p, "undefined password", "login-identifier", s, [email, parity.UNDEFINED])
    act(p, "zero password", "login-identifier", s, [email, 0])
    act(p, "wrong password", "login-identifier", s, [email, "wrong-password-1"])
    act(p, "numeric password", "login-identifier", s, [email, 12345678])
    act(p, "unknown username", "login-identifier", s, ["nobody-%s" % TAG, PASSWORD])
    act(p, "ambiguous name", "login-identifier", s, ["twin " + TAG, PASSWORD])
    act(p, "account without a password", "login-identifier", s, ["nopw.%s@actions.test" % TAG, PASSWORD])
    act(p, "banned", "login-identifier", s, ["banned.%s@actions.test" % TAG, PASSWORD])

    p = pair()
    act(p, "by email", "login-identifier", s, ["  " + email.upper() + " ", PASSWORD])
    compare_rows("session row of the email sign-in",
                 'SELECT ip_address, user_agent, impersonated_by IS NULL, expires_at > now() + interval \'6 days\' '
                 'FROM "session" WHERE user_id = %s ORDER BY created_at' % q(user))
    p = pair()
    act(p, "by username, any case", "login-identifier", s, [("PA_%s" % TAG).upper(), PASSWORD])
    p = pair()
    act(p, "by unique name", "login-identifier", s, [("named " + TAG).upper(), PASSWORD])
    p = pair()
    act(p, "two-factor account", "login-identifier", s, ["tfa.%s@actions.test" % TAG, PASSWORD])
    compare_rows("two-factor sign-in leaves no session",
                 'SELECT count(*) FROM "session" WHERE user_id = %s' % q("pa%stfa" % TAG))
    compare_rows("two-factor sign-in leaves a pending verification",
                 'SELECT count(*) FROM "verification" WHERE value = %s' % q("pa%stfa" % TAG))

    p = pair()
    for i in range(10):
        act(p, "limit %d" % (i + 1), "login-identifier", s, ["nobody-%s" % TAG, "x"])
    act(p, "11th call rate-limited", "login-identifier", s, [email, PASSWORD])
    act(p, "falsy input is checked before the limit", "login-identifier", s, ["", PASSWORD])


# ── invite ──────────────────────────────────────────────────────────────────


def scenario_invite():
    a = "acceptInvitation"
    invited, invited_email = seed_user("invited", password=None, email_verified=False)
    token = parity.random_id(43)
    seed_invite(invited, token)
    stale, _ = seed_user("stale", password=None, email_verified=False)
    stale_token = parity.random_id(43)
    seed_invite(stale, stale_token, expired=True)
    banned, _ = seed_user("ibanned", password=None, banned=True)
    banned_token = parity.random_id(43)
    seed_invite(banned, banned_token)

    p = pair()
    act(p, "unknown token", "invite", a, ["nope", "long-enough-1"])
    act(p, "short password", "invite", a, [token, "short"])
    act(p, "undefined password throws", "invite", a, [token, parity.UNDEFINED])
    act(p, "null password throws", "invite", a, [token, None])
    act(p, "numeric password, valid token, throws", "invite", a, [token, 12345678])
    act(p, "numeric password, unknown token", "invite", a, ["nope", 12345678])
    act(p, "false password", "invite", a, [token, False])
    act(p, "undefined token", "invite", a, [parity.UNDEFINED, "long-enough-1"])
    act(p, "numeric token throws", "invite", a, [5, "long-enough-1"])
    act(p, "short array password", "invite", a, [token, [1, 2]])
    compare_rows("invitation still pending",
                 'SELECT count(*) FROM "verification" WHERE value = %s' % q(invited))
    p = pair()  # the ten calls above used up this caller's budget
    act(p, "expired token", "invite", a, [stale_token, "long-enough-1"])
    compare_rows("expired invitation deleted", 'SELECT count(*) FROM "verification" WHERE value = %s' % q(stale))

    p = pair()
    act(p, "accept", "invite", a, [token, "long-enough-1"])
    compare_rows("accepted account",
                 'SELECT u.email_verified, a.provider_id, a.password IS NOT NULL, a.account_id = u.id '
                 'FROM "user" u JOIN "account" a ON a.user_id = u.id WHERE u.id = %s' % q(invited))
    compare_rows("accepted invitation consumed", 'SELECT count(*) FROM "verification" WHERE value = %s' % q(invited))
    compare_rows("accepted session", 'SELECT ip_address, user_agent FROM "session" WHERE user_id = %s' % q(invited))
    act(p, "same token again", "invite", a, [token, "long-enough-1"])
    p2 = pair()
    act(p2, "the new password signs in", "login-identifier", "signInWithIdentifier",
        [invited_email, "long-enough-1"])

    p = pair()
    act(p, "banned invitee: password set, sign-in fails", "invite", a, [banned_token, "long-enough-1"])
    compare_rows("banned invitee row", 'SELECT u.email_verified, a.password IS NOT NULL FROM "user" u '
                 'JOIN "account" a ON a.user_id = u.id WHERE u.id = %s' % q(banned))

    p = pair()
    for i in range(10):
        act(p, "invite limit %d" % (i + 1), "invite", a, ["nope", "long-enough-1"])
    act(p, "11th invite call rate-limited", "invite", a, ["nope", "short"])


def configure_mail(admin_pair, name, required=None):
    data = {"host": "127.0.0.1", "port": args.sink_port, "security": "none", "user": "",
            "from": "no-reply@sink.test", "fromName": "Parity"}
    act(admin_pair, "save mail settings " + name, "mail", "saveSmtpSettings", [data])
    if required is not None:
        act(admin_pair, "verification required " + name, "mail", "setVerificationRequired", [required])


def scenario_resend(admin_pair):
    r = "resendVerification"
    p = pair()
    act(p, "undefined address throws", "invite", r, [parity.UNDEFINED])
    act(p, "numeric address throws", "invite", r, [5])
    act(p, "mail not configured", "invite", r, ["someone.%s@actions.test" % TAG])

    configure_mail(admin_pair, "(resend)")
    unverified, unverified_email = seed_user("unverified", email_verified=False)
    seed_user("verified")

    p = pair()
    act(p, "unknown address", "invite", r, ["ghost.%s@actions.test" % TAG])
    SINKS.check("unknown address")
    act(p, "unverified address, mixed case and blanks", "invite", r, ["  " + unverified_email.upper() + " "])
    SINKS.check("unverified address")
    act(p, "verified address", "invite", r, ["verified.%s@actions.test" % TAG])
    SINKS.check("verified address")
    act(p, "same address again", "invite", r, [unverified_email])
    act(p, "same address a third time", "invite", r, [unverified_email])
    act(p, "fourth time for one address: rate-limited", "invite", r, [unverified_email])
    act(p, "sixth call from one caller: rate-limited", "invite", r, ["other.%s@actions.test" % TAG])
    SINKS.check("the repeated sends")

    # A signed-in caller must not turn this into "resend MY link".
    p = pair()
    parity.signed_in(p, REF_DB, RUST_DB, unverified)
    act(p, "signed in as somebody else", "invite", r, ["ghost2.%s@actions.test" % TAG])
    SINKS.check("signed-in caller")

    # Unverified sign-in once confirmation is required (and mail works).
    configure_mail(admin_pair, "(required)", required=True)
    p = pair()
    act(p, "unverified sign-in", "login-identifier", "signInWithIdentifier", [unverified_email, PASSWORD])
    SINKS.check("unverified sign-in")
    act(p, "unverified, wrong password", "login-identifier", "signInWithIdentifier",
        [unverified_email, "wrong-password-1"])
    act(admin_pair, "verification no longer required", "mail", "setVerificationRequired", [False])


def main():
    scenario_sign_in()
    scenario_invite()
    admin, _ = seed_user("admin", role="admin")
    admin_pair = pair()
    parity.signed_in(admin_pair, REF_DB, RUST_DB, admin)
    SINKS.start()
    try:
        scenario_resend(admin_pair)
    finally:
        act(admin_pair, "clear mail settings", "mail", "clearSmtpSettings", [])
        SINKS.stop()
    run.finish()


if __name__ == "__main__":
    main()

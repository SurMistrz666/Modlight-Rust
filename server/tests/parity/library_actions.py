"""Library server actions: app/actions/{uploads,folders,duplicates,shortlinks,
shortlink-unlock,pastes}.ts and the storage half of app/actions/admin.ts,
called through Next's own protocol on the reference and /_rpc on Rust, on
identical state seeded into both databases (and both storages).

    # every section but `instance`, against the shared scratch reference (defaults shown)
    PARITY_REF=http://localhost:8441 PARITY_RUST=http://localhost:8445 \\
      python3 server/tests/parity/library_actions.py \\
        --ref-app modlight-srvlib-ref-app-1 --ref-db modlight-srvlib-ref-postgres-1:modlight \\
        --ref-env _scratch/srv-library/ref/.env \\
        --rust-db modlight-devdb-postgres-1:modlight_srv_library_actions --rust-env _scratch/srv-library/rust.env \\
        --rust-storage _scratch/srv-library/actions/data/storage

    # everything, `instance` included, against a private reference stack
    # (_scratch/srv-library/actions/ref/docker-compose.yml, app on 8446)
    PARITY_REF=http://localhost:8446 PARITY_RUST=http://localhost:8445 \\
      python3 server/tests/parity/library_actions.py --ref-app modlight-srvlib-actions-ref-app-1 \\
        --ref-db modlight-srvlib-actions-ref-postgres-1:modlight --ref-env _scratch/srv-library/actions/ref/.env

Run from the main checkout (the default paths are relative to it). The Rust
server runs with TZ=UTC like the reference container (local-time date strings).

Sections (`--only a,b`): uploads, folders, duplicates, shortlinks, unlock,
pastes, admin (account deletion and the per-user admin reads), instance (the
instance-wide maintenance: orphans, orphan purge, quota recalculation, storage
scan). `instance` changes or reads every account of the reference, so it
refuses to run against the shared references (8420, 8441) unless
`--allow-shared`; run it against a private reference stack. Its comparisons
tolerate rows that exist on one side only (earlier runs against another
reference): totals are checked against each side's own database, the rest is
compared over this run's rows.

Both servers: NODE_ENV=production, APP_URL = their own origin; Rust with
TRUST_PROXY=true. Every run seeds its own accounts (ids and tags carry a random
suffix), so the script can be repeated against the same databases. What is
compared: the envelope (ok value / redirect / masked error / revalidate) and
every Set-Cookie, then the rows (and storage objects) each call left behind.
"""

import argparse
import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402
from library import ORIGIN_RE, Library, days_ago, seed_folder, seed_upload  # noqa: E402

# Trash stamps inside the 30-day window: a purge job (either server's) must
# not take seeded rows mid-run. Fixed per run, identical on both sides.
TRASHED_10, TRASHED_9, TRASHED_8, TRASHED_7, TRASHED_5 = (days_ago(n) for n in (10, 9, 8, 7, 5))

q = parity.sql_quote
UNDEFINED = parity.UNDEFINED
SHARED_REFS = ("8420", "8441")


def options():
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--ref-app", default="modlight-srvlib-ref-app-1")
    p.add_argument("--ref-db", default="modlight-srvlib-ref-postgres-1:modlight")
    p.add_argument("--ref-env", default="_scratch/srv-library/ref/.env")
    p.add_argument("--ref-storage", default="/app/data/storage")
    p.add_argument("--rust-db", default="modlight-devdb-postgres-1:modlight_srv_library_actions")
    p.add_argument("--rust-env", default="_scratch/srv-library/rust.env")
    p.add_argument("--rust-storage", default="_scratch/srv-library/actions/data/storage")
    p.add_argument("--only", default=None)
    p.add_argument("--allow-shared", action="store_true")
    a, _ = p.parse_known_args()
    return a


OPT = options()
run = parity.Run.from_args()
_rc, _rd = OPT.ref_db.split(":")
_uc, _ud = OPT.rust_db.split(":")
REF = parity.Db(_rc, _rd, parity.read_env_file(OPT.ref_env)["AUTH_SECRET"])
RUST = parity.Db(_uc, _ud, parity.read_env_file(OPT.rust_env)["AUTH_SECRET"])
lib = Library(run, REF, RUST)
acts = parity.Actions(run, OPT.ref_app)
SFX = lib.suffix
RUST_STORAGE = os.path.abspath(OPT.rust_storage)


# ── helpers ──────────────────────────────────────────────────────────────────


class Names(object):
    """Seeded values by name, so a result names `<png>` instead of a uuid."""

    def __init__(self):
        self.map = {}
        self.patterns = []

    def add(self, value, name):
        self.map[value] = "<%s>" % name
        return value

    def label(self, v):
        text = json.dumps(v, ensure_ascii=False)
        for value, name in sorted(self.map.items(), key=lambda kv: -len(kv[0])):
            text = text.replace(value, name)
        text = re.sub(ORIGIN_RE, "<origin>", text)
        text = text.replace(SFX, "<sfx>")
        for pattern, repl in self.patterns:
            text = re.sub(pattern, repl, text)
        return parity.normalize_json(json.loads(text))


NAMES = Names()


def envelopes(a, b):
    ea = NAMES.label(parity.next_action_envelope(a))
    eb = NAMES.label(parity.rpc_envelope(b))
    return ea, eb


def cookie_list(res):
    return [(n, v, [x for x in attrs if not x.lower().startswith("expires")]) for n, v, attrs in
            parity.cookie_shape(res)]


def act(pair, name, module, export, args, redact=(), sort_key=None, cookies=True, post=None):
    """One action on both servers; compares the envelopes (and cookies)."""
    a = acts.call_ref(pair.ref, module, export, args)
    b = acts.call_rust(pair.rust, module, export, args)
    ea, eb = envelopes(a, b)
    for env in (ea, eb):
        if redact and isinstance(env.get("ok"), (dict, list)):
            env["ok"] = parity.normalize_json(env["ok"], redact=redact)
        if sort_key and isinstance(env.get("ok"), list):
            env["ok"] = sorted(env["ok"], key=sort_key)
        if post:
            post(env)
    if cookies:
        ea["set-cookie"] = cookie_list(a)
        eb["set-cookie"] = cookie_list(b)
    full = "%s.%s: %s" % (module, export, name)
    if os.environ.get("PARITY_DUMP") and os.environ["PARITY_DUMP"] in full:
        print("DUMP %s\n%s" % (full, json.dumps(ea, indent=1, ensure_ascii=False)))
    run.compare(full, ea, eb)
    return ea, eb


def both(sql):
    return REF.sql(sql), RUST.sql(sql)


def rows(name, sql):
    a, b = both(sql)
    run.compare("rows: " + name, NAMES.label(a.splitlines()), NAMES.label(b.splitlines()))
    return a


def in_list(values):
    return ", ".join(q(v) for v in values)


def audit_rows(name, users, since_action_like="%"):
    """The audit rows written for these accounts (by them or about them),
    detail keys sorted, list values sorted (RETURNING order is physical)."""
    sql = ("SELECT action, coalesce(user_id, '-'), detail::text, ip FROM audit_log WHERE (user_id IN (%s) "
           "OR detail->>'ownerId' IN (%s)) AND action LIKE %s ORDER BY created_at, action"
           % (in_list(users), in_list(users), q(since_action_like)))
    a, b = both(sql)

    def norm(text):
        out = []
        for line in text.splitlines():
            parts = line.split("\t")
            if len(parts) == 4:
                d = json.loads(parts[2])
                for k, v in d.items():
                    if isinstance(v, list):
                        d[k] = sorted(v, key=lambda x: json.dumps(x))
                parts[2] = json.dumps(d, sort_keys=True)
            out.append(parts)
        return NAMES.label(out)

    run.compare("audit: " + name, norm(a), norm(b))


# ── storage on both sides ────────────────────────────────────────────────────


def ref_exec(script, *args, stdin=None):
    out = subprocess.run(["docker", "exec", "-i", OPT.ref_app, "sh", "-c", script, "sh"] + list(args),
                         input=stdin, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    return out.returncode, out.stdout


def put_file(key, data=b"bytes", age_seconds=None):
    """The same object on both storages; `age_seconds` back-dates its mtime."""
    mtime = int(time.time() - age_seconds) if age_seconds else None
    path = OPT.ref_storage.rstrip("/") + "/" + key
    script = 'mkdir -p "$(dirname "$1")" && cat > "$1"'
    if mtime:
        script += ' && touch -d @%d "$1"' % mtime
    rc, _ = ref_exec(script, path, stdin=data)
    if rc != 0:
        raise RuntimeError("could not write %s on the reference" % key)
    local = os.path.join(RUST_STORAGE, *key.split("/"))
    os.makedirs(os.path.dirname(local), exist_ok=True)
    with open(local, "wb") as f:
        f.write(data)
    if mtime:
        os.utime(local, (mtime, mtime))
        # The scan reports the mtime: name it, so the exact value is compared.
        NAMES.add(time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(mtime)), "mtime")


def file_state(key):
    """(exists, sha256) of one object on both storages."""
    rc, out = ref_exec('test -f "$1" && sha256sum "$1" | cut -d" " -f1', OPT.ref_storage.rstrip("/") + "/" + key)
    ref = (rc == 0, out.decode().strip() if rc == 0 else None)
    local = os.path.join(RUST_STORAGE, *key.split("/"))
    if os.path.isfile(local):
        with open(local, "rb") as f:
            rust = (True, hashlib.sha256(f.read()).hexdigest())
    else:
        rust = (False, None)
    return ref, rust


def lock_dir(directory, locked):
    """Make storage refuse to delete the objects in one directory (no write
    permission on it), on both sides; `locked=False` undoes it."""
    mode = "555" if locked else "755"
    rc, _ = ref_exec('chmod %s "$1"' % mode, OPT.ref_storage.rstrip("/") + "/" + directory)
    if rc != 0:
        raise RuntimeError("could not chmod %s on the reference" % directory)
    os.chmod(os.path.join(RUST_STORAGE, *directory.split("/")), 0o555 if locked else 0o755)


def files(name, keys):
    a, b = [], []
    for k in keys:
        r, u = file_state(k)
        a.append((NAMES.label(k), r))
        b.append((NAMES.label(k), u))
    run.compare("storage: " + name, a, b)
    return a


def scrypt_hash(password):
    """lib/password.ts hashPassword: scrypt$<salt b64url>$<key b64url>, N=16384 r=8 p=1.
    Computed by Node inside the reference container (this Python may lack
    hashlib.scrypt), from a test password passed on stdin."""
    script = ('let d="";process.stdin.on("data",c=>d+=c).on("end",()=>{const c=require("crypto");'
              'const s=c.randomBytes(16);const k=c.scryptSync(d,s,32);'
              'process.stdout.write("scrypt$"+s.toString("base64url")+"$"+k.toString("base64url"))})')
    out = subprocess.run(["docker", "exec", "-i", OPT.ref_app, "node", "-e", script], input=password.encode(),
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    return out.stdout.decode().strip()


def sha(c):
    return (c * 64)[:64]


def token_view(token, secret, parts, now):
    """lib/password.ts unlock token checked independently: (signature valid,
    seconds until expiry rounded to a minute)."""
    import hmac
    import urllib.parse
    if not token or "." not in token:
        return (False, None)
    exp, _, sig = urllib.parse.unquote(token).partition(".")
    message = "|".join(urllib.parse.quote(p, safe="-_.!~*'()") for p in parts + [exp])
    want = base64.urlsafe_b64encode(hmac.new(secret.encode(), message.encode(), hashlib.sha256).digest())
    return (hmac.compare_digest(want.decode().rstrip("="), sig), round((int(exp) - now) / 60.0))


def anonymous():
    """A browser pair with a garbage session cookie: it passes proxy.ts's
    optimistic /dashboard gate (which answers a cookie-less POST with a 307
    before any action runs) but is no session."""
    pair = run.pair()
    pair.ref.cookies[parity.SESSION_COOKIE] = "garbage"
    pair.rust.cookies[parity.SESSION_COOKIE] = "garbage"
    return pair


def signed_out_checks(module, cases):
    pair = anonymous()
    for export, args in cases:
        act(pair, "signed out", module, export, args)


# ── app/actions/uploads.ts ───────────────────────────────────────────────────


def uploads():
    pair = run.pair()
    owner, tag = lib.seed_user("upo", used_quota=5000)
    other, other_tag = lib.seed_user("upt", used_quota=900)
    admin, admin_tag = lib.seed_user("upa", role="admin")
    for v, n in ((owner, "owner"), (other, "other"), (admin, "admin"), (tag, "tag"), (other_tag, "other-tag"),
                 (admin_tag, "admin-tag")):
        NAMES.add(v, n)
    t = lambda n: "2026-01-01T00:00:%02d.000Z" % n
    ids = {}

    def up(key, who, code, **cols):
        ids[key] = NAMES.add(seed_upload(lib, who, code + SFX, created_at=t(len(ids)), **cols), key)
        return ids[key]

    up("live", owner, "ua")
    up("live2", owner, "ub", published=False)
    up("trashed", owner, "uc", deleted_at=TRASHED_10, size=1200,
       storage_key="%s/uc%s.png" % (tag, SFX), thumb_key="thumbs/%s/uc%s.webp" % (tag, SFX))
    up("trashed2", owner, "ud", deleted_at=TRASHED_9, size=300,
       storage_key="%s/ud%s.png" % (tag, SFX))
    up("theirs", other, "ue")
    up("theirs_trash", other, "uf", deleted_at=TRASHED_10, size=400,
       storage_key="%s/uf%s.png" % (other_tag, SFX))
    up("theirs_live2", other, "ug")
    keys = ["%s/uc%s.png" % (tag, SFX), "thumbs/%s/uc%s.webp" % (tag, SFX), "%s/ud%s.png" % (tag, SFX),
            "%s/uf%s.png" % (other_tag, SFX), "derived/%s/a.webp" % ids["live"], "derived/%s/b.webp" % ids["trashed"]]
    for k in keys:
        put_file(k, ("object " + k).encode())

    def upload_rows(name):
        rows("uploads " + name,
             'SELECT code, published, deleted_at IS NULL, deleted_at > now() - interval \'5 minutes\', '
             'updated_at > now() - interval \'5 minutes\', expires_at, max_views, password_hash IS NULL, '
             'password_hash LIKE \'scrypt$%%\', password_version FROM uploads WHERE owner_id IN (%s) ORDER BY created_at'
             % in_list([owner, other]))
        rows("quota " + name, 'SELECT id, used_quota FROM "user" WHERE id IN (%s) ORDER BY id COLLATE "C"'
             % in_list([owner, other]))

    signed_out_checks("uploads", [("togglePublish", [ids["live"]]), ("trashSummary", []),
                                        ("emptyTrash", []), ("setUploadPassword", [ids["live"], "x"])])

    parity.signed_in(pair, REF, RUST, owner)
    act(pair, "own", "uploads", "togglePublish", [ids["live"]])
    act(pair, "own again", "uploads", "togglePublish", [ids["live"]])
    act(pair, "unpublished one", "uploads", "togglePublish", [ids["live2"]])
    act(pair, "trashed row", "uploads", "togglePublish", [ids["trashed2"]])
    act(pair, "foreign as a user", "uploads", "togglePublish", [ids["theirs"]])
    act(pair, "non-uuid", "uploads", "togglePublish", ["nope"])
    act(pair, "number", "uploads", "togglePublish", [5])
    act(pair, "uppercase id", "uploads", "togglePublish", [ids["live2"].upper()])
    act(pair, "no argument", "uploads", "togglePublish", [])
    upload_rows("after togglePublish")

    act(pair, "own", "uploads", "setVanity", [ids["live"], "My Vanity " + SFX])
    act(pair, "same code again", "uploads", "setVanity", [ids["live"], "my-vanity-" + SFX])
    act(pair, "taken by another account", "uploads", "setVanity", [ids["live2"], "ue" + SFX])
    act(pair, "reserved", "uploads", "setVanity", [ids["live2"], "Dashboard"])
    act(pair, "empty after slug", "uploads", "setVanity", [ids["live2"], "!!!"])
    act(pair, "too long", "uploads", "setVanity", [ids["live2"], "a" * 65])
    act(pair, "number vanity", "uploads", "setVanity", [ids["live2"], 42])
    act(pair, "foreign", "uploads", "setVanity", [ids["theirs"], "x" + SFX])
    act(pair, "trashed row", "uploads", "setVanity", [ids["trashed2"], "Trash Code " + SFX])

    act(pair, "iso", "uploads", "setUploadExpiry", [ids["live"], "2031-05-06T07:08:09Z"])
    act(pair, "date only", "uploads", "setUploadExpiry", [ids["live2"], "2031-05-06"])
    act(pair, "a Date", "uploads", "setUploadExpiry", [ids["trashed2"], parity.JsDate("2030-01-02T03:04:05.678Z")])
    act(pair, "garbage", "uploads", "setUploadExpiry", [ids["live"], "soon"])
    act(pair, "number", "uploads", "setUploadExpiry", [ids["live2"], 86400000])
    act(pair, "cleared with null", "uploads", "setUploadExpiry", [ids["live2"], None])
    act(pair, "empty string", "uploads", "setUploadExpiry", [ids["trashed2"], ""])
    for label, value in (("day out of range", "2031-02-30"), ("RFC 1123", "Tue, 01 Jan 2030 00:00:00 GMT"),
                         ("space instead of T", "2030-01-01 10:00"), ("offset", "2030-06-01T12:00:00+02:00"),
                         ("year only", "2032"), ("boolean", True), ("array", [2030])):
        act(pair, label, "uploads", "setUploadExpiry", [ids["live2"], value])
        rows("expiry after " + label, 'SELECT expires_at FROM uploads WHERE id = %s' % q(ids["live2"]))
    act(pair, "foreign is a silent no-op", "uploads", "setUploadExpiry", [ids["theirs"], "2031-01-01T00:00:00Z"])
    act(pair, "non-uuid is a silent no-op", "uploads", "setUploadExpiry", ["nope", "2031-01-01T00:00:00Z"])

    act(pair, "five", "uploads", "setUploadMaxViews", [ids["live"], 5])
    act(pair, "fraction", "uploads", "setUploadMaxViews", [ids["live2"], 2.7])
    act(pair, "floored at one", "uploads", "setUploadMaxViews", [ids["trashed2"], -3])
    act(pair, "string", "uploads", "setUploadMaxViews", [ids["live"], "5"])
    act(pair, "NaN", "uploads", "setUploadMaxViews", [ids["live"], float("nan")])
    act(pair, "past int4", "uploads", "setUploadMaxViews", [ids["live"], 3e9])
    act(pair, "boolean", "uploads", "setUploadMaxViews", [ids["live"], True])
    act(pair, "infinity", "uploads", "setUploadMaxViews", [ids["live"], float("inf")])
    act(pair, "cleared", "uploads", "setUploadMaxViews", [ids["live2"], None])
    act(pair, "undefined clears", "uploads", "setUploadMaxViews", [ids["trashed2"], UNDEFINED])

    act(pair, "set", "uploads", "setUploadPassword", [ids["live"], "hunter2"])
    act(pair, "empty clears", "uploads", "setUploadPassword", [ids["live2"], ""])
    act(pair, "too long", "uploads", "setUploadPassword", [ids["live"], "x" * 1025])
    act(pair, "number clears", "uploads", "setUploadPassword", [ids["trashed2"], 5])
    act(pair, "null clears", "uploads", "setUploadPassword", [ids["trashed2"], None])
    upload_rows("after link settings")

    act(pair, "own live", "uploads", "trashUpload", [ids["live2"]])
    act(pair, "already trashed (no re-stamp)", "uploads", "trashUpload", [ids["trashed"]])
    act(pair, "foreign", "uploads", "trashUpload", [ids["theirs"]])
    act(pair, "own", "uploads", "restoreUpload", [ids["live2"]])
    act(pair, "live row (no-op write)", "uploads", "restoreUpload", [ids["live"]])
    act(pair, "live row refused", "uploads", "deleteUploadForever", [ids["live"]])
    act(pair, "foreign refused", "uploads", "deleteUploadForever", [ids["theirs_trash"]])
    act(pair, "trashed", "uploads", "deleteUploadForever", [ids["trashed"]])
    upload_rows("after trash, restore, delete")
    files("after deleteUploadForever", keys)

    act(pair, "own (default)", "uploads", "trashSummary", [])
    act(pair, "own tag spelled out", "uploads", "trashSummary", ["  " + tag + "  "])
    act(pair, "another account's tag", "uploads", "trashSummary", [other_tag])
    act(pair, "unknown tag", "uploads", "trashSummary", ["nobody" + SFX])
    act(pair, "blank tag means own", "uploads", "trashSummary", ["   "])
    act(pair, "non-string tag means own", "uploads", "trashSummary", [7])
    act(pair, "another account's trash", "uploads", "emptyTrash", [other_tag])
    act(pair, "own", "uploads", "emptyTrash", [])
    act(pair, "own, now empty", "uploads", "emptyTrash", [tag])
    upload_rows("after emptyTrash")
    files("after emptyTrash", keys)

    # Storage refuses one object: its row stays in the trash (`failed`), the
    # rest goes; once storage gives it up, the next call finishes the job.
    locked = "%s/locked" % tag
    stuck_key, free_key = "%s/st%s.png" % (locked, SFX), "%s/fr%s.png" % (tag, SFX)
    up("stuck", owner, "st", deleted_at=TRASHED_8, size=11, storage_key=stuck_key)
    up("free", owner, "fr", deleted_at=TRASHED_7, size=22, storage_key=free_key)
    put_file(stuck_key, b"stuck")
    put_file(free_key, b"free")
    lock_dir(locked, True)
    try:
        act(pair, "storage refuses one object", "uploads", "emptyTrash", [])
        upload_rows("after a refused object")
        files("after a refused object", [stuck_key, free_key])
        act(pair, "the refused row is still counted", "uploads", "trashSummary", [])
    finally:
        lock_dir(locked, False)
    act(pair, "storage gives it up", "uploads", "emptyTrash", [])
    files("after the retry", [stuck_key, free_key])

    # An administrator reaches every account's rows, and leaves an audit trail.
    parity.signed_in(pair, REF, RUST, admin)
    act(pair, "foreign as admin", "uploads", "togglePublish", [ids["theirs"]])
    act(pair, "foreign as admin", "uploads", "setVanity", [ids["theirs"], "Admin Picked " + SFX])
    act(pair, "foreign as admin", "uploads", "setUploadExpiry", [ids["theirs"], "2032-02-02T00:00:00.000Z"])
    act(pair, "foreign as admin", "uploads", "setUploadMaxViews", [ids["theirs"], 9])
    act(pair, "foreign as admin", "uploads", "setUploadPassword", [ids["theirs"], "pw"])
    act(pair, "foreign as admin", "uploads", "trashUpload", [ids["theirs_live2"]])
    act(pair, "foreign as admin", "uploads", "restoreUpload", [ids["theirs_live2"]])
    act(pair, "foreign as admin", "uploads", "trashUpload", [ids["theirs"]])
    act(pair, "another account's trash as admin", "uploads", "trashSummary", [other_tag])
    act(pair, "foreign as admin", "uploads", "deleteUploadForever", [ids["theirs"]])
    act(pair, "another account's trash as admin", "uploads", "emptyTrash", [other_tag])
    act(pair, "admin's own empty trash", "uploads", "emptyTrash", [])
    upload_rows("after admin")
    files("after admin", keys)
    audit_rows("uploads", [owner, other, admin])


# ── app/actions/folders.ts ───────────────────────────────────────────────────


def folders():
    pair = run.pair()
    owner, tag = lib.seed_user("fdo")
    other, _ = lib.seed_user("fdt")
    NAMES.add(owner, "owner")
    NAMES.add(other, "other")
    foreign = NAMES.add(seed_folder(lib, other, "Theirs"), "foreign-folder")
    t = lambda n: "2026-01-01T00:01:%02d.000Z" % n
    up = {}
    for i, k in enumerate(["a", "b", "c", "d", "e", "f"]):
        up[k] = NAMES.add(seed_upload(lib, owner, "f" + k + SFX, created_at=t(i)), "up-" + k)
    up["t"] = NAMES.add(seed_upload(lib, owner, "ft" + SFX, created_at=t(7), deleted_at=TRASHED_10),
                        "up-trashed")
    up["x"] = NAMES.add(seed_upload(lib, other, "fx" + SFX, created_at=t(8)), "up-theirs")

    def folder_rows(name):
        rows("folders " + name,
             'SELECT f.name, f.slug, coalesce(p.name, \'-\'), f.is_public, f.sort_order FROM folders f '
             'LEFT JOIN folders p ON p.id = f.parent_id WHERE f.owner_id IN (%s) ORDER BY f.name COLLATE "C", f.slug COLLATE "C"'
             % in_list([owner, other]))
        rows("folder of each upload " + name,
             'SELECT u.code, coalesce(f.name, \'-\'), u.deleted_at IS NULL, u.updated_at > now() - interval \'5 minutes\' '
             'FROM uploads u LEFT JOIN folders f ON f.id = u.folder_id WHERE u.owner_id IN (%s) ORDER BY u.code COLLATE "C"'
             % in_list([owner, other]))

    def folder_id(name):
        a, b = both('SELECT id FROM folders WHERE owner_id = %s AND name = %s ORDER BY created_at LIMIT 1'
                    % (q(owner), q(name)))
        a, b = a.strip(), b.strip()
        if a != b:
            # Folders created by an action get a fresh uuid on each side; the
            # script addresses them by name and passes each side its own id.
            return (a, b)
        return a

    def act_ids(name, export, args_ref, args_rust, **kw):
        """An action whose arguments name rows created by an earlier action
        (different uuids on the two sides)."""
        a = acts.call_ref(pair.ref, "folders", export, args_ref)
        b = acts.call_rust(pair.rust, "folders", export, args_rust)
        ea, eb = envelopes(a, b)
        for env in (ea, eb):
            if kw.get("post"):
                kw["post"](env)
        ea["set-cookie"] = cookie_list(a)
        eb["set-cookie"] = cookie_list(b)
        run.compare("folders.%s: %s" % (export, name), ea, eb)

    def split(*args):
        """(ref args, rust args) from values that may be (ref, rust) pairs."""
        ra = [x[0] if isinstance(x, tuple) else x for x in args]
        ua = [x[1] if isinstance(x, tuple) else x for x in args]
        return ra, ua

    def call(name, export, *args, **kw):
        ra, ua = split(*args)
        act_ids(name, export, ra, ua, **kw)

    signed_out_checks("folders", [("createFolder", ["x"]), ("listFolders", [])])
    parity.signed_in(pair, REF, RUST, owner)

    call("plain", "createFolder", "Projects")
    call("default name", "createFolder", "   ")
    call("non-string name", "createFolder", 42)
    call("same name again", "createFolder", "Projects")
    call("long name is cut to 120", "createFolder", "Ż" + "x" * 200)
    call("emoji only", "createFolder", "🚀")
    call("slug clash with a sibling", "createFolder", "projects!")
    projects = folder_id("Projects")
    call("nested", "createFolder", "Child", projects)
    call("foreign parent", "createFolder", "Nope", foreign)
    call("non-uuid parent", "createFolder", "Nope", "abc")
    call("empty-string parent", "createFolder", "Nope", "")
    call("number parent", "createFolder", "Nope", 5)
    call("false parent", "createFolder", "Nope", False)
    call("explicit undefined parent", "createFolder", "Rooted", UNDEFINED)
    parent = folder_id("Child")
    for depth in range(2, 9):
        call("depth %d" % (depth + 1), "createFolder", "Level%d" % (depth + 1), parent)
        parent = folder_id("Level%d" % (depth + 1))
    call("ninth level refused", "createFolder", "TooDeep", parent)
    folder_rows("after createFolder")

    child = folder_id("Child")
    call("rename", "renameFolder", child, "Renamed Child")
    call("same name moves the slug", "renameFolder", child, "Renamed Child")
    call("blank name refused", "renameFolder", child, "  ")
    call("non-string name refused", "renameFolder", child, None)
    call("foreign", "renameFolder", foreign, "Mine now")
    call("non-uuid", "renameFolder", "abc", "x")
    call("number id", "renameFolder", 7, "x")

    call("public", "setFolderVisibility", projects, True)
    call("string yes", "setFolderVisibility", child, "yes")
    call("number zero", "setFolderVisibility", child, 0)
    call("null", "setFolderVisibility", projects, None)
    call("undefined", "setFolderVisibility", projects, UNDEFINED)
    call("garbage string", "setFolderVisibility", projects, "maybe")
    call("foreign", "setFolderVisibility", foreign, True)
    folder_rows("after rename and visibility")

    call("into a folder", "moveUploadsToFolder", [up["a"], up["b"], up["t"], up["x"], "nope", 5], projects)
    call("into a child", "moveUploadsToFolder", [up["c"]], child)
    call("out of every folder", "moveUploadsToFolder", [up["b"]], None)
    call("foreign folder", "moveUploadsToFolder", [up["d"]], foreign)
    call("no usable id skips the folder check", "moveUploadsToFolder", ["nope"], foreign)
    call("empty list", "moveUploadsToFolder", [], "abc")
    call("not an array", "moveUploadsToFolder", "abc", projects)
    call("empty-string folder", "moveUploadsToFolder", [up["d"]], "")
    call("missing folder argument", "moveUploadsToFolder", [up["e"]])
    call("missing folder argument keeps a filed upload where it is", "moveUploadsToFolder", [up["a"], up["c"]])
    folder_rows("after moveUploadsToFolder")

    # countUnfiledUploads and moveFolder have no action id on the reference: no
    # client component imports them, so Next never registered them. They are
    # covered by tests/library_actions_db.rs.
    call("default name", "moveUnfiledToFolder")
    call("again reuses the folder", "moveUnfiledToFolder", "wszystko")
    call("null name", "moveUnfiledToFolder", None)
    call("custom name", "moveUnfiledToFolder", "  Inbox  ")
    folder_rows("after moveUnfiledToFolder")

    def unordered(env):
        if isinstance(env.get("ok"), list):
            env["ok"] = sorted(env["ok"], key=lambda r: (r.get("name"), r.get("slug")))
            for r in env["ok"]:
                r["id"] = "<id>"
                r["parentId"] = "<id>" if r.get("parentId") else None

    call("list (unordered upstream)", "listFolders", post=unordered)

    call("folder only", "deleteFolder", child)
    call("null opts after the ownership check", "deleteFolder", projects, None)
    call("null opts on a foreign folder", "deleteFolder", foreign, None)
    call("string opts reads as {}", "deleteFolder", folder_id("Level3"), "withContents")
    folder_rows("after deleteFolder (folder only)")
    call("with contents", "deleteFolder", projects, {"withContents": 1})
    call("foreign", "deleteFolder", foreign, {"withContents": True})
    call("non-uuid", "deleteFolder", "abc")
    folder_rows("after deleteFolder (with contents)")
    call("list after deletes", "listFolders", post=unordered)


# ── app/actions/duplicates.ts ────────────────────────────────────────────────


def duplicates():
    pair = run.pair()
    owner, tag = lib.seed_user("dpo")
    other, _ = lib.seed_user("dpt")
    lone, _ = lib.seed_user("dpl", skip_tag_in_url=True, preferred_domain="https://nowhere.example")
    for v, n in ((owner, "owner"), (other, "other"), (lone, "lone"), (tag, "tag")):
        NAMES.add(v, n)
    folder = NAMES.add(seed_folder(lib, owner, "Dupes"), "folder")
    t = lambda n: "2026-01-01T00:02:%02d.000Z" % n
    a1 = hashlib.sha256(("a" + SFX).encode()).hexdigest()
    b1 = hashlib.sha256(("b" + SFX).encode()).hexdigest()
    c1 = hashlib.sha256(("c" + SFX).encode()).hexdigest()
    for v, n in ((a1, "sha-a"), (b1, "sha-b"), (c1, "sha-c")):
        NAMES.add(v, n)
    ids = {}
    n = [0]

    def up(key, who, checksum, **cols):
        n[0] += 1
        ids[key] = NAMES.add(seed_upload(lib, who, "d%s%s" % (key, SFX), created_at=t(n[0]), checksum=checksum,
                                         **cols), "d-" + key)

    up("a1", owner, a1, size=1000, views=3, downloads=1, blurhash="LEHV6nWB2yk8")
    up("a2", owner, a1, size=1000, folder_id=folder)
    up("a3", owner, a1, size=1000)
    up("b1", owner, b1, size=50)
    up("b2", owner, b1, size=50, deleted_at=TRASHED_5)
    up("b3", owner, b1, size=50)
    up("c1", owner, c1, size=7)
    up("n1", owner, None)
    up("n2", owner, "")
    up("x1", other, a1, size=1000)
    up("x2", other, a1, size=1000)

    def dup_rows(name):
        rows("uploads " + name, 'SELECT code, deleted_at IS NULL FROM uploads WHERE owner_id IN (%s) ORDER BY code COLLATE "C"'
             % in_list([owner, other]))
        rows("dismissals " + name, 'SELECT owner_id, checksum, copies, bytes FROM duplicate_dismissals '
             'WHERE owner_id IN (%s) ORDER BY owner_id COLLATE "C", checksum COLLATE "C"' % in_list([owner, other]))

    signed_out_checks("duplicates", [(e, [] if e == "scanMyDuplicates" else [[ids["a1"]]]) for e in (
        "scanMyDuplicates", "trashDuplicates", "keepDuplicateSets", "countDuplicateSetsAgain")])

    parity.signed_in(pair, REF, RUST, owner)
    scan = lambda env: env.get("ok", {}).get("scan", {}).update({"scannedAt": "<iso>"}) if isinstance(
        env.get("ok"), dict) and isinstance(env["ok"].get("scan"), dict) else None
    act(pair, "scan", "duplicates", "scanMyDuplicates", [], post=scan)

    for label, arg in (("not an array", "x"), ("empty", []), ("malformed", ["nope"]), ("number entry", [5]),
                       ("too many", [str(uuid.uuid4()) for _ in range(2001)])):
        act(pair, label, "duplicates", "trashDuplicates", [arg])
    act(pair, "own copies, a foreign one, a trashed one, repeats", "duplicates", "trashDuplicates",
        [[ids["a2"], " " + ids["a3"].upper() + " ", ids["a3"], ids["x1"], ids["b2"], str(uuid.uuid4())]])
    dup_rows("after trashDuplicates")
    act(pair, "the same again (nothing left to stamp)", "duplicates", "trashDuplicates", [[ids["a2"]]])
    act(pair, "scan after trashing", "duplicates", "scanMyDuplicates", [], post=scan)

    for label, arg in (("not an array", "x"), ("empty", []), ("malformed", [{"checksum": "nope"}]),
                       ("missing checksum", [{"copies": 2}]), ("primitive entry", [b1]),
                       ("too many", [{"checksum": sha("f")} for _ in range(501)])):
        act(pair, label, "duplicates", "keepDuplicateSets", [arg])
    act(pair, "normalised and collapsed", "duplicates", "keepDuplicateSets",
        [[{"checksum": " " + b1.upper(), "copies": 2, "bytes": 100},
          {"checksum": b1, "copies": "3", "bytes": 150.9},
          {"checksum": c1}]])
    act(pair, "again updates the counts", "duplicates", "keepDuplicateSets",
        [[{"checksum": b1, "copies": -1, "bytes": "x"}]])
    act(pair, "a checksum nobody has", "duplicates", "keepDuplicateSets", [[{"checksum": sha("e"), "copies": 1e12}]])
    dup_rows("after keepDuplicateSets")
    act(pair, "scan with dismissals", "duplicates", "scanMyDuplicates", [], post=scan)

    for label, arg in (("not an array", None), ("empty", []), ("malformed", ["zz"])):
        act(pair, label, "duplicates", "countDuplicateSetsAgain", [arg])
    act(pair, "restore one, one unknown", "duplicates", "countDuplicateSetsAgain", [[b1.upper(), sha("9")]])
    dup_rows("after countDuplicateSetsAgain")

    parity.signed_in(pair, REF, RUST, other)
    act(pair, "another account cannot restore mine", "duplicates", "countDuplicateSetsAgain", [[c1]])
    act(pair, "scan of the other account", "duplicates", "scanMyDuplicates", [], post=scan)
    parity.signed_in(pair, REF, RUST, lone)
    act(pair, "empty scan", "duplicates", "scanMyDuplicates", [], post=scan)
    dup_rows("at the end")
    audit_rows("duplicates", [owner, other])


# ── app/actions/shortlinks.ts ────────────────────────────────────────────────


def shortlinks():
    pair = run.pair()
    owner, tag = lib.seed_user("slo")
    notag, _ = lib.seed_user("sln")
    lib.both_sql('UPDATE "user" SET user_code = NULL WHERE id = %s' % q(notag))
    root, root_tag = lib.seed_user("slr", allow_root_short_links=True)
    for v, n in ((owner, "owner"), (notag, "notag"), (root, "root"), (tag, "tag"), (root_tag, "root-tag")):
        NAMES.add(v, n)
    # humanCode(7): six letters of either case, one digit.
    NAMES.patterns.append((r"<tag>/[A-Za-z]{6}[0-9]\b", "<tag>/<random>"))
    NAMES.patterns.append((r'"code": "[A-Za-z]{6}[0-9]"', '"code": "<random>"'))
    NAMES.patterns.append((r"/s/[A-Za-z]{6}[0-9]\b", "/s/<random>"))

    def link_rows(name):
        rows("short_links " + name,
             'SELECT owner_id, regexp_replace(code, \'(^|/)[A-Za-z]{6}[0-9]$\', \'\\1<random>\'), target, '
             'password_hash IS NULL, clicks, last_used_at, expires_at IS NULL, '
             'round(extract(epoch FROM expires_at - created_at) / 60) FROM short_links WHERE owner_id IN (%s) '
             'ORDER BY owner_id COLLATE "C", target COLLATE "C", code COLLATE "C"' % in_list([owner, notag, root]))

    signed_out_checks("shortlinks", [("createShortLink", [{"target": "example.com"}]), ("listShortLinks", [])])
    parity.signed_in(pair, REF, RUST, owner)
    sl = lambda name, inp: act(pair, name, "shortlinks", "createShortLink", [inp])
    sl("bare host", {"target": "example.com/a b"})
    sl("custom code", {"target": "https://example.com/x", "customCode": "  My Code " + SFX})
    sl("custom code taken", {"target": "https://example.com/y", "customCode": "my-code-" + SFX})
    sl("custom code slugs to nothing", {"target": "https://example.com/y", "customCode": "!!!"})
    sl("custom code over 256", {"target": "https://example.com/y", "customCode": "a" * 257})
    sl("scoped code over 64", {"target": "https://example.com/y", "customCode": "b" * 60})
    sl("blank custom code is random", {"target": "https://example.com/z", "customCode": "   "})
    sl("javascript url", {"target": "javascript:alert(1)"})
    sl("no dot in host", {"target": "intranet"})
    sl("empty target", {"target": "  "})
    sl("root without permission", {"target": "example.com", "useUserCode": False})
    sl("useUserCode as a string is still scoped", {"target": "example.org", "useUserCode": "false",
                                                    "customCode": "str-" + SFX})
    sl("password and expiry", {"target": "https://example.com/p", "customCode": "pw-" + SFX,
                               "password": "  secret  ", "expiresInDays": 2})
    sl("expiry clamped to ten years", {"target": "https://example.com/e", "customCode": "ten-" + SFX,
                                       "expiresInDays": 99999})
    sl("string expiry ignored", {"target": "https://example.com/s", "customCode": "se-" + SFX,
                                 "expiresInDays": "5"})
    sl("number target", {"target": 5})
    sl("missing target", {})
    sl("null input", None)
    sl("string input", "example.com")
    sl("number custom code", {"target": "example.com", "customCode": 5})
    sl("unicode host", {"target": "żółw.pl/ścieżka?q=ą", "customCode": "idn-" + SFX})
    sl("scheme case and default port", {"target": "HTTP://Example.COM:80/a/../b", "customCode": "port-" + SFX})
    sl("credentials kept", {"target": "https://user:pa ss@example.com/", "customCode": "cred-" + SFX})
    sl("protocol-relative", {"target": "//example.com/x", "customCode": "rel-" + SFX})
    sl("space in host", {"target": "exa mple.com"})
    sl("ipv4 host", {"target": "127.0.0.1:8080/x", "customCode": "ip-" + SFX})
    sl("ipv6 host has no dot", {"target": "http://[::1]/x"})
    sl("unicode custom code", {"target": "example.com", "customCode": "Zażółć " + SFX})
    sl("NaN expiry", {"target": "example.com", "customCode": "nan-" + SFX, "expiresInDays": float("nan")})
    sl("fractional expiry", {"target": "example.com", "customCode": "frac-" + SFX, "expiresInDays": 0.25})
    link_rows("after createShortLink")

    parity.signed_in(pair, REF, RUST, notag)
    sl("no tag", {"target": "example.com"})
    parity.signed_in(pair, REF, RUST, root)
    sl("root allowed", {"target": "example.com/root", "useUserCode": False, "customCode": "root-" + SFX})
    sl("root random", {"target": "example.com/root2", "useUserCode": False})
    link_rows("after root links")

    parity.signed_in(pair, REF, RUST, owner)
    redact_hash = lambda env: [r.update({"passwordHash": "<hash>" if r.get("passwordHash") else None})
                               for r in env.get("ok", []) if isinstance(r, dict)] if isinstance(env.get("ok"),
                                                                                                list) else None
    _, listed = act(pair, "own links", "shortlinks", "listShortLinks", [], post=redact_hash)
    a_ids = REF.sql('SELECT id FROM short_links WHERE owner_id = %s AND code = %s'
                    % (q(owner), q("%s/my-code-%s" % (tag, SFX)))).strip()
    u_ids = RUST.sql('SELECT id FROM short_links WHERE owner_id = %s AND code = %s'
                     % (q(owner), q("%s/my-code-%s" % (tag, SFX)))).strip()
    r_root = (REF.sql('SELECT id FROM short_links WHERE owner_id = %s LIMIT 1' % q(root)).strip(),
              RUST.sql('SELECT id FROM short_links WHERE owner_id = %s LIMIT 1' % q(root)).strip())

    def two(name, export, args_ref, args_rust, post=None):
        a = acts.call_ref(pair.ref, "shortlinks", export, args_ref)
        b = acts.call_rust(pair.rust, "shortlinks", export, args_rust)
        ea, eb = envelopes(a, b)
        ea["set-cookie"], eb["set-cookie"] = cookie_list(a), cookie_list(b)
        run.compare("shortlinks.%s: %s" % (export, name), ea, eb)

    two("retarget", "retargetShortLink", [a_ids, "  example.net/new  "], [u_ids, "  example.net/new  "])
    two("retarget to javascript", "retargetShortLink", [a_ids, "javascript:x"], [u_ids, "javascript:x"])
    two("retarget someone else's", "retargetShortLink", [r_root[0], "example.net"], [r_root[1], "example.net"])
    act(pair, "unknown id", "shortlinks", "retargetShortLink", [str(uuid.uuid4()), "example.net"])
    act(pair, "non-uuid id", "shortlinks", "retargetShortLink", ["abc", "example.net"])
    act(pair, "null id", "shortlinks", "retargetShortLink", [None, "example.net"])
    act(pair, "number target", "shortlinks", "retargetShortLink", [str(uuid.uuid4()), 5])
    act(pair, "null target", "shortlinks", "retargetShortLink", [str(uuid.uuid4()), None])
    two("delete someone else's (silent)", "deleteShortLink", [r_root[0]], [r_root[1]])
    act(pair, "delete unknown", "shortlinks", "deleteShortLink", [str(uuid.uuid4())])
    act(pair, "delete non-uuid", "shortlinks", "deleteShortLink", ["abc"])
    act(pair, "delete null", "shortlinks", "deleteShortLink", [None])
    act(pair, "delete without an id", "shortlinks", "deleteShortLink", [])
    act(pair, "undefined id", "shortlinks", "retargetShortLink", [UNDEFINED, "example.net"])
    act(pair, "boolean id", "shortlinks", "deleteShortLink", [True])
    sl("a code spelled true", {"target": "example.com/t", "customCode": "true"})
    two("delete own", "deleteShortLink", [a_ids], [u_ids])
    link_rows("after retarget and delete")
    act(pair, "own links after", "shortlinks", "listShortLinks", [], post=redact_hash)


# ── app/actions/shortlink-unlock.ts ──────────────────────────────────────────


def unlock():
    owner, tag = lib.seed_user("ulo")
    NAMES.add(owner, "owner")
    NAMES.add(tag, "tag")
    link_id = NAMES.add(str(uuid.uuid4()), "link")
    open_id = NAMES.add(str(uuid.uuid4()), "open-link")
    code = "%s/gate-%s" % (tag, SFX)
    hashed = scrypt_hash("  spaced secret  ")
    lib.both_sql("INSERT INTO short_links (id, owner_id, code, target, password_hash) VALUES (%s, %s, %s, %s, %s)"
                 % (q(link_id), q(owner), q(code), q("https://example.com/gated"), q(hashed)))
    lib.both_sql("INSERT INTO short_links (id, owner_id, code, target) VALUES (%s, %s, %s, %s)"
                 % (q(open_id), q(owner), q("open-" + SFX), q("https://example.com/open")))
    lib.both_sql("INSERT INTO short_links (id, owner_id, code, target, password_hash) VALUES (%s, %s, %s, %s, %s)"
                 % (q(str(uuid.uuid4())), q(owner), q("7" + SFX), q("https://example.com/n"), q(hashed)))

    pair = run.pair()  # a fresh per-run X-Real-IP: the throttle starts empty
    v = lambda name, args: act(pair, name, "shortlink-unlock", "verifyShortlinkPassword", args)
    v("no code", ["", "x"])
    v("no password", [code, ""])
    v("number password", [code, 5])
    v("password missing", [code])
    v("unknown code", ["nope-" + SFX, "x"])
    v("link without a password", ["open-" + SFX, "x"])
    v("wrong password", [code, "nope"])
    v("trimmed password (stored untrimmed)", [code, "spaced secret"])
    v("number code", [int("7" + SFX), "  spaced secret  "])
    v("right password sets the cookie", [code, "  spaced secret  "])

    # The token itself: `<exp>.<HMAC-SHA256(secret, "shortlink|<id>|<exp>")>`,
    # each side checked against its own secret, expiring in an hour.
    cookie = "ml_sl_" + link_id
    now = int(time.time())
    run.compare("unlock: the cookie is a valid shortlink token for the link, an hour long",
                [token_view(pair.ref.cookies.get(cookie), REF.auth_secret, ["shortlink", link_id], now),
                 token_view(pair.rust.cookies.get(cookie), RUST.auth_secret, ["shortlink", link_id], now)],
                [(True, 60), (True, 60)])

    # And it opens the gate (each server is handed the cookie it minted). The
    # /s/ route is another port's; until it is registered Rust answers 501.
    a, b = pair.both("GET", "/s/" + code, cookies=True, origin=False)
    if b.status == 501:
        print("SKIP unlock: /s/<code> is not ported on the Rust server yet (501)")
    else:
        run.compare("unlock: the minted cookie passes /s/<code>",
                    (a.status, a.header("location")), (b.status, b.header("location")))
        a, b = pair.both("GET", "/s/" + code, cookies=False, origin=False)
        run.compare("unlock: without the cookie /s/<code> sends to the gate",
                    (a.status, NAMES.label(a.header("location"))), (b.status, NAMES.label(b.header("location"))))

    # Five failures per (ip, code) per 15 minutes, the sixth is refused before
    # the password is even looked at; unknown codes count too.
    limited = run.pair()
    for i in range(5):
        act(limited, "wrong #%d" % (i + 1), "shortlink-unlock", "verifyShortlinkPassword", [code, "bad%d" % i])
    act(limited, "sixth attempt is rate-limited", "shortlink-unlock", "verifyShortlinkPassword",
        [code, "  spaced secret  "])
    act(limited, "another code has its own bucket", "shortlink-unlock", "verifyShortlinkPassword",
        ["open-" + SFX, "x"])
    for i in range(6):
        act(limited, "unknown code #%d" % (i + 1), "shortlink-unlock", "verifyShortlinkPassword",
            ["ghost-" + SFX, "x"])


# ── app/actions/pastes.ts ────────────────────────────────────────────────────


def pastes():
    pair = run.pair()
    owner, tag = lib.seed_user("pso", used_quota=10)
    hidden, htag = lib.seed_user("psh", hide_uploads=True, preferred_domain="https://nowhere.example")
    tight, ttag = lib.seed_user("pst", max_quota=20, used_quota=10)
    defpw, dtag = lib.seed_user("psd", default_password_hash=scrypt_hash("dflt"), default_password_version=3)
    for v, n in ((owner, "owner"), (hidden, "hidden"), (tight, "tight"), (defpw, "defpw"), (tag, "tag"),
                 (htag, "htag"), (ttag, "ttag"), (dtag, "dtag")):
        NAMES.add(v, n)
    # humanCode(10): consonant/vowel pairs of either case, then two digits.
    NAMES.patterns.append((r"\b(?:[b-df-hj-np-tv-zB-DF-HJ-NP-TV-Z][aeiouAEIOU]){4}[0-9]{2}\b", "<code>"))

    def paste_rows(name, who):
        text = rows("pastes " + name,
                    'SELECT filename, ext, kind, language, mime_type, size, checksum, published, '
                    'expires_at IS NULL, round(extract(epoch FROM expires_at - created_at) / 60), '
                    'password_hash IS NULL, password_version, max_views, folder_id IS NULL, '
                    'length(delete_token), regexp_replace(storage_key, \'/[A-Za-z]{8}[0-9]{2}\\.\', \'/<code>.\') '
                    'FROM uploads WHERE owner_id IN (%s) ORDER BY created_at' % in_list(who))
        rows("quota " + name, 'SELECT id, used_quota FROM "user" WHERE id IN (%s) ORDER BY id COLLATE "C"' % in_list(who))
        return text

    def stored_objects(name, who):
        """The bytes each server wrote, by the stored key."""
        a, b = both('SELECT storage_key, size, checksum FROM uploads WHERE owner_id IN (%s) ORDER BY created_at'
                    % in_list(who))
        ka = [line.split("\t") for line in a.splitlines()]
        kb = [line.split("\t") for line in b.splitlines()]
        sa, sb = [], []
        for (key, size, checksum), (key2, size2, checksum2) in zip(ka, kb):
            r, _ = file_state(key)
            _, u = file_state(key2)
            sa.append((r, r[1] == checksum))
            sb.append((u, u[1] == checksum2))
        run.compare("storage: " + name, sa, sb)

    signed_out_checks("pastes", [("createPaste", [{"content": "x"}])])
    parity.signed_in(pair, REF, RUST, owner)
    p = lambda name, inp: act(pair, name, "pastes", "createPaste", [inp])
    p("plain text", {"content": "hello\nworld\n"})
    p("html with a title", {"content": "<h1>Hi</h1>", "language": "HTML", "title": "  My page: v2!  "})
    p("ts is not video", {"content": "let x = 1", "language": "ts"})
    p("unknown language", {"content": "x", "language": "brainfuck"})
    p("minutes", {"content": "m", "expiresInMinutes": 90.7})
    p("days", {"content": "d", "expiresInDays": 2, "expiresInMinutes": None})
    p("negative days", {"content": "n", "expiresInDays": -1})
    p("string days", {"content": "s", "expiresInDays": "1"})
    p("half a day is none", {"content": "h", "expiresInDays": 0.5})
    p("password", {"content": "p", "password": "pw"})
    p("max views", {"content": "v", "maxViews": 3.9})
    p("max views capped", {"content": "v", "maxViews": 5e9})
    p("max views string", {"content": "v", "maxViews": "2"})
    p("max views zero", {"content": "v", "maxViews": 0})
    p("unicode", {"content": "Zażółć gęślą jaźń 🚀", "title": "Zażółć"})
    p("empty", {"content": "   \n "})
    p("missing content", {})
    p("number content", {"content": 5})
    p("null input", None)
    p("number language", {"content": "x", "language": 5})
    p("padded language keeps txt but takes the type", {"content": "x", "language": "  JS  "})
    p("CRLF and tabs", {"content": "a\r\n\tb\r\n"})
    p("NaN views", {"content": "x", "maxViews": "abc"})
    p("emoji title", {"content": "x", "title": "🚀🚀"})
    p("long unicode title", {"content": "x", "title": "ą" * 100})
    p("false password uses the default", {"content": "x", "password": False})
    paste_rows("after createPaste", [owner])
    stored_objects("after createPaste", [owner])

    # Failures after the bytes were written leave the object behind upstream.
    before = both('SELECT count(*) FROM uploads WHERE owner_id = %s' % q(owner))
    p("number title (after the write)", {"content": "late", "title": 5})
    p("number password (after the write)", {"content": "late", "password": 5})
    p("language longer than the column", {"content": "late", "language": "x" * 40})
    p("expiry past the Date range", {"content": "late", "expiresInMinutes": 1e20})
    after = both('SELECT count(*) FROM uploads WHERE owner_id = %s' % q(owner))
    run.compare("pastes: failed creates insert nothing", before, after)
    a, b = ref_exec('ls "$1" | wc -l', OPT.ref_storage.rstrip("/") + "/" + tag)[1].decode().strip(), str(
        len(os.listdir(os.path.join(RUST_STORAGE, tag))))
    run.compare("pastes: objects in the tag directory (orphans of failed creates included)", a, b)

    parity.signed_in(pair, REF, RUST, hidden)
    p("hidden uploads, unknown preferred domain", {"content": "secret"})
    parity.signed_in(pair, REF, RUST, tight)
    p("quota exceeded", {"content": "x" * 11})
    p("quota exactly reached", {"content": "x" * 10})
    p("quota now full", {"content": "y"})
    parity.signed_in(pair, REF, RUST, defpw)
    p("account default password stamped", {"content": "d"})
    p("own password wins", {"content": "o", "password": "mine"})
    paste_rows("other accounts", [hidden, tight, defpw])


# ── app/actions/admin.ts: account deletion and per-user reads ────────────────


def admin():
    pair = run.pair()
    admin_id, _ = lib.seed_user("ada", role="admin")
    user, utag = lib.seed_user("adu")
    victim, vtag = lib.seed_user("adv", used_quota=2500)
    for v, n in ((admin_id, "admin"), (user, "user"), (victim, "victim"), (vtag, "vtag"), (utag, "utag")):
        NAMES.add(v, n)
    t = lambda n: "2026-01-01T00:03:%02d.000Z" % n
    vids = []
    for i in range(3):
        vids.append(NAMES.add(seed_upload(lib, victim, "v%d%s" % (i, SFX), created_at=t(i), size=500 + i,
                                          storage_key="%s/v%d%s.png" % (vtag, i, SFX),
                                          deleted_at=(TRASHED_10 if i == 2 else None)),
                              "v%d" % i))
    keys = ["%s/v%d%s.png" % (vtag, i, SFX) for i in range(3)]
    for k in keys:
        put_file(k, ("victim " + k).encode())
    seed_folder(lib, victim, "VF")
    lib.seed_token(victim, "full")
    lib.both_sql("INSERT INTO short_links (owner_id, code, target) VALUES (%s, %s, 'https://example.com')"
                 % (q(victim), q("%s/l%s" % (vtag, SFX))))
    lib.both_sql("INSERT INTO verification (id, identifier, value, expires_at, created_at, updated_at) VALUES "
                 "(%s, %s, %s, now() + interval '1 day', now(), now())"
                 % (q(str(uuid.uuid4())), q("invite:" + hashlib.sha256(SFX.encode()).hexdigest()), q(victim)))
    lib.both_sql("INSERT INTO account (id, account_id, provider_id, user_id, password, created_at, updated_at) "
                 "VALUES (%s, %s, 'credential', %s, 'x:y', now(), now())" % (q("acc" + SFX), q(victim), q(victim)))
    # A live session of the account, which removeUser deletes.
    REF.mint_session(victim)
    RUST.mint_session(victim)

    gated = [("loadDeletionImpact", [victim]), ("deleteUserAccount", [victim, "delete"]), ("loadOrphans", []),
             ("deleteOrphan", [str(uuid.uuid4())]), ("purgeAllOrphans", ["1"]), ("recalculateEveryQuota", []),
             ("runStorageScan", [])]
    signed_out_checks("admin", gated)
    parity.signed_in(pair, REF, RUST, user)
    for export, args in gated:
        act(pair, "not an admin", "admin", export, args)

    parity.signed_in(pair, REF, RUST, admin_id)
    act(pair, "seeded account", "admin", "loadDeletionImpact", [victim])
    act(pair, "empty id", "admin", "loadDeletionImpact", [""])
    act(pair, "unknown id", "admin", "loadDeletionImpact", ["nobody" + SFX])
    act(pair, "number id", "admin", "loadDeletionImpact", [5])
    act(pair, "missing id", "admin", "loadDeletionImpact", [])
    act(pair, "falsy id", "admin", "deleteOrphan", [""])
    act(pair, "unknown uuid", "admin", "deleteOrphan", [str(uuid.uuid4())])
    act(pair, "an owned upload is not an orphan", "admin", "deleteOrphan", [vids[0]])
    act(pair, "non-uuid", "admin", "deleteOrphan", ["nope"])
    act(pair, "empty id", "admin", "deleteUserAccount", ["", "delete"])
    act(pair, "self", "admin", "deleteUserAccount", [admin_id, "delete"])
    act(pair, "unknown", "admin", "deleteUserAccount", ["nobody" + SFX, "delete"])

    def account_rows(name):
        rows("account rows " + name,
             'SELECT (SELECT count(*) FROM "user" WHERE id = %(v)s), (SELECT count(*) FROM session WHERE user_id = %(v)s), '
             '(SELECT count(*) FROM account WHERE user_id = %(v)s), (SELECT count(*) FROM folders WHERE owner_id = %(v)s), '
             '(SELECT count(*) FROM api_tokens WHERE user_id = %(v)s), (SELECT count(*) FROM short_links WHERE owner_id = %(v)s), '
             '(SELECT count(*) FROM verification WHERE value = %(v)s), '
             '(SELECT count(*) FROM uploads WHERE code LIKE \'v_%(s)s\')' % {"v": q(victim), "s": SFX})

    account_rows("before")
    act(pair, "delete the files too", "admin", "deleteUserAccount", [victim, "delete"])
    account_rows("after deleting with files")
    files("after deleting with files", keys)
    act(pair, "the account is gone", "admin", "loadDeletionImpact", [victim])
    act(pair, "deleting it again", "admin", "deleteUserAccount", [victim, "delete"])
    audit_rows("admin", [admin_id, victim])


# ── app/actions/admin.ts: instance-wide maintenance ──────────────────────────


def instance():
    if any(":" + port in run.ref for port in SHARED_REFS) and not OPT.allow_shared:
        print("SKIP instance: %s is a shared reference (use a private stack, or --allow-shared)" % run.ref)
        return
    pair = run.pair()
    admin_id, _ = lib.seed_user("ina", role="admin")
    keeper, ktag = lib.seed_user("ink", used_quota=1)
    drift, dtag = lib.seed_user("ind")
    nulled, _ = lib.seed_user("inn")
    for v, n in ((admin_id, "admin"), (keeper, "keeper"), (drift, "drift"), (nulled, "nulled"), (ktag, "ktag"),
                 (dtag, "dtag")):
        NAMES.add(v, n)
    parity.signed_in(pair, REF, RUST, admin_id)

    # Settle every account first (rows other runs left behind on either side),
    # so what the measured call corrects is exactly this run's drift.
    acts.call_ref(pair.ref, "admin", "recalculateEveryQuota", [])
    acts.call_rust(pair.rust, "admin", "recalculateEveryQuota", [])
    act(pair, "nothing left to purge first", "admin", "purgeAllOrphans", ["0"])
    act(pair, "no orphans", "admin", "loadOrphans", [])

    t = lambda n: NAMES.add("2026-01-01T00:04:%02d.000Z" % n, "t%d" % n)
    NAMES.add(TRASHED_10, "trashed-at")
    kids = []
    for i in range(4):
        kids.append(NAMES.add(seed_upload(lib, keeper, "k%d%s" % (i, SFX), created_at=t(i), size=100 * (i + 1),
                                          storage_key="%s/k%d%s.png" % (ktag, i, SFX),
                                          thumb_key=("thumbs/%s/k%d%s.webp" % (ktag, i, SFX) if i == 0 else None),
                                          deleted_at=(TRASHED_10 if i == 3 else None)),
                              "k%d" % i))
    keys = ["%s/k%d%s.png" % (ktag, i, SFX) for i in range(4)] + ["thumbs/%s/k0%s.webp" % (ktag, SFX)]
    for k in keys:
        put_file(k, ("keeper " + k).encode(), age_seconds=7200)
    seed_upload(lib, drift, "dr" + SFX, size=4321, created_at=t(9))
    seed_upload(lib, nulled, "nu" + SFX, size=17, created_at=t(10), deleted_at=TRASHED_10)
    lib.both_sql('UPDATE "user" SET used_quota = NULL WHERE id IN (%s)' % in_list([keeper, nulled]))
    lib.both_sql('UPDATE "user" SET used_quota = 999999 WHERE id = %s' % q(drift))

    act(pair, "keep the files", "admin", "deleteUserAccount", [keeper, "keep"])
    rows("orphaned rows", 'SELECT code, owner_id IS NULL, size FROM uploads WHERE code LIKE %s ORDER BY code COLLATE "C"'
         % q("k_" + SFX))
    act(pair, "orphans after keep", "admin", "loadOrphans", [])
    act(pair, "impact of an account with drift", "admin", "loadDeletionImpact", [drift])

    # Storage scan: objects no row points at, old enough to be reported.
    put_file("%s/stray-old%s.bin" % (dtag, SFX), b"x" * 123, age_seconds=7200)
    put_file("%s/stray-new%s.bin" % (dtag, SFX), b"y" * 7)
    put_file("derived/%s/c.webp" % kids[1], b"derived", age_seconds=7200)
    put_file("avatars/%s-x.webp" % drift, b"avatar", age_seconds=7200)
    put_file("%s/avatars/hidden%s.png" % (dtag, SFX), b"nested", age_seconds=7200)

    def scan_view(env):
        ok = env.get("ok")
        if not isinstance(ok, dict) or "sample" not in ok:
            return
        mine = [s for s in ok["sample"] if "<dtag>" in s.get("key", "") or "<ktag>" in s.get("key", "")]
        # Filtered, not sorted: the walk order (depth first, names sorted) is compared too.
        env["ok"] = {"supported": ok["supported"], "driver": ok["driver"], "truncated": ok["truncated"],
                     "mine": mine}

    act(pair, "scan (this run's objects)", "admin", "runStorageScan", [], post=scan_view)

    # Totals against each side's own storage and rows: same rules, own data.
    a = acts.call_ref(pair.ref, "admin", "runStorageScan", [])
    b = acts.call_rust(pair.rust, "admin", "runStorageScan", [])
    oa = parity.next_action_envelope(a).get("ok") or {}
    ob = parity.rpc_envelope(b).get("ok") or {}
    run.compare("admin.runStorageScan: scanned = files outside the untracked trees",
                (oa.get("scanned"), oa.get("orphans") + oa.get("skippedRecent") <= oa.get("scanned")),
                (count_files(REF, "ref"), True))
    run.compare("admin.runStorageScan: rust scanned = files outside the untracked trees",
                (ob.get("scanned"), ob.get("orphans") + ob.get("skippedRecent") <= ob.get("scanned")),
                (count_files(RUST, "rust"), True))

    # Quota: the drifted counters are corrected; NULL reads as 0.
    def recalc_view(env):
        ok = env.get("ok")
        if isinstance(ok, dict) and "changes" in ok:
            env["ok"] = dict(ok, accounts="<n>", changes=sorted(ok["changes"], key=lambda c: c["id"]))

    act(pair, "corrects this run's drift", "admin", "recalculateEveryQuota", [], post=recalc_view)
    rows("quota after recalculation", 'SELECT id, used_quota, updated_at > now() - interval \'5 minutes\' '
         'FROM "user" WHERE id IN (%s) ORDER BY id COLLATE "C"' % in_list([admin_id, drift, nulled]))
    a = acts.call_ref(pair.ref, "admin", "recalculateEveryQuota", [])
    b = acts.call_rust(pair.rust, "admin", "recalculateEveryQuota", [])
    oa = parity.next_action_envelope(a).get("ok") or {}
    ob = parity.rpc_envelope(b).get("ok") or {}
    run.compare("admin.recalculateEveryQuota: accounts = every user row",
                (oa.get("accounts"), oa.get("corrected"), oa.get("drift"), oa.get("changes")),
                (int(REF.sql('SELECT count(*) FROM "user"').strip()), 0, 0, []))
    run.compare("admin.recalculateEveryQuota: rust accounts = every user row",
                (ob.get("accounts"), ob.get("corrected"), ob.get("drift"), ob.get("changes")),
                (int(RUST.sql('SELECT count(*) FROM "user"').strip()), 0, 0, []))

    # Orphans: one by one, then the typed-count purge.
    one = (REF.sql("SELECT id FROM uploads WHERE code = %s" % q("k1" + SFX)).strip())
    act(pair, "an orphan", "admin", "deleteOrphan", [one])
    act(pair, "the same one again", "admin", "deleteOrphan", [one])
    files("after deleteOrphan", keys)
    act(pair, "orphans after deleting one", "admin", "loadOrphans", [])
    act(pair, "wrong count", "admin", "purgeAllOrphans", ["2"])
    act(pair, "not a string", "admin", "purgeAllOrphans", [3])
    act(pair, "missing confirmation", "admin", "purgeAllOrphans", [])
    act(pair, "grouped count", "admin", "purgeAllOrphans", [" 3 "])
    files("after purgeAllOrphans", keys)
    act(pair, "nothing left", "admin", "purgeAllOrphans", ["3"])
    act(pair, "no orphans at the end", "admin", "loadOrphans", [])

    # Deleting an account whose storage refuses one object: that row is
    # parked (`failed`), never retried in the same call, and orphaned when the
    # account goes; the purge then fails on it until storage gives it up.
    stuck, stag = lib.seed_user("ins", used_quota=33)
    NAMES.add(stuck, "stuck")
    NAMES.add(stag, "stag")
    locked = "%s/locked" % stag
    skeys = ["%s/s0%s.png" % (locked, SFX), "%s/s1%s.png" % (stag, SFX), "%s/s2%s.png" % (stag, SFX)]
    for i, key in enumerate(skeys):
        NAMES.add(seed_upload(lib, stuck, "s%d%s" % (i, SFX), created_at=t(20 + i), size=11, storage_key=key),
                  "s%d" % i)
        put_file(key, b"stuck " + key.encode())
    lock_dir(locked, True)
    try:
        act(pair, "storage refuses one object", "admin", "deleteUserAccount", [stuck, "delete"])
        rows("rows after a refused object", 'SELECT code, owner_id IS NULL FROM uploads WHERE code LIKE %s '
             'ORDER BY code COLLATE "C"' % q("s_" + SFX))
        files("after a refused object", skeys)
        act(pair, "the refused row is an orphan", "admin", "loadOrphans", [])
        act(pair, "purge fails on it", "admin", "purgeAllOrphans", ["1"])
    finally:
        lock_dir(locked, False)
    act(pair, "purge once storage gives it up", "admin", "purgeAllOrphans", ["1"])
    files("after the purge", skeys)

    # Any disposition but "delete" leaves the files (as "keep" does), but only
    # "keep" reports them as orphaned: upstream answers `orphaned: failed` (0).
    odd, _ = lib.seed_user("ino")
    NAMES.add(odd, "odd")
    seed_upload(lib, odd, "o1" + SFX, created_at=t(30), size=5)
    act(pair, "an unknown disposition", "admin", "deleteUserAccount", [odd, "bogus"])
    act(pair, "missing disposition", "admin", "loadOrphans", [])
    act(pair, "purge them", "admin", "purgeAllOrphans", ["1"])
    rows("orphan rows at the end", 'SELECT count(*) FROM uploads WHERE owner_id IS NULL')


def count_files(db, side):
    """Regular files under the storage root outside derived/, backgrounds/ and
    avatars/ at the top level: what the scan must have examined."""
    if side == "ref":
        _, out = ref_exec('cd "$1" 2>/dev/null && find . -type f ! -path "./derived/*" ! -path "./backgrounds/*" '
                          '! -path "./avatars/*" | wc -l', OPT.ref_storage)
        return int(out.decode().strip() or 0)
    total = 0
    for dirpath, dirnames, filenames in os.walk(RUST_STORAGE):
        if dirpath == RUST_STORAGE:
            dirnames[:] = [d for d in dirnames if d not in ("derived", "backgrounds", "avatars")]
        total += sum(1 for f in filenames if os.path.isfile(os.path.join(dirpath, f)))
    return total


SECTIONS = [("uploads", uploads), ("folders", folders), ("duplicates", duplicates), ("shortlinks", shortlinks),
            ("unlock", unlock), ("pastes", pastes), ("admin", admin), ("instance", instance)]


def main():
    only = set(OPT.only.split(",")) if OPT.only else None
    for name, fn in SECTIONS:
        if only and name not in only:
            continue
        print("== %s" % name)
        fn()
    run.finish()


if __name__ == "__main__":
    main()

"""Parity of the panel shell (`app/(app)/layout.tsx`) and `/dashboard/admin`
(CONTRACT 5.8, 6.20) between the reference (the original server) and Rust.

The reference has no `/_page`, so its side is read from what it renders:
- the `(app)` layout: the props of its client components in the RSC payload
  inlined in the HTML (`ThemeApplier`, `TransfersProvider`, `AppShell`,
  `DuplicateReminder`), and the final `<html>` skin = what ThemeApplier's
  blocking inline script sets (`data-theme`, `--ml-bg-image`, `--ml-bg-dim`,
  `data-bg`);
- the admin page: the props of `StatsCards`, `SettingsForm`, `MailSettings`
  and `UserManager` in the RSC payload, plus `GET /api/admin/disk`, which
  upstream answers from the SAME two read models (`serverStorageUsage`,
  `diskInsights`) the page renders (`<ServerDisk>` is a server component, so
  its data never appears as props).

The Rust side is `/_page` (`layouts["(app)"]`, `props`, `meta`) and the
document's `<html data-theme style data-bg>`.

The rest of `lib/data/admin.ts` has no Rust HTTP surface yet (the admin
actions belong to the library port): the reference answers through its own
actions (`admin.loadOrphans`, `loadDeletionImpact`, `recalculateEveryQuota`,
`runStorageScan`, Next's protocol), the Rust functions through
`tests/admin_data_parity.rs` run against the Rust parity database, with
identical storage trees written into both servers' storage roots.

Seeds IDENTICAL state on both databases first, wiping `user` (CASCADE),
`uploads`, `folders`, `duplicate_dismissals` and the settings it varies: run
it only against your own scratch stacks, never the shared reference (8420).

    python3 server/tests/parity/panel.py --ref http://localhost:8428 --rust http://localhost:8436

Expected (explained) differences, checked as such rather than compared:
- `/dashboard` documents: the reference renders the page (200); the Rust
  dashboard page is another port's (501 payload with the layouts).
- `disk.disk` (statfs): two different filesystems; checked for shape only.
- the 307 of the `(app)` layout's `requireUser()`: the reference sends a whole
  HTML document as its body, the skeleton none (CONTRACT 2.1, "no body
  needed"); status, reason, Location and cookies are compared.
- a non-admin on `/dashboard/admin`: the reference answers 200 with a
  `<meta http-equiv="refresh" content="1;url=/dashboard">` (the page runs
  inside `dashboard/loading.tsx`'s boundary); the Rust skeleton answers 307
  to the same target (CONTRACT 2.1). The targets are compared.
- an upload whose extension is `constructor` with a useless stored type: the
  reference's `MIME_BY_EXT["constructor"]` is `Object`, so `mediaKind` throws
  and the disk report fails (500); Rust treats it as "no entry" (other).
"""

import argparse
import datetime
import html as htmllib
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

SCRATCH = "/path/to/modlight/_scratch/srv-auth-a"


def options():
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--ref-db-container", default="modlight-srvauth-a-ref-postgres-1")
    p.add_argument("--ref-db", default="modlight")
    p.add_argument("--ref-env", default=SCRATCH + "/ref/.env", help="file with the reference AUTH_SECRET")
    p.add_argument("--rust-db-container", default="modlight-devdb-postgres-1")
    p.add_argument("--rust-db", default="modlight_srv_auth_a")
    p.add_argument("--rust-env", default=SCRATCH + "/rust.env", help="file with the Rust AUTH_SECRET")
    p.add_argument("--node", default="node", help="node binary (encrypts the 'set' secrets per side)")
    p.add_argument("--ref-app-container", default="modlight-srvauth-a-ref-app-1")
    p.add_argument("--ref-storage", default="/app/data/storage", help="STORAGE_LOCAL_PATH inside the reference")
    p.add_argument("--rust-storage", default=SCRATCH + "/data/storage", help="STORAGE_LOCAL_PATH of the Rust server")
    a, _ = p.parse_known_args()
    return a


# ── Flight (RSC) payload of a reference document ───────────────────────────


def flight_rows(document):
    """Rows of the RSC stream inlined as self.__next_f.push([1,"..."])."""
    chunks = []
    for m in re.finditer(r"self\.__next_f\.push\((\[.*?\])\)</script>", document, re.S):
        arr = json.loads(m.group(1))
        if arr[0] == 1:
            chunks.append(arr[1])
    stream = "".join(chunks)
    rows = {}
    i, n = 0, len(stream)
    while i < n:
        colon = stream.find(":", i)
        if colon < 0:
            break
        key = stream[i:colon]
        j = colon + 1
        if j < n and stream[j] == "T":  # T<hex byte length>,<text>
            comma = stream.find(",", j)
            length = int(stream[j + 1:comma], 16)
            text = stream[comma + 1:].encode("utf-8")[:length].decode("utf-8")
            rows[key] = ("T", text)
            i = comma + 1 + len(text)
            continue
        nl = stream.find("\n", j)
        nl = n if nl < 0 else nl
        rows[key] = stream[j:nl]
        i = nl + 1
    return rows


class Flight(object):
    def __init__(self, document):
        self.rows = flight_rows(document)
        self.names = {}
        self.parsed = {}
        for k, v in self.rows.items():
            if isinstance(v, tuple):
                continue
            if v.startswith("I["):
                self.names[k] = json.loads(v[1:])[2]
                continue
            try:
                self.parsed[k] = json.loads(v)
            except ValueError:
                pass

    def decode(self, v):
        if isinstance(v, str) and v.startswith("$"):
            if v.startswith("$$"):
                return v[1:]
            if v == "$undefined":
                return "<undefined>"
            if v.startswith("$D"):
                return "<date:%s>" % v[2:]
            m = re.match(r"^\$([0-9a-f]+)((?::[^:]+)*)$", v)
            if m:
                target = self.parsed.get(m.group(1))
                for part in [p for p in m.group(2).split(":") if p]:
                    target = target[int(part)] if isinstance(target, list) else target[part]
                return self.decode(target)
            return v
        if isinstance(v, list):
            return [self.decode(x) for x in v]
        if isinstance(v, dict):
            return dict((k, self.decode(x)) for k, x in v.items())
        return v

    def elements(self, name):
        """Props of every rendered client component exported as `name`."""
        ids = set(k for k, n in self.names.items() if n == name)
        found = []

        def walk(v):
            if isinstance(v, list):
                if len(v) >= 4 and v[0] == "$" and isinstance(v[1], str) and v[1].startswith("$L") \
                        and v[1][2:] in ids and isinstance(v[3], dict):
                    found.append(v[3])
                for x in v:
                    walk(x)
            elif isinstance(v, dict):
                for x in v.values():
                    walk(x)

        for row in self.parsed.values():
            walk(row)
        return [self.decode(dict((k, x) for k, x in props.items() if k != "children")) for props in found]

    def one(self, name):
        found = self.elements(name)
        if len(found) != 1:
            raise AssertionError("expected one <%s> in the RSC payload, found %d" % (name, len(found)))
        return found[0]


def ref_app_layout(flight):
    """The (app) layout's data as the reference rendered it."""
    applier = flight.one("ThemeApplier")
    transfers = flight.one("TransfersProvider")
    shell = flight.one("AppShell")
    reminders = flight.elements("DuplicateReminder")
    return {
        "theme": applier.get("theme"),
        "background": applier.get("background"),
        "transferFolders": transfers.get("folders"),
        "defaultFolderId": transfers.get("defaultFolderId"),
        "user": shell.get("user"),
        "sidebar": shell.get("sidebar"),
        "defaultCollapsed": shell.get("defaultCollapsed"),
        "duplicateReminder": reminders[0] if reminders else None,
    }


def ref_skin(document):
    """What ThemeApplier's blocking script leaves on <html>."""
    m = re.search(r'var r=document\.documentElement,t=("(?:[^"\\]|\\.)*"),u=(null|"(?:[^"\\]|\\.)*");', document)
    if not m:
        return {"applier": None}
    theme, url = json.loads(m.group(1)), json.loads(m.group(2))
    d = re.search(r'r\.style\.setProperty\("--ml-bg-dim",("(?:[^"\\]|\\.)*")\)', document)
    return {"theme": theme, "bgImage": ('url("%s")' % url) if url else None,
            "bgDim": json.loads(d.group(1)) if url else None, "dataBg": "on" if url else None}


def html_attrs(document):
    tag = re.search(r"<html\b([^>]*)>", document, re.I).group(1)
    return dict((k, htmllib.unescape(v)) for k, v in re.findall(r'([\w-]+)="([^"]*)"', tag))


def rust_skin(document):
    attrs = html_attrs(document)
    style = attrs.get("style") or ""
    image = re.search(r"--ml-bg-image:(url\(\".*?\"\))", style)
    dim = re.search(r"--ml-bg-dim:([^;]*)", style)
    return {"theme": attrs.get("data-theme"), "bgImage": image.group(1) if image else None,
            "bgDim": dim.group(1) if dim else None, "dataBg": attrs.get("data-bg")}


def head_of(document):
    title = re.search(r"<title>(.*?)</title>", document, re.S)

    def meta(name):
        m = re.search(r'<meta name="%s" content="([^"]*)"' % re.escape(name), document)
        return htmllib.unescape(m.group(1)) if m else None

    return {"title": htmllib.unescape(title.group(1)) if title else None,
            "description": meta("description"), "applicationName": meta("application-name")}


def inlined_payload(document):
    m = re.search(r'<script id="__MODLIGHT_PAGE__" type="application/json">(.*?)</script>', document, re.S)
    return json.loads(m.group(1)) if m else None


# ── Seed ───────────────────────────────────────────────────────────────────

T0 = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=654321)


def ago(days, micro=0):
    """An absolute timestamp `days` before the seed instant, with microseconds
    (Postgres keeps them; both servers must cut them to milliseconds)."""
    return (T0 - datetime.timedelta(days=days, microseconds=micro)).isoformat()


def uid(name):
    return (name + "0" * 32)[:32]


ADMIN, USER, DECO, BARE = uid("pnlAdmin"), uid("pnlUser"), uid("pnlDeco"), uid("pnlBare")
BANNED, OWN_A, OWN_B, OWN_C = uid("pnlBanned"), uid("pnlOwnA"), uid("pnlOwnB"), uid("pnlOwnC")


def fid(n):
    return "00000000-0000-4000-8000-%012d" % n


def upid(n):
    return "00000000-0000-4000-9000-%012d" % n


F_ALPHA, F_BETA, F_GAMMA, F_DELTA, F_ORPHAN, F_CYCLE_A, F_CYCLE_B, F_SELF = [fid(i) for i in range(1, 9)]
UF_ZETA, UF_ALPHA, AF_INBOX = fid(11), fid(12), fid(21)
MISSING_PARENT = "ffffffff-ffff-4fff-bfff-000000000001"
SHA = lambda c: c * 64  # noqa: E731

FOLDERS = [
    # (id, owner, parent, name, is_public, sort_order)
    (F_ALPHA, DECO, None, "Alpha", False, 0),
    (F_BETA, DECO, F_ALPHA, "Beta", True, 0),
    (F_GAMMA, DECO, F_BETA, "Gamma", False, 0),
    (F_DELTA, DECO, None, "Delta", False, 1),
    (F_ORPHAN, DECO, MISSING_PARENT, "Orphanage", False, 0),
    (F_CYCLE_A, DECO, F_CYCLE_B, "CycleA", False, 0),
    (F_CYCLE_B, DECO, F_CYCLE_A, "CycleB", False, 0),
    (F_SELF, DECO, F_SELF, "Selfie", False, 0),
    (UF_ZETA, USER, None, "Zeta", False, 0),
    (UF_ALPHA, USER, None, "Alpha", False, 5),
    (AF_INBOX, ADMIN, None, "Inbox", False, 0),
]


def uploads():
    rows = []

    def up(owner, size, mime, ext, **kw):
        n = len(rows) + 1
        row = {"id": upid(n), "owner_id": owner, "code": "pnl%04d" % n, "filename": "file%d.%s" % (n, ext or "bin"),
               "ext": ext, "kind": kw.pop("kind", "file"), "mime_type": mime, "size": size,
               "storage_key": "parity/pnl%04d.%s" % (n, ext), "checksum": kw.pop("checksum", None),
               "delete_token": "pnldt%04d" % n, "views": kw.pop("views", 0),
               "created_at": kw.pop("created_at", ago(1, n)), "updated_at": ago(1, n),
               "last_accessed_at": kw.pop("last_accessed_at", None), "deleted_at": kw.pop("deleted_at", None),
               "thumb_key": kw.pop("thumb_key", None),
               "folder_id": kw.pop("folder_id", None)}
        assert not kw, kw
        rows.append(row)

    # admin: a duplicate pair over a gibibyte reclaimable (due by bytes)
    up(ADMIN, 1288490189, "video/mp4", "mp4", checksum=SHA("a"), folder_id=AF_INBOX, created_at=ago(10, 7), views=5)
    up(ADMIN, 1288490188, "video/mp4", "mp4", checksum=SHA("a"), created_at=ago(9, 3), views=1)
    up(ADMIN, 4096, "image/png", "png", views=3, thumb_key="thumbs/parity/pnl0003.webp")
    # three more owners with distinct totals, so the account list folds a tail
    up(OWN_A, 600000000, "application/zip", "zip", created_at=ago(400, 11), last_accessed_at=ago(190, 5))
    up(OWN_A, 300000000, "application/octet-stream", "avif", created_at=ago(5, 9), views=7)
    up(OWN_B, 400000000, "audio/mpeg", "mp3", created_at=ago(300, 1), last_accessed_at=ago(10, 1), views=2)
    up(OWN_B, 300000001, "Image/PNG; charset=x", "png", created_at=ago(6, 6))
    up(OWN_C, 299000000, "application/pdf", "pdf", created_at=ago(30, 2))
    up(OWN_C, 1000000, "image/svg+xml", "svg", created_at=ago(50, 2), deleted_at=ago(25, 4))
    up(BANNED, 100000000, "application/x-msdownload", "exe", created_at=ago(210, 8))
    # orphans: owner deleted, files kept
    up(None, 500000000, "video/mp4", "mp4", created_at=ago(20, 1), views=11)
    up(None, 1000, "text/plain", "txt", created_at=ago(60, 1), deleted_at=ago(40, 9))
    up(None, 2000, "application/pdf", "pdf", created_at=ago(250, 4))
    # the decorated user: 12 live copies of one file (11 redundant: due by count)
    spread = [F_ALPHA, F_ALPHA, F_BETA, F_GAMMA, F_GAMMA, F_GAMMA, F_DELTA, F_ORPHAN, F_CYCLE_A, F_CYCLE_A, F_SELF,
              None]
    for i, folder in enumerate(spread):
        up(DECO, 1000, "image/jpeg", "jpg", checksum=SHA("b"), folder_id=folder, created_at=ago(100 + i, i))
    up(DECO, 1000, "image/jpeg", "jpg", checksum=SHA("b"), deleted_at=ago(3, 1))  # trashed copy: not counted
    for _ in range(3):  # a set the owner said to keep
        up(DECO, 2000, "text/plain", "txt", checksum=SHA("c"))
    for _ in range(2):  # empty checksums never form a set
        up(DECO, 500, "application/json", "json", checksum="")
    up(DECO, 7000, "", "mov")
    up(DECO, 3000, "application/octet-stream", "gr2")
    up(DECO, 300, "text/plain", "txt", kind="paste")
    # the plain user: under both thresholds
    for _ in range(5):
        up(USER, 3000, "audio/ogg", "ogg", checksum=SHA("d"), folder_id=UF_ZETA)
    for _ in range(2):
        up(USER, 100, "text/csv", "csv", checksum=SHA("e"), folder_id=UF_ALPHA)
    up(USER, 5000, "image/gif", "gif", deleted_at=ago(29, 2))
    return rows


DISMISSALS = [(DECO, SHA("c"), 3, 6000), (USER, SHA("e"), 2, 200)]

SETTING_KEYS = ["register_enabled", "registration_key_enc", "smtp_host", "smtp_port", "smtp_security", "smtp_user",
                "smtp_password_enc", "mail_from", "mail_from_name", "email_verification_required"]


def insert(db, table, rows):
    if not rows:
        return
    cols = list(rows[0].keys())
    values = ",\n".join("(%s)" % ", ".join(parity.sql_quote(r[c]) for c in cols) for r in rows)
    db.sql('INSERT INTO "%s" (%s) VALUES %s' % (table, ", ".join('"%s"' % c for c in cols), values))


def seed(db):
    db.sql('TRUNCATE "user", "uploads", "folders", "duplicate_dismissals" CASCADE')
    db.sql('DELETE FROM "settings" WHERE "key" IN (%s)' % ", ".join(parity.sql_quote(k) for k in SETTING_KEYS))
    db.create_user(ADMIN, "pnl-admin@parity.test", "Panel Admin", role="admin", user_code="pnladm")
    db.create_user(USER, "pnl-user@parity.test", "Panel User", role="user", user_code="pnlusr", theme=None,
                   max_quota=10737418240)
    db.create_user(DECO, "pnl-deco@parity.test", "Panel Deco", role="user", user_code="pnldeco", theme="synthwave",
                   image="avatars/%s-av9tok.webp" % DECO, background_key="backgrounds/%s-bgtok42.webp" % DECO,
                   background_dim=45, max_quota=5368709120)
    db.create_user(BARE, "pnl-bare@parity.test", "Panel Bare", role=None, user_code=None, theme="nonsense",
                   image="https://cdn.example.test/a.png", max_quota=None)
    db.create_user(BANNED, "pnl-banned@parity.test", "Panel Banned", user_code="pnlban", banned=True)
    db.create_user(OWN_A, "pnl-a@parity.test", "Owner A", user_code="pnla")
    db.create_user(OWN_B, "pnl-b@parity.test", "Owner B", user_code="pnlb")
    db.create_user(OWN_C, "pnl-c@parity.test", "Owner C", user_code=None)
    insert(db, "folders", [{"id": f, "owner_id": o, "parent_id": p, "name": n, "slug": n.lower(), "is_public": pub,
                            "sort_order": s, "created_at": ago(500)} for (f, o, p, n, pub, s) in FOLDERS])
    db.sql('UPDATE "user" SET "default_folder_id" = %s WHERE "id" = %s' % (parity.sql_quote(F_ALPHA),
                                                                           parity.sql_quote(DECO)))
    insert(db, "uploads", uploads())
    insert(db, "duplicate_dismissals", [{"owner_id": o, "checksum": c, "copies": n, "bytes": b}
                                        for (o, c, n, b) in DISMISSALS])


def set_settings(db, values):
    db.sql('DELETE FROM "settings" WHERE "key" IN (%s)' % ", ".join(parity.sql_quote(k) for k in SETTING_KEYS))
    rows = [{"key": k, "value": v} for k, v in values.items()]
    insert(db, "settings", rows)


NODE_ENCRYPT = r"""
const crypto = require("crypto");
const [purpose, plaintext] = process.argv.slice(1);
const key = Buffer.from(crypto.hkdfSync("sha256", Buffer.from(process.env.SB_SECRET, "utf8"),
  Buffer.from("modlight/secret-box/2026", "utf8"), Buffer.from("modlight:" + purpose + ":v1", "utf8"), 32));
const iv = crypto.randomBytes(12);
const cipher = crypto.createCipheriv("aes-256-gcm", key, iv);
const ct = Buffer.concat([cipher.update(plaintext, "utf8"), cipher.final()]);
process.stdout.write(["v1", iv.toString("base64url"), cipher.getAuthTag().toString("base64url"),
  ct.toString("base64url")].join("."));
"""


def secret_box(node, secret, purpose, plaintext):
    """lib/secret-box.ts `encryptSecret` under one server's AUTH_SECRET (the
    secret goes through the environment and is never printed)."""
    env = dict(os.environ, SB_SECRET=secret)
    out = subprocess.run([node, "-e", NODE_ENCRYPT, purpose, plaintext], env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, check=True)
    return out.stdout.decode("ascii")


# ── Checks ─────────────────────────────────────────────────────────────────


def page_data(browser, path, query=""):
    q = "path=" + urllib.parse.quote(path, safe="") + ("&" + query if query else "")
    return browser.request("GET", "/_page?" + q, origin=False)


def main():
    opt = options()
    run = parity.Run.from_args()
    if ":8420" in run.ref:
        sys.exit("refusing to seed the shared reference (8420)")
    ref_env, rust_env = parity.read_env_file(opt.ref_env), parity.read_env_file(opt.rust_env)
    ref_db = parity.Db(opt.ref_db_container, opt.ref_db, ref_env["AUTH_SECRET"])
    rust_db = parity.Db(opt.rust_db_container, opt.rust_db, rust_env["AUTH_SECRET"])
    for db in (ref_db, rust_db):
        seed(db)

    def signed(user_id, **cookies):
        pair = run.pair()
        parity.signed_in(pair, ref_db, rust_db, user_id)
        for k, v in cookies.items():
            pair.ref.cookies[k] = v
            pair.rust.cookies[k] = v
        return pair

    # ── (app) layout, per user and per cookie ──────────────────────────────
    now_ms = int(datetime.datetime.now(datetime.timezone.utc).timestamp() * 1000)
    cases = [
        ("admin", ADMIN, {}),
        ("user (theme NULL, ml_skin=paper)", USER, {"ml_skin": "paper"}),
        ("deco", DECO, {}),
        ("deco, sidebar collapsed", DECO, {"ml_sidebar": "collapsed"}),
        ("deco, sidebar Collapsed (case)", DECO, {"ml_sidebar": "Collapsed"}),
        ("deco, snoozed", DECO, {"ml_dupes_snooze": str(now_ms + 3600 * 1000)}),
        ("deco, snooze lapsed", DECO, {"ml_dupes_snooze": str(now_ms - 1000)}),
        ("deco, snooze garbage", DECO, {"ml_dupes_snooze": "soon"}),
        ("deco, snooze Infinity", DECO, {"ml_dupes_snooze": "Infinity"}),
        ("bare (no role, no tag, absolute image)", BARE, {}),
    ]
    for label, user_id, cookies in cases:
        pair = signed(user_id, **cookies)
        ref_doc = pair.ref.request("GET", "/dashboard?all=1", origin=False)
        rust_doc = pair.rust.request("GET", "/dashboard?all=1", origin=False)
        rust_page = page_data(pair.rust, "/dashboard", "all=1")
        run.compare("(app) %s: document statuses (reference page 200, Rust page not ported 501)" % label,
                    [200, 501], [ref_doc.status, rust_doc.status])
        ref_layout = ref_app_layout(Flight(ref_doc.text()))
        body = rust_page.json() or {}
        rust_layout = parity._decode_tagged((body.get("layouts") or {}).get("(app)"))
        run.compare("(app) %s: layout data" % label, ref_layout, rust_layout)
        inlined = inlined_payload(rust_doc.text()) or {}
        run.compare("(app) %s: document payload carries the same layout" % label, rust_layout,
                    parity._decode_tagged((inlined.get("layouts") or {}).get("(app)")))
        run.compare("(app) %s: <html> skin and wallpaper" % label, ref_skin(ref_doc.text()),
                    rust_skin(rust_doc.text()))

    # ── /dashboard/admin gates ─────────────────────────────────────────────
    anon = run.pair()
    anon.check("admin page signed out: proxy gate", "GET", "/dashboard/admin", body="empty",
               headers_to_compare=("location",), origin=False)
    garbage = run.pair()
    garbage.ref.cookies[parity.SESSION_COOKIE] = "garbage"
    garbage.rust.cookies[parity.SESSION_COOKIE] = "garbage"
    ref_res, rust_res = garbage.check("admin page with a dead session: (app) layout requireUser", "GET",
                                      "/dashboard/admin", body="none", headers_to_compare=("location",),
                                      origin=False)
    # Next renders the layout's redirect into a whole HTML document under the
    # 307; the skeleton sends the redirect without a body (CONTRACT 2.1:
    # "Location header, no body needed"). Browsers only read Location.
    run.compare("admin page with a dead session: 307 body (reference HTML, Rust none: CONTRACT 2.1)",
                {"reference": "html", "rust": "empty"},
                {"reference": "html" if b"<html" in ref_res.body else "other",
                 "rust": "empty" if not rust_res.body else "%d bytes" % len(rust_res.body)})
    run.compare("admin page with a dead session: /_page", {"redirect": "/login", "status": 307},
                page_data(garbage.rust, "/dashboard/admin").json())
    nonadmin = signed(USER)
    ref_doc = nonadmin.ref.request("GET", "/dashboard/admin", origin=False)
    rust_doc = nonadmin.rust.request("GET", "/dashboard/admin", origin=False)
    refresh = re.search(r'<meta id="__next-page-redirect" http-equiv="refresh" content="1;url=([^"]*)"',
                        ref_doc.text())
    run.compare("admin page as a non-admin: redirect target (reference 200 + refresh, Rust 307)",
                {"status": 200, "target": "/dashboard"}, {"status": ref_doc.status,
                                                           "target": refresh.group(1) if refresh else None})
    run.compare("admin page as a non-admin: Rust document and /_page",
                {"status": 307, "location": "/dashboard", "page": {"redirect": "/dashboard", "status": 307}},
                {"status": rust_doc.status, "location": rust_doc.header("location"),
                 "page": page_data(nonadmin.rust, "/dashboard/admin").json()})

    # ── /dashboard/admin props, under several settings states ──────────────
    def expect_state(label, props, expected):
        """The reference's own answer must be the state the scenario set up,
        so a shared mistake (say, a secret box neither side can open) cannot
        pass as parity."""
        run.compare("admin (%s): the scenario's state on the reference" % label, expected,
                    dict((k, props.get(k)) for k in expected))

    def admin_props(label):
        pair = signed(ADMIN)
        ref_doc = pair.ref.request("GET", "/dashboard/admin", origin=False)
        disk_api = pair.ref.request("GET", "/api/admin/disk", origin=False)
        rust_page = page_data(pair.rust, "/dashboard/admin")
        rust_doc = pair.rust.request("GET", "/dashboard/admin", origin=False)
        run.compare("admin %s: statuses (page, /api/admin/disk, /_page, document)" % label, [200, 200, 200, 200],
                    [ref_doc.status, disk_api.status, rust_page.status, rust_doc.status])
        flight = Flight(ref_doc.text())
        settings_form, mail_settings = flight.one("SettingsForm"), flight.one("MailSettings")
        users = flight.one("UserManager")
        api = disk_api.json() or {}
        ref_props = {
            "session": {"user": {"id": users.get("currentUserId"), "email": mail_settings.get("adminEmail")}},
            "regEnabled": settings_form["registration"]["enabled"],
            "keyState": settings_form["registration"]["keyState"],
            "mail": mail_settings.get("initial"),
            "stats": flight.one("StatsCards").get("stats"),
            "disk": dict((k, api.get(k)) for k in ("driver", "liveFiles", "liveBytes", "trashedFiles",
                                                    "trashedBytes", "disk", "minFreeBytes")),
            "diskInsights": dict((k, api.get(k)) for k in ("driver", "local", "kinds", "kindBytes", "accounts",
                                                            "biggest", "cold", "coldTotals", "trash")),
            "defaultQuota": settings_form.get("defaultQuota"),
        }
        body = rust_page.json() or {}
        rust_props = parity._decode_tagged(body.get("props"))
        # UserManager is handed mail.ready separately: it must agree.
        run.compare("admin %s: UserManager mailReady = mail.ready" % label, ref_props["mail"]["ready"],
                    users.get("mailReady"))
        for side in (ref_props, rust_props or {}):
            disk = (side.get("disk") or {}).get("disk")
            if isinstance(disk, dict) and sorted(disk) == ["freeBytes", "totalBytes", "usedBytes"] \
                    and all(isinstance(x, int) and x >= 0 for x in disk.values()) \
                    and disk["freeBytes"] <= disk["totalBytes"] and disk["usedBytes"] <= disk["totalBytes"]:
                side["disk"]["disk"] = "<statfs of this server's storage filesystem>"
        run.compare("admin %s: props" % label, ref_props, rust_props)
        run.compare("admin %s: meta" % label, head_of(ref_doc.text()), dict(
            (k, (body.get("meta") or {}).get(k)) for k in ("title", "description", "applicationName")))
        run.compare("admin %s: document head" % label, head_of(ref_doc.text()), head_of(rust_doc.text()))
        return ref_props

    props = admin_props("(no settings)")
    expect_state("no settings", props, {"regEnabled": False, "keyState": "unset", "defaultQuota": "20.0 GB",
                                        "mail": {"source": "none", "host": "", "port": 587, "security": "starttls",
                                                 "user": "", "from": "no-reply@localhost", "fromName": "Modlight",
                                                 "ready": False, "passwordState": "unset",
                                                 "verificationRequired": False, "envAvailable": False}})
    # A few facts that must hold whatever both sides say, so a shared mistake
    # cannot pass as parity.
    insights = props["diskInsights"]
    run.compare("admin: the account list folds the tail and keeps the orphan row", {
        "rows": 7, "orphaned": 1,
        "rest": {"accounts": 2, "liveFiles": 27, "liveBytes": 44500, "trashedFiles": 2, "trashedBytes": 6000}},
        {"rows": len(insights["accounts"]), "orphaned": sum(1 for a in insights["accounts"] if a["orphaned"]),
         "rest": dict((k, insights["accounts"][-1][k])
                      for k in ("accounts", "liveFiles", "liveBytes", "trashedFiles", "trashedBytes"))})
    run.compare("admin: the cold list, oldest touch first", ["pnl0013", "pnl0010", "pnl0004"],
                [f["code"] for f in insights["cold"]])
    run.compare("admin: kinds reconcile with the live bytes", props["disk"]["liveBytes"], insights["kindBytes"])

    for db, env in ((ref_db, ref_env), (rust_db, rust_env)):
        set_settings(db, {"register_enabled": "on", "registration_key_enc": "v1.AAAA.BBBB.CCCC",
                          "smtp_host": " smtp.parity.test ", "smtp_port": "2525", "smtp_security": "tls",
                          "smtp_user": " mailer ", "mail_from": " robot@parity.test ",
                          "mail_from_name": " Parity Robot ", "email_verification_required": "on"})
    expect_state("panel SMTP without password, undecryptable key",
                 admin_props("(panel SMTP without password, undecryptable key)"),
                 {"regEnabled": True, "keyState": "undecryptable", "defaultQuota": "20.0 GB",
                  "mail": {"source": "panel", "host": "smtp.parity.test", "port": 2525, "security": "tls",
                           "user": "mailer", "from": "robot@parity.test", "fromName": "Parity Robot",
                           "ready": False, "passwordState": "unset", "verificationRequired": False,
                           "envAvailable": False}})

    for db, env in ((ref_db, ref_env), (rust_db, rust_env)):
        set_settings(db, {"register_enabled": "true",
                          "registration_key_enc": secret_box(opt.node, env["AUTH_SECRET"], "registration-key",
                                                             "parity-key"),
                          "smtp_host": "smtp.parity.test", "smtp_port": "abc", "smtp_security": "bogus",
                          "smtp_user": "mailer", "mail_from": "", "email_verification_required": "1",
                          "smtp_password_enc": secret_box(opt.node, env["AUTH_SECRET"], "smtp", "hunter2")})
    expect_state("panel SMTP ready, key set", admin_props("(panel SMTP ready, key set)"),
                 {"regEnabled": True, "keyState": "set", "defaultQuota": "20.0 GB",
                  "mail": {"source": "panel", "host": "smtp.parity.test", "port": 587, "security": "starttls",
                           "user": "mailer", "from": "no-reply@localhost", "fromName": "Modlight",
                           "ready": True, "passwordState": "set", "verificationRequired": True,
                           "envAvailable": False}})

    for db in (ref_db, rust_db):
        set_settings(db, {"register_enabled": "yes", "smtp_host": "smtp.parity.test", "smtp_user": "",
                          "smtp_password_enc": "v1.x.y.z"})
    expect_state("undecryptable SMTP password", admin_props("(undecryptable SMTP password)"),
                 {"regEnabled": False, "keyState": "unset", "defaultQuota": "20.0 GB",
                  "mail": {"source": "panel", "host": "smtp.parity.test", "port": 587, "security": "starttls",
                           "user": "", "from": "no-reply@localhost", "fromName": "Modlight", "ready": False,
                           "passwordState": "undecryptable", "verificationRequired": False,
                           "envAvailable": False}})
    for db in (ref_db, rust_db):
        set_settings(db, {})

    # ── The one deliberate difference: MIME_BY_EXT["constructor"] ───────────
    quirk = dict(uploads()[0], id=upid(999), owner_id=OWN_A, code="pnlquirk", filename="x.constructor",
                 ext="constructor", mime_type="application/octet-stream", size=10, checksum=None,
                 storage_key="parity/x.constructor", delete_token="pnldtquirk", folder_id=None)
    for db in (ref_db, rust_db):
        insert(db, "uploads", [quirk])
    pair = signed(ADMIN)
    ref_api = pair.ref.request("GET", "/api/admin/disk", origin=False)
    rust_page = page_data(pair.rust, "/dashboard/admin")
    other = [k for k in ((rust_page.json() or {}).get("props") or {}).get("diskInsights", {}).get("kinds", [])
             if k["kind"] == "other"]
    run.compare("ext 'constructor': reference disk report fails (Object.prototype lookup), Rust files it as other",
                {"reference": 500, "rust": 200, "rust other files": 5},
                {"reference": ref_api.status, "rust": rust_page.status,
                 "rust other files": other[0]["files"] if other else None})
    for db in (ref_db, rust_db):
        db.sql("DELETE FROM \"uploads\" WHERE \"id\" = '%s'" % upid(999))

    admin_data_checks(run, opt, ref_db, rust_db, signed)
    run.finish()


# ── lib/data/admin.ts behind the admin actions ─────────────────────────────

OLD_MTIME, OLD_MTIME_2, OLD_MTIME_3 = 1735787045.0, 1735787046.5, 1704067200.123456

# (key under the storage root, size, mtime in seconds or None for "just now")
STORAGE_FILES = [
    ("parity/pnl0001.mp4", 10, OLD_MTIME),  # a row's storage_key
    ("parity/pnl0002.mp4", 10, OLD_MTIME),
    ("thumbs/parity/pnl0003.webp", 20, OLD_MTIME),  # a row's thumb_key
    ("parity/stray-old.bin", 1234, OLD_MTIME),  # no row: an orphan
    ("parity/stray-new.bin", 99, None),  # no row, but inside the grace window
    ("derived/x/y.bin", 5, OLD_MTIME),  # untracked top-level trees are never walked
    ("backgrounds/a.webp", 6, OLD_MTIME),
    ("avatars/b.webp", 7, OLD_MTIME),
    ("nested/derived/z.bin", 8, OLD_MTIME),  # only TOP-LEVEL prefixes are skipped
    ("zz/a.bin", 9, OLD_MTIME_2),
    ("aa/b.bin", 11, OLD_MTIME_3),
]
STORAGE_LINKS = [("parity/link.bin", "stray-old.bin")]  # neither a file nor a directory

NODE_TREE = r"""
const fs = require("fs"), path = require("path");
const root = process.argv[1], spec = JSON.parse(process.argv[2]);
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(root, { recursive: true });
for (const [rel, size, mtime] of spec.files) {
  const p = path.join(root, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, Buffer.alloc(size, 120));
  if (mtime !== null) fs.utimesSync(p, mtime, mtime);
}
for (const [rel, target] of spec.links) fs.symlinkSync(target, path.join(root, rel));
"""


def write_storage_trees(opt):
    spec = json.dumps({"files": STORAGE_FILES, "links": STORAGE_LINKS})
    subprocess.run(["docker", "exec", opt.ref_app_container, "node", "-e", NODE_TREE, opt.ref_storage, spec],
                   check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    root = opt.rust_storage
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    for rel, size, mtime in STORAGE_FILES:
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(b"x" * size)
        if mtime is not None:
            os.utime(path, (mtime, mtime))
    for rel, target in STORAGE_LINKS:
        os.symlink(target, os.path.join(root, rel))


def rust_admin_data(opt, users):
    """tests/admin_data_parity.rs against the Rust parity database."""
    server = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    common = subprocess.run(["git", "-C", server, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                            stdout=subprocess.PIPE, check=True).stdout.decode().strip()
    devdb = parity.read_env_file(os.path.join(os.path.dirname(common), "server", "docker", ".env.devdb"))
    env = dict(os.environ)
    env["PATH"] = "/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:" + env.get("PATH", "")
    env["MODLIGHT_PARITY_DATABASE_URL"] = "postgresql://modlight:%s@127.0.0.1:55421/%s" % (
        devdb["POSTGRES_PASSWORD"], opt.rust_db)
    env["MODLIGHT_PARITY_STORAGE"] = opt.rust_storage
    env["MODLIGHT_PARITY_USERS"] = ",".join(users)
    out = subprocess.run(["cargo", "test", "--offline", "--quiet", "--test", "admin_data_parity", "--", "--ignored",
                          "--nocapture"], cwd=server, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    for line in out.stdout.decode("utf-8", "replace").splitlines():
        if line.startswith("PARITY-JSON "):
            return json.loads(line[len("PARITY-JSON "):])
    raise RuntimeError("tests/admin_data_parity.rs printed no result (exit %d):\n%s" % (
        out.returncode, out.stderr.decode("utf-8", "replace")[-2000:]))


def iso_ms(stamp):
    """Postgres keeps microseconds; a JavaScript Date (and JsDate) keeps milliseconds."""
    d = datetime.datetime.fromisoformat(stamp).astimezone(datetime.timezone.utc)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + "%03dZ" % (d.microsecond // 1000)


def admin_data_checks(run, opt, ref_db, rust_db, signed):
    # Counters that disagree with the rows, so the recalculation has work.
    skew = {ADMIN: 0, USER: 999, DECO: None, BARE: 0, BANNED: 100000000, OWN_A: 900000000, OWN_B: 5, OWN_C: 0}
    for db in (ref_db, rust_db):
        for user_id, used in skew.items():
            db.sql('UPDATE "user" SET "used_quota" = %s WHERE "id" = %s' % (parity.sql_quote(used),
                                                                            parity.sql_quote(user_id)))
    write_storage_trees(opt)

    pair = signed(ADMIN)
    actions = parity.Actions(run, opt.ref_app_container)

    def ref_call(export, *args):
        env = parity.next_action_envelope(actions.call_ref(pair.ref, "admin", export, list(args)))
        if "ok" not in env:
            raise AssertionError("admin.%s failed on the reference: %r" % (export, env))
        return env["ok"]

    impact_ids = [DECO, USER, OWN_C, BARE, "nobody00000000000000000000000000"]
    reference = {
        "orphans": ref_call("loadOrphans"),
        "impacts": [ref_call("loadDeletionImpact", i) for i in impact_ids],
        "recalc": ref_call("recalculateEveryQuota"),
        "scan": ref_call("runStorageScan"),
    }
    rust = rust_admin_data(opt, impact_ids)
    for key in ("orphans", "impacts", "recalc", "scan"):
        run.compare("lib/data/admin: %s (reference action vs Rust function)" % key, reference[key], rust[key])

    # What the reference itself must have said, from the seed alone.
    seeded = dict((u["code"], u) for u in uploads())
    run.compare("lib/data/admin: orphan summary and listing on the reference", {
        "summary": {"files": 3, "bytes": 500003000, "trashed": 1,
                    "oldest": iso_ms(seeded["pnl0013"]["created_at"]),
                    "newest": iso_ms(seeded["pnl0011"]["created_at"])},
        "codes": ["pnl0013", "pnl0012", "pnl0011"],
        "deletedAt": [None, iso_ms(seeded["pnl0012"]["deleted_at"]), None]},
        {"summary": reference["orphans"]["summary"], "codes": [r["code"] for r in reference["orphans"]["items"]],
         "deletedAt": [r["deletedAt"] for r in reference["orphans"]["items"]]})
    run.compare("lib/data/admin: deletion impact of the decorated account on the reference", {
        "id": DECO, "email": "pnl-deco@parity.test", "name": "Panel Deco", "tag": "pnldeco", "files": 21,
        "bytes": 30300, "trashed": 1, "folders": 8, "shortLinks": 0, "tokens": 0}, reference["impacts"][0])
    run.compare("lib/data/admin: unknown account has no impact", None, reference["impacts"][-1])
    run.compare("lib/data/admin: quota recalculation on the reference", {
        "accounts": 8, "corrected": 5,
        "drift": (2576984473 - 0) + (20200 - 999) + (30300 - 0) + (700000001 - 5) + (300000000 - 0)},
        dict((k, reference["recalc"][k]) for k in ("accounts", "corrected", "drift")))
    scan = reference["scan"]
    # Node's readdir hands names back sorted (libuv scandir + strcmp) and the
    # walk pops the LAST queued directory first, which fixes the sample order.
    run.compare("lib/data/admin: storage scan on the reference", {
        "supported": True, "driver": "local", "scanned": 8, "orphans": 4, "orphanBytes": 1262,
        "truncated": False, "skippedRecent": 1,
        "sample": [["zz/a.bin", 9, "2025-01-02T03:04:06.500Z"],
                   ["parity/stray-old.bin", 1234, "2025-01-02T03:04:05.000Z"],
                   ["nested/derived/z.bin", 8, "2025-01-02T03:04:05.000Z"],
                   ["aa/b.bin", 11, "2024-01-01T00:00:00.123Z"]]},
        dict([(k, scan[k]) for k in ("supported", "driver", "scanned", "orphans", "orphanBytes", "truncated",
                                     "skippedRecent")]
             + [("sample", [[x["key"], x["size"], x["modifiedAt"]] for x in scan["sample"]])]))

if __name__ == "__main__":
    main()

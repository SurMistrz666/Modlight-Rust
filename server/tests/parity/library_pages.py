"""Page parity of the library's panel pages (CONTRACT 6.11-6.17, 6.21, 6.24,
6.25) between the scratch reference and Rust.

The reference has no `/_page`, so its side is read from what it renders: the
props of the client components in the RSC payload inlined in the document
(`FileGrid`/`FileList`, `Toolbar`, `Pagination`, `EmptyState`,
`FolderBrowser`, `TrashList`, `DuplicatesScreen`, `LinkManager`,
`AdminFileToolbar`, `ThemeApplier`, `UnlockForm`), the `<title>`, and for
`redirect()`/`notFound()` inside the dashboard's loading boundary the meta
refresh / noindex the streamed document carries. The Rust side is `/_page`.

    PARITY_REF=http://localhost:8441 PARITY_RUST=http://localhost:8422 \
        python3 server/tests/parity/library_pages.py

Seeds its own users (random suffix); state-changing only for them (plus the
audit rows the admin files page writes).
"""

import json
import os
import re
import sys
import uuid
import html as htmllib
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402
import library  # noqa: E402
from library import Library, seed_upload, seed_folder, ORIGIN_RE  # noqa: E402


# ── The reference's RSC payload (as srv-auth's panel.py reads it) ──────────

def flight_rows(document):
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
        if j < n and stream[j] == "T":
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
        return found[0] if len(found) == 1 else ("<%d x %s>" % (len(found), name))


def title_of(document):
    m = re.search(r"<title>(.*?)</title>", document, re.S)
    return htmllib.unescape(m.group(1)) if m else None


def refresh_target(document):
    """`redirect()` inside the loading boundary: <meta http-equiv="refresh" content="1;url=...">."""
    m = re.search(r'<meta[^>]*http-equiv="refresh"[^>]*content="\d+;url=([^"]*)"', document)
    return htmllib.unescape(m.group(1)) if m else None


def is_error_document(document):
    """An uncaught error rendered by app/(app)/dashboard/error.tsx: a digest
    that is not one of Next's redirect/not-found control errors."""
    return bool(re.search(r'digest\\":\\"(?!NEXT_)', document))


def is_not_found_document(document):
    return bool(re.search(r'<meta name="robots" content="noindex"', document))


def undef(v):
    return None if v == "<undefined>" else v


# ── Checks ─────────────────────────────────────────────────────────────────

class Pages(object):
    def __init__(self, run, lib, pair):
        self.run = run
        self.lib = lib
        self.pair = pair
        self.sfx = lib.suffix

    def norm(self, v):
        text = json.dumps(v, ensure_ascii=False, sort_keys=True)
        text = re.sub(ORIGIN_RE, "<origin>", text)
        text = text.replace(self.sfx, "<sfx>")
        return parity.normalize_json(json.loads(text))

    def fetch(self, path, query=""):
        url = path + (("?" + query) if query else "")
        ref = self.pair.ref.request("GET", url, origin=False, headers={"Accept": "text/html"})
        q = "path=" + urllib.parse.quote(path, safe="") + (("&" + query) if query else "")
        rust = self.pair.rust.request("GET", "/_page?" + q, origin=False)
        return ref, rust.json() or {"status": rust.status, "text": rust.text()[:200]}

    def compare(self, name, a, b):
        self.run.compare("pages: " + name, self.norm(a), self.norm(b))

    def outcome(self, name, path, query=""):
        """Redirect / notFound / page, as both servers decide it."""
        ref, rust = self.fetch(path, query)
        doc = ref.text()
        if ref.status in (301, 302, 303, 307, 308):
            a = {"redirect": ref.header("location"), "status": ref.status}
        elif refresh_target(doc):
            a = {"redirect": refresh_target(doc), "status": 307}
        elif is_error_document(doc):
            a = {"error": True}
        elif ref.status == 404 or is_not_found_document(doc):
            a = {"notFound": True}
        else:
            a = {"page": ref.status}
        if "redirect" in rust:
            b = {"redirect": rust["redirect"], "status": rust.get("status")}
        elif rust.get("notFound"):
            b = {"notFound": True}
        elif "props" in rust:
            b = {"page": 200}
        elif "error" in rust:
            b = {"error": True}
        else:
            b = rust
        self.compare(name + " (outcome)", a, b)
        return doc, rust

    def title(self, name, doc, rust):
        self.compare(name + " (title)", title_of(doc), (rust.get("meta") or {}).get("title"))


def ref_app_layout(flight):
    """The (app) layout's data as the reference rendered it (srv-auth's panel.py)."""
    applier = flight.one("ThemeApplier")
    transfers = flight.one("TransfersProvider")
    shell = flight.one("AppShell")
    reminders = flight.elements("DuplicateReminder")
    return {
        "theme": applier.get("theme") if isinstance(applier, dict) else applier,
        "background": applier.get("background") if isinstance(applier, dict) else None,
        "transferFolders": transfers.get("folders") if isinstance(transfers, dict) else transfers,
        "defaultFolderId": transfers.get("defaultFolderId") if isinstance(transfers, dict) else None,
        "user": shell.get("user") if isinstance(shell, dict) else shell,
        "sidebar": shell.get("sidebar") if isinstance(shell, dict) else None,
        "defaultCollapsed": shell.get("defaultCollapsed") if isinstance(shell, dict) else None,
        "duplicateReminder": reminders[0] if reminders else None,
    }


def app_layout(pages, name, doc, rust):
    """The panel shell around a library page: folder tree, kind counts, stats,
    wallpaper, avatar and the duplicate reminder, all from the library's data
    functions."""
    pages.compare(name + " ((app) layout)", ref_app_layout(Flight(doc)), (rust.get("layouts") or {}).get("(app)"))


def dashboard(pages, owner_id, tag, query, name):
    doc, rust = pages.outcome("dashboard " + name, "/dashboard", query)
    if "props" not in rust:
        return
    f = Flight(doc)
    p = rust["props"]
    pages.title("dashboard " + name, doc, rust)
    if name in ("all", "folder with sub"):
        app_layout(pages, "dashboard " + name, doc, rust)
    toolbar = f.one("Toolbar")
    pages.compare("dashboard %s (toolbar)" % name, toolbar, {
        "search": p.get("search") or "", "sort": p["sort"], "view": p["view"], "kind": p.get("kind") or "",
        "visibility": p.get("visibility") or "", "size": p.get("rawSize") or "", "after": p.get("rawAfter") or "",
        "before": p.get("rawBefore") or "", "counts": p["counts"], "folderActive": bool(p.get("folderId")),
        "includeSubfolders": p["includeSubfolders"]})
    grid = f.elements("FileList" if p["view"] == "list" else "FileGrid")
    if p["items"]:
        pages.compare("dashboard %s (files)" % name, grid[0] if grid else None, {
            "items": p["items"], "userCode": p["userCode"], "appUrl": p["linkBase"], "copyRaw": p["copyRaw"],
            "currentFolderId": p.get("folderId")})
        pages.compare("dashboard %s (pagination)" % name, f.one("Pagination"), p["result"])
    else:
        # EmptyState is a server component (no props in the payload): the
        # reference must simply have drawn no file list either.
        pages.compare("dashboard %s (no files drawn)" % name, len(f.elements("FileGrid") + f.elements("FileList")), 0)
    if p.get("currentFolder"):
        h1 = re.search(r"<h1[^>]*>(.*?)</h1>", doc, re.S)
        pages.compare("dashboard %s (heading)" % name, htmllib.unescape(h1.group(1)) if h1 else None,
                      p["currentFolder"]["name"])


def main():
    run = parity.Run.from_args()
    import argparse
    ap = argparse.ArgumentParser(allow_abbrev=False)
    ap.add_argument("--ref-db", default="modlight-srvlib-ref-postgres-1:modlight")
    ap.add_argument("--rust-db", default="modlight-devdb-postgres-1:modlight_srv_library")
    ap.add_argument("--ref-env", default="_scratch/srv-library/ref/.env")
    ap.add_argument("--rust-env", default="_scratch/srv-library/rust.env")
    a, _ = ap.parse_known_args()
    rc, rd = a.ref_db.split(":")
    uc, ud = a.rust_db.split(":")
    ref_db = parity.Db(rc, rd, parity.read_env_file(a.ref_env)["AUTH_SECRET"])
    rust_db = parity.Db(uc, ud, parity.read_env_file(a.rust_env)["AUTH_SECRET"])
    lib = Library(run, ref_db, rust_db)
    sfx = lib.suffix

    owner, tag = lib.seed_user("pg", copy_raw=True)
    flat, flat_tag = lib.seed_user("pf")
    admin, admin_tag = lib.seed_user("pa", role="admin")
    pair = run.pair()
    pages = Pages(run, lib, pair)

    # Folders: a tree, a public one, one unreachable (a cycle).
    fa = seed_folder(lib, owner, "Alpha")
    fb = seed_folder(lib, owner, "Beta", parent=fa)
    fc = seed_folder(lib, owner, "Gamma", parent=fb)
    fd = seed_folder(lib, owner, "Delta")
    lib.both_sql("UPDATE folders SET is_public = true WHERE id = %s" % parity.sql_quote(fd))
    cy1 = seed_folder(lib, owner, "Cy1")
    cy2 = seed_folder(lib, owner, "Cy2", parent=cy1)
    lib.both_sql("UPDATE folders SET parent_id = %s WHERE id = %s" % (parity.sql_quote(cy2), parity.sql_quote(cy1)))
    t = lambda n: "2026-03-%02dT10:00:00.000Z" % n
    c = lambda k: ("pg%s%s" % (k, sfx))[:40]
    seed_upload(lib, owner, c("a"), created_at=t(1), folder_id=fa, size=100, checksum="d" * 64)
    seed_upload(lib, owner, c("b"), created_at=t(2), folder_id=fb, size=200, checksum="d" * 64)
    seed_upload(lib, owner, c("c"), created_at=t(3), folder_id=fc, size=300, filename="clip.mp4", ext="mp4",
                mime_type="video/mp4")
    seed_upload(lib, owner, c("d"), created_at=t(4), size=400, filename="Notes.TXT", ext="txt",
                mime_type="text/plain", published=False, expires_at="2031-01-01T00:00:00.000Z", max_views=3,
                password_hash="x:y")
    seed_upload(lib, owner, c("e"), created_at=t(5), size=50 * 1024 * 1024, filename="big.zip", ext="zip",
                mime_type="application/zip", checksum="d" * 64)
    seed_upload(lib, owner, c("f"), created_at=t(6), size=10, filename="pic.avif", ext="avif",
                mime_type="application/octet-stream", folder_id=fd)
    # Recent enough that neither server's purge job takes them mid-run.
    seed_upload(lib, owner, c("g"), created_at=t(7), size=10, deleted_at=library.days_ago(2))
    seed_upload(lib, owner, c("h"), created_at=t(8), size=20, deleted_at=library.days_ago(29, 3600),
                folder_id=fa)
    seed_upload(lib, owner, c("i"), created_at=t(9), size=5, kind="paste", language="python",
                filename="snippet.py", ext="py", mime_type="text/plain")
    for n in range(10, 45):
        seed_upload(lib, flat, "pfl%d%s" % (n, sfx), created_at="2026-04-01T00:00:%02d.000Z" % (n % 60), size=n)
    library.seed_short_link(lib, owner, "pgs" + sfx, "https://example.org/a", clicks=3,
                            last_used_at="2026-03-01T00:00:00.000Z")
    library.seed_short_link(lib, owner, "%s/pgt%s" % (tag, sfx), "https://example.org/b", password_hash="x:y",
                            expires_at="2026-01-01T00:00:00.000Z")

    parity.signed_in(pair, ref_db, rust_db, owner)

    # /dashboard
    pages.outcome("dashboard bare (has folders)", "/dashboard")
    for name, q in [("all", "all=1"), ("list view, oldest", "view=list&sort=oldest"),
                    ("page 2 of 3", "per=3&page=2"), ("odd per", "per=6e1&page=abc"),
                    ("search", "search=%20notes%20"), ("kind video", "kind=video"), ("kind bogus", "kind=bogus"),
                    ("visibility private", "visibility=private"), ("size 10mb", "size=10mb"),
                    ("size unknown", "size=2mb"), ("after/before", "after=2026-03-03&before=2026-03-05"),
                    ("bad dates", "after=nonsense&before=2026-02-30"), ("iso after", "after=2026-03-04T12:00:00Z"),
                    ("folder none", "folder=none"), ("folder", "folder=" + fa), ("folder with sub", "folder=%s&sub=1" % fa),
                    ("public folder", "folder=" + fd), ("unknown folder", "folder=" + str(uuid.uuid4())),
                    ("empty folder value", "folder="), ("filtered empty", "search=zzzz-nothing"),
                    ("empty folder", "folder=%s&kind=audio" % fc)]:
        dashboard(pages, owner, tag, q, name)
    pages.outcome("dashboard non-uuid folder (error)", "/dashboard", "folder=abc")

    # The shell of a fuller account: wallpaper, avatar, default folder, a quota
    # and a duplicate reminder that is due (more than 10 redundant copies).
    rich, rich_tag = lib.seed_user("pr", max_quota=5000000, background_dim=33, theme="midnight",
                                   image="avatars/%s-0123456789abcdef.webp" % ("libpr" + sfx),
                                   background_key="backgrounds/x-fedcba9876543210.webp")
    rf = seed_folder(lib, rich, "Inbox")
    lib.both_sql('UPDATE "user" SET default_folder_id = %s WHERE id = %s' % (parity.sql_quote(rf), parity.sql_quote(rich)))
    for n in range(12):
        seed_upload(lib, rich, "prd%d%s" % (n, sfx), size=1000 + n, checksum="e" * 64,
                    created_at="2026-05-01T00:00:%02d.000Z" % n, folder_id=rf if n % 2 else None)
    rpair = run.pair()
    parity.signed_in(rpair, ref_db, rust_db, rich)
    rpages = Pages(run, lib, rpair)
    doc, rust = rpages.outcome("rich account folders", "/dashboard/folders")
    if "props" in rust:
        app_layout(rpages, "rich account folders", doc, rust)
        shell = rust["layouts"]["(app)"]
        run.compare("pages: rich account (the seed reaches every branch)",
                    [True, True, True, True],
                    [shell["duplicateReminder"] is not None, shell["background"] is not None,
                     shell["user"]["image"] is not None, shell["defaultFolderId"] is not None])
    rpair.ref.cookies["ml_dupes_snooze"] = rpair.rust.cookies["ml_dupes_snooze"] = str(10 ** 13)
    rpair.ref.cookies["ml_sidebar"] = rpair.rust.cookies["ml_sidebar"] = "collapsed"
    doc, rust = rpages.outcome("rich account, snoozed and collapsed", "/dashboard/folders")
    if "props" in rust:
        app_layout(rpages, "rich account, snoozed and collapsed", doc, rust)

    # /dashboard/folders
    for name, q in [("root", ""), ("alpha", "folder=" + fa), ("gamma", "folder=" + fc),
                    ("unknown", "folder=" + str(uuid.uuid4())), ("cycle", "folder=" + cy1), ("non-uuid", "folder=x"),
                    ("upper-case id", "folder=" + fa.upper())]:
        doc, rust = pages.outcome("folders " + name, "/dashboard/folders", q)
        if "props" in rust:
            pages.title("folders " + name, doc, rust)
            p = rust["props"]
            cur = p["current"]
            pages.compare("folders %s (browser)" % name, Flight(doc).one("FolderBrowser"), {
                "folders": p["children"], "currentFolderId": cur["id"] if cur else None,
                "currentFolderName": cur["name"] if cur else None,
                "filesHere": {"count": cur["count"], "size": cur["size"]} if cur else None,
                "unsorted": p["unsorted"], "userCode": p["userCode"]})

    # /dashboard/trash
    doc, rust = pages.outcome("trash", "/dashboard/trash")
    if "props" in rust:
        p = rust["props"]
        pages.title("trash", doc, rust)
        pages.compare("trash (list)", Flight(doc).one("TrashList"),
                      {"items": p["items"], "userCode": p["tag"], "appUrl": p["linkBase"], "retentionDays": 30})
        pages.compare("trash (totals)", p["totals"], {"files": 2, "bytes": 30})

    # /dashboard/duplicates
    doc, rust = pages.outcome("duplicates", "/dashboard/duplicates")
    if "props" in rust:
        pages.title("duplicates", doc, rust)
        p = rust["props"]
        scan_ref = Flight(doc).one("DuplicatesScreen")
        for s in (scan_ref.get("scan") if isinstance(scan_ref, dict) else None, p["scan"]):
            if isinstance(s, dict):
                s["scannedAt"] = "<now>"
        pages.compare("duplicates (screen)", scan_ref, {"scan": p["scan"], "userCode": p["userCode"]})

    # /dashboard/links
    doc, rust = pages.outcome("links", "/dashboard/links")
    if "props" in rust:
        pages.title("links", doc, rust)
        p = rust["props"]
        lm = Flight(doc).one("LinkManager")
        if isinstance(lm, dict):
            lm.pop("appUrl", None)
            lm.pop("shortBase", None)
        pages.compare("links (manager)", lm, p)

    # /dashboard/paste and /dashboard/upload: metadata only.
    for path in ("/dashboard/paste", "/dashboard/upload"):
        doc, rust = pages.outcome(path, path)
        pages.title(path, doc, rust)
        d = re.search(r'<meta name="description" content="([^"]*)"', doc)
        pages.compare(path + " (description)", htmllib.unescape(d.group(1)) if d else None,
                      (rust.get("meta") or {}).get("description"))
        pages.compare(path + " (props)", {}, rust.get("props"))

    # /dashboard/admin/users/[id]/files as a non-admin, then as the admin.
    pages.outcome("admin files as a user", "/dashboard/admin/users/%s/files" % flat)
    parity.signed_in(pair, ref_db, rust_db, admin)
    before = lib.rows('SELECT count(*) FROM audit_log WHERE user_id = %s' % parity.sql_quote(admin))
    for name, target, q in [("flat", flat, ""), ("flat list page 2", flat, "view=list&per=30&page=2"),
                            ("owner trash", owner, "trashed=1"), ("owner search", owner, "search=notes&sort=name"),
                            ("owner kind", owner, "kind=document"), ("unknown", "nobody" + sfx, ""),
                            ("flat again (dedup)", flat, "")]:
        doc, rust = pages.outcome("admin files " + name, "/dashboard/admin/users/%s/files" % target, q)
        if "props" not in rust:
            continue
        f = Flight(doc)
        p = rust["props"]
        d = p["data"]
        pages.title("admin files " + name, doc, rust)
        tb = f.one("AdminFileToolbar")
        pages.compare("admin files %s (toolbar)" % name, tb, {
            "search": p.get("search") or "", "sort": p["sort"], "kind": p.get("kind") or "", "view": p["view"],
            "trashed": p["trashed"]} if not isinstance(tb, dict) else dict(
            (k, v) for k, v in {"search": p.get("search") or "", "sort": p["sort"], "kind": p.get("kind") or "",
                                "view": p["view"], "trashed": p["trashed"]}.items() if k in tb))
        if p["trashed"]:
            tl = f.one("TrashList")
            pages.compare("admin files %s (trash)" % name, tl if not isinstance(tl, dict) else tl.get("items"),
                          d["trashItems"])
        elif d["items"]:
            grid = f.elements("FileList" if p["view"] == "list" else "FileGrid")
            pages.compare("admin files %s (files)" % name, grid[0].get("items") if grid else None, d["items"])
        if d["items"]:
            pages.compare("admin files %s (pagination)" % name, f.one("Pagination"),
                          {"total": d["total"], "page": d["page"], "perPage": d["perPage"]})
    a, b = lib.rows("SELECT action, detail FROM audit_log WHERE user_id = %s ORDER BY created_at"
                    % parity.sql_quote(admin))
    run.compare("pages: admin files (audit rows written)", a.splitlines(), b.splitlines())

    # /s/[code]/unlock and /s/[code]/[name]/unlock (no session needed).
    anon = run.pair()
    apages = Pages(run, lib, anon)
    lib.both_sql("UPDATE \"user\" SET theme = 'midnight', background_key = %s, background_dim = 40 WHERE id = %s"
                 % (parity.sql_quote("backgrounds/%s-abcdef0123456789.webp" % owner), parity.sql_quote(owner)))
    for name, path in [("protected", "/s/%s/pgt%s/unlock" % (tag, sfx)), ("not protected", "/s/pgs%s/unlock" % sfx),
                       ("unknown", "/s/nope%s/unlock" % sfx), ("unknown scoped", "/s/%s/nope/unlock" % tag)]:
        doc, rust = apages.outcome("unlock " + name, path)
        if "props" in rust:
            apages.title("unlock " + name, doc, rust)
            f = Flight(doc)
            p = rust["props"]
            apages.compare("unlock %s (theme)" % name, f.one("ThemeApplier"),
                           {"theme": p["ownerTheme"], "background": p["background"]})
            apages.compare("unlock %s (form)" % name, f.one("UnlockForm"), {"code": p["code"]})
    run.finish()


if __name__ == "__main__":
    main()

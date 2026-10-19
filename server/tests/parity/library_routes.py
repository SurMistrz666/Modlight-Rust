"""Library route parity: POST /api/uploads/check, /api/folders, /api/duplicates,
/api/shortlinks, the /s/{code} and /s/{tag}/{code} redirects and
GET /api/admin/disk, against a scratch reference stack.

    cd /Users/marcinasdfg/Desktop/modlight && PARITY_REF=http://localhost:8441 PARITY_RUST=http://localhost:8443 \\
        python3 _wt/srv-library-routes/server/tests/parity/library_routes.py [--only folders] [-v]

Database and secret arguments as in library.py (`--ref-db`, `--rust-db`,
`--ref-env`, `--rust-env`; the defaults are the srv-library stacks). Both
servers: NODE_ENV=production, APP_URL = their own origin; Rust with
TRUST_PROXY=true. Every area seeds its own users (random suffix), sends the
same requests to both servers and compares status, the headers that matter
and the normalised body, then the rows each server wrote.

The admin disk report is GLOBAL (every upload in the database), and the two
databases hold different libraries. That area seeds accounts and files large
enough to own the top of every list, compares those parts across the
servers, checks every global figure against SQL run on the server's OWN
database, and deletes what it seeded at the end.

State-changing: never point it at the shared reference on 8420.
"""

import argparse
import base64
import hashlib
import hmac
import json
import os
import re
import sys
import time
import urllib.parse
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402
from library import Library, ORIGIN_RE, seed_folder, seed_upload  # noqa: E402

Q = parity.sql_quote


def sha(n):
    """A syntactically valid sha256, distinct per number (tests/unit's `sum`)."""
    return format(n, "x").rjust(64, "0")


class Area(object):
    """One pair of browsers plus the label table that maps seeded and created
    ids to stable names, so bodies and rows compare across the databases."""

    def __init__(self, run, lib, prefix):
        self.run = run
        self.lib = lib
        self.prefix = prefix
        self.pair = run.pair()
        self.names = {}

    def name(self, value, label):
        self.names[value] = label
        return value

    def label(self, v, extra=()):
        text = json.dumps(v, ensure_ascii=False)
        # Longest first, so an id never matches inside a longer one.
        for uid in sorted(self.names, key=len, reverse=True):
            text = text.replace(uid, self.names[uid])
        text = re.sub(ORIGIN_RE, "<origin>", text)
        text = text.replace(self.lib.suffix, "<sfx>")
        for pattern, repl in extra:
            text = re.sub(pattern, repl, text)
        return parity.normalize_json(json.loads(text))

    def shape(self, r, extra=(), headers=("content-type", "cache-control", "retry-after", "location", "allow")):
        out = {"status": r.status}
        for h in headers:
            v = r.header(h)
            if v is not None and h == "location":
                v = self.label(v, extra)
            if v is not None and h == "retry-after":
                # Both servers count the same window; allow the second the
                # two requests were apart in.
                v = "<1-60>" if v.isdigit() and 0 < int(v) <= 60 else v
            out[h] = v
        body = r.json()
        out["body"] = self.label(body, extra) if body is not None else self.label(r.text(), extra)
        return out

    def call(self, name, method, path, token=None, body=None, raw=None, ctype=None, headers=None, extra=(),
             per_side=None, compare_body=True, internal_error=False):
        """The same request on both servers. `per_side(side)` may return
        overrides ({"body": ..., "path": ...}) for 'ref' and 'rust'."""
        out = []
        for side, br in (("ref", self.pair.ref), ("rust", self.pair.rust)):
            o = per_side(side) if per_side else {}
            h = dict(headers or {})
            if token:
                h["x-api-key"] = token
            if ctype:
                h["Content-Type"] = ctype
            kw = {"headers": h, "origin": False}
            b = o.get("body", body)
            if b is not None:
                kw["json_body"] = b
                if ctype:
                    kw["raw"] = json.dumps(b)
                    del kw["json_body"]
            elif raw is not None:
                kw["raw"] = raw
            out.append(br.request(method, o.get("path", path), **kw))
        shapes = [self.shape(r, extra) for r in out]
        if internal_error:
            # A thrown error: Next answers an empty 500, the port's
            # AppError::Internal a JSON `{"message":"Internal server error"}`
            # (crate-wide convention, SKELETON section 3). Only the status
            # is compared.
            shapes = [{"status": sh["status"]} for sh in shapes]
        if not compare_body:
            for sh in shapes:
                sh["body"] = "<not compared>"
        self.run.compare(self.prefix + ": " + name, shapes[0], shapes[1])
        return out

    def rows(self, name, sql, extra=(), sort=False):
        a, b = self.lib.both_sql(sql)
        la = self.label(a, extra).splitlines()
        lb = self.label(b, extra).splitlines()
        if sort:
            la, lb = sorted(la), sorted(lb)
        self.run.compare(self.prefix + " rows: " + name, la, lb)

    def signed_in(self, user_id):
        parity.signed_in(self.pair, self.lib.ref_db, self.lib.rust_db, user_id)

    def signed_out(self):
        self.pair.ref.cookies.pop(parity.SESSION_COOKIE, None)
        self.pair.rust.cookies.pop(parity.SESSION_COOKIE, None)


def common_refusals(a, method, path, full, reader, uploader, body, write):
    """Credential refusals every token surface shares."""
    a.call(method + " without auth", method, path, body=body)
    a.call(method + " unknown token", method, path, token="nope", body=body)
    a.call(method + " bearer unknown", method, path, headers={"Authorization": "Bearer nope"}, body=body)
    a.call(method + " empty x-api-key", method, path, headers={"x-api-key": ""}, body=body)
    if write:
        a.call(method + " read-scoped token", method, path, token=reader, body=body)
    else:
        a.call(method + " upload-scoped token", method, path, token=uploader, body=body)
    if body is not None:
        a.call(method + " text/plain without auth", method, path, raw=b"not json", ctype="text/plain")
        a.call(method + " text/plain", method, path, token=full, raw=json.dumps(body).encode(), ctype="text/plain")
        a.call(method + " no content type", method, path, token=full, raw=json.dumps(body).encode())
        a.call(method + " array body", method, path, token=full, raw=b"[1]", ctype="application/json")
        a.call(method + " broken json", method, path, token=full, raw=b'{"x":', ctype="application/json")
        a.call(method + " json null", method, path, token=full, raw=b"null", ctype="application/json")
        a.call(method + " charset content type", method, path, token=full, raw=b"[]",
               ctype="Application/JSON; charset=utf-8")


# ── POST /api/uploads/check ────────────────────────────────────────────────


def uploads_check(run, lib):
    a = Area(run, lib, "uploads check")
    owner, tag = lib.seed_user("ca")
    full = lib.seed_token(owner, "full")
    reader = lib.seed_token(owner, "read")
    uploader = lib.seed_token(owner, "upload")
    other, _ = lib.seed_user("cb", copy_raw=True, skip_tag_in_url=True)
    other_read = lib.seed_token(other, "read")
    sfx = lib.suffix
    folder = a.name(seed_folder(lib, owner, "Screens"), "<folder>")
    foreign_folder = a.name(seed_folder(lib, other, "Theirs"), "<foreign-folder>")
    mine, theirs, trashed, absent = sha(1), sha(2), sha(3), sha(4)
    t = lambda n: "2026-01-01T00:00:%02d.000Z" % n
    a.name(seed_upload(lib, owner, "ka" + sfx, checksum=mine, folder_id=folder, created_at=t(2), filename="a.png"),
           "<mine-1>")
    a.name(seed_upload(lib, owner, "kb" + sfx, checksum=mine, created_at=t(1), filename="older copy.png",
                       size=777), "<mine-0>")
    a.name(seed_upload(lib, other, "kc" + sfx, checksum=theirs, folder_id=foreign_folder, created_at=t(3)),
           "<theirs>")
    a.name(seed_upload(lib, owner, "kd" + sfx, checksum=trashed, created_at=t(4),
                       deleted_at="2026-02-01T00:00:00.000Z"), "<trashed>")
    # Another account's row pointing at MY folder: the name lookup is scoped.
    a.name(seed_upload(lib, other, "ke" + sfx, checksum=mine, folder_id=folder, created_at=t(5)), "<theirs-in-mine>")
    path = "/api/uploads/check"

    common_refusals(a, "POST", path, full, reader, uploader, {"checksums": [mine]}, write=False)
    a.call("read token, duplicate", "POST", path, token=reader, body={"checksums": [mine]})
    a.call("full token", "POST", path, token=full, body={"checksums": [mine]})
    a.call("another account's checksum", "POST", path, token=full, body={"checksums": [theirs]})
    a.call("a trashed file is not a duplicate", "POST", path, token=full, body={"checksums": [trashed]})
    a.call("order kept", "POST", path, token=full, body={"checksums": [absent, mine, theirs]})
    a.call("repeat collapsed, case folded", "POST", path, token=full, body={"checksums": [mine, mine.upper()]})
    a.call("trimmed", "POST", path, token=full, body={"checksums": ["  %s\n" % mine]})
    a.call("no checksums key", "POST", path, token=full, body={})
    a.call("checksums a string", "POST", path, token=full, body={"checksums": "abc"})
    a.call("checksums empty", "POST", path, token=full, body={"checksums": []})
    a.call("checksums null", "POST", path, token=full, body={"checksums": None})
    a.call("malformed entry", "POST", path, token=full, body={"checksums": [mine, "not-a-checksum"]})
    a.call("number entry", "POST", path, token=full, body={"checksums": [1]})
    a.call("63 characters", "POST", path, token=full, body={"checksums": [mine[:63]]})
    a.call("non-hex", "POST", path, token=full, body={"checksums": ["z" * 64]})
    a.call("lone surrogate escape", "POST", path, token=full, raw=b'{"checksums":["\\ud800"]}',
           ctype="application/json")
    a.call("too many", "POST", path, token=full, body={"checksums": [sha(i + 10) for i in range(501)]})
    a.call("a full batch", "POST", path, token=full, body={"checksums": [sha(i + 10) for i in range(500)]})
    a.call("oversized body", "POST", path, token=full, body={"checksums": [mine], "pad": "x" * (64 * 1024)})
    a.call("as the other account (copy_raw, tagless)", "POST", path, token=other_read,
           body={"checksums": [mine, theirs]})
    a.call("GET is not a method", "GET", path, token=full)
    a.call("OPTIONS", "OPTIONS", path)

    # A checksumless live file makes the library partly blind: `unknown`.
    a.name(seed_upload(lib, owner, "kf" + sfx, created_at=t(6)), "<unchecksummed>")
    a.call("blind library, absent", "POST", path, token=full, body={"checksums": [absent, mine]})
    lib.both_sql("UPDATE uploads SET checksum = '' WHERE code = %s" % Q("kf" + sfx))
    a.call("empty-string checksum is blind too", "POST", path, token=full, body={"checksums": [absent]})

    a.signed_in(owner)
    a.call("with a session", "POST", path, body={"checksums": [mine]})
    a.call("with a session and a bad token", "POST", path, token="nope", body={"checksums": [mine]})
    lib.both_sql('UPDATE "user" SET banned = true WHERE id = %s' % Q(owner))
    a.call("banned session", "POST", path, body={"checksums": [mine]})
    a.call("banned account's token", "POST", path, token=full, body={"checksums": [mine]})
    lib.both_sql('UPDATE "user" SET banned = false WHERE id = %s' % Q(owner))
    a.signed_out()


# ── /api/folders ───────────────────────────────────────────────────────────


def folders_api(run, lib):
    a = Area(run, lib, "folders")
    owner, tag = lib.seed_user("fa")
    full = lib.seed_token(owner, "full")
    reader = lib.seed_token(owner, "read")
    uploader = lib.seed_token(owner, "upload")
    writer = lib.seed_token(owner, "write")
    other, _ = lib.seed_user("fb")
    other_full = lib.seed_token(other, "full")
    sfx = lib.suffix
    mine = a.name(seed_folder(lib, owner, "Mine"), "<mine>")
    theirs = a.name(seed_folder(lib, other, "Theirs"), "<theirs>")
    inner = a.name(seed_folder(lib, owner, "Inner", parent=mine), "<inner>")
    t = lambda n: "2026-01-01T00:00:%02d.000Z" % n
    files = {}
    files["in_mine"] = a.name(seed_upload(lib, owner, "fa" + sfx, folder_id=mine, size=100, created_at=t(1)),
                              "<file-in-mine>")
    files["in_mine2"] = a.name(seed_upload(lib, owner, "fb" + sfx, folder_id=mine, size=23, created_at=t(2)),
                               "<file-in-mine-2>")
    files["in_inner"] = a.name(seed_upload(lib, owner, "fc" + sfx, folder_id=inner, size=5, created_at=t(3)),
                               "<file-in-inner>")
    files["trash_mine"] = a.name(seed_upload(lib, owner, "fd" + sfx, folder_id=mine, size=999, created_at=t(4),
                                             deleted_at="2026-02-01T00:00:00.000Z"), "<trashed-in-mine>")
    files["loose"] = a.name(seed_upload(lib, owner, "fe" + sfx, size=7, created_at=t(5)), "<loose>")
    files["loose_trash"] = a.name(seed_upload(lib, owner, "ff" + sfx, size=70, created_at=t(6),
                                              deleted_at="2026-02-01T00:00:00.000Z"), "<loose-trashed>")
    path = "/api/folders"

    def created(responses, label):
        """Record the folder id each server created under one label."""
        ids = {}
        for side, res in zip(("ref", "rust"), responses):
            body = res.json() or {}
            fid = (body.get("folder") or {}).get("id")
            if fid:
                a.name(fid, label)
            ids[side] = fid
        return ids

    def folder_rows(name):
        a.rows(name, 'SELECT f.id, f.name, f.slug, f.parent_id, f.is_public, f.sort_order, f.description, '
                     'f.cover_upload_id FROM folders f WHERE f.owner_id IN (%s, %s) ORDER BY f.name, f.slug'
               % (Q(owner), Q(other)))

    def upload_rows(name):
        a.rows(name, 'SELECT code, folder_id, deleted_at IS NULL FROM uploads WHERE owner_id = %s ORDER BY code'
               % Q(owner))

    # GET
    common_refusals(a, "GET", path, full, reader, uploader, None, write=False)
    a.call("GET read token", "GET", path, token=reader)
    a.call("GET as the other account", "GET", path, token=other_full)
    a.call("PUT is not a method", "PUT", path, token=full, body={})
    a.call("OPTIONS", "OPTIONS", path)
    a.call("HEAD", "HEAD", path, token=full)

    # Writes: the shared refusals, for each verb.
    for verb in ("POST", "PATCH", "DELETE"):
        common_refusals(a, verb, path, full, reader, uploader, {"id": mine, "name": "x"}, write=True)
    a.call("POST upload-scoped token", "POST", path, token=uploader, body={"name": "x"})

    # POST
    a.call("POST no name", "POST", path, token=full, body={})
    a.call("POST name number", "POST", path, token=full, body={"name": 7})
    a.call("POST name null", "POST", path, token=full, body={"name": None})
    a.call("POST parentId number", "POST", path, token=full, body={"name": "x", "parentId": 7})
    a.call("POST parentId false", "POST", path, token=full, body={"name": "x", "parentId": False})
    a.call("POST foreign parent", "POST", path, token=full, body={"name": "Nope", "parentId": theirs})
    a.call("POST malformed parent", "POST", path, token=full, body={"name": "x", "parentId": "nope"})
    a.call("POST unknown parent", "POST", path, token=full, body={"name": "x", "parentId": str(uuid.uuid4())})
    a.call("POST uppercase parent", "POST", path, token=full, body={"name": "Upper", "parentId": mine.upper()})
    folder_rows("after POST refusals")
    created(a.call("POST create", "POST", path, token=full, body={"name": "Holiday Photos " + sfx}), "<holiday>")
    created(a.call("POST same name again", "POST", path, token=full, body={"name": "Holiday Photos " + sfx}),
            "<holiday-2>")
    created(a.call("POST blank name falls back", "POST", path, token=full, body={"name": "   "}), "<blank>")
    created(a.call("POST unicode name", "POST", path, token=full, body={"name": "Zdjęcia ąę ŁÓDŹ"}), "<unicode>")
    created(a.call("POST long name", "POST", path, token=full, body={"name": "N" * 200 + "ś"}), "<long>")
    created(a.call("POST emoji cut at the name ceiling", "POST", path, token=full,
                   body={"name": "E" * 119 + "\U0001F600"}), "<emoji>")
    created(a.call("POST lone surrogate escape", "POST", path, token=full, raw=b'{"name":"ab\\ud800cd\\udc00"}',
                   ctype="application/json"), "<surrogate>")
    created(a.call("POST root sentinel", "POST", path, token=full, body={"name": "Top", "parentId": "root"}),
            "<top>")
    created(a.call("POST empty parent", "POST", path, token=full, body={"name": "Top2", "parentId": ""}), "<top2>")
    created(a.call("POST null parent", "POST", path, token=full, body={"name": "Top3", "parentId": None}), "<top3>")
    child = created(a.call("POST nested", "POST", path, token=writer, body={"name": "Child", "parentId": mine}),
                    "<child>")
    folder_rows("after creates")

    # The depth ceiling: a chain of 8 under the root, then a 9th.
    chain = {"ref": None, "rust": None}
    for i in range(8):
        res = a.call("POST level %d" % (i + 1), "POST", path, token=full, body=None,
                     per_side=lambda side: {"body": {"name": "Level %d" % (i + 1), "parentId": chain[side]}})
        ids = created(res, "<level-%d>" % (i + 1))
        chain = ids
    a.call("POST too deep", "POST", path, token=full,
           per_side=lambda side: {"body": {"name": "Too deep", "parentId": chain[side]}})

    # PATCH
    a.call("PATCH no id", "PATCH", path, token=full, body={"name": "x"})
    a.call("PATCH empty id", "PATCH", path, token=full, body={"id": "", "name": "x"})
    a.call("PATCH id number", "PATCH", path, token=full, body={"id": 5, "name": "x"})
    a.call("PATCH nothing to change", "PATCH", path, token=full, body={"id": mine})
    a.call("PATCH unknown keys only", "PATCH", path, token=full, body={"id": mine, "slug": "x"})
    a.call("PATCH name number", "PATCH", path, token=full, body={"id": mine, "name": 7})
    a.call("PATCH name null", "PATCH", path, token=full, body={"id": mine, "name": None})
    a.call("PATCH isPublic string", "PATCH", path, token=full, body={"id": mine, "isPublic": "yes"})
    a.call("PATCH parentId number", "PATCH", path, token=full, body={"id": mine, "parentId": 7})
    a.call("PATCH foreign folder", "PATCH", path, token=full, body={"id": theirs, "name": "Hijacked"})
    a.call("PATCH malformed id with bad field", "PATCH", path, token=full, body={"id": "nope", "name": "   "})
    a.call("PATCH blank name", "PATCH", path, token=full, body={"id": mine, "name": "   "})
    a.call("PATCH rename", "PATCH", path, token=full, body={"id": mine, "name": "Renamed " + sfx})
    a.call("PATCH rename to the same name", "PATCH", path, token=full, body={"id": mine, "name": "Renamed " + sfx})
    a.call("PATCH publish", "PATCH", path, token=full, body={"id": mine, "isPublic": True})
    a.call("PATCH unpublish (uppercase id)", "PATCH", path, token=full, body={"id": mine.upper(), "isPublic": False})
    a.call("PATCH move child to root", "PATCH", path, token=full,
           per_side=lambda side: {"body": {"id": child[side], "parentId": None}})
    a.call("PATCH move child back", "PATCH", path, token=full,
           per_side=lambda side: {"body": {"id": child[side], "parentId": mine}})
    a.call("PATCH rename and move together", "PATCH", path, token=full,
           per_side=lambda side: {"body": {"id": child[side], "name": "Top", "parentId": "root"}})
    a.call("PATCH all three", "PATCH", path, token=full,
           per_side=lambda side: {"body": {"id": child[side], "name": "Back", "parentId": mine, "isPublic": True}})
    a.call("PATCH cycle", "PATCH", path, token=full, body={"id": mine, "parentId": inner})
    a.call("PATCH into itself", "PATCH", path, token=full, body={"id": mine, "parentId": mine})
    a.call("PATCH into a foreign folder", "PATCH", path, token=full, body={"id": inner, "parentId": theirs})
    a.call("PATCH into a malformed folder", "PATCH", path, token=full, body={"id": inner, "parentId": "nope"})
    a.call("PATCH too deep", "PATCH", path, token=full,
           per_side=lambda side: {"body": {"id": mine, "parentId": chain[side]}})
    a.call("PATCH rename, then a refused move (the rename stays)", "PATCH", path, token=full,
           body={"id": inner, "name": "Inner2", "parentId": theirs})
    folder_rows("after PATCH")

    # GET with what the writes built (counts are DIRECT contents only).
    a.call("GET after writes", "GET", path, token=full)

    # DELETE
    a.call("DELETE no id", "DELETE", path, token=full, body={})
    a.call("DELETE withContents string", "DELETE", path, token=full, body={"id": mine, "withContents": "true"})
    a.call("DELETE withContents null", "DELETE", path, token=full, body={"id": mine, "withContents": None})
    a.call("DELETE foreign", "DELETE", path, token=full, body={"id": theirs, "withContents": True})
    a.call("DELETE malformed", "DELETE", path, token=full, body={"id": "nope"})
    a.call("DELETE unknown", "DELETE", path, token=full, body={"id": str(uuid.uuid4())})
    a.call("DELETE safe mode (inner)", "DELETE", path, token=full, body={"id": inner})
    upload_rows("after safe delete")
    folder_rows("after safe delete")
    a.call("DELETE withContents false", "DELETE", path, token=full,
           per_side=lambda side: {"body": {"id": chain[side], "withContents": False}})
    a.call("DELETE with contents", "DELETE", path, token=full, body={"id": mine, "withContents": True})
    upload_rows("after delete with contents")
    folder_rows("after delete with contents")
    a.call("GET after deletes", "GET", path, token=full)

    # A session instead of a token (the lightweight caller: no ban check).
    a.signed_in(owner)
    a.call("GET with a session", "GET", path)
    created(a.call("POST with a session", "POST", path, body={"name": "Session " + sfx}), "<session>")
    lib.both_sql('UPDATE "user" SET banned = true WHERE id = %s' % Q(owner))
    a.call("GET with a banned session", "GET", path)
    a.call("GET with a banned account's token", "GET", path, token=full)
    lib.both_sql('UPDATE "user" SET banned = false WHERE id = %s' % Q(owner))
    a.signed_out()


# ── /api/duplicates ────────────────────────────────────────────────────────


def duplicates_api(run, lib):
    a = Area(run, lib, "duplicates")
    owner, tag = lib.seed_user("da")
    full = lib.seed_token(owner, "full")
    reader = lib.seed_token(owner, "read")
    uploader = lib.seed_token(owner, "upload")
    other, _ = lib.seed_user("db")
    other_full = lib.seed_token(other, "full")
    empty, _ = lib.seed_user("dc")
    empty_read = lib.seed_token(empty, "read")
    sfx = lib.suffix
    set_a, set_b, single, theirs = sha(0xA1), sha(0xB2), sha(0xC3), sha(0xD4)
    folder = a.name(seed_folder(lib, owner, "Dupes"), "<folder>")
    t = lambda n: "2026-01-01T00:00:%02d.000Z" % n
    a.name(seed_upload(lib, owner, "da" + sfx, checksum=set_a, size=100, created_at=t(1), views=3, downloads=1,
                       blurhash="LEHV6nWB2yk8"), "<a1>")
    a.name(seed_upload(lib, owner, "db" + sfx, checksum=set_a, size=100, created_at=t(2), folder_id=folder,
                       filename="copy.png"), "<a2>")
    a.name(seed_upload(lib, owner, "dc" + sfx, checksum=set_a, size=100, created_at=t(3)), "<a3>")
    a.name(seed_upload(lib, owner, "dd" + sfx, checksum=set_b, size=5000, created_at=t(4), ext="mp4",
                       mime_type="video/mp4", filename="clip.mp4"), "<b1>")
    a.name(seed_upload(lib, owner, "de" + sfx, checksum=set_b, size=5000, created_at=t(5), ext="mp4",
                       mime_type="video/mp4", filename="clip (1).mp4"), "<b2>")
    a.name(seed_upload(lib, owner, "df" + sfx, checksum=set_b, size=5000, created_at=t(6),
                       deleted_at="2026-02-01T00:00:00.000Z"), "<b-trashed>")
    a.name(seed_upload(lib, owner, "dg" + sfx, checksum=single, size=10, created_at=t(7)), "<single>")
    a.name(seed_upload(lib, owner, "dh" + sfx, size=10, created_at=t(8)), "<no-checksum>")
    a.name(seed_upload(lib, other, "di" + sfx, checksum=set_a, size=100, created_at=t(9)), "<theirs-a>")
    a.name(seed_upload(lib, other, "dj" + sfx, checksum=theirs, size=100, created_at=t(10)), "<theirs-1>")
    a.name(seed_upload(lib, other, "dk" + sfx, checksum=theirs, size=100, created_at=t(11)), "<theirs-2>")
    path = "/api/duplicates"

    def dismissal_rows(name):
        a.rows(name, 'SELECT owner_id, checksum, copies, bytes FROM duplicate_dismissals WHERE owner_id IN (%s, %s) '
                     'ORDER BY owner_id, checksum' % (Q(owner), Q(other)))

    def audit_rows(name):
        a.rows(name, "SELECT user_id, action, detail::text, ip FROM audit_log WHERE user_id IN (%s, %s) "
                     "ORDER BY created_at, action" % (Q(owner), Q(other)))

    common_refusals(a, "GET", path, full, reader, uploader, None, write=False)
    a.call("GET read token", "GET", path, token=reader)
    a.call("GET an empty library", "GET", path, token=empty_read)
    a.call("GET as the other account", "GET", path, token=other_full)
    a.call("PUT is not a method", "PUT", path, token=full, body={})
    a.call("OPTIONS", "OPTIONS", path)

    common_refusals(a, "POST", path, full, reader, uploader, {"countAgain": [set_a]}, write=True)
    a.call("POST neither verb", "POST", path, token=full, body={})
    a.call("POST both verbs", "POST", path, token=full, body={"keep": [{"checksum": set_a}], "countAgain": [set_a]})
    a.call("POST keep not an array", "POST", path, token=full, body={"keep": {"checksum": set_a}})
    a.call("POST countAgain a string", "POST", path, token=full, body={"countAgain": set_a})
    a.call("POST countAgain malformed", "POST", path, token=full, body={"countAgain": ["not-a-digest"]})
    a.call("POST countAgain empty", "POST", path, token=full, body={"countAgain": []})
    a.call("POST countAgain too many", "POST", path, token=full, body={"countAgain": [sha(i) for i in range(501)]})
    a.call("POST keep empty", "POST", path, token=full, body={"keep": []})
    a.call("POST keep scalars only", "POST", path, token=full, body={"keep": [1, "x", None]})
    a.call("POST keep an array entry", "POST", path, token=full, body={"keep": [[set_a]]})
    a.call("POST keep without checksum", "POST", path, token=full, body={"keep": [{"copies": 2}]})
    a.call("POST keep malformed", "POST", path, token=full, body={"keep": [{"checksum": "nope"}]})
    a.call("POST keep too many", "POST", path, token=full, body={"keep": [{"checksum": sha(i)} for i in range(501)]})
    a.call("POST oversized body", "POST", path, token=full, body={"countAgain": [set_a], "pad": "x" * (96 * 1024)})
    a.call("POST a full keep batch", "POST", path, token=other_full,
           body={"keep": [{"checksum": sha(i + 1000), "copies": 2, "bytes": 110} for i in range(500)]})
    dismissal_rows("after refusals")

    a.call("POST keep (upper case, copies, bytes)", "POST", path, token=full,
           body={"keep": [{"checksum": set_a.upper(), "copies": 3, "bytes": 300}]})
    a.call("GET after keep", "GET", path, token=full)
    a.call("POST keep with odd numbers", "POST", path, token=full,
           body={"keep": [{"checksum": set_b, "copies": "5", "bytes": True},
                          {"checksum": " %s " % single, "copies": [2], "bytes": -9},
                          {"checksum": set_a, "copies": 4.9, "bytes": "0x10"},
                          {"checksum": set_a, "copies": None, "bytes": [1, 2]},
                          7, None]})
    dismissal_rows("after keeps")
    a.call("POST keep re-dismisses (upsert)", "POST", path, token=full,
           body={"keep": [{"checksum": set_b, "copies": 9, "bytes": 90}]})
    dismissal_rows("after upsert")
    a.call("GET with everything dismissed", "GET", path, token=full)
    a.call("POST countAgain", "POST", path, token=full, body={"countAgain": [set_a, set_a.upper(), theirs]})
    a.call("POST countAgain again", "POST", path, token=full, body={"countAgain": [set_a]})
    a.call("POST countAgain another account's set", "POST", path, token=full,
           body={"countAgain": [sha(1000)]})
    dismissal_rows("after countAgain")
    a.call("POST keep with copies past int range", "POST", path, token=full,
           body={"keep": [{"checksum": set_a, "copies": 3e9, "bytes": 1}]}, internal_error=True)
    a.call("POST keep with bytes past bigint range", "POST", path, token=full,
           body={"keep": [{"checksum": set_a, "copies": 1, "bytes": 1e19}]}, internal_error=True)
    dismissal_rows("after out-of-range keeps")
    audit_rows("audit log")

    a.signed_in(owner)
    a.call("GET with a session", "GET", path)
    a.call("POST with a session", "POST", path, body={"countAgain": [set_b]})
    lib.both_sql('UPDATE "user" SET banned = true WHERE id = %s' % Q(owner))
    a.call("GET with a banned session", "GET", path)
    a.call("POST with a banned account's token", "POST", path, token=full, body={"countAgain": [set_b]})
    lib.both_sql('UPDATE "user" SET banned = false WHERE id = %s' % Q(owner))
    a.signed_out()

    # The reminder threshold: 11 redundant copies of one set make it due.
    for i in range(12):
        seed_upload(lib, empty, "dz%02d%s" % (i, sfx), checksum=sha(0xE5), size=1, created_at=t(20 + i))
    a.call("GET due", "GET", path, token=empty_read,
           extra=[(r"<uuid>", "<uuid>")])


# ── /api/shortlinks ────────────────────────────────────────────────────────


RANDOM_CODE = (r'("code": "(?:L[a-z]+<sfx>/)?)[A-Za-z0-9]{7}(")', r"\1<random>\2")
RANDOM_URL = (r'(/s/(?:L[a-z]+<sfx>/)?)[A-Za-z0-9]{7}(")', r"\1<random>\2")


def shortlinks_api(run, lib):
    a = Area(run, lib, "shortlinks")
    owner, tag = lib.seed_user("sa")
    full = lib.seed_token(owner, "full")
    reader = lib.seed_token(owner, "read")
    uploader = lib.seed_token(owner, "upload")
    tagless, _ = lib.seed_user("sb")
    lib.both_sql('UPDATE "user" SET user_code = NULL WHERE id = %s' % Q(tagless))
    tagless_full = lib.seed_token(tagless, "full")
    rooted, rooted_tag = lib.seed_user("sc", allow_root_short_links=True)
    rooted_full = lib.seed_token(rooted, "full")
    admin, admin_tag = lib.seed_user("sd", role="admin")
    admin_full = lib.seed_token(admin, "full")
    limited, _ = lib.seed_user("se")
    limited_full = lib.seed_token(limited, "full")
    sfx = lib.suffix
    path = "/api/shortlinks"
    extra = [RANDOM_CODE, RANDOM_URL]

    def link_rows(name):
        a.rows(name, "SELECT owner_id, code, target, expires_at IS NULL, "
                     "(expires_at - created_at) > interval '1 day', password_hash IS NULL, clicks "
                     "FROM short_links WHERE owner_id IN (%s, %s, %s, %s) ORDER BY owner_id, target, code"
               % (Q(owner), Q(tagless), Q(rooted), Q(admin)), extra=[(r"(<sfx>/)[A-Za-z0-9]{7}\\t", r"\1<random>\\t")],
               sort=True)

    common_refusals(a, "GET", path, full, reader, uploader, None, write=False)
    a.call("GET read token, nothing yet", "GET", path, token=reader)
    a.call("PUT is not a method", "PUT", path, token=full, body={})
    a.call("OPTIONS", "OPTIONS", path)
    common_refusals(a, "POST", path, full, reader, uploader, {"target": "https://example.com"}, write=True)

    body = lambda **kw: dict({"target": "https://example.com/" + sfx}, **kw)
    a.call("POST no target", "POST", path, token=full, body={})
    a.call("POST target number", "POST", path, token=full, body={"target": 5})
    a.call("POST customCode number", "POST", path, token=full, body=body(customCode=5))
    a.call("POST expiresInDays string", "POST", path, token=full, body=body(expiresInDays="5"))
    a.call("POST password number", "POST", path, token=full, body=body(password=5))
    a.call("POST password too long", "POST", path, token=full, body=body(password="p" * 1025))
    a.call("POST useUserCode string", "POST", path, token=full, body=body(useUserCode="no"))
    a.call("POST oversized body", "POST", path, token=full, body=body(pad="x" * (16 * 1024)))
    a.call("POST javascript target", "POST", path, token=full, body={"target": "javascript:alert(1)"})
    a.call("POST data target", "POST", path, token=full, body={"target": "data:text/html,x"})
    a.call("POST dotless host", "POST", path, token=full, body={"target": "intranet"})
    a.call("POST blank target", "POST", path, token=full, body={"target": "   "})
    a.call("POST custom code", "POST", path, token=full, body=body(customCode="My Code " + sfx))
    a.call("POST custom code taken", "POST", path, token=full, body=body(customCode="my-code-" + sfx))
    a.call("POST custom code empty after slug", "POST", path, token=full, body=body(customCode="!!!"))
    a.call("POST custom code too long", "POST", path, token=full, body=body(customCode="c" * 257))
    a.call("POST scoped code over 64", "POST", path, token=full, body=body(customCode="c" * 60))
    a.call("POST custom code blank means random", "POST", path, token=full, body=body(customCode="   "),
           extra=extra)
    a.call("POST random code", "POST", path, token=full, body={"target": "www.example.org/a b?x=1"}, extra=extra)
    a.call("POST customCode null", "POST", path, token=full, body=body(customCode=None, expiresInDays=None),
           extra=extra)
    a.call("POST expiring", "POST", path, token=full, body=body(customCode="exp" + sfx, expiresInDays=2))
    a.call("POST expiry clamped", "POST", path, token=full, body=body(customCode="far" + sfx, expiresInDays=1e9))
    a.call("POST expiry zero", "POST", path, token=full, body=body(customCode="zero" + sfx, expiresInDays=0))
    a.call("POST with a password", "POST", path, token=full, body=body(customCode="pw" + sfx, password=" secret "))
    a.call("POST blank password", "POST", path, token=full, body=body(customCode="nopw" + sfx, password="  "))
    a.call("POST useUserCode true", "POST", path, token=full, body=body(customCode="ut" + sfx, useUserCode=True))
    a.call("POST root without the right", "POST", path, token=full, body=body(customCode="r" + sfx,
                                                                                useUserCode=False))
    a.call("POST root with allow_root_short_links", "POST", path, token=rooted_full,
           body=body(customCode="root" + sfx, useUserCode=False))
    a.call("POST root as admin", "POST", path, token=admin_full, body=body(customCode="adm" + sfx,
                                                                           useUserCode=False))
    a.call("POST root code taken", "POST", path, token=admin_full, body=body(customCode="adm" + sfx,
                                                                             useUserCode=False))
    a.call("POST tagless account", "POST", path, token=tagless_full, body=body())
    a.call("POST tagless account, root refused", "POST", path, token=tagless_full, body=body(useUserCode=False))
    link_rows("after creates")
    a.call("GET the list", "GET", path, token=reader, extra=extra)
    a.call("GET as admin", "GET", path, token=admin_full)

    a.signed_in(owner)
    a.call("GET with a session", "GET", path, extra=extra)
    a.call("POST with a session", "POST", path, body=body(customCode="sess" + sfx))
    lib.both_sql('UPDATE "user" SET banned = true WHERE id = %s' % Q(owner))
    a.call("GET with a banned session", "GET", path, extra=extra)
    a.call("POST with a banned account's token", "POST", path, token=full, body=body())
    lib.both_sql('UPDATE "user" SET banned = false WHERE id = %s' % Q(owner))
    a.signed_out()

    # The creation limiter: 60 per minute per account, counted before the
    # body is read, then 429 with retry-after.
    for _ in range(60):
        a.pair.both("POST", path, headers={"x-api-key": limited_full, "Content-Type": "text/plain"}, raw=b"x",
                    origin=False)
    a.call("POST rate limited", "POST", path, token=limited_full, body=body())
    a.call("GET is not rate limited", "GET", path, token=limited_full)


# ── /s/{code} and /s/{tag}/{code} ──────────────────────────────────────────


def unlock_token(secret, link_id, ttl=3600):
    exp = int(time.time()) + ttl
    message = "|".join(urllib.parse.quote(p, safe="-_.!~*'()") for p in ("shortlink", link_id, str(exp)))
    mac = hmac.new(secret.encode(), message.encode(), hashlib.sha256).digest()
    return "%d.%s" % (exp, base64.urlsafe_b64encode(mac).decode().rstrip("="))


def short_redirects(run, lib):
    a = Area(run, lib, "short redirects")
    owner, tag = lib.seed_user("ra")
    sfx = lib.suffix
    links = {}

    def link(code, target, **cols):
        row = {"id": str(uuid.uuid4()), "owner_id": owner, "code": code, "target": target}
        row.update(cols)
        names = ", ".join('"%s"' % k for k in row)
        values = ", ".join(Q(v) for v in row.values())
        lib.both_sql("INSERT INTO short_links (%s) VALUES (%s)" % (names, values))
        links[code] = row["id"]
        a.name(row["id"], "<link-%s>" % code.replace(sfx, "<sfx>"))
        return row["id"]

    link("r" + sfx, "https://example.com/root?x=1")
    link("%s/promo" % tag, "https://example.com/promo")
    link("%s/old" % tag, "https://example.com/old", expires_at="2020-01-01T00:00:00.000Z")
    link("%s/later" % tag, "https://example.com/later", expires_at="2099-01-01T00:00:00.000Z")
    pw = link("%s/pw" % tag, "https://example.com/secret", password_hash="scrypt$x$y")
    rpw = link("rpw" + sfx, "https://example.com/root-secret", password_hash="scrypt$x$y")
    link("%s/space" % tag, "https://example.com/a%20b/ą?q=ł")
    link("%s/Mixed" % tag, "https://example.com/mixed")

    def get(name, p, method="GET", **kw):
        a.call(name, method, p, **kw)

    get("root code", "/s/r" + sfx)
    get("namespaced code", "/s/%s/promo" % tag)
    get("HEAD namespaced code", "/s/%s/promo" % tag, method="HEAD")
    get("unknown root code", "/s/nothing" + sfx)
    get("unknown namespaced code", "/s/%s/nothing" % tag)
    get("unknown tag", "/s/nobody%s/promo" % sfx)
    get("expired", "/s/%s/old" % tag)
    get("expiry in the future", "/s/%s/later" % tag)
    get("password gate", "/s/%s/pw" % tag)
    get("root password gate", "/s/rpw" + sfx)
    get("target with unicode", "/s/%s/space" % tag)
    get("case matters", "/s/%s/mixed" % tag)
    get("exact case", "/s/%s/Mixed" % tag)
    get("percent-encoded slash", "/s/%s%%2Fpromo" % tag)
    get("percent-encoded leaf", "/s/%s/pr%%6Fmo" % tag)
    get("trailing slash", "/s/%s/promo/" % tag)
    get("POST is not a method", "/s/r" + sfx, method="POST")
    get("DELETE is not a method", "/s/%s/promo" % tag, method="DELETE")
    get("OPTIONS", "/s/r" + sfx, method="OPTIONS")
    # Not a route: the not-found document (the SPA shell differs from Next's
    # HTML by design, so only status and headers are compared).
    get("three segments", "/s/%s/promo/extra" % tag, compare_body=False)

    # A valid unlock cookie opens the gate; a forged or foreign one does not.
    for side, br, db in (("ref", a.pair.ref, lib.ref_db), ("rust", a.pair.rust, lib.rust_db)):
        br.cookies["ml_sl_" + pw] = unlock_token(db.auth_secret, pw)
        br.cookies["ml_sl_" + rpw] = unlock_token(db.auth_secret, pw)  # minted for the OTHER link
    get("unlocked by a valid cookie", "/s/%s/pw" % tag)
    get("a cookie minted for another link", "/s/rpw" + sfx)
    for br in (a.pair.ref, a.pair.rust):
        br.cookies["ml_sl_" + pw] = "9999999999.forged"
    get("forged cookie", "/s/%s/pw" % tag)
    for side, br, db in (("ref", a.pair.ref, lib.ref_db), ("rust", a.pair.rust, lib.rust_db)):
        br.cookies["ml_sl_" + pw] = unlock_token(db.auth_secret, pw, ttl=-10)
    get("expired cookie", "/s/%s/pw" % tag)
    a.pair.ref.cookies.clear()
    a.pair.rust.cookies.clear()

    # Clicks are counted in the background, after the redirect was sent.
    time.sleep(1.0)
    a.rows("clicks", "SELECT code, clicks, last_used_at IS NULL FROM short_links WHERE owner_id = %s ORDER BY code"
           % Q(owner))


# ── GET /api/admin/disk ────────────────────────────────────────────────────

DISK_EMAIL = "disk.parity"
TOP_KEYS = ["driver", "local", "liveFiles", "liveBytes", "trashedFiles", "trashedBytes", "disk", "minFreeBytes",
            "kinds", "kindBytes", "accounts", "biggest", "cold", "coldTotals", "trash", "trashRetentionDays",
            "trashDueSoonDays"]


def disk_cleanup(lib):
    lib.both_sql(
        "DELETE FROM uploads WHERE owner_id IN (SELECT id FROM \"user\" WHERE email LIKE '%%@%s') "
        "OR storage_key LIKE 'diskparity/%%'; DELETE FROM \"user\" WHERE email LIKE '%%@%s'" % (DISK_EMAIL, DISK_EMAIL))


def expected_disk(db):
    """The global figures of the report, computed by SQL on ONE database."""
    rows = db.sql(
        "SELECT (count(*) FILTER (WHERE deleted_at IS NULL)), coalesce(sum(size) FILTER (WHERE deleted_at IS NULL), 0), "
        "(count(*) FILTER (WHERE deleted_at IS NOT NULL)), coalesce(sum(size) FILTER (WHERE deleted_at IS NOT NULL), 0), "
        "to_char(min(deleted_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), "
        "(count(*) FILTER (WHERE deleted_at < now() - interval '23 days' AND deleted_at >= now() - interval '30 days')), "
        "coalesce(sum(size) FILTER (WHERE deleted_at < now() - interval '23 days' AND deleted_at >= now() - interval '30 days'), 0), "
        "(count(*) FILTER (WHERE deleted_at < now() - interval '30 days')), "
        "coalesce(sum(size) FILTER (WHERE deleted_at < now() - interval '30 days'), 0), "
        "(count(*) FILTER (WHERE deleted_at IS NULL AND coalesce(last_accessed_at, created_at) < now() - interval '180 days')), "
        "coalesce(sum(size) FILTER (WHERE deleted_at IS NULL AND coalesce(last_accessed_at, created_at) < now() - interval '180 days'), 0) "
        "FROM uploads").strip().split("\t")
    n = [int(x) if x.lstrip("-").isdigit() else (x or None) for x in rows]
    groups = []
    for line in db.sql(
            "SELECT coalesce(u.owner_id, ''), (count(*) FILTER (WHERE u.deleted_at IS NULL)), "
            "coalesce(sum(u.size) FILTER (WHERE u.deleted_at IS NULL), 0), "
            "(count(*) FILTER (WHERE u.deleted_at IS NOT NULL)), "
            "coalesce(sum(u.size) FILTER (WHERE u.deleted_at IS NOT NULL), 0) FROM uploads u GROUP BY u.owner_id"
    ).splitlines():
        oid, lf, lb, tf, tb = line.split("\t")
        groups.append({"id": oid or None, "liveFiles": int(lf), "liveBytes": int(lb), "trashedFiles": int(tf),
                       "trashedBytes": int(tb)})
    groups.sort(key=lambda g: -(g["liveBytes"] + g["trashedBytes"]))
    head = []
    for g in groups[:6]:
        head.append({"id": g["id"], "orphaned": g["id"] is None, "rest": False, "accounts": 1,
                     "liveFiles": g["liveFiles"], "liveBytes": g["liveBytes"], "trashedFiles": g["trashedFiles"],
                     "trashedBytes": g["trashedBytes"]})
    if len(groups) > 6:
        tail = groups[6:]
        head.append({"id": None, "orphaned": False, "rest": True, "accounts": len(tail),
                     "liveFiles": sum(g["liveFiles"] for g in tail), "liveBytes": sum(g["liveBytes"] for g in tail),
                     "trashedFiles": sum(g["trashedFiles"] for g in tail),
                     "trashedBytes": sum(g["trashedBytes"] for g in tail)})
    for row in head:
        name = email = tagv = None
        if row["id"]:
            line = db.sql('SELECT name, email, coalesce(user_code, \'\x01\') FROM "user" WHERE id = %s' % Q(row["id"]))
            if line.strip():
                name, email, tagv = line.rstrip("\n").split("\t")
                tagv = None if tagv == "\x01" else tagv
        row["name"], row["email"], row["tag"] = name, email, tagv
    return {
        "liveFiles": n[0], "liveBytes": n[1], "trashedFiles": n[2], "trashedBytes": n[3],
        "trash": {"files": n[2], "bytes": n[3], "oldest": n[4], "dueSoonFiles": n[5], "dueSoonBytes": n[6],
                  "overdueFiles": n[7], "overdueBytes": n[8]},
        "coldTotals": {"files": n[9], "bytes": n[10], "days": 180},
        "accounts": head,
    }


def keys_deep(v):
    """The key ORDER of every object, recursively (values dropped)."""
    if isinstance(v, dict):
        return [[k, keys_deep(x)] for k, x in v.items()]
    if isinstance(v, list):
        return [keys_deep(x) for x in v[:1]]
    return type(v).__name__


def admin_disk(run, lib):
    a = Area(run, lib, "admin disk")
    disk_cleanup(lib)
    sfx = lib.suffix

    def disk_user(key, role="user", **cols):
        uid = ("dsk%s%s" % (key, sfx))[:32]
        for db in (lib.ref_db, lib.rust_db):
            db.create_user(uid, "%s@%s" % (uid, DISK_EMAIL), "Disk %s" % key, role=role, user_code="D%s%s" % (key, sfx),
                           **cols)
        a.name(uid, "<user-%s>" % key)
        return uid

    admin = disk_user("ad", role="admin")
    admin_full = lib.seed_token(admin, "full")
    admin_write = lib.seed_token(admin, "write")
    admin_upload = lib.seed_token(admin, "upload")
    admin_read = lib.seed_token(admin, "read")
    # (An unrecognised stored scope cannot be seeded: api_tokens_scope_check
    # refuses it. The Rust unit tests cover that branch.)
    plain = disk_user("pl")
    plain_full = lib.seed_token(plain, "full")
    plain_write = lib.seed_token(plain, "write")
    banned = disk_user("bn", role="admin", banned=True)
    banned_full = lib.seed_token(banned, "full")
    pacer = disk_user("pc", role="admin")
    pacer_full = lib.seed_token(pacer, "full")
    path = "/api/admin/disk"

    a.call("no credential", "GET", path)
    a.call("unknown token", "GET", path, token="nope")
    a.call("bearer unknown", "GET", path, headers={"Authorization": "Bearer nope"})
    a.call("empty x-api-key", "GET", path, headers={"x-api-key": ""})
    a.call("non-admin full token", "GET", path, token=plain_full)
    a.call("non-admin write token", "GET", path, token=plain_write)
    a.call("admin write token", "GET", path, token=admin_write)
    a.call("admin upload token", "GET", path, token=admin_upload)
    a.call("admin read token", "GET", path, token=admin_read)
    a.call("banned admin", "GET", path, token=banned_full)
    a.call("POST is not a method", "POST", path, token=admin_full)
    a.call("OPTIONS", "OPTIONS", path)
    a.signed_in(plain)
    a.call("non-admin session", "GET", path)
    a.call("non-admin session with a bad token", "GET", path, token="nope")
    a.signed_out()

    # Accounts and files big enough to own the top of every list on both
    # databases: seven heavy accounts and one heavy orphan.
    heavy = []
    for i in range(7):
        uid = disk_user("h%d" % i)
        heavy.append(uid)
        e14 = 10 ** 14
        a.name(seed_upload(lib, uid, "hv%d%s" % (i, sfx), size=(7 - i) * e14, filename="heavy%d.mp4" % i, ext="mp4",
                           mime_type="video/mp4", created_at="2026-01-0%dT00:00:00.000Z" % (i + 1),
                           storage_key="diskparity/hv%d" % i), "<heavy-%d>" % i)
        a.name(seed_upload(lib, uid, "ht%d%s" % (i, sfx), size=1000 + i, created_at="2026-01-01T00:00:00.000Z",
                           deleted_at="1990-0%d-01T00:00:00.000Z" % (i + 1), storage_key="diskparity/ht%d" % i),
               "<trashed-%d>" % i)
        a.name(seed_upload(lib, uid, "hc%d%s" % (i, sfx), size=10 + i, filename="cold%d.zip" % i, ext="zip",
                           mime_type="application/zip", created_at="1990-01-0%dT00:00:00.000Z" % (i + 1),
                           views=i, storage_key="diskparity/hc%d" % i), "<cold-%d>" % i)
    orphan = seed_upload(lib, heavy[0], "ho" + sfx, size=int(4.5 * 10 ** 14), filename="orphan.avif", ext="avif",
                         mime_type="application/octet-stream", created_at="2026-01-09T00:00:00.000Z",
                         last_accessed_at="2026-03-01T00:00:00.000Z", storage_key="diskparity/orphan")
    lib.both_sql("UPDATE uploads SET owner_id = NULL WHERE id = %s" % Q(orphan))
    a.name(orphan, "<orphan>")
    # Due soon: trashed 25 days ago (purged in 5).
    seed_upload(lib, heavy[6], "hd" + sfx, size=12345, storage_key="diskparity/hd",
                deleted_at=time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(time.time() - 25 * 86400)))

    def report(name, token=None, session=None):
        if session:
            a.signed_in(session)
        out = []
        for side, br, db in (("ref", a.pair.ref, lib.ref_db), ("rust", a.pair.rust, lib.rust_db)):
            h = {"x-api-key": token} if token else {}
            # The databases are shared with other parity runs that may be
            # writing right now: judge a report only when the database said
            # the same thing just before and just after it was made.
            for _ in range(6):
                before = expected_disk(db)
                res = br.request("GET", path, headers=h, origin=False)
                after = expected_disk(db)
                if before == after:
                    break
                time.sleep(2)
            out.append((res, before if before == after else None))
        if session:
            a.signed_out()
        shapes = []
        for res, exp in out:
            body = res.json()
            shape = {"status": res.status, "content-type": res.header("content-type"),
                     "cache-control": res.header("cache-control")}
            if res.status != 200 or not isinstance(body, dict):
                shape["body"] = body
                shapes.append(shape)
                continue
            if exp is None:
                shape["own-database-checks"] = ["the database never held still for one report"]
                shapes.append(shape)
                continue
            problems = []
            shape["keys"] = keys_deep(body)
            for k in ("liveFiles", "liveBytes", "trashedFiles", "trashedBytes", "trash", "coldTotals", "accounts"):
                got = body.get(k)
                want = exp[k]
                if k == "accounts":
                    got = [dict(r) for r in got]
                if got != want:
                    problems.append("%s: got %s, own database says %s" % (k, json.dumps(got), json.dumps(want)))
            kinds = body.get("kinds") or []
            if sum(k["bytes"] for k in kinds) != body.get("kindBytes") or body.get("kindBytes") != body.get("liveBytes"):
                problems.append("kinds do not add up to liveBytes")
            if sum(k["files"] for k in kinds) != body.get("liveFiles"):
                problems.append("kinds do not add up to liveFiles")
            if [k["bytes"] for k in kinds] != sorted([k["bytes"] for k in kinds], reverse=True):
                problems.append("kinds not heaviest first")
            disk = body.get("disk")
            disk_ok = disk is None or (list(disk) == ["totalBytes", "freeBytes", "usedBytes"]
                                       and all(isinstance(x, int) and x >= 0 for x in disk.values()))
            if not disk_ok:
                problems.append("disk shape")
            shape["own-database-checks"] = problems or "ok"
            # What both databases share: the seeded tops of the lists, the
            # constants and the configuration.
            shape["driver"] = body["driver"]
            shape["local"] = body["local"]
            shape["disk-null"] = disk is None
            shape["minFreeBytes"] = body["minFreeBytes"]
            shape["trashRetentionDays"] = body["trashRetentionDays"]
            shape["trashDueSoonDays"] = body["trashDueSoonDays"]
            shape["top-kind"] = kinds[0]["kind"] if kinds else None
            shape["biggest"] = a.label(body["biggest"])
            shape["cold"] = a.label(body["cold"])
            # (`trash.oldest` is checked against each database only: the trash
            # sweepers of both stacks may purge the overdue rows seeded here.)
            shape["accounts-named"] = a.label([r for r in body["accounts"] if not r["orphaned"] and not r["rest"]])
            shape["accounts-pseudo"] = [(r["orphaned"], r["rest"]) for r in body["accounts"]]
            shapes.append(shape)
        run.compare("admin disk: " + name, shapes[0], shapes[1])
        # Equal on both sides is not enough here: each must agree with its own
        # database.
        for side, shape in zip(("reference", "rust"), shapes):
            if "own-database-checks" in shape:
                run.compare("admin disk: %s, %s against its own database" % (name, side), "ok",
                            shape["own-database-checks"])
                if run.verbose:
                    print(json.dumps({k: shape[k] for k in ("biggest", "cold", "accounts-named", "accounts-pseudo",
                                                            "top-kind")}, indent=1)[:3000])

    report("admin full token", token=admin_full)
    report("admin session", session=admin)
    report("banned admin's session still reads (no ban check on the session path)", session=banned)

    # 30 reports per minute per account, then 429 with retry-after.
    for _ in range(30):
        a.pair.both("GET", path, headers={"x-api-key": pacer_full}, origin=False)
    a.call("rate limited", "GET", path, token=pacer_full)
    a.call("another admin is not paced by it", "GET", path, token=admin_read)

    disk_cleanup(lib)


def main():
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--ref-db", default="modlight-srvlib-ref-postgres-1:modlight")
    p.add_argument("--rust-db", default="modlight-devdb-postgres-1:modlight_srv_library")
    p.add_argument("--ref-env", default="_scratch/srv-library/ref/.env")
    p.add_argument("--rust-env", default="_scratch/srv-library/rust.env")
    p.add_argument("--only", default=None)
    a, _ = p.parse_known_args()
    run = parity.Run.from_args()
    rc, rd = a.ref_db.split(":")
    uc, ud = a.rust_db.split(":")
    ref_db = parity.Db(rc, rd, parity.read_env_file(a.ref_env)["AUTH_SECRET"])
    rust_db = parity.Db(uc, ud, parity.read_env_file(a.rust_env)["AUTH_SECRET"])
    lib = Library(run, ref_db, rust_db)
    areas = [("uploads_check", uploads_check), ("folders", folders_api), ("duplicates", duplicates_api),
             ("shortlinks", shortlinks_api), ("redirects", short_redirects), ("admin_disk", admin_disk)]
    for name, fn in areas:
        if a.only and a.only not in name.split(","):
            if not (a.only and name in a.only.split(",")):
                continue
        fn(run, lib)
    run.finish()


if __name__ == "__main__":
    main()

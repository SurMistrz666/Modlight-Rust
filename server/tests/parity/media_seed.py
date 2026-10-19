"""Seeding for the media serving parity scripts (media_serve.py).

Every file is uploaded through ShareX (`POST /upload` with an upload token) on
BOTH servers, so each server stores the bytes and computes the metadata its
own way. The two rows are then made to agree on everything a response can
show: the same id (so render tickets, unlock cookies and view grants minted by
either server name the same file), the same public code, delete token and
creation time. Only `storage_key` differs, and nothing serves it.

Gating columns (published, password, expiry, view cap, kind/language) are then
set by SQL on both rows alike.

Only the Python standard library; Python 3.9.
"""

import json
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402
from library import multipart  # noqa: E402

Q = parity.sql_quote

CORPUS = os.environ.get(
    "MEDIA_CORPUS", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "..", "..",
                                 "_scratch", "srv-media-b", "corpus"))

# key -> (filename, content type sent by the client)
FILES = {
    "png": ("tiny.png", "image/png"),
    "jpg": ("eagle.jpg", "image/jpeg"),
    "wide": ("macblue_2400.jpg", "image/jpeg"),
    "gif": ("anim.gif", "image/gif"),
    "webp": ("anim.webp", "image/webp"),
    "heic": ("owl.heic", "image/heic"),
    "mp4": ("clip.mp4", "video/mp4"),
    "mp3": ("tone.mp3", "audio/mpeg"),
    "txt": ("notes.txt", "text/plain"),
    "bigtxt": ("big.txt", "text/plain"),
    "html": ("page.html", "text/html"),
    "svg": ("logo.svg", "image/svg+xml"),
    "json": ("data.json", "application/json"),
    "pdf": ("doc.pdf", "application/pdf"),
    "zip": ("bundle.zip", "application/zip"),
    "docx": ("report.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    "mlenc": ("secret.mlenc", "application/vnd.modlight.encrypted"),
}


def corpus_bytes(name):
    with open(os.path.join(CORPUS, name), "rb") as f:
        return f.read()


class Seeder(object):
    def __init__(self, run, ref_db, rust_db, ref_url, rust_url):
        self.run = run
        self.ref_db = ref_db
        self.rust_db = rust_db
        self.ref_url = ref_url
        self.rust_url = rust_url
        self.suffix = "%05d" % random.randint(0, 99999)

    def both_sql(self, statement):
        return self.ref_db.sql(statement), self.rust_db.sql(statement)

    def seed_user(self, key, **columns):
        """The same user on both servers; returns (id, tag)."""
        user_id = ("ms%s%s" % (key, self.suffix))[:32]
        tag = ("M%s%s" % (key, self.suffix))[:32]
        for db in (self.ref_db, self.rust_db):
            db.create_user(user_id, "%s@media.parity" % user_id, "Media %s" % key, user_code=tag, **columns)
        return user_id, tag

    def seed_token(self, user_id, scope="upload"):
        token = "ms_" + parity.random_id(28)
        for db in (self.ref_db, self.rust_db):
            db.create_api_token(user_id, token, scope=scope)
        return token

    def upload(self, owner, token, key, label, **cols):
        """Upload FILES[key] to both servers and align the two rows. Returns a
        dict with id, code, ext, filename, delete_token, mime."""
        filename, ctype = FILES[key]
        data = corpus_bytes(filename)
        name = "%s-%s" % (label, filename)
        ct, body = multipart(files=[("file", name, ctype, data)])
        ids = []
        for side, base, db in (("ref", self.ref_url, self.ref_db), ("rust", self.rust_url, self.rust_db)):
            br = parity.Browser(base, real_ip=self.run.real_ip)
            r = br.request("POST", "/upload", headers={"x-api-key": token, "Content-Type": ct}, raw=body,
                           origin=False)
            if r.status not in (200, 201):
                raise RuntimeError("upload %s on %s: %s %s" % (key, side, r.status, r.text()[:300]))
            row = db.sql('SELECT id FROM uploads WHERE owner_id = %s AND filename = %s ORDER BY created_at DESC '
                         'LIMIT 1' % (Q(owner), Q(name))).strip()
            ids.append(row)
        ref_id, rust_id = ids
        code = ("c%s%s" % (label, self.suffix)).replace("-", "")[:40]
        common = {"code": code, "delete_token": "dt" + parity.random_id(20),
                  "created_at": "2026-02-03T04:05:06.789Z"}
        common.update(cols)
        sets = ", ".join('"%s" = %s' % (k, Q(v)) for k, v in common.items())
        self.ref_db.sql('UPDATE uploads SET %s WHERE id = %s' % (sets, Q(ref_id)))
        self.rust_db.sql('UPDATE uploads SET id = %s, %s WHERE id = %s' % (Q(ref_id), sets, Q(rust_id)))
        info = self.row(ref_id)
        info["data"] = data
        return info

    def row(self, media_id, side="ref"):
        db = self.ref_db if side == "ref" else self.rust_db
        out = db.sql("SELECT row_to_json(u) FROM (SELECT id, code, ext, filename, mime_type, size, views, downloads, "
                     "max_views, delete_token, published, width, height, frames, kind FROM uploads WHERE id = %s) u"
                     % Q(media_id)).strip()
        return json.loads(out) if out else None

    def set_cols(self, media_id, **cols):
        sets = ", ".join('"%s" = %s' % (k, Q(v)) for k, v in cols.items())
        self.both_sql('UPDATE uploads SET %s WHERE id = %s' % (sets, Q(media_id)))

    def counters(self, media_id):
        """(views, downloads) on each side."""
        q = "SELECT views, downloads FROM uploads WHERE id = %s" % Q(media_id)
        a, b = self.both_sql(q)
        return tuple(a.strip().split("\t")), tuple(b.strip().split("\t"))

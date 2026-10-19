"""Library parity: uploads (ShareX multipart, tus), listings, folders, short
links, duplicates, trash and quota, against a scratch reference stack.

    python3 server/tests/parity/library.py --ref http://localhost:8441 --rust http://localhost:8422 \
        --ref-db modlight-srvlib-ref-postgres-1:modlight --rust-db modlight-devdb-postgres-1:modlight_srv_library \
        --ref-env _scratch/srv-library/ref/.env --rust-env _scratch/srv-library/rust.env

Both servers: NODE_ENV=production, APP_URL = their own origin; Rust with
TRUST_PROXY=true. The script seeds the SAME users (fixed ids and tags) and
the same API tokens into both databases, drives both servers with identical
requests, and compares statuses, the headers that matter and bodies (origins,
uuids, dates and random tokens normalised), then the rows each server wrote.

State-changing: never point it at the shared reference on 8420.
"""

import argparse
import hashlib
import json
import os
import random
import re
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

ORIGIN_RE = r"http://localhost:\d+"


def multipart(fields=(), files=(), boundary=None):
    """fields: [(name, value)], files: [(name, filename, content_type or None, bytes)].
    Returns (content_type, body). Order is exactly as given."""
    boundary = boundary or "----parity" + uuid.uuid4().hex
    out = b""
    for name, value in fields:
        out += ("--%s\r\nContent-Disposition: form-data; name=\"%s\"\r\n\r\n" % (boundary, name)).encode()
        out += value.encode() if isinstance(value, str) else value
        out += b"\r\n"
    for name, filename, ctype, data in files:
        disp = "--%s\r\nContent-Disposition: form-data; name=\"%s\"; filename=\"%s\"\r\n" % (boundary, name, filename)
        out += disp.encode("utf-8")
        if ctype:
            out += ("Content-Type: %s\r\n" % ctype).encode()
        out += b"\r\n" + data + b"\r\n"
    out += ("--%s--\r\n" % boundary).encode()
    return "multipart/form-data; boundary=%s" % boundary, out


class Library(object):
    def __init__(self, run, ref_db, rust_db):
        self.run = run
        self.ref_db = ref_db
        self.rust_db = rust_db
        self.suffix = "%06d" % random.randint(0, 999999)

    def both_sql(self, statement):
        return self.ref_db.sql(statement), self.rust_db.sql(statement)

    def seed_user(self, key, **columns):
        """The same user on both servers; returns (id, tag)."""
        user_id = ("lib%s%s" % (key, self.suffix))[:32]
        tag = ("L%s%s" % (key, self.suffix))[:32]
        for db in (self.ref_db, self.rust_db):
            db.create_user(user_id, "%s@library.parity" % user_id, "Parity %s" % key, user_code=tag, **columns)
        return user_id, tag

    def seed_token(self, user_id, scope="upload"):
        token = "lib_" + parity.random_id(28)
        for db in (self.ref_db, self.rust_db):
            db.create_api_token(user_id, token, scope=scope)
        return token

    def rows(self, sql):
        a, b = self.both_sql(sql)
        norm = lambda s: parity.normalize_text(s, [(ORIGIN_RE, "<origin>")])
        return norm(a), norm(b)

    def compare_rows(self, name, sql):
        a, b = self.rows(sql)
        self.run.compare(name, a.splitlines(), b.splitlines())


def upload_extra():
    """Normalisations for ShareX answers: origins, delete tokens, random codes."""
    return [(ORIGIN_RE, "<origin>"), (r"/delete/[A-Za-z0-9_-]{22}", "/delete/<token>")]


def norm_upload_body(res):
    body = res.json()
    if body is None:
        return parity.normalize_text(res.text())
    text = json.dumps(body, sort_keys=False, ensure_ascii=False)
    for pattern, repl in upload_extra():
        text = re.sub(pattern, repl, text)
    return parity.normalize_json(json.loads(text))


def sharex(run, lib):
    pair = run.pair()
    owner, tag = lib.seed_user("sx", naming_scheme="filename")
    token = lib.seed_token(owner, "upload")
    reader = lib.seed_token(owner, "read")

    def post(name, path="/upload", headers=None, raw=b"", ctype=None, body_checks=True):
        h = dict(headers or {})
        if ctype:
            h["Content-Type"] = ctype
        a, b = pair.both("POST", path, headers=h, raw=raw, origin=False)
        shape = lambda r: {"status": r.status, "content-type": r.header("content-type"), "body": norm_upload_body(r)}
        run.compare("sharex: " + name, shape(a), shape(b))
        return a, b

    ctype, body = multipart(files=[("sharex", "notes.txt", "text/plain", b"hello parity\n")])
    post("no content type", raw=body)
    post("unknown token", headers={"x-api-key": "nope"}, ctype=ctype, raw=body)
    post("read-scoped token", headers={"x-api-key": reader}, ctype=ctype, raw=body)
    post("bearer header, unknown", headers={"Authorization": "Bearer nope"}, ctype=ctype, raw=body)
    post("no token at all", ctype=ctype, raw=body)
    post("urlencoded body", ctype="application/x-www-form-urlencoded", raw=b"token=" + token.encode())
    ct2, only_fields = multipart(fields=[("token", token)])
    post("multipart without a file", ctype=ct2, raw=only_fields)
    post("no boundary", ctype="multipart/form-data", raw=body)
    post("json body", ctype="application/json", raw=b"{}")

    post("header token, txt file", headers={"x-api-key": token}, ctype=ctype, raw=body)
    post("same bytes again (duplicate_of)", headers={"x-api-key": token}, ctype=ctype, raw=body)
    post("/api/upload path", path="/api/upload", headers={"x-api-key": token}, ctype=ctype, raw=body)
    ct3, legacy = multipart(fields=[("token", token)], files=[("file", "legacy.txt", None, b"legacy field token\n")])
    post("legacy token field before the file", ctype=ct3, raw=legacy)
    ct4, late = multipart(files=[("file", "late.txt", "text/plain", b"token after the file\n")], fields=())
    late = late.replace(b"--\r\n", b"\r\n")[:-2]  # reopen to append a field after the file
    ct4 = ct4
    ct5, utf = multipart(files=[("file", "Chłopak żółw.md", "text/markdown", b"# utf\n")])
    post("utf-8 filename", headers={"x-api-key": token}, ctype=ct5, raw=utf)
    ct6, custom = multipart(fields=[("code", "My Custom Code")], files=[("file", "c.txt", "text/plain", b"custom\n")])
    post("custom code", headers={"x-api-key": token}, ctype=ct6, raw=custom)
    post("custom code taken", headers={"x-api-key": token}, ctype=ct6, raw=custom)
    ct7, reserved = multipart(fields=[("code", "login")], files=[("file", "r.txt", "text/plain", b"reserved\n")])
    post("reserved custom code", headers={"x-api-key": token}, ctype=ct7, raw=reserved)
    ct8, root = multipart(fields=[("folderId", "root")], files=[("file", "root.txt", "text/plain", b"root\n")])
    post("folder root", headers={"x-api-key": token}, ctype=ct8, raw=root)
    ct9, unknown_folder = multipart(fields=[("folderId", str(uuid.uuid4()))],
                                    files=[("file", "uf.txt", "text/plain", b"unknown folder\n")])
    post("unknown folder", headers={"x-api-key": token}, ctype=ct9, raw=unknown_folder)
    ct10, octet = multipart(files=[("file", "", "application/octet-stream", b"\x89PNG\r\n\x1a\nnot really")])
    post("octet-stream part without filename", headers={"x-api-key": token}, ctype=ct10, raw=octet)
    ct11, noname = multipart(files=[("file", "noext", None, b"plain text body\n")])
    post("file part without content-type", headers={"x-api-key": token}, ctype=ct11, raw=noname)

    lib.compare_rows(
        "sharex: rows written",
        'SELECT code, filename, ext, kind, mime_type, size, storage_key, checksum, published, views, '
        'folder_id IS NULL, password_hash IS NULL, length(delete_token) FROM uploads WHERE owner_id = %s '
        'ORDER BY created_at, code' % parity.sql_quote(owner))
    lib.compare_rows("sharex: quota", 'SELECT used_quota FROM "user" WHERE id = %s' % parity.sql_quote(owner))

    # Quota: a 10-byte ceiling refuses the next upload with 507.
    lib.both_sql('UPDATE "user" SET max_quota = used_quota + 5 WHERE id = %s' % parity.sql_quote(owner))
    ct12, big = multipart(files=[("file", "q.txt", "text/plain", b"x" * 64)])
    post("over quota", headers={"x-api-key": token}, ctype=ct12, raw=big)
    lib.compare_rows("sharex: quota after refusal", 'SELECT used_quota FROM "user" WHERE id = %s' % parity.sql_quote(owner))


# ── tus (/api/files) ────────────────────────────────────────────────────────

TUS_HEADERS = ("tus-resumable", "tus-version", "tus-extension", "tus-max-size", "upload-offset", "upload-length",
               "upload-defer-length", "upload-metadata", "upload-expires", "location", "cache-control",
               "content-type", "connection", "access-control-allow-origin", "access-control-expose-headers",
               "access-control-allow-methods", "access-control-allow-headers", "access-control-max-age",
               "x-media-url", "x-media-raw-url", "x-media-copy-url", "content-disposition")


def tus_shape(res):
    out = {"status": res.status}
    for h in TUS_HEADERS:
        v = res.header(h)
        if v is None:
            continue
        if h == "connection" and v.lower() == "keep-alive":
            continue  # Node's own keep-alive header, not the application's
        v = re.sub(r"http://[^/\s]+/api/files/[0-9a-f]{32}", "<location>", v)
        v = re.sub(ORIGIN_RE, "<origin>", v)
        out[h] = parity.normalize_text(v)
    out["body"] = res.text()
    return out


def b64(s):
    import base64
    return base64.b64encode(s.encode("utf-8")).decode()


def tus(run, lib):
    owner, tag = lib.seed_user("tus", naming_scheme="filename")
    token = lib.seed_token(owner, "upload")
    reader = lib.seed_token(owner, "read")
    small, small_tag = lib.seed_user("tq", naming_scheme="filename", max_quota=10)
    small_token = lib.seed_token(small, "upload")
    pair = run.pair()
    base = {"Tus-Resumable": "1.0.0"}

    def step(name, method, path, headers=None, raw=None, which=None):
        """Run on both servers; `path` may be a callable (browser -> path)."""
        out = []
        for br in (pair.ref, pair.rust):
            p = path(br) if callable(path) else path
            out.append(br.request(method, p, headers=headers, raw=raw, origin=False))
        run.compare("tus: " + name, tus_shape(out[0]), tus_shape(out[1]))
        return out

    def h(**kw):
        d = dict(base)
        for k, v in kw.items():
            d[k.replace("_", "-")] = v
        return d

    step("OPTIONS with origin", "OPTIONS", "/api/files", headers={"Origin": "http://example.test"})
    step("OPTIONS without tus-resumable", "OPTIONS", "/api/files/abc")
    step("POST without Tus-Resumable", "POST", "/api/files", headers={"Upload-Length": "5"})
    step("POST bad version and length", "POST", "/api/files", headers={"Tus-Resumable": "0.2", "Upload-Length": "x"})
    step("POST unauthenticated", "POST", "/api/files", headers=h(Upload_Length="5"))
    step("POST unknown token", "POST", "/api/files", headers=h(Upload_Length="5", x_api_key="nope"))
    step("POST read-scoped token", "POST", "/api/files", headers=h(Upload_Length="5", x_api_key=reader))
    step("POST deferred length", "POST", "/api/files", headers=h(Upload_Defer_Length="1", x_api_key=token))
    step("POST both lengths", "POST", "/api/files", headers=h(Upload_Defer_Length="1", Upload_Length="5", x_api_key=token))
    step("POST no length", "POST", "/api/files", headers=h(x_api_key=token))
    step("POST unknown folder", "POST", "/api/files",
         headers=h(Upload_Length="5", x_api_key=token, Upload_Metadata="folderId " + b64(str(uuid.uuid4()))))
    step("POST bad metadata", "POST", "/api/files", headers=h(Upload_Length="5", x_api_key=token, Upload_Metadata="a b c"))
    step("POST concat", "POST", "/api/files", headers=h(Upload_Length="5", x_api_key=token, Upload_Concat="partial"))
    step("POST over quota (maxSize)", "POST", "/api/files", headers=h(Upload_Length="50", x_api_key=small_token))
    step("PUT method", "PUT", "/api/files/abc", headers=h())
    step("HEAD unknown id", "HEAD", "/api/files/0123456789abcdef0123456789abcdef", headers=h())
    step("HEAD the path itself", "HEAD", "/api/files", headers=h())
    step("PATCH unknown id", "PATCH", "/api/files/0123456789abcdef0123456789abcdef",
         headers=h(Upload_Offset="0", Content_Type="application/offset+octet-stream"), raw=b"x")
    step("GET unknown id", "GET", "/api/files/0123456789abcdef0123456789abcdef")

    # A complete upload: create, resume, finish.
    locs = {}
    meta = "filename %s,filetype %s,folderId %s" % (b64("tus notes.txt"), b64("text/plain"), b64("root"))
    created = step("POST create", "POST", "/api/files", headers=h(Upload_Length="11", x_api_key=token, Upload_Metadata=meta))
    for br, res in zip((pair.ref, pair.rust), created):
        locs[br.base] = urllib_path(res.header("location"))
    loc = lambda br: locs[br.base]
    step("HEAD fresh", "HEAD", loc, headers=h())
    step("PATCH first part", "PATCH", loc, headers=h(Upload_Offset="0", Content_Type="application/offset+octet-stream"),
         raw=b"hello")
    step("HEAD after part", "HEAD", loc, headers=h())
    step("PATCH wrong offset", "PATCH", loc, headers=h(Upload_Offset="0", Content_Type="application/offset+octet-stream"),
         raw=b"x")
    step("PATCH without content type", "PATCH", loc, headers=h(Upload_Offset="5"), raw=b"x")
    step("PATCH wrong content type", "PATCH", loc, headers=h(Upload_Offset="5", Content_Type="text/plain"), raw=b"x")
    step("PATCH without offset", "PATCH", loc, headers=h(Content_Type="application/offset+octet-stream"), raw=b"x")
    step("PATCH past the length", "PATCH", loc,
         headers=h(Upload_Offset="5", Content_Type="application/offset+octet-stream"), raw=b"0123456789")
    step("GET unfinished", "GET", loc)
    step("PATCH rest (finish)", "PATCH", loc,
         headers=h(Upload_Offset="5", Content_Type="application/offset+octet-stream"), raw=b" world")
    step("HEAD finished", "HEAD", loc, headers=h())
    step("DELETE finished", "DELETE", loc, headers=h())

    # Termination of an unfinished upload.
    created = step("POST create (to delete)", "POST", "/api/files",
                   headers=h(Upload_Length="3", x_api_key=token, Upload_Metadata="filename " + b64("gone.bin")))
    for br, res in zip((pair.ref, pair.rust), created):
        locs[br.base] = urllib_path(res.header("location"))
    step("DELETE unfinished", "DELETE", loc, headers=h())
    step("HEAD deleted", "HEAD", loc, headers=h())

    # creation-with-upload, partial body.
    created = step("POST with data (partial)", "POST", "/api/files",
                   headers=h(Upload_Length="6", x_api_key=token, Upload_Metadata="filename " + b64("cwu.txt"),
                             Content_Type="application/offset+octet-stream"), raw=b"abc")
    for br, res in zip((pair.ref, pair.rust), created):
        locs[br.base] = urllib_path(res.header("location"))
    step("PATCH finish creation-with-upload", "PATCH", loc,
         headers=h(Upload_Offset="3", Content_Type="application/offset+octet-stream"), raw=b"def")

    # Zero-byte upload: finalized at creation (the reference then 404s).
    step("POST zero length", "POST", "/api/files",
         headers=h(Upload_Length="0", x_api_key=token, Upload_Metadata="filename " + b64("empty.txt")))

    # A browser session instead of a token.
    parity.signed_in(pair, lib.ref_db, lib.rust_db, owner)
    created = step("POST with a session", "POST", "/api/files",
                   headers=h(Upload_Length="4", Upload_Metadata="filename %s,name %s" % (b64(""), b64("named.txt"))))
    for br, res in zip((pair.ref, pair.rust), created):
        locs[br.base] = urllib_path(res.header("location"))
    step("PATCH session upload", "PATCH", loc,
         headers=h(Upload_Offset="0", Content_Type="application/offset+octet-stream"), raw=b"data")

    lib.compare_rows(
        "tus: rows written",
        'SELECT code, filename, ext, kind, mime_type, size, storage_key, checksum IS NULL, published, '
        'folder_id IS NULL FROM uploads WHERE owner_id = %s ORDER BY created_at, code' % parity.sql_quote(owner))
    lib.compare_rows("tus: quota", 'SELECT used_quota FROM "user" WHERE id = %s' % parity.sql_quote(owner))


def urllib_path(location):
    if not location:
        return "/api/files/missing"
    import urllib.parse
    return urllib.parse.urlparse(location).path


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
    for name, fn in [("sharex", sharex), ("tus", tus)]:
        if a.only and a.only != name:
            continue
        fn(run, lib)
    run.finish()


if __name__ == "__main__":
    main()

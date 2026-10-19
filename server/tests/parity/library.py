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


# ── Media metadata at finalize (libvips, ffprobe) ───────────────────────────

SCRATCH = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../../../_scratch/srv-library"))
MEDIA_FILES = [
    ("media/corpus/anim.gif", "image/gif"), ("media/corpus/anim.webp", "image/webp"),
    ("media/corpus/eagle.jpg", "image/jpeg"), ("media/corpus/parrot.png", "image/png"),
    ("media/corpus/parrot.avif", "image/avif"), ("media/corpus/tiny.png", None),
    ("media/corpus/landscape_o6.jpg", "image/jpeg"), ("media/corpus/cactus.heic", "image/heic"),
    ("media/corpus/earth.tiff", "image/tiff"), ("media/corpus/eagle_grey.jpg", "application/octet-stream"),
    ("fixtures/clip.mp4", "video/mp4"), ("fixtures/tone.mp3", "audio/mpeg"),
]


def media_meta(run, lib):
    owner, tag = lib.seed_user("mm", naming_scheme="filename")
    token = lib.seed_token(owner, "upload")
    pair = run.pair()
    for rel, ctype in MEDIA_FILES:
        path = os.path.join(SCRATCH, rel)
        with open(path, "rb") as f:
            data = f.read()
        ct, body = multipart(files=[("file", os.path.basename(rel), ctype, data)])
        a, b = pair.both("POST", "/upload", headers={"x-api-key": token, "Content-Type": ct}, raw=body, origin=False)
        shape = lambda r: {"status": r.status, "body": norm_upload_body(r)}
        run.compare("media meta: upload " + os.path.basename(rel), shape(a), shape(b))
    # One more through tus (finalize from the tus file).
    with open(os.path.join(SCRATCH, "media/corpus/eagle.webp"), "rb") as f:
        data = f.read()
    statuses = []
    for br in (pair.ref, pair.rust):
        meta = "filename %s,filetype %s" % (b64("eagle-tus.webp"), b64("image/webp"))
        res = br.request("POST", "/api/files", headers={"Tus-Resumable": "1.0.0", "x-api-key": token,
                                                         "Upload-Length": str(len(data)), "Upload-Metadata": meta,
                                                         "Content-Type": "application/offset+octet-stream"},
                         raw=data, origin=False)
        # Finished at creation: finalized, then answered like an unknown id
        # (the same upstream quirk as a zero-length upload).
        statuses.append(res.status)
    run.compare("media meta: tus creation-with-upload status", statuses[0], statuses[1])
    # Durations are rounded to 0.1 s: ffprobe 5.1 (the reference image, Debian
    # 12) counts an MP3's encoder padding and ffprobe 9 (Homebrew) does not.
    lib.compare_rows(
        "media meta: rows",
        'SELECT filename, ext, kind, mime_type, size, width, height, blurhash, frames, '
        'round(duration_seconds::numeric, 1), fps FROM uploads WHERE owner_id = %s ORDER BY filename'
        % parity.sql_quote(owner))


# ── Avatars and wallpapers (routes, actions, serving) ──────────────────────

def picture_shape(res, sha=True):
    import hashlib as _h
    out = {"status": res.status}
    for h in ("content-type", "cache-control", "x-content-type-options", "content-disposition"):
        out[h] = res.header(h)
    if res.status == 200 and res.header("content-type") != "application/json":
        # Set by the handler for pictures; for JSON and empty answers Node
        # chunks where hyper counts, which is transport, not the contract.
        out["content-length"] = res.header("content-length")
    if res.header("content-type") == "application/json":
        text = re.sub(r"\?v=[0-9a-f]{16}", "?v=<token>", res.text())
        out["body"] = parity.normalize_json(json.loads(text)) if text else None
    elif sha:
        out["sha256"] = _h.sha256(res.body).hexdigest() if res.body else None
    else:
        out["bytes"] = len(res.body)
    return out


def action_envelope_signed_out(res):
    """A signed-out action POST on the reference never reaches the action:
    proxy.ts answers the page path with a 307 to /login. /_rpc has no page
    path, so the Rust framework answers with the action's own redirect."""
    if res.status == 307 and (res.header("location") or "").split("?")[0].endswith("/login"):
        return {"redirect": "/login"}
    return parity.next_action_envelope(res)


def avatars(run, lib):
    owner, tag = lib.seed_user("av", naming_scheme="filename")
    token = lib.seed_token(owner, "upload")
    pair = run.pair()
    anon = run.pair()
    actions = parity.Actions(run, "modlight-srvlib-ref-app-1")
    corpus = lambda name: open(os.path.join(SCRATCH, "media/corpus", name), "rb").read()

    def both(name, method, path, browser_pair=None, sha=True, **kw):
        a, b = (browser_pair or pair).both(method, path, origin=False, **kw)
        run.compare("pictures: " + name, picture_shape(a, sha), picture_shape(b, sha))
        return a, b

    for route in ("/api/avatar", "/api/background"):
        both("POST %s signed out" % route, "POST", route, browser_pair=anon, raw=b"x",
             headers={"Content-Type": "image/png"})
        both("DELETE %s signed out" % route, "DELETE", route, browser_pair=anon)
    parity.signed_in(pair, lib.ref_db, lib.rust_db, owner)
    for route in ("/api/avatar", "/api/background"):
        both("POST %s wrong type" % route, "POST", route, raw=b"<svg/>", headers={"Content-Type": "image/svg+xml"})
        both("POST %s no type" % route, "POST", route, raw=b"x")
        both("POST %s empty" % route, "POST", route, raw=b"", headers={"Content-Type": "image/png"})
        both("POST %s not an image" % route, "POST", route, raw=b"not a png at all",
             headers={"Content-Type": "image/png; charset=binary"})
    both("GET /avatar before any", "GET", "/avatar/" + tag)
    both("GET /bg before any", "GET", "/bg/" + tag)
    both("POST /api/avatar png", "POST", "/api/avatar", raw=corpus("parrot.png"), headers={"Content-Type": "image/png"})
    lib.compare_rows("pictures: avatar key shape",
                     "SELECT regexp_replace(image, '[0-9a-f]{16}', '<t>') FROM \"user\" WHERE id = %s"
                     % parity.sql_quote(owner))
    both("GET /avatar (no v)", "GET", "/avatar/" + tag)
    keys = lib.both_sql('SELECT image FROM "user" WHERE id = %s' % parity.sql_quote(owner))
    toks = [k.strip().rsplit("-", 1)[-1].replace(".webp", "") for k in keys]
    for br, tok in zip((pair.ref, pair.rust), toks):
        br.v = tok
    a = pair.ref.request("GET", "/avatar/%s?v=%s" % (tag, toks[0]), origin=False)
    b = pair.rust.request("GET", "/avatar/%s?v=%s" % (tag, toks[1]), origin=False)
    run.compare("pictures: GET /avatar?v=<own token>", picture_shape(a), picture_shape(b))
    both("GET /avatar?v=wrong", "GET", "/avatar/%s?v=nope" % tag)
    both("GET /avatar unknown tag", "GET", "/avatar/nobody-%s" % lib.suffix)
    both("POST /api/avatar animated gif", "POST", "/api/avatar", raw=corpus("anim.gif"),
         headers={"Content-Type": "image/gif"})
    both("GET /avatar animated", "GET", "/avatar/" + tag)
    both("POST /api/avatar jpeg (orientation 6)", "POST", "/api/avatar", raw=corpus("landscape_o6.jpg"),
         headers={"Content-Type": "image/jpeg"})
    both("GET /avatar rotated", "GET", "/avatar/" + tag)
    lib.both_sql("INSERT INTO user_code_aliases (code, user_id) VALUES (%s, %s)" % (
        parity.sql_quote("old" + tag), parity.sql_quote(owner)))
    both("GET /avatar via an alias", "GET", "/avatar/old" + tag)
    both("POST /api/background jpeg", "POST", "/api/background", raw=corpus("eagle_big.jpg"),
         headers={"Content-Type": "image/jpeg"})
    both("GET /bg", "GET", "/bg/" + tag)
    both("GET /bg via an alias", "GET", "/bg/old" + tag)
    both("POST /api/background png", "POST", "/api/background", raw=corpus("parrot.png"),
         headers={"Content-Type": "image/png"})
    both("GET /bg after replacing", "GET", "/bg/" + tag)

    # The picker and the actions.
    files = [("parrot.png", "image/png"), ("anim.gif", "image/gif"), ("eagle.jpg", "image/jpeg"),
             ("pic.avif", "application/octet-stream"), ("logo.svg", "image/svg+xml")]
    for name, ctype in files:
        data = corpus("parrot.avif") if name == "pic.avif" else (
            b'<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>' if name == "logo.svg" else corpus(name))
        ct, body = multipart(files=[("file", name, ctype, data)])
        pair.both("POST", "/upload", headers={"x-api-key": token, "Content-Type": ct}, raw=body, origin=False)
    ids = {}
    for name, _ in files:
        a, b = lib.both_sql("SELECT id FROM uploads WHERE owner_id = %s AND filename = %s" % (
            parity.sql_quote(owner), parity.sql_quote(name)))
        ids[name] = (a.strip(), b.strip())
    big = seed_upload(lib, owner, "big" + lib.suffix, size=9 * 1024 * 1024, filename="big.png")
    ids["big"] = (big, big)
    gone = seed_upload(lib, owner, "gone" + lib.suffix, filename="gone.png")
    ids["gone"] = (gone, gone)

    def act(name, export, args_for):
        a = actions.call_ref(pair.ref, "avatar", export, args_for(0))
        b = actions.call_rust(pair.rust, "avatar", export, args_for(1))
        norm = lambda env: json.loads(re.sub(r"\?v=[0-9a-f]{16}", "?v=<token>", json.dumps(env)))
        ea = parity.normalize_json(norm(parity.next_action_envelope(a)))
        eb = parity.normalize_json(norm(parity.rpc_envelope(b)))
        run.compare("pictures: action " + name, ea, eb)

    # myAvatar is in CONTRACT 7.3 but no client component of the reference
    # imports it, so its build does not expose it: not comparable here.
    act("listAvatarPicks()", "listAvatarPicks", lambda i: [])
    act("listAvatarPicks(search)", "listAvatarPicks", lambda i: [{"search": "  parrot "}])
    act("listAvatarPicks(page 2)", "listAvatarPicks", lambda i: [{"page": 2}])
    act("listAvatarPicks(null)", "listAvatarPicks", lambda i: [None])
    act("setAvatarFromUpload(png)", "setAvatarFromUpload", lambda i: [ids["parrot.png"][i]])
    act("setAvatarFromUpload(gif)", "setAvatarFromUpload", lambda i: [ids["anim.gif"][i]])
    act("setAvatarFromUpload(avif as octet-stream)", "setAvatarFromUpload", lambda i: [ids["pic.avif"][i]])
    act("setAvatarFromUpload(svg)", "setAvatarFromUpload", lambda i: [ids["logo.svg"][i]])
    act("setAvatarFromUpload(too large)", "setAvatarFromUpload", lambda i: [ids["big"][i]])
    act("setAvatarFromUpload(bytes missing)", "setAvatarFromUpload", lambda i: [ids["gone"][i]])
    act("setAvatarFromUpload(not a uuid)", "setAvatarFromUpload", lambda i: ["nope"])
    act("setAvatarFromUpload(unknown)", "setAvatarFromUpload", lambda i: [str(uuid.uuid4())])
    act("setAvatarFromUpload(number)", "setAvatarFromUpload", lambda i: [5])
    both("GET /avatar after the action", "GET", "/avatar/" + tag)

    both("DELETE /api/avatar", "DELETE", "/api/avatar")
    both("GET /avatar after DELETE", "GET", "/avatar/" + tag)
    both("DELETE /api/background", "DELETE", "/api/background")
    both("GET /bg after DELETE", "GET", "/bg/" + tag)
    lib.compare_rows("pictures: columns after", 'SELECT image IS NULL, background_key IS NULL FROM "user" WHERE id = %s'
                     % parity.sql_quote(owner))
    a = actions.call_ref(anon.ref, "avatar", "listAvatarPicks", [])
    b = actions.call_rust(anon.rust, "avatar", "listAvatarPicks", [])
    run.compare("pictures: action listAvatarPicks signed out", action_envelope_signed_out(a), parity.rpc_envelope(b))


# ── /api/uploads (GET, PATCH, DELETE) ──────────────────────────────────────


def seed_upload(lib, owner, code, **cols):
    """The same upload row (same id) on both servers; returns its id."""
    row = {
        "id": str(uuid.uuid4()), "owner_id": owner, "code": code, "filename": code + ".png", "ext": "png",
        "kind": "file", "mime_type": "image/png", "size": 1000, "storage_key": "%s/%s.png" % (owner, code),
        "delete_token": "tok-" + code, "created_at": "2026-01-01T00:00:00.000Z",
    }
    row.update(cols)
    names = ", ".join('"%s"' % k for k in row)
    values = ", ".join(parity.sql_quote(v) for v in row.values())
    lib.both_sql('INSERT INTO uploads (%s) VALUES (%s)' % (names, values))
    return row["id"]


def seed_folder(lib, owner, name, parent=None):
    fid = str(uuid.uuid4())
    lib.both_sql('INSERT INTO folders (id, owner_id, parent_id, name, slug) VALUES (%s, %s, %s, %s, %s)' % (
        parity.sql_quote(fid), parity.sql_quote(owner), parity.sql_quote(parent), parity.sql_quote(name),
        parity.sql_quote(name.lower())))
    return fid


def uploads_api(run, lib):
    pair = run.pair()
    owner, tag = lib.seed_user("ua")
    full = lib.seed_token(owner, "full")
    reader = lib.seed_token(owner, "read")
    uploader = lib.seed_token(owner, "upload")
    other, other_tag = lib.seed_user("ub", copy_raw=True, skip_tag_in_url=True)
    other_full = lib.seed_token(other, "full")
    sfx = lib.suffix
    folder = seed_folder(lib, owner, "Shots")
    foreign_folder = seed_folder(lib, other, "Theirs")
    t = lambda n: "2026-01-01T00:00:%02d.000Z" % n
    ids = {}
    ids["png"] = seed_upload(lib, owner, "pa" + sfx, created_at=t(1), width=640, height=480, blurhash="LEHV6nWB2yk8")
    ids["gif"] = seed_upload(lib, owner, "pb" + sfx, filename="anim.gif", ext="gif", mime_type="image/gif",
                             frames=12, size=3000, created_at=t(2), folder_id=folder)
    ids["mp4"] = seed_upload(lib, owner, "pc" + sfx, filename="clip.mp4", ext="mp4", mime_type="video/mp4",
                             size=5000000, created_at=t(3), duration_seconds=4.5, fps=30)
    ids["svg"] = seed_upload(lib, owner, "pd" + sfx, filename="logo.svg", ext="svg", mime_type="image/svg+xml",
                             created_at=t(4))
    ids["txt"] = seed_upload(lib, owner, "pe" + sfx, filename="notes.txt", ext="txt", mime_type="text/plain",
                             created_at=t(5), expires_at="2030-01-01T00:00:00.000Z", max_views=3,
                             password_hash="x:y")
    ids["big"] = seed_upload(lib, owner, "pf" + sfx, filename="huge.jpg", ext="jpg", mime_type="image/jpeg",
                             size=40 * 1024 * 1024, created_at=t(6))
    ids["oct"] = seed_upload(lib, owner, "pg" + sfx, filename="pic.avif", ext="avif",
                             mime_type="application/octet-stream", created_at=t(7), folder_id=folder)
    ids["trash"] = seed_upload(lib, owner, "ph" + sfx, created_at=t(8), deleted_at="2026-02-01T00:00:00.000Z")
    ids["trash2"] = seed_upload(lib, owner, "pi" + sfx, created_at=t(9), deleted_at="2026-02-02T00:00:00.000Z")
    ids["theirs"] = seed_upload(lib, other, "pj" + sfx, created_at=t(10))
    ids["theirs_trash"] = seed_upload(lib, other, "pk" + sfx, created_at=t(11), deleted_at="2026-02-01T00:00:00.000Z")
    ids["lone"] = seed_upload(lib, owner, "pl" + sfx, created_at=t(12))

    names = dict((v, "<%s>" % k) for k, v in ids.items())
    names[folder] = "<folder>"
    names[foreign_folder] = "<foreign-folder>"

    def label(v):
        """Seeded ids by name, then the usual normalisation."""
        text = json.dumps(v, ensure_ascii=False)
        for uid, name in names.items():
            text = text.replace(uid, name)
        text = re.sub(ORIGIN_RE, "<origin>", text)
        text = text.replace(sfx, "<sfx>")
        return parity.normalize_json(json.loads(text))

    def shape(r, unordered=False):
        body = r.json()
        if unordered and isinstance(body, dict) and isinstance(body.get("items"), list):
            # actionableUploads has no ORDER BY: rows come back in whatever
            # order Postgres finds them, which differs between two databases.
            body["items"] = sorted(body["items"], key=lambda i: i.get("id") or "")
        return {"status": r.status, "content-type": r.header("content-type"),
                "cache-control": r.header("cache-control"), "retry-after": r.header("retry-after"),
                "body": label(body) if body is not None else parity.normalize_text(r.text())}

    def call(name, method, path, token=full, body=None, raw=None, ctype=None):
        h = {}
        if token:
            h["x-api-key"] = token
        if ctype:
            h["Content-Type"] = ctype
        kw = {"headers": h, "origin": False}
        if body is not None:
            kw["json_body"] = body
        elif raw is not None:
            kw["raw"] = raw
        a, b = pair.both(method, path, **kw)
        run.compare("uploads api: " + name, shape(a, method == "PATCH"), shape(b, method == "PATCH"))
        return a, b

    def rows(name):
        lib.compare_rows(
            "uploads api rows: " + name,
            'SELECT code, published, folder_id IS NULL, expires_at, max_views, password_hash IS NULL, '
            'password_version, deleted_at IS NULL FROM uploads WHERE owner_id IN (%s, %s) ORDER BY created_at'
            % (parity.sql_quote(owner), parity.sql_quote(other)))

    # GET
    call("GET without auth", "GET", "/api/uploads", token=None)
    call("GET unknown token", "GET", "/api/uploads", token="nope")
    call("GET upload-scoped token", "GET", "/api/uploads", token=uploader)
    call("GET read token", "GET", "/api/uploads", token=reader)
    for q in ("?perPage=2&page=2", "?sort=oldest", "?sort=largest&perPage=3", "?sort=name", "?kind=image",
              "?kind=video", "?search=clip", "?trashed=1", "?trashed=true", "?folder=root", "?folder=" + folder,
              "?folder=" + foreign_folder, "?folder=" + str(uuid.uuid4()), "?folder=nope", "?folder=",
              "?page=0&perPage=-5", "?page=abc", "?perPage=1000", "?sort=bogus&kind=bogus", "?q=x&search=%20%20",
              "?search=NOTES", "?folder=ROOT", "?folder=" + folder.upper()):
        call("GET " + q.replace(folder, "<folder>").replace(foreign_folder, "<foreign>"), "GET", "/api/uploads" + q)
    call("GET as the other account (copy_raw, tagless)", "GET", "/api/uploads", token=other_full)
    call("POST is not a method", "POST", "/api/uploads", body={})

    # PATCH: refusals
    call("PATCH without auth", "PATCH", "/api/uploads", token=None, body={"ids": [ids["png"]], "published": False})
    call("PATCH read token", "PATCH", "/api/uploads", token=reader, body={"ids": [ids["png"]], "published": False})
    call("PATCH text/plain", "PATCH", "/api/uploads", raw=b'{"ids":[]}', ctype="text/plain")
    call("PATCH array body", "PATCH", "/api/uploads", raw=b'[1]', ctype="application/json")
    call("PATCH broken json", "PATCH", "/api/uploads", raw=b'{"ids":', ctype="application/json")
    call("PATCH no ids", "PATCH", "/api/uploads", body={"published": False})
    call("PATCH empty-string ids", "PATCH", "/api/uploads", body={"ids": ["", 5, None], "published": False})
    call("PATCH too many ids", "PATCH", "/api/uploads", body={"ids": ["x%d" % i for i in range(501)],
                                                              "published": False})
    call("PATCH nothing to change", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "name": "x"})
    call("PATCH folderId number", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "folderId": 5})
    call("PATCH published string", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "published": "yes"})
    call("PATCH code number", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "code": 5})
    call("PATCH code with two ids", "PATCH", "/api/uploads", body={"ids": [ids["png"], ids["gif"]], "code": "x"})
    call("PATCH expiresAt number", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "expiresAt": 5})
    call("PATCH maxViews string", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "maxViews": "5"})
    call("PATCH password number", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "password": 5})
    call("PATCH later field malformed writes nothing", "PATCH", "/api/uploads",
         body={"ids": [ids["png"]], "published": False, "password": 5})
    rows("after refusals")

    # PATCH: writes
    call("PATCH unpublish two", "PATCH", "/api/uploads", body={"ids": [ids["png"], ids["gif"]], "published": False})
    call("PATCH publish again", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "published": True})
    call("PATCH move to folder", "PATCH", "/api/uploads", body={"ids": [ids["png"], ids["mp4"]], "folderId": folder})
    call("PATCH move to root", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "folderId": "root"})
    call("PATCH move to empty string", "PATCH", "/api/uploads", body={"ids": [ids["mp4"]], "folderId": ""})
    call("PATCH move to null", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "folderId": None})
    call("PATCH move to foreign folder", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "folderId": foreign_folder})
    call("PATCH move to non-uuid folder", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "folderId": "abc"})
    call("PATCH move non-uuid ids to bad folder", "PATCH", "/api/uploads", body={"ids": ["zzz"], "folderId": "abc"})
    call("PATCH expiry", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "expiresAt": "2031-05-06T07:08:09Z"})
    call("PATCH expiry unreadable", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "expiresAt": "soon"})
    call("PATCH expiry date only", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "expiresAt": "2031-05-06"})
    call("PATCH expiry cleared", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "expiresAt": None})
    call("PATCH view cap", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "maxViews": 7.9})
    call("PATCH view cap floored at one", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "maxViews": -3})
    call("PATCH view cap cleared", "PATCH", "/api/uploads", body={"ids": [ids["txt"]], "maxViews": None})
    call("PATCH password set", "PATCH", "/api/uploads", body={"ids": [ids["png"], ids["mp4"]], "password": "hunter2"})
    call("PATCH password cleared", "PATCH", "/api/uploads", body={"ids": [ids["txt"]], "password": None})
    call("PATCH password empty clears", "PATCH", "/api/uploads", body={"ids": [ids["mp4"]], "password": ""})
    call("PATCH password too long", "PATCH", "/api/uploads", body={"ids": [ids["mp4"]], "password": "x" * 1025})
    call("PATCH code", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "code": "My New Code " + sfx})
    call("PATCH code it already has", "PATCH", "/api/uploads", body={"ids": [ids["png"]], "code": "my-new-code-" + sfx})
    call("PATCH code taken by another account", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "code": "pj" + sfx})
    call("PATCH code reserved", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "code": "login"})
    call("PATCH code empty after slug", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "code": "!!!"})
    call("PATCH code too long", "PATCH", "/api/uploads", body={"ids": [ids["gif"]], "code": "a" * 65})
    call("PATCH code on a foreign file", "PATCH", "/api/uploads", body={"ids": [ids["theirs"]], "code": "x" + sfx})
    call("PATCH code on an unknown id", "PATCH", "/api/uploads", body={"ids": [str(uuid.uuid4())], "code": "y" + sfx})
    call("PATCH code on a non-uuid", "PATCH", "/api/uploads", body={"ids": ["nope"], "code": "z" + sfx})
    call("PATCH foreign and non-uuid ids", "PATCH", "/api/uploads",
         body={"ids": [ids["theirs"], "nope", ids["svg"]], "published": False})
    call("PATCH trashed row (no links)", "PATCH", "/api/uploads", body={"ids": [ids["trash"]], "published": False})
    call("PATCH combined", "PATCH", "/api/uploads",
         body={"ids": [ids["oct"]], "published": False, "expiresAt": "2032-01-01T00:00:00.000Z", "maxViews": 2,
               "password": "pw", "code": "Combined " + sfx, "folderId": "root"})
    rows("after writes")

    # DELETE
    call("DELETE without auth", "DELETE", "/api/uploads", token=None, body={"ids": [ids["svg"]]})
    call("DELETE read token", "DELETE", "/api/uploads", token=reader, body={"ids": [ids["svg"]]})
    call("DELETE no body", "DELETE", "/api/uploads")
    call("DELETE broken json", "DELETE", "/api/uploads", raw=b"{", ctype="application/json")
    call("DELETE text body without content type", "DELETE", "/api/uploads", raw=('{"ids":["%s"],"mode":"bogus"}'
                                                                                % ids["svg"]).encode())
    rows("after a text-typed delete")
    call("DELETE null body", "DELETE", "/api/uploads", raw=b"null", ctype="application/json")
    call("DELETE number body", "DELETE", "/api/uploads", raw=b"5", ctype="application/json")
    call("DELETE non-uuid ids", "DELETE", "/api/uploads", body={"ids": ["nope", 5]})
    call("DELETE too many", "DELETE", "/api/uploads", body={"ids": [str(uuid.uuid4()) for _ in range(501)]})
    call("DELETE trash already trashed", "DELETE", "/api/uploads", body={"ids": [ids["trash"]]})
    call("DELETE restore live and trashed", "DELETE", "/api/uploads",
         body={"ids": [ids["trash"], ids["lone"]], "mode": "restore"})
    call("DELETE trash live", "DELETE", "/api/uploads", body={"ids": [ids["lone"], ids["lone"]], "mode": "trash"})
    call("DELETE foreign", "DELETE", "/api/uploads",
         body={"ids": [ids["theirs"], ids["theirs_trash"]], "mode": "permanent"})
    call("DELETE permanent trashed", "DELETE", "/api/uploads", body={"ids": [ids["trash2"]], "mode": "permanent"})
    call("DELETE permanent live (uppercase id)", "DELETE", "/api/uploads",
         body={"ids": [ids["big"].upper()], "mode": "permanent"})
    rows("after deletes")

    # The password limiter: 30 per minute per account, then 429 with retry-after.
    for i in range(31):
        a, b = pair.both("PATCH", "/api/uploads", headers={"x-api-key": other_full}, origin=False,
                         json_body={"ids": [ids["theirs"]], "password": "p%d" % i})
    run.compare("uploads api: password limiter", shape(a), shape(b))

    # A browser session instead of a token; a banned account's session.
    parity.signed_in(pair, lib.ref_db, lib.rust_db, owner)
    call("GET with a session", "GET", "/api/uploads?perPage=3", token=None)
    call("PATCH with a session", "PATCH", "/api/uploads", token=None, body={"ids": [ids["svg"]], "published": True})
    call("GET with a session and a bad token", "GET", "/api/uploads", token="nope")
    lib.both_sql('UPDATE "user" SET banned = true WHERE id = %s' % parity.sql_quote(owner))
    call("GET with a banned session", "GET", "/api/uploads", token=None)
    call("GET with a banned account's token", "GET", "/api/uploads", token=full)
    lib.both_sql('UPDATE "user" SET banned = false WHERE id = %s' % parity.sql_quote(owner))


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
    for name, fn in [("sharex", sharex), ("tus", tus), ("uploads_api", uploads_api), ("media_meta", media_meta), ("avatars", avatars)]:
        if a.only and a.only != name:
            continue
        fn(run, lib)
    run.finish()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""End-to-end check of docx_parity.py's reference side: the same documents
fetched through the reference server's real route,
GET /{tag}/{code}/render?t=<ticket> with `Sec-Fetch-Dest: iframe`, must give
the bytes docxToHtml gave in reference.json (and so the Rust output).

    python3 server/tests/parity_media/docx_route_e2e.py --corpus <dir> --work <dir> \
        --base http://127.0.0.1:8446 --app modlight-srvmedia-a-ref-app-1 \
        --db modlight-srvmedia-a-ref-postgres-1 name.docx ...

Seeds one user and one upload per document into the (private) reference
database, copies the files into its local storage, mints render tickets inside
the app container with its own AUTH_SECRET (never printed), and removes the
rows and files again at the end.
"""
import argparse
import io
import json
import os
import secrets
import subprocess
import tarfile
import urllib.request

DOCX_MIME = "application/vnd.openxmlformats-officedocument.wordprocessingml.document"


def docker(*args, **kw):
    env = dict(os.environ, DOCKER_CONTEXT=os.environ.get("DOCKER_CONTEXT", "colima-modlight"))
    return subprocess.run(["docker", *args], env=env, check=True, **kw)


def psql(db, sql):
    out = docker("exec", "-i", db, "sh", "-c", 'psql -U "$POSTGRES_USER" -d modlight -tA -v ON_ERROR_STOP=1',
                 input=sql.encode(), capture_output=True).stdout.decode()
    return [line for line in out.splitlines() if line]


def q(s):
    return "'" + s.replace("'", "''") + "'"


def wire(s):
    return s.encode("utf-16", "surrogatepass").decode("utf-16", "replace")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--base", default="http://127.0.0.1:8446")
    ap.add_argument("--app", default="modlight-srvmedia-a-ref-app-1")
    ap.add_argument("--db", default="modlight-srvmedia-a-ref-postgres-1")
    ap.add_argument("names", nargs="+")
    args = ap.parse_args()

    ref = json.load(open(os.path.join(args.work, "reference.json")))
    rust = json.load(open(os.path.join(args.work, "rust.json")))
    run = secrets.token_hex(4)
    user_id = "docxe2e" + run
    tag = "dx" + run
    psql(args.db, 'INSERT INTO "user" (id, name, email, email_verified, role, banned, two_factor_enabled, user_code, '
                  'used_quota, max_quota, hide_uploads, copy_raw, theme, created_at, updated_at) VALUES '
                  f"({q(user_id)}, 'Docx E2E', {q(user_id + '@e2e.test')}, true, 'user', false, false, {q(tag)}, "
                  "0, -1, false, false, 'modern', now(), now())")
    uploads = {}
    tar_buf = io.BytesIO()
    try:
        with tarfile.open(fileobj=tar_buf, mode="w") as tar:
            for i, name in enumerate(args.names):
                data = open(os.path.join(args.corpus, name), "rb").read()
                code = f"dx{run}{i}"
                key = f"docx-e2e/{run}/{i}.docx"
                info = tarfile.TarInfo(key)
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
                upload_id = psql(args.db, 'INSERT INTO uploads (owner_id, code, filename, ext, mime_type, size, '
                                          f'storage_key, delete_token) VALUES ({q(user_id)}, {q(code)}, '
                                          f"{q('doc.docx')}, 'docx', {q(DOCX_MIME)}, {len(data)}, {q(key)}, "
                                          f"{q('dt' + code)}) RETURNING id")[0]
                uploads[name] = (upload_id, code)
        docker("exec", "-i", args.app, "sh", "-c", "tar x -C /app/data/storage", input=tar_buf.getvalue())
        ids = " ".join(u for u, _ in uploads.values())
        mint = ("const {createHmac}=require('node:crypto');const exp=Math.floor(Date.now()/1000)+900;"
                "for (const id of process.argv.slice(1)) { const m=['render',id,String(exp)].map(encodeURIComponent).join('|');"
                "console.log(id+'.'+exp+'.'+createHmac('sha256',process.env.AUTH_SECRET).update(m).digest('base64url')); }")
        tickets = docker("exec", args.app, "node", "-e", mint, *ids.split(), capture_output=True).stdout.decode().split()
        ok = bad = 0
        for (name, (upload_id, code)), ticket in zip(uploads.items(), tickets):
            assert ticket.startswith(upload_id)
            req = urllib.request.Request(f"{args.base}/{tag}/{code}/render?t={ticket}",
                                         headers={"Sec-Fetch-Dest": "iframe"})
            try:
                with urllib.request.urlopen(req) as resp:
                    status, body, refused = resp.status, resp.read().decode(), None
            except urllib.error.HTTPError as e:
                status, body, refused = e.code, e.read().decode(), e.headers.get("X-Preview-Refused")
            r, s = ref[name], rust[name]
            if r.get("ok"):
                same = status == 200 and body == wire(r["html"]) and body == s.get("html")
            else:
                same = refused == r.get("reason") == s.get("reason") and status in (413, 415)
            ok += same
            bad += not same
            print(("SAME     " if same else "DIFFERENT"), name, status, refused or "")
        print("ROUTE", json.dumps({"same": ok, "different": bad}))
    finally:
        psql(args.db, f"DELETE FROM uploads WHERE owner_id = {q(user_id)}; DELETE FROM \"user\" WHERE id = {q(user_id)}")
        docker("exec", args.app, "sh", "-c", f"rm -rf /app/data/storage/docx-e2e/{run}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Parity of the .docx preview: upstream docxToHtml (in the reference container)
against the Rust docx_to_html, document by document.

    python3 server/tests/parity_media/docx_parity.py \
        --corpus _scratch/srv-media-a/docx/corpus --work _scratch/srv-media-a/docx/run \
        --container modlight-srvmedia-a-ref-app-1

Every document runs in a node process of its own (iso-* ones pollute the
process; iso-hang-* never finish upstream: after --timeout seconds the process
is killed by its PID and the Rust side is not run on them). Output: one line per document and a summary; the JSON of both sides and
the diffs are kept in --work.

Comparison: the HTML must be identical once the embedded WebP data URIs are
replaced by their decoded size. The pictures themselves are compared apart
(dimensions, then a pixel difference through the `vips` CLI), because the
reference encodes with sharp-libvips 8.17.3 and the port with the libvips the
host has.
"""
import argparse
import base64
import difflib
import io
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))

# Documents where the port deliberately or unavoidably answers differently
# (server/docs/SKELETON.md, "Document preview (docx)").
EXPECTED = {
    "iso-proto-pollution.docx": "upstream writes the style onto Object.prototype (prototype pollution); not reproduced",
    "x-deep-tables-400.docx": "upstream runs out of V8 stack at 307 nested tables; the port only bounds XML depth",
    "x-deep-sdt-920.docx": "upstream runs out of V8 stack at 910 nested w:sdt; the port only bounds XML depth",
}
SERVER = os.path.dirname(os.path.dirname(HERE))
REPO = os.path.dirname(SERVER)


def upstream_root():
    # The worktree lives in <modlight>/_wt/<name>; upstream in <modlight>/upstream.
    d = REPO
    while d != "/":
        cand = os.path.join(d, "upstream", "Modlight")
        if os.path.isdir(cand):
            return cand
        d = os.path.dirname(d)
    sys.exit("upstream/Modlight not found")


PNPM = "/app/node_modules/.pnpm"
REWRITES = [
    ('import "server-only";\n', ""),
    ('from "mammoth";', f'from "{PNPM}/mammoth@1.12.1/node_modules/mammoth/lib/index.js";'),
    ('from "sharp";', f'from "{PNPM}/sharp@0.34.5/node_modules/sharp/lib/index.js";'),
    ('from "@/lib/media/sharp-runtime";', 'from "./sharp-runtime.ts";'),
    ('from "@/lib/media/zip-guard";', 'from "./zip-guard.ts";'),
    ('from "@/lib/media/docx-support";', 'from "./docx-support.ts";'),
    ('from "@/lib/runtime/limits";', 'from "./limits.ts";'),
]
FILES = ["lib/media/docx.ts", "lib/media/docx-support.ts", "lib/media/zip-guard.ts",
         "lib/media/sharp-runtime.ts", "lib/runtime/limits.ts"]


def staged_upstream():
    root = upstream_root()
    out = {}
    for f in FILES:
        src = open(os.path.join(root, f)).read()
        for a, b in REWRITES:
            src = src.replace(a, b)
        assert "@/" not in src, f"unrewritten import in {f}"
        out["upstream/" + os.path.basename(f)] = src.encode()
    out["docx_reference.mjs"] = open(os.path.join(HERE, "docx_reference.mjs"), "rb").read()
    return out


def docker(*args, **kw):
    env = dict(os.environ, DOCKER_CONTEXT=os.environ.get("DOCKER_CONTEXT", "colima-modlight"))
    return subprocess.run(["docker", *args], env=env, **kw)


def ship(container, corpus, names):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tar:
        for path, data in staged_upstream().items():
            info = tarfile.TarInfo(path)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        for n in names:
            tar.add(os.path.join(corpus, n), arcname="corpus/" + n)
    docker("exec", "-i", container, "sh", "-c",
           "rm -rf /tmp/docx-parity && mkdir -p /tmp/docx-parity && tar x -C /tmp/docx-parity",
           input=buf.getvalue(), check=True)


def run_reference(container, name, timeout):
    """One node process for one document: its result, or {"timeout": true}
    (the process is then killed by the PID it wrote)."""
    pidfile = "/tmp/docx-parity/pids/" + name + ".pid"
    cmd = f"mkdir -p /tmp/docx-parity/pids; echo $$ > '{pidfile}'; exec node --no-warnings " \
          f"/tmp/docx-parity/docx_reference.mjs /tmp/docx-parity/corpus '{name}'"
    try:
        p = docker("exec", container, "sh", "-c", cmd, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        pid = docker("exec", container, "cat", pidfile, capture_output=True, text=True).stdout.strip()
        if pid.isdigit():
            docker("exec", container, "sh", "-c", f"kill -9 {pid}", capture_output=True)
        return {"timeout": True}
    if p.returncode != 0:
        return {"crashed": p.stderr.decode(errors="replace")[-2000:]}
    return json.loads(p.stdout)[name]


def run_rust(corpus, out_path):
    image = os.environ.get("DOCX_RUST_IMAGE")
    if image:
        # The production image (libvips-cpp 8.17.3) with the media_docx test
        # binary added (_scratch/srv-integration/parity/probe/Dockerfile).
        root = "/path/to/modlight"
        p = docker("run", "--rm", "--network", "none", "--cpus", "2", "-u", "%d:%d" % (os.getuid(), os.getgid()),
                   "-v", "%s:%s" % (root, root), "-e", "DOCX_CORPUS=" + corpus, "-e", "DOCX_OUT=" + out_path,
                   "--entrypoint", "/usr/local/bin/media_docx_test", image, "parity_dump", "--ignored",
                   capture_output=True, text=True)
        if p.returncode != 0:
            sys.stderr.write(p.stdout[-4000:] + p.stderr[-4000:])
            raise SystemExit("rust run failed in " + image)
        return json.load(open(out_path))
    env = dict(os.environ, DOCX_CORPUS=corpus, DOCX_OUT=out_path)
    env["PATH"] = "/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:" + env.get("PATH", "")
    p = subprocess.run(["cargo", "test", "-q", "--test", "media_docx", "parity_dump", "--", "--ignored"],
                       cwd=SERVER, env=env, capture_output=True, text=True)
    if p.returncode != 0:
        sys.stderr.write(p.stdout[-4000:] + p.stderr[-4000:])
        raise SystemExit("rust run failed")
    return json.load(open(out_path))


def wire(s):
    """What the HTML is on the wire: lone surrogates become U+FFFD."""
    return s.encode("utf-16", "surrogatepass").decode("utf-16", "replace")


DATA_URI = re.compile(r"data:image/webp;base64,([A-Za-z0-9+/=]*)")


def webp_size(data):
    if data[:4] != b"RIFF" or data[8:12] != b"WEBP":
        return None
    chunk = data[12:16]
    if chunk == b"VP8X":
        w = 1 + int.from_bytes(data[24:27], "little")
        h = 1 + int.from_bytes(data[27:30], "little")
        return w, h
    if chunk == b"VP8 ":
        w, h = struct.unpack("<HH", data[26:30])
        return w & 0x3FFF, h & 0x3FFF
    if chunk == b"VP8L":
        b = int.from_bytes(data[21:25], "little")
        return (b & 0x3FFF) + 1, ((b >> 14) & 0x3FFF) + 1
    return None


def normalise(html):
    imgs = []

    def sub(m):
        raw = base64.b64decode(m.group(1))
        imgs.append(raw)
        size = webp_size(raw)
        return "data:image/webp;base64,<webp %s>" % ("%dx%d" % size if size else "?")

    return DATA_URI.sub(sub, html), imgs


def pixel_diff(a, b, tmp):
    """(mean, max) absolute difference per band, through the vips CLI."""
    pa, pb = os.path.join(tmp, "a.webp"), os.path.join(tmp, "b.webp")
    open(pa, "wb").write(a)
    open(pb, "wb").write(b)
    d = os.path.join(tmp, "d.v")
    try:
        subprocess.run(["vips", "subtract", pa, pb, d], check=True, capture_output=True)
        subprocess.run(["vips", "abs", d, os.path.join(tmp, "ad.v")], check=True, capture_output=True)
        mean = float(subprocess.run(["vips", "avg", os.path.join(tmp, "ad.v")], check=True,
                                    capture_output=True, text=True).stdout)
        mx = float(subprocess.run(["vips", "max", os.path.join(tmp, "ad.v")], check=True,
                                  capture_output=True, text=True).stdout)
        return mean, mx
    except (subprocess.CalledProcessError, FileNotFoundError, ValueError):
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--container", default="modlight-srvmedia-a-ref-app-1")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--only", nargs="*")
    ap.add_argument("--skip-reference", action="store_true", help="reuse work/reference.json")
    args = ap.parse_args()

    corpus = os.path.abspath(args.corpus)
    args.work = os.path.abspath(args.work)
    os.makedirs(args.work, exist_ok=True)
    names = sorted(n for n in os.listdir(corpus) if not n.startswith("."))
    if args.only:
        names = [n for n in names if any(re.search(o, n) for o in args.only)]

    ref_path = os.path.join(args.work, "reference.json")
    if args.skip_reference and os.path.exists(ref_path):
        ref = json.load(open(ref_path))
    else:
        ship(args.container, corpus, names)
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(4) as pool:
            results = pool.map(lambda n: (n, run_reference(args.container, n, args.timeout)), names)
            ref = dict(results)
        json.dump(ref, open(ref_path, "w"))

    rust_dir = os.path.join(args.work, "rust-corpus")
    shutil.rmtree(rust_dir, ignore_errors=True)
    os.makedirs(rust_dir)
    for n in names:
        os.symlink(os.path.join(corpus, n), os.path.join(rust_dir, n))
    rust = run_rust(rust_dir, os.path.join(args.work, "rust.json"))

    counts = {"identical": 0, "images-only": 0, "differs": 0, "expected": 0, "upstream-hangs": 0}
    diffs = []
    with tempfile.TemporaryDirectory() as tmp:
        for n in names:
            r, s = ref.get(n), rust.get(n)
            if s is None:
                print(f"SKIP      {n}: {json.dumps(r)[:120]}")
                continue
            if r.get("timeout"):
                counts["upstream-hangs"] += 1
                print(f"HANGS     {n}: upstream never finishes; rust {json.dumps({k: v for k, v in s.items() if k != 'html'})}")
                continue
            r_html, s_html = wire(r.get("html", "")), s.get("html", "")
            r_meta = {k: r.get(k) for k in ("ok", "reason", "images", "imagesDropped", "timeout", "threw", "crashed")}
            s_meta = {k: s.get(k) for k in ("ok", "reason", "images", "imagesDropped", "timeout", "threw", "crashed")}
            if r_meta == s_meta and r_html == s_html:
                counts["identical"] += 1
                print(f"IDENTICAL {n} {json.dumps({k: v for k, v in r_meta.items() if v is not None})}")
                continue
            rn, r_imgs = normalise(r_html)
            sn, s_imgs = normalise(s_html)
            if r_meta == s_meta and rn == sn:
                counts["images-only"] += 1
                stats = []
                for a, b in zip(r_imgs, s_imgs):
                    if a == b:
                        stats.append("same bytes")
                    else:
                        pd = pixel_diff(a, b, tmp)
                        stats.append("%s, %d vs %d bytes, mean|max diff %s" % (
                            webp_size(a), len(a), len(b), "%.3f|%.0f" % pd if pd else "?"))
                print(f"IMAGES    {n}: " + "; ".join(stats))
                continue
            if n in EXPECTED:
                counts["expected"] += 1
                print(f"EXPECTED  {n}: {EXPECTED[n]}")
            else:
                counts["differs"] += 1
                print(f"DIFFERS   {n}: ref {json.dumps(r_meta)} rust {json.dumps(s_meta)}")
            ud = "\n".join(list(difflib.unified_diff(
                re.sub(r"(<[^/])", r"\n\1", rn).splitlines(), re.sub(r"(<[^/])", r"\n\1", sn).splitlines(),
                "reference", "rust", lineterm="", n=2))[:60])
            diffs.append(f"== {n}\n{ud}\n")
            for line in ud.splitlines()[:16]:
                print("    " + line[:200])
    open(os.path.join(args.work, "diffs.txt"), "w").write("\n".join(diffs))
    print("SUMMARY", json.dumps(counts), "of", len(names))


if __name__ == "__main__":
    main()

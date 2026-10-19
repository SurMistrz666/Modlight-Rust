"""Media processing parity: the reference server against this crate.

    python3 media_parity.py ref     --work DIR   # upload the corpus, collect the reference's outputs
    python3 media_parity.py rust    --work DIR [--ffmpeg host|ref]
    python3 media_parity.py compare --work DIR [--ffmpeg host|ref]

`ref` seeds a user on the scratch reference (ORIGINAL image, private copy,
see SCRATCH below), uploads every corpus file through ShareX, reads the rows
back from its database and asks the reference for every conversion, dry run,
moving tile, timeline and poster; the ffmpeg/ffprobe argument vectors are
taken from the reference's logging shim (FFMPEG_PATH). It writes
DIR/ref/<case>/{meta.json,body.bin}, DIR/ref/args.json and DIR/manifest.json
(the rows and the cases).

`rust` copies the same stored bytes under the same storage keys into the
Rust scratch storage and runs `examples/media_probe.rs batch` on the manifest,
through a logging shim of its own: `--ffmpeg host` runs Homebrew's ffmpeg
(the version the dev box has), `--ffmpeg ref` runs the REFERENCE IMAGE's
ffmpeg 5.1 in a helper container with the work tree mounted at the same path,
which separates version effects from port bugs.

`compare` checks, case by case: status and the headers that carry meaning,
the ffmpeg/ffprobe argument vectors (paths and the thread count normalised),
and the outputs - byte identity first, then by kind: pictures by size, page
count and a pixel difference (libvips CLI), audio/video by ffprobe (streams,
codecs, dimensions, frame counts, duration), PDFs by structure, WebVTT by
text.

Only the Python standard library, plus `vips`, `vipsheader`, `ffprobe`,
`docker` on the PATH. Never prints secrets.
"""

import argparse
import glob
import hashlib
import json
import mimetypes
import os
import re
import shutil
import subprocess
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "parity"))
import parity  # noqa: E402

SCRATCH = "/Users/marcinasdfg/Desktop/modlight/_scratch/srv-media-a"
REF = "http://127.0.0.1:8446"
REF_APP = "modlight-srvmedia-a-ref-app-1"
REF_DB = "modlight-srvmedia-a-ref-postgres-1"
REF_ENV = os.path.join(SCRATCH, "ref", ".env")
FF_HELPER = "modlight-srvmedia-a-ff"
SERVER = os.path.abspath(os.path.join(HERE, "..", ".."))
SEP = "\x1f"

IMAGE_TARGETS = ["pdf", "png", "jpg", "webp", "avif", "gif", "mp4"]
VIDEO_TARGETS = ["mp4", "mov", "mkv", "webm", "webmp4", "mp3", "ogg", "wav", "gif", "webp", "png"]
AUDIO_TARGETS = ["mp3", "ogg", "wav", "mp4", "gif", "png"]

VIDEO = {"mp4", "mov", "mkv", "webm"}
AUDIO = {"mp3", "wav", "flac", "ogg", "m4a"}


def docker(*args, input=None, check=True):
    env = dict(os.environ, DOCKER_CONTEXT="colima-modlight")
    return subprocess.run(["docker", *args], input=input, capture_output=True, env=env, check=check)


def kind_of(name):
    ext = name.rsplit(".", 1)[-1].lower()
    if ext in VIDEO:
        return "video"
    if ext in AUDIO:
        return "audio"
    if ext in ("txt", "bin"):
        return "other"
    return "image"


def cases_for(name):
    """(case name, op, params) for one corpus file."""
    kind = kind_of(name)
    out = []
    targets = {"image": IMAGE_TARGETS, "video": VIDEO_TARGETS, "audio": AUDIO_TARGETS, "other": ["png", "mp4"]}[kind]
    for to in targets:
        out.append(("%s~check~%s" % (name, to), "convert", {"to": to, "check": True}))
        out.append(("%s~convert~%s" % (name, to), "convert", {"to": to}))
    if kind == "video":
        out.append(("%s~convert~gif-d3-t2" % name, "convert", {"to": "gif", "d": 3, "t": 2}))
        out.append(("%s~convert~gif-d20-t9999" % name, "convert", {"to": "gif", "d": 20, "t": 9999}))
        out.append(("%s~public~mp3" % name, "convert", {"to": "mp3", "privileged": False, "check": True}))
        out.append(("%s~public~mkv" % name, "convert", {"to": "mkv", "privileged": False, "check": True}))
        # Served again: from the derived cache, the note read back from beside
        # the cached bytes; and a dry run that the cache answers.
        for to in ("mp4", "mov"):
            out.append(("%s~again~%s" % (name, to), "convert", {"to": to}))
            out.append(("%s~check-again~%s" % (name, to), "convert", {"to": to, "check": True}))
        out.append(("%s~timeline.vtt" % name, "timeline.vtt", {}))
        out.append(("%s~timeline.jpg" % name, "timeline.jpg", {}))
        out.append(("%s~poster" % name, "poster", {}))
    if kind in ("video", "image"):
        out.append(("%s~motion~480-cover" % name, "motion", {"w": 480, "fit": "cover"}))
        out.append(("%s~motion~256-inside" % name, "motion", {"w": 256, "fit": "inside"}))
    if kind == "image" and name.endswith(".gif"):
        out.append(("%s~timeline.vtt" % name, "timeline.vtt", {}))
    return out


def camel(k):
    head, *rest = k.split("_")
    return head + "".join(p[:1].upper() + p[1:] for p in rest)


def row_json(raw):
    """A reference `uploads` row as the Upload model reads it (timestamps fixed:
    nothing in media processing reads them)."""
    out = {}
    for k, v in raw.items():
        if k in ("created_at", "updated_at"):
            v = "2026-10-20T00:00:00.000Z"
        elif k in ("last_accessed_at", "deleted_at", "converted_at", "expires_at") and v is not None:
            v = "2026-10-20T00:00:00.000Z"
        out[camel(k)] = v
    return out


def read_args_log(text):
    """[(tool, [args])] from the shim's log."""
    calls = []
    for line in text.splitlines():
        if not line or line.startswith("#case"):
            continue
        parts = line.split(SEP)
        calls.append((parts[0], parts[1:]))
    return calls


# ---------------------------------------------------------------------------
# Reference
# ---------------------------------------------------------------------------


def ref_get(browser, path):
    docker("exec", REF_APP, "sh", "-c", ": > /app/data/av-args.log")
    res = browser.request("GET", path, origin=False)
    log = docker("exec", REF_APP, "cat", "/app/data/av-args.log").stdout.decode("utf-8", "replace")
    return res, read_args_log(log)


def path_for(op, tag, code, p):
    if op == "convert":
        q = "to=%s" % p["to"]
        if p.get("check"):
            q += "&check=1"
        if "d" in p:
            q += "&d=%s" % p["d"]
        if "t" in p:
            q += "&t=%s" % p["t"]
        return "/%s/%s/convert?%s" % (tag, code, q)
    if op == "motion":
        q = "motion=1&w=%d" % p["w"]
        if p.get("fit") == "inside":
            q += "&fit=inside"
        return "/%s/%s/raw?%s" % (tag, code, q)
    if op in ("timeline.vtt", "timeline.jpg"):
        return "/%s/%s/%s" % (tag, code, op)
    if op == "poster":
        return "/%s/%s/raw?w=1200&fit=inside" % (tag, code)
    raise ValueError(op)


def write_case(dirname, name, status, headers, body, extra=None):
    d = os.path.join(dirname, safe(name))
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "body.bin"), "wb") as f:
        f.write(body)
    meta = {"name": name, "status": status, "headers": headers, "size": len(body)}
    meta.update(extra or {})
    with open(os.path.join(d, "meta.json"), "w") as f:
        json.dump(meta, f, indent=2)


def safe(name):
    return "".join(c if c.isalnum() or c in ".-_" else "_" for c in name)


def cmd_ref(work, corpus):
    env = parity.read_env_file(REF_ENV)
    db = parity.Db(REF_DB, "modlight", env["AUTH_SECRET"])
    suffix = "%06d" % (int.from_bytes(os.urandom(3), "big") % 1000000)
    user_id = "mpa%s" % suffix
    tag = "MPA%s" % suffix
    db.create_user(user_id, "%s@media.parity" % user_id, "Media parity", user_code=tag)
    token = "mpa_" + parity.random_id(28)
    db.create_api_token(user_id, token, scope="upload")
    owner = parity.Browser(REF, real_ip="198.51.100.%d" % (int(suffix) % 250 + 1))
    owner.cookies[parity.SESSION_COOKIE] = db.mint_session(user_id)
    anonymous = parity.Browser(REF, real_ip="198.51.100.254")

    sys.path.insert(0, os.path.join(HERE, "..", "parity"))
    from library import multipart  # noqa: E402

    refdir = os.path.join(work, "ref")
    shutil.rmtree(refdir, ignore_errors=True)
    os.makedirs(refdir)
    manifest = {"tag": tag, "user": user_id, "cases": [], "files": {}}
    args_by_case = {}

    for name in sorted(os.listdir(corpus)):
        with open(os.path.join(corpus, name), "rb") as f:
            data = f.read()
        ctype = mimetypes.guess_type(name)[0] or "application/octet-stream"
        ct, body = multipart(files=[("file", name, ctype, data)])
        res = owner.request("POST", "/upload", headers={"x-api-key": token, "Content-Type": ct}, raw=body, origin=False)
        if res.status not in (200, 201):
            print("upload %s -> %d %s" % (name, res.status, res.text()[:200]))
            continue
        raw = db.sql(
            "select row_to_json(u) from uploads u where owner_id = %s and filename = %s order by created_at desc limit 1"
            % (parity.sql_quote(user_id), parity.sql_quote(name)))
        row = row_json(json.loads(raw.strip().splitlines()[0]))
        manifest["files"][name] = row
        for case, op, params in cases_for(name):
            browser = anonymous if params.get("privileged") is False else owner
            res, calls = ref_get(browser, path_for(op, tag, row["code"], params))
            headers = {k: v for k, v in res.headers}
            body = res.body
            extra = {}
            if op == "poster":
                # The route hands the poster to the thumbnail path; the poster
                # itself is the derived file it cached.
                key = "derived/%s/%s.1.poster.jpg" % (row["id"], row.get("checksum") or "s%d" % row["size"])
                got = docker("exec", REF_APP, "cat", "/app/data/storage/" + key, check=False)
                body = got.stdout if got.returncode == 0 else b""
                extra["route_status"] = res.status
                status = 200 if body else 415
            else:
                status = res.status
            write_case(refdir, case, status, headers, body, extra)
            args_by_case[case] = calls
            spec = {"name": case, "op": op, "row": row, "file": name}
            spec.update(params)
            manifest["cases"].append(spec)
            print("%4d  %s" % (status, case))

    with open(os.path.join(refdir, "args.json"), "w") as f:
        json.dump(args_by_case, f, indent=1)
    with open(os.path.join(work, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=1)
    # The stored bytes, for the Rust side (same keys).
    stage = os.path.join(work, "ref-storage.tar")
    with open(stage, "wb") as f:
        f.write(docker("exec", REF_APP, "tar", "-C", "/app/data/storage", "-cf", "-", tag).stdout)
    print("reference: %d cases" % len(manifest["cases"]))


# ---------------------------------------------------------------------------
# Rust
# ---------------------------------------------------------------------------

SHIM = """#!/usr/bin/python3 -I
# Logs the argument vector, then runs the real tool (written by media_parity.py).
# Everything is baked in: the server runs ffmpeg with an environment of PATH alone.
import os, sys
tool, mode, helper, log, home = %r, %r, %r, %r, %r
args = sys.argv[1:]
os.environ["HOME"] = home
with open(log, "a") as f:
    f.write("\\x1f".join([tool] + args) + "\\n")
if mode == "host":
    os.execv("/opt/homebrew/bin/" + tool, [tool] + args)
# The reference image's ffmpeg 5.1, the work tree mounted at the same path.
# Its thread count is the container's (8): the host's value was logged above
# as this server built it, and is replaced for the run only.
fixed = [("8" if i > 0 and args[i - 1] == "-threads" else a) for i, a in enumerate(args)]
os.environ["DOCKER_CONTEXT"] = "colima-modlight"
os.execv("/opt/homebrew/bin/docker", ["docker", "exec", "-i", helper, "/usr/bin/" + tool] + fixed)
"""


def write_shims(work, mode, log):
    d = os.path.join(work, "shim-" + mode)
    os.makedirs(d, exist_ok=True)
    for tool in ("ffmpeg", "ffprobe"):
        path = os.path.join(d, tool)
        with open(path, "w") as f:
            f.write(SHIM % (tool, mode, FF_HELPER, log, os.environ["HOME"]))
        os.chmod(path, 0o755)
    return d


def ensure_ff_helper():
    got = docker("ps", "-q", "-f", "name=^%s$" % FF_HELPER, check=False).stdout.strip()
    if got:
        return
    docker("rm", "-f", FF_HELPER, check=False)
    docker("run", "-d", "--name", FF_HELPER, "--entrypoint", "sleep", "--network", "none",
           "-u", "%d:%d" % (os.getuid(), os.getgid()),
           "-v", "/Users/marcinasdfg/Desktop/modlight:/Users/marcinasdfg/Desktop/modlight",
           "modlight-wzorzec-app:latest", "infinity")


def cmd_rust(work, mode):
    manifest = json.load(open(os.path.join(work, "manifest.json")))
    storage = os.path.join(work, "rust-storage")
    tmp = os.path.join(work, "rust-tmp")
    shutil.rmtree(storage, ignore_errors=True)
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(storage)
    os.makedirs(tmp)
    subprocess.run(["tar", "-C", storage, "-xf", os.path.join(work, "ref-storage.tar")], check=True)
    if mode == "ref":
        ensure_ff_helper()
    out = os.path.join(work, "rust-" + mode)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    log = os.path.join(out, "args.log")
    shims = write_shims(work, mode, log)
    env = dict(os.environ)
    env.update({
        "STORAGE_DRIVER": "local", "STORAGE_LOCAL_PATH": storage, "UPLOAD_TMP_PATH": tmp,
        "FFMPEG_PATH": os.path.join(shims, "ffmpeg"), "FFPROBE_PATH": os.path.join(shims, "ffprobe"),
        "MEDIA_PROBE_ARGLOG": log, "MODLIGHT_ROOT": SERVER, "APP_URL": "http://localhost:8426",
        "NODE_ENV": "production", "SKIP_ENV_VALIDATION": "1", "AUTH_SECRET": "x" * 40, "RUST_LOG": "warn",
    })
    with open(os.path.join(out, "manifest.json"), "w") as f:
        json.dump(manifest, f)
    subprocess.run([os.path.join(SERVER, "target", "debug", "examples", "media_probe"), "batch",
                    os.path.join(out, "manifest.json"), out], env=env, check=True)
    # Split the argument log by case marker.
    by_case, current = {}, None
    for line in open(log, encoding="utf-8", errors="replace").read().splitlines() if os.path.exists(log) else []:
        if line.startswith("#case" + SEP):
            current = line.split(SEP, 1)[1]
            by_case[current] = []
        elif current is not None and line:
            parts = line.split(SEP)
            by_case[current].append((parts[0], parts[1:]))
    with open(os.path.join(out, "args.json"), "w") as f:
        json.dump(by_case, f, indent=1)


# ---------------------------------------------------------------------------
# Compare
# ---------------------------------------------------------------------------

MEANINGFUL = ["content-type", "x-convert-refused", "x-convert-note", "limit", "x-motion-frames", "x-motion-refused",
              "x-timeline-refused"]


def norm_args(calls, tag):
    out = []
    for tool, args in calls:
        if args == ["-version"]:
            # The once-per-process availability check (ffmpegAvailable).
            continue
        norm = []
        prev = None
        for a in args:
            a = re.sub(r"^.*/av-[A-Za-z0-9]{6}/", "<ws>/", a)
            a = re.sub(r"^.*storage/" + re.escape(tag) + "/", "<storage>/", a)
            a = re.sub(r"^.*/public/brand/", "<public>/brand/", a)
            if prev == "-threads":
                a = "<n>"
            norm.append(a)
            prev = a
        out.append([tool] + norm)
    return out


def vips_stats(path_a, path_b, pages=False):
    """(width, height, pages, bands) of each, and the mean / max absolute pixel
    difference when the shapes agree."""
    def header(p):
        fields = {}
        for f in ("width", "height", "bands", "n-pages", "page-height"):
            got = subprocess.run(["vipsheader", "-f", f, p], capture_output=True, text=True)
            fields[f] = got.stdout.strip() if got.returncode == 0 else None
        return fields
    ha, hb = header(path_a), header(path_b)
    result = {"ref": ha, "rust": hb}
    if ha["width"] and ha["width"] == hb["width"] and ha["height"] == hb["height"] and ha["bands"] == hb["bands"]:
        suffix = "[n=-1]" if pages else ""
        tmp = os.path.join(os.path.dirname(path_b), "_diff.v")
        tmp2 = os.path.join(os.path.dirname(path_b), "_abs.v")
        try:
            subprocess.run(["vips", "subtract", path_a + suffix, path_b + suffix, tmp], check=True, capture_output=True)
            subprocess.run(["vips", "abs", tmp, tmp2], check=True, capture_output=True)
            result["max"] = float(subprocess.run(["vips", "max", tmp2], capture_output=True, text=True).stdout.replace(",", ".") or "nan")
            result["mean"] = float(subprocess.run(["vips", "avg", tmp2], capture_output=True, text=True).stdout.replace(",", ".") or "nan")
        except subprocess.CalledProcessError as e:
            result["error"] = e.stderr.decode()[:200]
        finally:
            for t in (tmp, tmp2):
                if os.path.exists(t):
                    os.remove(t)
    return result


def ffprobe_shape(path):
    got = subprocess.run(["ffprobe", "-v", "error", "-show_entries",
                          "format=format_name,duration:stream=codec_type,codec_name,width,height,nb_frames,sample_rate,channels,pix_fmt,profile",
                          "-of", "json", path], capture_output=True, text=True)
    if got.returncode != 0:
        return {"error": got.stderr.strip()[:200]}
    data = json.loads(got.stdout)
    fmt = data.get("format", {})
    dur = fmt.get("duration")
    return {
        "format": fmt.get("format_name"),
        "duration": round(float(dur), 1) if dur else None,
        "streams": [{k: s.get(k) for k in ("codec_type", "codec_name", "width", "height", "nb_frames", "sample_rate",
                                             "channels", "pix_fmt", "profile")} for s in data.get("streams", [])],
    }


def pdf_shape(body):
    """Objects of a pdf-lib file with their streams inflated, the volatile
    parts (dates, the random image name) normalised."""
    out = []
    for m in re.finditer(rb"(\d+) 0 obj\n(<<.*?>>)\n(?:stream\n)?", body, re.S):
        num, d = m.group(1).decode(), m.group(2)
        entry = {"obj": num}
        length = re.search(rb"/Length (\d+)", d)
        if b"stream" in body[m.end() - 7:m.end()] and length:
            data = body[m.end():m.end() + int(length.group(1))]
            if b"FlateDecode" in d:
                data = zlib.decompress(data)
                d = re.sub(rb"/Length \d+", b"/Length <n>", d)
            entry["stream"] = hashlib.sha256(re.sub(rb"D:\d{14}Z", b"D:<date>", re.sub(rb"Image-\d+", b"Image-<n>", data))).hexdigest()[:16]
        entry["dict"] = re.sub(rb"Image-\d+", b"Image-<n>", d).decode("latin1")
        out.append(entry)
    return out


def classify(meta, path):
    ct = meta.get("headers", {}).get("content-type", "")
    if ct.startswith("image/") or meta.get("name", "").endswith(("~poster", "~timeline.jpg")) or "~motion~" in meta.get("name", ""):
        return "image"
    if ct.startswith(("video/", "audio/")):
        return "av"
    if ct.startswith("application/pdf"):
        return "pdf"
    if ct.startswith("text/vtt"):
        return "text"
    return "text"


def cmd_compare(work, mode):
    manifest = json.load(open(os.path.join(work, "manifest.json")))
    tag = manifest["tag"]
    ref_args = json.load(open(os.path.join(work, "ref", "args.json")))
    rust_dir = os.path.join(work, "rust-" + mode)
    rust_args = json.load(open(os.path.join(rust_dir, "args.json")))
    report = []
    counts = {}

    def note(case, check, ok, detail=None):
        counts.setdefault(check, [0, 0])
        counts[check][0 if ok else 1] += 1
        if not ok:
            report.append((case, check, detail))

    for case in manifest["cases"]:
        name = case["name"]
        a_dir, b_dir = os.path.join(work, "ref", safe(name)), os.path.join(rust_dir, safe(name))
        a, b = json.load(open(os.path.join(a_dir, "meta.json"))), json.load(open(os.path.join(b_dir, "meta.json")))
        note(name, "status", a["status"] == b["status"], (a["status"], b["status"], b["headers"].get("x-error")))
        if a["status"] == b["status"]:
            ha = {k: a["headers"].get(k) for k in MEANINGFUL}
            hb = {k: b["headers"].get(k) for k in MEANINGFUL}
            if case["op"] in ("convert",) and a["status"] == 200 and case.get("to") in ("pdf", "png", "jpg", "avif") \
                    and kind_of(case["file"]) == "image":
                # The image branch of the route (the media serving port) sets
                # the type from the output table; the harness does too.
                pass
            # The type of a poster, a timeline answer and a motion refusal is
            # the route's (the media serving port), not this engine's.
            if case["op"] in ("poster", "timeline.vtt", "timeline.jpg") or (case["op"] == "motion" and a["status"] != 200):
                ha["content-type"] = hb["content-type"] = None
            note(name, "headers", ha == hb, (ha, hb))
        ra, rb = norm_args(ref_args.get(name, []), tag), norm_args(rust_args.get(name, []), tag)
        note(name, "ffmpeg args", ra == rb, (ra, rb))
        if a["status"] in (200,) and b["status"] == a["status"] and a["size"] and b["size"]:
            pa, pb = os.path.join(a_dir, "body.bin"), os.path.join(b_dir, "body.bin")
            same = open(pa, "rb").read() == open(pb, "rb").read()
            counts.setdefault("bytes identical", [0, 0])
            counts["bytes identical"][0 if same else 1] += 1
            if same:
                continue
            kind = classify(a, pa)
            if kind == "image":
                ext = {"image/png": ".png", "image/jpeg": ".jpg", "image/webp": ".webp", "image/avif": ".avif",
                       "image/gif": ".gif"}.get(a["headers"].get("content-type", ""), ".jpg" if "poster" in name or "timeline" in name else ".webp")
                ca, cb = pa + ext, pb + ext
                shutil.copyfile(pa, ca)
                shutil.copyfile(pb, cb)
                st = vips_stats(ca, cb, pages=ext in (".webp", ".gif"))
                same_shape = st["ref"] == st["rust"]
                note(name, "picture shape", same_shape, st)
                if same_shape and "mean" in st:
                    note(name, "picture pixels (mean abs diff < 3/255)", st["mean"] < 3.0, st)
            elif kind == "av":
                sa, sb = ffprobe_shape(pa), ffprobe_shape(pb)
                note(name, "media shape", sa == sb, (sa, sb))
            elif kind == "pdf":
                sa, sb = pdf_shape(open(pa, "rb").read()), pdf_shape(open(pb, "rb").read())
                # The JPEG inside is libvips' output, so its /Length (and with
                # it the xref offsets) follows the libvips build; the content
                # stream and the object stream must match after inflating.
                strip = lambda shape: [(x["obj"], re.sub(r"/Length \d+", "/Length <n>", x["dict"]),
                                        x.get("stream") if x["obj"] in ("6", "7") else None) for x in shape]
                note(name, "pdf structure", strip(sa) == strip(sb), (sa, sb))
            else:
                note(name, "text", False, (open(pa, "rb").read()[:300], open(pb, "rb").read()[:300]))

    print("\n=== media parity (%s ffmpeg) ===" % mode)
    for check, (ok, bad) in counts.items():
        print("%-45s %4d ok %4d differ" % (check, ok, bad))
    with open(os.path.join(work, "report-%s.json" % mode), "w") as f:
        json.dump([{"case": c, "check": k, "detail": d} for c, k, d in report], f, indent=1, default=str)
    print("details: %s" % os.path.join(work, "report-%s.json" % mode))


# Every card the app can emit: each category's word, and every extension the
# card table names (lib/media/card.ts claim() lists).
CARD_EXTS = {
    "code": "html htm css scss less js mjs cjs jsx ts tsx php py rb go rs java kt swift c h cpp hpp cs sh bash zsh ps1 "
            "bat sql json xml yml yaml toml ini env vue svelte lua pl r dart ex exs astro graphql proto tf",
    "archive": "zip rar 7z tar gz tgz bz2 xz zst lz lzma arj cab iso dmg jar war apk xapk pkg deb rpm",
    "document": "pdf doc docx odt rtf tex epub mobi pages ppt pptx odp key",
    "sheet": "xls xlsx xlsm csv tsv ods numbers",
    "audio": "mp3 wav flac ogg oga opus m4a aac aiff wma mid midi amr",
    "text": "txt md markdown log nfo srt vtt rst cfg conf diff patch",
    "image": "svg psd ai eps heic heif xcf raw cr2 cr3 nef arw dng orf rw2",
    "binary": "exe msi dll so dylib bin dat o a wasm class pyc ttf otf woff woff2",
}
CARD_WORDS = {"code": "CODE", "archive": "ARCHIVE", "document": "DOCUMENT", "sheet": "SHEET", "audio": "AUDIO",
              "text": "TEXT", "image": "IMAGE", "binary": "FILE"}


def cmd_cards(work):
    """Every embed card: the reference's /embed/v1/<card>.png against
    `media_probe card` (card_art + card_render), pixel for pixel."""
    import urllib.request
    d = os.path.join(work, "cards")
    os.makedirs(d, exist_ok=True)
    files = []
    for category, exts in CARD_EXTS.items():
        files.append("%s-%s.png" % (category, CARD_WORDS[category]))
        files += ["%s-%s.png" % (category, e.upper()) for e in exts.split()]
    probe = os.path.join(SERVER, "target", "debug", "examples", "media_probe")
    env = dict(os.environ, MODLIGHT_ROOT=SERVER)
    counts = {"pixel-identical": 0, "differ": 0, "bytes identical": 0}
    for f in files:
        ref, rust = os.path.join(d, "ref-" + f), os.path.join(d, "rust-" + f)
        with urllib.request.urlopen(REF + "/embed/v1/" + f) as res, open(ref, "wb") as out:
            out.write(res.read())
        subprocess.run([probe, "card", f, rust], env=env, check=True)
        if open(ref, "rb").read() == open(rust, "rb").read():
            counts["bytes identical"] += 1
        st = vips_stats(ref, rust)
        if st["ref"] == st["rust"] and st.get("max") == 0.0:
            counts["pixel-identical"] += 1
        else:
            counts["differ"] += 1
            print("DIFFER %s %s" % (f, st))
    print("cards: %d, %s" % (len(files), counts))


def main():
    p = argparse.ArgumentParser()
    p.add_argument("command", choices=["ref", "rust", "compare", "cards"])
    p.add_argument("--work", default=os.path.join(SCRATCH, "parity"))
    p.add_argument("--corpus", default=os.path.join(SCRATCH, "corpus"))
    p.add_argument("--ffmpeg", choices=["host", "ref"], default="host")
    a = p.parse_args()
    os.makedirs(a.work, exist_ok=True)
    if a.command == "ref":
        cmd_ref(a.work, a.corpus)
    elif a.command == "rust":
        cmd_rust(a.work, a.ffmpeg)
    elif a.command == "cards":
        cmd_cards(a.work)
    else:
        cmd_compare(a.work, a.ffmpeg)


if __name__ == "__main__":
    main()

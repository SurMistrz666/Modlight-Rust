"""Parity of the convert server actions (app/actions/convert.ts) and of
convert-in-place: the same seeded rows and bytes on the scratch reference and
on the Rust server, the same calls, the envelopes compared, then the rows,
the stored objects and the audit log.

    python3 convert_actions.py [--ref http://localhost:8446 --rust http://localhost:8426]

Defaults are this branch's scratch stack (_scratch/srv-media-a): reference
`modlight-srvmedia-a-ref-*` (8446), Rust server on 8426 with the database
`modlight_srv_media_a` and storage `_scratch/srv-media-a/data/storage`. Run
the Rust server with FFMPEG_PATH/FFPROBE_PATH pointing at the reference
image's ffmpeg (parity/shim-ref from media_parity.py) to compare the audio
and video conversions byte for byte; with Homebrew's ffmpeg 9 those differ in
the bytes (and audio -> mp4 fails, see the SKELETON notes).

Never prints secrets. State-changing: only against a private reference.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "parity"))
import parity  # noqa: E402
from library import Library, seed_upload  # noqa: E402

SCRATCH = "/path/to/modlight/_scratch/srv-media-a"
p = argparse.ArgumentParser(allow_abbrev=False)
p.add_argument("--ref-app", default="modlight-srvmedia-a-ref-app-1")
p.add_argument("--ref-db", default="modlight-srvmedia-a-ref-postgres-1:modlight")
p.add_argument("--rust-db", default="modlight-devdb-postgres-1:modlight_srv_media_a")
p.add_argument("--rust-storage", default=os.path.join(SCRATCH, "data", "storage"))
p.add_argument("--corpus", default=os.path.join(SCRATCH, "corpus"))
p.add_argument("--ref-env", default=os.path.join(SCRATCH, "ref", ".env"))
p.add_argument("--rust-env", default=os.path.join(SCRATCH, "rust.env"))
OPT, _ = p.parse_known_args()
if not os.environ.get("PARITY_REF"):
    os.environ["PARITY_REF"] = "http://localhost:8446"
if not os.environ.get("PARITY_RUST"):
    os.environ["PARITY_RUST"] = "http://localhost:8426"

run = parity.Run.from_args()
_rc, _rd = OPT.ref_db.split(":")
_uc, _ud = OPT.rust_db.split(":")
REF = parity.Db(_rc, _rd, parity.read_env_file(OPT.ref_env)["AUTH_SECRET"])
RUST = parity.Db(_uc, _ud, parity.read_env_file(OPT.rust_env)["AUTH_SECRET"])
lib = Library(run, REF, RUST)
acts = parity.Actions(run, OPT.ref_app)
SFX = lib.suffix
UNDEFINED = parity.UNDEFINED
q = parity.sql_quote
NAMES = {}


def label(v):
    text = json.dumps(v, ensure_ascii=False)
    for value, name in sorted(NAMES.items(), key=lambda kv: -len(kv[0])):
        text = text.replace(value, name)
    text = text.replace(SFX, "<sfx>")
    # Storage keys of converted objects end in 12 random hex digits.
    text = re.sub(r"-[0-9a-f]{12}\.", "-<hex12>.", text)
    return parity.normalize_json(json.loads(text))


def put(key, data):
    subprocess.run(["docker", "exec", "-i", OPT.ref_app, "sh", "-c",
                    'mkdir -p "$(dirname "/app/data/storage/$1")" && cat > "/app/data/storage/$1"', "sh", key],
                   input=data, check=True, env=dict(os.environ, DOCKER_CONTEXT="colima-modlight"))
    path = os.path.join(OPT.rust_storage, key)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def stored(key):
    """(exists, sha256) of an object on each side."""
    got = subprocess.run(["docker", "exec", OPT.ref_app, "cat", "/app/data/storage/" + key], capture_output=True,
                         env=dict(os.environ, DOCKER_CONTEXT="colima-modlight"))
    a = hashlib.sha256(got.stdout).hexdigest()[:16] if got.returncode == 0 else None
    path = os.path.join(OPT.rust_storage, key)
    b = hashlib.sha256(open(path, "rb").read()).hexdigest()[:16] if os.path.exists(path) else None
    return a, b


def reachable(export):
    """Only `convertUpload` has an action id in the reference build: no client
    component imports the other three exports (CONTRACT section 7), so Next
    never registered them. Those are checked on the Rust side against the
    reference's `convertUpload` run over the same files."""
    try:
        acts.ids.get("convert", export)
        return True
    except KeyError:
        return False


def rust_only(pair, name, export, args):
    b = acts.call_rust(pair.rust, "convert", export, args)
    env = label(parity.rpc_envelope(b))
    print("RUST %s: %s -> %s" % (export, name, json.dumps(env, ensure_ascii=False)[:400]))
    return env


def act(pair, name, export, args, redact=()):
    if not reachable(export):
        return None, rust_only(pair, name, export, args)
    a = acts.call_ref(pair.ref, "convert", export, args)
    b = acts.call_rust(pair.rust, "convert", export, args)
    ea, eb = label(parity.next_action_envelope(a)), label(parity.rpc_envelope(b))
    for env in (ea, eb):
        ok = env.get("ok")
        if isinstance(ok, dict) and ok.get("status") == "created":
            ok["code"] = "<new code>"
        if redact and isinstance(env.get("ok"), (dict, list)):
            env["ok"] = parity.normalize_json(env["ok"], redact=redact)
    run.compare("convert.%s: %s" % (export, name), ea, eb)
    return ea, eb


ROW_SQL = ("SELECT code, filename, ext, mime_type, kind, size, checksum, width, height, frames, duration_seconds, "
           "fps, blurhash, thumb_key, published, original_ext, converted_at IS NOT NULL, deleted_at IS NOT NULL, "
           "storage_key, password_version, max_views, folder_id, length(delete_token) FROM uploads "
           "WHERE owner_id = %s ORDER BY filename, ext")


def rows(name, owner, content_volatile=()):
    """The owner's rows on both sides. Columns that describe BYTES (size,
    checksum, blurhash) are compared only for files whose encoder output is
    deterministic across the two libvips builds (see media_parity.py: PNG,
    AVIF, PDF and Matroska bytes differ between libvips 8.17/8.18 and between
    two Matroska/Ogg muxes)."""
    a, b = REF.sql(ROW_SQL % q(owner)), RUST.sql(ROW_SQL % q(owner))

    def norm(text):
        out = []
        for line in text.splitlines():
            cols = line.split("\t")
            if cols and cols[0] not in NAMES:
                # A copy's code comes from the owner's naming scheme (random).
                cols = [c.replace(cols[0], "<new code>") for c in cols]
            if len(cols) > 6 and cols[2] in content_volatile:
                cols[5] = cols[6] = cols[12] = "<bytes>"
            out.append(cols)
        return label(out)

    run.compare("rows: " + name, norm(a), norm(b))
    return a


def main():
    pair = run.pair()
    alice, alice_tag = lib.seed_user("ca")
    bob, _ = lib.seed_user("cb")
    admin, _ = lib.seed_user("cx", role="admin")
    for v, n in ((alice, "alice"), (bob, "bob"), (admin, "admin"), (alice_tag, "tag")):
        NAMES[v] = "<%s>" % n

    corpus = lambda n: open(os.path.join(OPT.corpus, n), "rb").read()
    ids = {}

    def seed(name, code, filename, ext, mime, data, **cols):
        key = "%s/%s.%s" % (alice_tag, code, ext)
        put(key, data)
        cols.setdefault("checksum", hashlib.sha256(data).hexdigest())
        ids[name] = seed_upload(lib, alice, code, filename=filename, ext=ext, mime_type=mime, size=len(data),
                                storage_key=key, **cols)
        NAMES[ids[name]] = "<%s>" % name
        NAMES[code] = "<code:%s>" % name
        return ids[name]

    s = SFX
    seed("jpg", "ja" + s, "eagle.jpg", "jpg", "image/jpeg", corpus("eagle.jpg"), width=512, height=512, frames=1)
    seed("png", "pa" + s, "parrot.png", "png", "image/png", corpus("parrot.png"), frames=1)
    seed("gif", "ga" + s, "anim.gif", "gif", "image/gif", corpus("anim.gif"), frames=12)
    seed("mp4", "va" + s, "clip.mp4", "mp4", "video/mp4", corpus("h264_aac.mp4"), width=320, height=240)
    seed("mp3", "aa" + s, "tone.mp3", "mp3", "audio/mpeg", corpus("tone.mp3"))
    seed("txt", "ta" + s, "notes.txt", "txt", "text/plain", corpus("notes.txt"))
    seed("paste", "xa" + s, "paste.ts", "ts", "text/plain; charset=utf-8", b"const x = 1;\n", kind="paste",
         language="typescript")
    seed("trashed", "da" + s, "gone.jpg", "jpg", "image/jpeg", corpus("eagle_grey.jpg"),
         deleted_at="2026-10-19T00:00:00.000Z")
    seed("expired", "ea" + s, "old.png", "png", "image/png", corpus("tiny.png"), expires_at="2026-01-01T00:00:00.000Z")
    seed("jpg2", "jb" + s, "landscape.jpg", "jpg", "image/jpeg", corpus("landscape_o6.jpg"))

    # Signed out: every entry point throws. (On the reference the action is
    # posted to the page that imports it, /dashboard, whose proxy gate answers
    # 307 before the action runs; /_rpc has no such gate, so the action's own
    # "Unauthorized" is what the Rust side answers. Compared on Rust only.)
    for export, args in (("convertTargetsForUpload", [ids["jpg"]]), ("convertTargetsForSelection", [[ids["jpg"]]]),
                         ("convertUpload", [ids["jpg"], "png", "replace"]), ("convertUploads", [[ids["jpg"]], "png", "replace"])):
        rust_only(pair, "signed out", export, args)

    parity.signed_in(pair, REF, RUST, alice)
    for name in ("jpg", "png", "gif", "mp4", "mp3", "txt", "paste", "trashed"):
        act(pair, name, "convertTargetsForUpload", [ids[name]])
    act(pair, "not a uuid", "convertTargetsForUpload", ["nope"])
    act(pair, "undefined", "convertTargetsForUpload", [UNDEFINED])
    every = [ids[n] for n in ("jpg", "png", "gif", "mp4", "mp3", "txt", "paste", "trashed", "expired")]
    act(pair, "everything, replace", "convertTargetsForSelection", [every])
    act(pair, "everything, copy", "convertTargetsForSelection", [every, "copy"])
    act(pair, "empty", "convertTargetsForSelection", [[]])
    act(pair, "duplicates and blanks", "convertTargetsForSelection", [[ids["jpg"], ids["jpg"], "", None]])

    # A mixed selection to PNG: trashed, expired, a paste, text, and two that
    # convert. The reference converts it file by file through convertUpload
    # (convertUploads is unreachable there); the Rust side runs convertUploads,
    # and its report must be those outcomes with the counts in front.
    mixed = [ids[n] for n in ("trashed", "expired", "paste", "txt", "jpg", "png")]
    ref_results = []
    for upload in mixed:
        a = acts.call_ref(pair.ref, "convert", "convertUpload", [upload, "png", "replace"])
        ref_results.append(label(parity.next_action_envelope(a)).get("ok"))
    ref_report = {"to": "png", "mode": "replace",
                  "converted": sum(1 for r in ref_results if r["status"] in ("converted", "created")),
                  "skipped": sum(1 for r in ref_results if r["status"] == "skipped"),
                  "failed": sum(1 for r in ref_results if r["status"] == "failed"),
                  "results": ref_results}
    rust_env = label(parity.rpc_envelope(acts.call_rust(pair.rust, "convert", "convertUploads", [mixed, "png", "replace"])))
    run.compare("convert.convertUploads: mixed to png (against convertUpload one by one)",
                {"ok": parity.normalize_json(ref_report, redact=("size",)), "revalidate": True},
                dict(rust_env, ok=parity.normalize_json(rust_env.get("ok"), redact=("size",))))
    rows("after mixed to png", alice, content_volatile=("png",))
    act(pair, "jpg again, already png", "convertUpload", [ids["jpg"], "png", "replace"])
    act(pair, "copy to webp", "convertUpload", [ids["png"], "webp", "copy"])
    act(pair, "animated gif to webp in place", "convertUpload", [ids["gif"], "webp", "replace"])
    act(pair, "mp4 to mkv in place", "convertUpload", [ids["mp4"], "mkv", "replace"], redact=("size",))
    act(pair, "mp3 as an mp4 copy", "convertUpload", [ids["mp3"], "mp4", "copy"], redact=("size",))
    act(pair, "jpg to pdf copy", "convertUpload", [ids["jpg2"], "pdf", "copy"], redact=("size",))
    act(pair, "unknown target", "convertUpload", [ids["jpg2"], "bmp", "replace"])
    act(pair, "null target", "convertUpload", [ids["jpg2"], None, "replace"])
    act(pair, "not a uuid", "convertUpload", ["nope", "png", "replace"])
    act(pair, "null id", "convertUpload", [None, "png", "replace"])
    rows("after the conversions", alice, content_volatile=("png", "mkv", "pdf", "mp4"))

    parity.signed_in(pair, REF, RUST, bob)
    act(pair, "somebody else's file", "convertUpload", [ids["jpg2"], "png", "replace"])
    act(pair, "somebody else's selection", "convertTargetsForSelection", [every])

    parity.signed_in(pair, REF, RUST, admin)
    act(pair, "admin, an odd mode", "convertUpload", [ids["jpg2"], "webp", "move"])
    a, b = (db.sql("SELECT action, detail::text FROM audit_log WHERE user_id = %s ORDER BY created_at" % q(admin))
            for db in (REF, RUST))
    run.compare("audit: admin", label(a.splitlines()), label(b.splitlines()))

    # Quota: a copy charged in full against a full account.
    REF.sql("UPDATE \"user\" SET used_quota = 10, max_quota = 10 WHERE id = %s" % q(alice))
    RUST.sql("UPDATE \"user\" SET used_quota = 10, max_quota = 10 WHERE id = %s" % q(alice))
    parity.signed_in(pair, REF, RUST, alice)
    act(pair, "over quota", "convertUpload", [ids["jpg2"], "jpg", "copy"])
    a, b = (db.sql('SELECT used_quota, max_quota FROM "user" WHERE id = %s' % q(alice)) for db in (REF, RUST))
    run.compare("quota after", a, b)
    rows("final", alice, content_volatile=("png", "mkv", "pdf", "mp4", "webp"))

    # The objects behind the final rows: the old bytes are gone, the new ones there.
    keys = [line.split("\t") for line in REF.sql(
        "SELECT ext, storage_key FROM uploads WHERE owner_id = %s ORDER BY filename, ext" % q(alice)).splitlines()]
    rust_keys = [line.split("\t") for line in RUST.sql(
        "SELECT ext, storage_key FROM uploads WHERE owner_id = %s ORDER BY filename, ext" % q(alice)).splitlines()]
    present = [(ext, stored(k)[0] is not None) for ext, k in keys]
    present_rust = [(ext, stored(k)[1] is not None) for ext, k in rust_keys]
    run.compare("objects behind every row", present, present_rust)
    run.finish()


main()

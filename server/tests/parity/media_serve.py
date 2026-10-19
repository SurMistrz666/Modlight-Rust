"""Media serving parity: every public URL shape of a set of uploads, requested
from the scratch reference and from the Rust server, compared.

    cd /path/to/modlight && \\
    PARITY_REF=http://localhost:8447 PARITY_RUST=http://localhost:8427 \\
        python3 _wt/srv-media-b/server/tests/parity/media_serve.py [--only raw] [-v]

Areas (`--only`): raw, thumbs, gates, fix1, render, pages, bots, tagless, public,
zip, oembed, delete, unlock, embed.

Both servers run with the SAME AUTH_SECRET (`_scratch/srv-media-b/secrets.env`),
and media_seed.py gives every uploaded file the same id on both, so a render
ticket, an unlock cookie or a view grant minted by one server is valid on the
other and responses compare byte for byte.

Intended differences are checked as such, never silently skipped:
- security fix 1 (`fix1`): the view-limit bypasses must be OPEN on the
  reference and CLOSED on Rust; the legitimate flows must work on Rust;
- security fix 6 (`oembed`): a capped file's oEmbed carries no raw URL on Rust;
- part A (media processing) not merged yet: thumbnails of videos, motion
  tiles, timelines, conversions, document rendering and card images are
  reported as `pending-A` (counted, not failed) until it lands.

State-changing (view counters, deletes): never point it at the shared
reference on 8420.
"""

import argparse
import hashlib
import io
import json
import os
import re
import sys
import urllib.parse
import zipfile
import html as htmllib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402
from library_pages import Flight  # noqa: E402
from media_seed import Seeder, Q  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "..", ".."))
SCRATCH = os.path.join(ROOT, "_scratch", "srv-media-b")

BROWSER_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Safari/605.1.15"
BOTS = {
    "discord": "Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)",
    "telegram": "TelegramBot (like TwitterBot)",
    "slack": "Slackbot-LinkExpanding 1.0 (+https://api.slack.com/robots)",
    "twitter": "Mozilla/5.0 (compatible; Twitterbot/1.0)",
}

RAW_HEADERS = ("content-type", "content-disposition", "cache-control", "etag", "accept-ranges", "content-range",
               "x-content-type-options", "content-security-policy", "location", "x-convert-refused",
               "x-motion-refused", "x-timeline-refused", "x-motion-frames", "retry-after", "referrer-policy",
               "x-preview-refused", "access-control-allow-origin", "x-frame-options")


class Area(object):
    def __init__(self, ctx):
        self.ctx = ctx
        self.run = ctx.run
        self.pending = []

    def norm(self, v):
        text = json.dumps(v, ensure_ascii=False, sort_keys=True)
        text = re.sub(r"http://localhost:\d+", "<origin>", text)
        return parity.normalize_json(json.loads(text))


class Ctx(object):
    def __init__(self, run, seed, actions):
        self.run = run
        self.seed = seed
        self.actions = actions
        self.pending = []  # (name, reference shape, rust shape) of part-A checks

    def pair(self, ua=BROWSER_UA, ip=None):
        return self.run.pair(user_agent=ua, real_ip=ip)

    def compare(self, name, a, b, pending_a=False):
        a = norm(a)
        b = norm(b)
        if pending_a:
            if a == b:
                self.run.passed += 1
            else:
                self.pending.append(name)
                if self.run.verbose:
                    print("PENDING-A %s" % name)
            return a == b
        return self.run.compare(name, a, b)

    def expect(self, name, ok, detail=None):
        """An assertion about one server (the intended differences)."""
        if ok:
            self.run.passed += 1
            if self.run.verbose:
                print("PASS %s" % name)
        else:
            self.run.failures.append((name, json.dumps(detail, default=str)))
            print("FAIL %s\n%s" % (name, json.dumps(detail, indent=2, default=str)))


def norm(v):
    text = json.dumps(v, ensure_ascii=False, sort_keys=True, default=str)
    text = re.sub(r"http://localhost:\d+", "<origin>", text)
    text = re.sub(r"http%3A%2F%2Flocalhost%3A\d+", "<origin>", text)
    return parity.normalize_json(json.loads(text))


def shape(r, body="hash", headers=RAW_HEADERS):
    d = {"status": r.status}
    for h in headers:
        v = r.header(h)
        if v is not None:
            d[h] = v
    d["set-cookie"] = [n for n, _, _ in r.set_cookies()]
    if body == "hash":
        d["body"] = "%d:%s" % (len(r.body), hashlib.sha256(r.body).hexdigest()[:16])
    elif body == "text":
        d["body"] = r.text()
    elif body == "json":
        d["body"] = r.json()
    elif body == "len":
        d["body"] = len(r.body)
    return d


def both(pair, method, path, headers=None, cookies=True):
    return pair.both(method, path, headers=headers or {}, origin=False, cookies=cookies)


# ── raw, raw.ext, download ─────────────────────────────────────────────────

def area_raw(ctx, f, tag):
    pair = ctx.pair()
    for key, info in f.items():
        code, ext = info["code"], info["ext"]
        paths = ["/%s/%s/raw" % (tag, code), "/%s/%s/download" % (tag, code)]
        if ext:
            paths.insert(1, "/%s/%s/raw.%s" % (tag, code, ext))
        for path in paths:
            for hname, h in [("plain", {}), ("range 0-", {"Range": "bytes=0-"}), ("range 2-9", {"Range": "bytes=2-9"}),
                             ("suffix", {"Range": "bytes=-5"}), ("range past end", {"Range": "bytes=999999999-"}),
                             ("two ranges", {"Range": "bytes=0-1,4-5"}), ("image dest", {"Sec-Fetch-Dest": "image"}),
                             ("if-none-match", {"If-None-Match": '"x"'})]:
                a, b = both(pair, "GET", path, h)
                ctx.compare("raw %s %s %s" % (key, path.rsplit("/", 1)[1], hname), shape(a), shape(b))
        a, b = both(pair, "HEAD", "/%s/%s/raw" % (tag, code))
        ctx.compare("raw %s HEAD" % key, shape(a), shape(b))
    # Not found shapes.
    for path in ["/%s/nope/raw" % tag, "/nobody/%s/raw" % f["png"]["code"], "/%s/%s/raws" % (tag, f["png"]["code"]),
                 "/%s/%s/RAW" % (tag, f["png"]["code"]), "/%s/%s/poster" % (tag, f["png"]["code"])]:
        a, b = both(pair, "GET", path)
        ctx.compare("raw missing %s" % path.split("/", 2)[2], shape(a, "text"), shape(b, "text"))
    # The counters every one of those requests left behind.
    for key, info in f.items():
        a, b = ctx.seed.counters(info["id"])
        ctx.compare("raw counters %s" % key, a, b)


# ── thumbnails, motion, timelines, conversions ─────────────────────────────

def area_thumbs(ctx, f, tag):
    pair = ctx.pair()
    for key in ("png", "jpg", "wide", "gif", "webp", "heic", "mp4", "pdf", "txt"):
        info = f[key]
        base = "/%s/%s/raw" % (tag, info["code"])
        for q in ["w=64", "w=1200&fit=inside", "h=128", "w=1&h=99999", "w=0", "w=abc", "motion=1", "motion=1&w=64",
                  "motion=yes"]:
            a, b = both(pair, "GET", base + "?" + q)
            pending = key in ("mp4",) or q.startswith("motion=1")
            ctx.compare("thumb %s %s" % (key, q), shape(a), shape(b), pending_a=pending)
    for key in ("mp4", "png"):
        for action in ("timeline.vtt", "timeline.jpg"):
            a, b = both(pair, "GET", "/%s/%s/%s" % (tag, f[key]["code"], action))
            ctx.compare("timeline %s %s" % (key, action), shape(a, "len"), shape(b, "len"), pending_a=key == "mp4")
    for key, targets in (("png", ["jpg", "webp", "avif", "pdf", "png", "mp4", "bogus", ""]),
                         ("gif", ["mp4", "webp", "png"]), ("mp4", ["mov", "mkv", "gif", "mp3", "webmp4"]),
                         ("mp3", ["ogg", "wav", "mp4"]), ("zip", ["png"])):
        for to in targets:
            for q in ("to=%s&check=1" % to, "to=%s" % to):
                a, b = both(pair, "GET", "/%s/%s/convert?%s" % (tag, f[key]["code"], q))
                engine_needed = not (key == "png" and to in ("bogus", "", "png", "mp4")) and key != "zip"
                ctx.compare("convert %s %s" % (key, q), shape(a, "len"), shape(b, "len"), pending_a=engine_needed)


# ── gates: unpublished, password, expiry ───────────────────────────────────

def area_gates(ctx, f, tag, owner, admin):
    seed = ctx.seed
    anon = ctx.pair()
    own = ctx.pair()
    parity.signed_in(own, seed.ref_db, seed.rust_db, owner)
    adm = ctx.pair()
    parity.signed_in(adm, seed.ref_db, seed.rust_db, admin)
    read_token = seed.seed_token(owner, "read")
    upload_token = seed.seed_token(owner, "upload")

    priv = f["gate_private"]
    pw = f["gate_password"]
    exp = f["gate_expired"]
    seed.set_cols(priv["id"], published=False)
    # A real scrypt hash, made by the Rust server's own action and copied over.
    seed.set_cols(exp["id"], expires_at="2020-01-01T00:00:00.000Z")
    for who, pair, hdr in [("anon", anon, {}), ("owner", own, {}), ("admin", adm, {}),
                           ("read token", anon, {"x-api-key": read_token}),
                           ("upload token", anon, {"x-api-key": upload_token}),
                           ("bearer read", anon, {"Authorization": "Bearer " + read_token})]:
        for label, info in (("private", priv), ("expired", exp)):
            for action in ("raw", "download"):
                a, b = both(pair, "GET", "/%s/%s/%s" % (tag, info["code"], action), hdr)
                ctx.compare("gate %s %s %s" % (label, action, who), shape(a), shape(b))
    # Password: set through the Rust action on the Rust row, copied to the reference.
    own_rust = parity.Browser(ctx.run.rust, real_ip=ctx.run.real_ip, user_agent=BROWSER_UA)
    own_rust.cookies[parity.SESSION_COOKIE] = seed.rust_db.mint_session(owner)
    r = own_rust.rpc("uploads", "setUploadPassword", [pw["id"], "hunter22"])
    if r.status != 200:
        print("setUploadPassword failed: %s %s" % (r.status, r.text()[:200]))
    stored = seed.rust_db.sql("SELECT password_hash FROM uploads WHERE id = %s" % Q(pw["id"])).strip()
    seed.ref_db.sql("UPDATE uploads SET password_hash = %s WHERE id = %s" % (Q(stored), Q(pw["id"])))
    for who, pair in (("anon", anon), ("owner", own), ("admin", adm)):
        for action in ("raw", "download", "timeline.vtt"):
            a, b = both(pair, "GET", "/%s/%s/%s" % (tag, pw["code"], action))
            ctx.compare("gate password %s %s" % (action, who), shape(a), shape(b))
    # Unlock through the action on both, then read.
    visitor = ctx.pair(ip="203.0.113.%d" % (hash(tag) % 200 + 10))
    for args, label in (([pw["id"], "wrong"], "wrong password"), ([pw["id"], ""], "empty password"),
                        (["not-a-uuid", "x"], "bad id"), ([pw["id"], "hunter22"], "right password"),
                        ([pw["id"], "hunter22", True], "remember")):
        ctx.actions.check(visitor, "unlock %s" % label, "unlock", "verifyMediaPassword", args)
    for action in ("raw", "download"):
        a, b = both(visitor, "GET", "/%s/%s/%s" % (tag, pw["code"], action))
        ctx.compare("gate password %s unlocked" % action, shape(a), shape(b))
    a, b = ctx.seed.counters(pw["id"])
    ctx.compare("gate password counters", a, b)
    # Throttling: five wrong answers, then rate-limited.
    thr = ctx.pair(ip="198.18.0.%d" % (hash(tag) % 200 + 10))
    for i in range(6):
        ctx.actions.check(thr, "unlock throttle %d" % i, "unlock", "verifyMediaPassword", [pw["id"], "nope%d" % i])


# ── security fix 1 ─────────────────────────────────────────────────────────

def area_fix1(ctx, f, tag):
    """The bypasses of docs/audyt/serwer-pliki-wyciek.md (A1, L1, and the ones
    found while porting), each on a fresh capped file: OPEN on the reference
    (bytes served, no view spent), CLOSED on Rust (a view spent, or 404)."""
    seed = ctx.seed
    bypasses = [
        ("raw Range bytes=1-", "raw", {"Range": "bytes=1-"}, ""),
        ("raw Sec-Fetch-Dest image", "raw", {"Sec-Fetch-Dest": "image"}, ""),
        ("raw Sec-Fetch-Dest video", "raw", {"Sec-Fetch-Dest": "video"}, ""),
        ("raw.ext Range bytes=1-", "raw.txt", {"Range": "bytes=1-"}, ""),
        ("download Range bytes=1-", "download", {"Range": "bytes=1-"}, ""),
        ("download Sec-Fetch-Dest image", "download", {"Sec-Fetch-Dest": "image"}, ""),
        ("download ?w=1", "download", {}, "?w=1"),
        ("raw ?w=64 falling back to the original", "raw", {}, "?w=64"),
        ("raw ?h=1 falling back to the original", "raw", {}, "?h=1"),
    ]
    for i, (label, action, hdr, q) in enumerate(bypasses):
        info = f["fix1_%d" % i]
        seed.set_cols(info["id"], max_views=1)
        path = "/%s/%s/%s%s" % (tag, info["code"], action, q)
        refs, rusts = [], []
        pair = ctx.pair()
        for _ in range(3):
            a, b = pair.both("GET", path, headers=hdr, origin=False, cookies=False)
            refs.append(a.status)
            rusts.append(b.status)
        (rv, _), (sv, _) = seed.counters(info["id"])
        ctx.expect("fix1 %s: open on the reference" % label, refs == [206 if "Range" in hdr else 200] * 3 and rv == "0",
                   {"statuses": refs, "views": rv})
        ctx.expect("fix1 %s: closed on Rust" % label, rusts[0] in (200, 206) and rusts[1:] == [404, 404] and sv == "1",
                   {"statuses": rusts, "views": sv})

    # Preview bots on the page of a capped text file: no text, no ticket.
    info = f["fix1_page_text"]
    seed.set_cols(info["id"], max_views=1)
    bot = ctx.pair(ua=BOTS["twitter"])
    a, b = bot.both("GET", "/_page?path=" + urllib.parse.quote("/%s/%s" % (tag, info["code"]), safe=""),
                    origin=False)
    rust_viewer = ((b.json() or {}).get("props") or {}).get("viewer") or {}
    ctx.expect("fix1 bot page of a capped text: Rust withholds the text", rust_viewer.get("textContent") is None,
               rust_viewer)
    doc = bot.ref.request("GET", "/%s/%s" % (tag, info["code"]), origin=False).text()
    ref_source = Flight(doc).one("SourceView")
    ctx.expect("fix1 bot page of a capped text: the reference hands it over",
               isinstance(ref_source, dict) and bool(ref_source.get("content")), ref_source)
    (rv, _), (sv, _) = seed.counters(info["id"])
    ctx.expect("fix1 bot page: no view spent on either", rv == "0" and sv == "0", [rv, sv])
    # ...and its card carries no excerpt on Rust.
    rdoc = bot.rust.request("GET", "/%s/%s" % (tag, info["code"]), origin=False).text()
    ctx.expect("fix1 capped text card: no excerpt on Rust", "const answer" not in head_text(rdoc), head_text(rdoc))
    ctx.expect("fix1 capped text card: the excerpt on the reference", "const answer" in head_text(doc), head_text(doc))

    # Legitimate flows on Rust.
    # 1. A browser opens the page of a capped image: one view, then the page's
    #    own <img> and a reload of it within the grant.
    info = f["fix1_page_image"]
    seed.set_cols(info["id"], max_views=2)
    br = parity.Browser(ctx.run.rust, real_ip=ctx.run.real_ip, user_agent=BROWSER_UA)
    page = br.request("GET", "/%s/%s.%s" % (tag, info["code"], info["ext"]))
    granted = "ml_view_" + info["id"] in br.cookies
    img = br.request("GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]), headers={"Sec-Fetch-Dest": "image"})
    img2 = br.request("GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]), headers={"Sec-Fetch-Dest": "image"})
    (_, _), (sv, _) = seed.counters(info["id"])
    ctx.expect("fix1 flow: page then its <img> spends exactly one view",
               page.status == 200 and granted and img.status == 200 and img2.status == 200 and sv == "1",
               {"page": page.status, "grant": granted, "img": img.status, "img2": img2.status, "views": sv})
    # 2. The LAST view: the page spends it, the page's own <img> still loads.
    page = br.request("GET", "/%s/%s.%s" % (tag, info["code"], info["ext"]))
    img = br.request("GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]), headers={"Sec-Fetch-Dest": "image"})
    again = br.request("GET", "/%s/%s.%s" % (tag, info["code"], info["ext"]))
    stranger = parity.Browser(ctx.run.rust, real_ip=ctx.run.real_ip).request(
        "GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]), headers={"Sec-Fetch-Dest": "image"})
    ctx.expect("fix1 flow: the last view's <img> loads, nothing after it",
               page.status == 200 and img.status == 200 and again.status == 404 and stranger.status == 404,
               {"page": page.status, "img": img.status, "reload": again.status, "stranger": stranger.status})
    # 3. An encrypted file read by the in-browser viewer: two ranged reads, one view.
    info = f["fix1_encrypted"]
    seed.set_cols(info["id"], max_views=1)
    br = parity.Browser(ctx.run.rust, real_ip=ctx.run.real_ip, user_agent=BROWSER_UA)
    page = br.request("GET", "/%s/%s.%s" % (tag, info["code"], info["ext"]))
    head = br.request("GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]), headers={"Range": "bytes=0-4095"})
    rest = br.request("GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]), headers={"Range": "bytes=4096-"})
    (_, _), (sv, _) = seed.counters(info["id"])
    ctx.expect("fix1 flow: an encrypted one-time file opens (page, head, rest) for one view",
               page.status == 200 and head.status == 206 and rest.status == 206 and sv == "1"
               and len(head.body) + len(rest.body) == info["size"],
               {"page": page.status, "head": head.status, "rest": rest.status, "views": sv})
    # The same on the reference: its second request 404s (upstream's own bug,
    # which the grant fixes).
    rb = parity.Browser(ctx.run.ref, real_ip=ctx.run.real_ip, user_agent=BROWSER_UA)
    info2 = f["fix1_encrypted_ref"]
    seed.set_cols(info2["id"], max_views=1)
    rb.request("GET", "/%s/%s.%s" % (tag, info2["code"], info2["ext"]))
    h2 = rb.request("GET", "/%s/%s/raw.%s" % (tag, info2["code"], info2["ext"]), headers={"Range": "bytes=0-4095"})
    r2 = rb.request("GET", "/%s/%s/raw.%s" % (tag, info2["code"], info2["ext"]), headers={"Range": "bytes=4096-"})
    ctx.expect("fix1 reference: an encrypted one-time file cannot be read to the end",
               h2.status == 206 and r2.status == 404, {"head": h2.status, "rest": r2.status})
    # 4. A video player: the opening request counts, its seeks ride on the grant.
    info = f["fix1_video"]
    seed.set_cols(info["id"], max_views=1)
    br = parity.Browser(ctx.run.rust, real_ip=ctx.run.real_ip, user_agent=BROWSER_UA)
    first = br.request("GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]), headers={"Range": "bytes=0-"})
    seeks = [br.request("GET", "/%s/%s/raw.%s" % (tag, info["code"], info["ext"]),
                        headers={"Range": "bytes=%d-" % n, "Sec-Fetch-Dest": "video"}).status for n in (100, 5000, 9)]
    (_, _), (sv, _) = seed.counters(info["id"])
    ctx.expect("fix1 flow: a player's seeks after the counted opening request",
               first.status == 206 and seeks == [206, 206, 206] and sv == "1",
               {"first": first.status, "seeks": seeks, "views": sv})
    # 5. A grant is bound to its file.
    other = f["fix1_other"]
    seed.set_cols(other["id"], max_views=1)
    stolen = br.request("GET", "/%s/%s/raw" % (tag, other["code"]), headers={"Range": "bytes=1-"})
    stolen2 = parity.Browser(ctx.run.rust, real_ip=ctx.run.real_ip).request(
        "GET", "/%s/%s/raw" % (tag, other["code"]), headers={"Range": "bytes=1-"})
    ctx.expect("fix1: a grant opens nothing but its own file",
               stolen.status == 206 and stolen2.status == 404, {"first": stolen.status, "second": stolen2.status})
    # 6. The album ZIP: a capped item pays a view to travel, and is left out
    #    once the budget is spent (the reference builds no ZIP at all, see
    #    area_zip).
    zinfo = f["fix1_zip"]
    seed.set_cols(zinfo["id"], max_views=1)
    folder = ctx.album_slug
    b = ctx.pair().rust.request("GET", "/a/%s/%s/zip" % (tag, folder), origin=False)
    b2 = ctx.pair().rust.request("GET", "/a/%s/%s/zip" % (tag, folder), origin=False)
    names = lambda r: sorted(zipfile.ZipFile(io.BytesIO(r.body)).namelist()) if r.status == 200 else r.status
    (_, _), (sv, sd) = seed.counters(zinfo["id"])
    zname = zinfo["filename"]
    ctx.expect("fix1 zip: Rust ships a capped item once, counted as a download",
               zname in names(b) and zname not in names(b2) and (sv, sd) == ("1", "1"),
               {"first": names(b), "second": names(b2), "counters": [sv, sd]})


# ── render ─────────────────────────────────────────────────────────────────

def render_path_of(ctx, tag, info, pair):
    r = pair.rust.request("GET", "/_page?path=" + urllib.parse.quote("/%s/%s" % (tag, info["code"]), safe=""),
                          origin=False)
    return (((r.json() or {}).get("props") or {}).get("viewer") or {}).get("renderPath")


def area_render(ctx, f, tag, owner):
    seed = ctx.seed
    own = ctx.pair()
    parity.signed_in(own, seed.ref_db, seed.rust_db, owner)
    for key in ("html", "svg", "docx", "txt"):
        info = f[key]
        path = render_path_of(ctx, tag, info, own)
        if key == "txt":
            ctx.expect("render: no ticket for plain text", path is None, path)
            path = "/%s/%s/render?t=x" % (tag, info["code"])
        for dest in ("iframe", "document", None, "image"):
            h = {"Sec-Fetch-Dest": dest} if dest else {}
            a, b = both(own, "GET", path, h)
            ctx.compare("render %s dest=%s" % (key, dest), shape(a, "len"), shape(b, "len"), pending_a=key == "docx"
                        and dest == "iframe")
        a, b = both(own, "GET", path.replace("/render", "/%72ender"), {"Sec-Fetch-Dest": "iframe"})
        ctx.compare("render %s encoded action" % key, shape(a), shape(b))
        stranger = ctx.pair()
        a, b = both(stranger, "GET", path, {"Sec-Fetch-Dest": "iframe"})
        ctx.compare("render %s stranger with the ticket" % key, shape(a, "len"), shape(b, "len"),
                    pending_a=key == "docx")
    a, b = both(own, "GET", "/%s/%s/render?t=%s.9999999999.forged" % (tag, f["html"]["code"], f["html"]["id"]),
                {"Sec-Fetch-Dest": "iframe"})
    ctx.compare("render forged ticket", shape(a, "text"), shape(b, "text"))


# ── pages ──────────────────────────────────────────────────────────────────

HEAD_TAG_RE = re.compile(r'<(title|meta|link)\b([^>]*)>(?:(.*?)</title>)?', re.S)


def head_tags(document):
    """The link-preview relevant head tags, in order. Searched in the whole
    document: Next 16 streams the metadata of a slow `generateMetadata` into
    the body for browsers (it blocks the head only for the bots it knows)."""
    head = document.split('<script id="__MODLIGHT_PAGE__"')[0]
    out = []
    for tagname, attrs, text in HEAD_TAG_RE.findall(head):
        a = dict((k, htmllib.unescape(v)) for k, v in re.findall(r'([\w:-]+)="([^"]*)"', attrs))
        if tagname == "title":
            out.append(("title", htmllib.unescape(text)))
        elif tagname == "meta" and ("property" in a or a.get("name") in (
                "description", "application-name", "robots", "twitter:card", "twitter:title", "twitter:description",
                "twitter:image")):
            out.append((a.get("property") or a.get("name"), a.get("content")))
        elif tagname == "link" and a.get("rel") in ("canonical", "alternate"):
            out.append((a.get("rel"), a.get("type"), a.get("href")))
    return out


def head_text(document):
    return json.dumps(head_tags(document), ensure_ascii=False)


ACTION_KEYS = ("publicUrl", "rawUrl", "downloadUrl", "copyRaw", "mimeType", "ext", "frames", "canEncode", "size",
               "width", "height", "signInUrl")


def undef(v):
    return None if v == "<undefined>" else v


THEME_IDS = ("default", "glass", "synthwave", "terminal", "paper", "win98", "nebula", "grimoire", "cartoon", "mono",
             "deco", "riso", "discord", "spotify")


def resolve_theme(v):
    """lib/themes.ts resolveTheme."""
    if v in THEME_IDS:
        return v
    return "default" if v == "modern" else "discord"


def ticketless(path):
    return re.sub(r"render\?t=.*$", "render?t=<ticket>", path) if isinstance(path, str) else path


def ref_page_props(doc):
    """The reference's page data, read off the CLIENT components it rendered
    (upstream's MediaViewer is a server component, so its own props never
    reach the payload; MediaActions, SourceView, DocumentView, MediaPlayer and
    PlaybackNote carry everything it was given)."""
    fl = Flight(doc)
    out = {"ThemeApplier": fl.one("ThemeApplier")}
    for name in ("PasswordGate", "EncryptedViewer"):
        found = fl.elements(name)
        if found:
            out[name] = found[0]
    actions = fl.elements("MediaActions")
    if actions:
        v = dict((k, undef(actions[0].get(k))) for k in ACTION_KEYS)
        source = fl.elements("SourceView")
        if source:
            v["textContent"] = undef(source[0].get("content"))
            v["textUnavailable"] = undef(source[0].get("unavailable"))
            pages = [r.get("href") for r in source[0].get("readings") or [] if r.get("mode") == "page"]
            v["renderPath"] = ticketless(pages[0]) if pages else None
        document = fl.elements("DocumentView")
        if document:
            v["renderPath"] = ticketless(undef(document[0].get("renderPath")))
        player = [p for p in fl.elements("MediaPlayer") if p.get("variant") != "audio"]
        if player:
            v["timelineTrack"] = undef(player[0].get("timelineTrack"))
            v["posterUrl"] = undef(player[0].get("poster"))
        note = fl.elements("PlaybackNote")
        v["playback"] = note[0].get("diagnosis") if note else None
        v["playbackRepair"] = undef(note[0].get("repair")) if note else None
        out["MediaViewer"] = v
    return out


def rust_page_props(payload, ref):
    """The Rust payload projected onto what the reference shows (`ref`)."""
    p = (payload or {}).get("props") or {}
    out = {"ThemeApplier": {"theme": p.get("ownerTheme"), "background": p.get("ownerBackground")}}
    if p.get("view") == "media":
        viewer = p["viewer"]
        v = dict((k, viewer.get(k)) for k in ACTION_KEYS)
        shown = (ref or {}).get("MediaViewer") or {}
        for k in ("textContent", "textUnavailable"):
            if k in shown:
                v[k] = viewer.get(k)
        if "renderPath" in shown:
            v["renderPath"] = ticketless(viewer.get("renderPath"))
        if "timelineTrack" in shown:
            v["timelineTrack"] = viewer.get("timelineTrack")
            v["posterUrl"] = viewer.get("posterUrl")
        v["playback"] = viewer.get("playback")
        v["playbackRepair"] = viewer.get("playbackRepair")
        out["MediaViewer"] = v
        # Nothing the reference would not have drawn may travel either.
        if "textContent" not in shown and viewer.get("textContent") is not None:
            out["unexpected textContent"] = True
    elif p.get("view") == "gate":
        out["PasswordGate"] = {"mediaId": p["mediaId"], "appName": "Modlight", "onDefaultPassword": p["onDefaultPassword"]}
    elif p.get("view") == "encrypted":
        out["EncryptedViewer"] = {"rawPath": p["rawPath"], "cipherSize": p["cipherSize"], "createdAt": p["createdAt"],
                                  "views": p["views"], "downloads": p["downloads"], "appName": "Modlight"}
    return out


def tidy_viewer(props):
    t = props.get("ThemeApplier")
    if isinstance(t, dict):
        for k in list(t):
            t[k] = undef(t[k])
    return props


def page_outcome(r):
    if r.status in (301, 302, 303, 307, 308):
        return {"redirect": r.header("location"), "status": r.status}
    if r.status == 404:
        return {"notFound": True}
    return {"page": r.status}


def rust_payload(document):
    m = re.search(r'<script id="__MODLIGHT_PAGE__" type="application/json">(.*?)</script>', document, re.S)
    return json.loads(m.group(1)) if m else None


def check_page(ctx, name, pair, path, props=True, pending_a=False):
    a = pair.ref.request("GET", path, origin=False, headers={"Accept": "text/html"})
    b = pair.rust.request("GET", path, origin=False, headers={"Accept": "text/html"})
    ctx.compare("page %s (outcome)" % name, page_outcome(a), page_outcome(b))
    if a.status != 200 or b.status != 200:
        return a, b
    doc, rdoc = a.text(), b.text()
    ctx.compare("page %s (head)" % name, head_tags(doc), head_tags(rdoc))
    html_attr = lambda d: dict(re.findall(r'(data-theme|data-bg)="([^"]*)"', re.search(r"<html[^>]*>", d).group(0)))
    if props:
        ref = tidy_viewer(ref_page_props(doc))
        ctx.compare("page %s (props)" % name, ref, tidy_viewer(rust_page_props(rust_payload(rdoc), ref)),
                    pending_a=pending_a)
        # The document carries the owner's skin and wallpaper the way the
        # reference's <ThemeApplier> script paints them (CONTRACT 2.2, 12.2).
        applier = ref.get("ThemeApplier") or {}
        want = {"data-theme": resolve_theme(applier.get("theme"))}
        if applier.get("background"):
            want["data-bg"] = "on"
        ctx.compare("page %s (document skin)" % name, want, html_attr(rdoc))
    return a, b


def area_pages(ctx, f, tag, owner, admin):
    seed = ctx.seed
    for who in ("anon", "owner", "admin"):
        pair = ctx.pair()
        if who != "anon":
            parity.signed_in(pair, seed.ref_db, seed.rust_db, owner if who == "owner" else admin)
        for key in ("png", "jpg", "wide", "gif", "webp", "heic", "mp4", "mp3", "txt", "bigtxt", "html", "svg", "json",
                    "pdf", "zip", "docx", "mlenc", "paste"):
            info = f[key]
            for spelling in ([info["code"]] + (["%s.%s" % (info["code"], info["ext"])] if info["ext"] else [])):
                check_page(ctx, "%s %s %s" % (who, key, spelling), pair, "/%s/%s" % (tag, spelling))
    # Gated pages.
    anon = ctx.pair()
    check_page(ctx, "password gate", anon, "/%s/%s" % (tag, f["gate_password"]["code"]))
    check_page(ctx, "private (anon)", anon, "/%s/%s" % (tag, f["gate_private"]["code"]))
    own = ctx.pair()
    parity.signed_in(own, seed.ref_db, seed.rust_db, owner)
    check_page(ctx, "private (owner)", own, "/%s/%s" % (tag, f["gate_private"]["code"]))
    check_page(ctx, "password (owner)", own, "/%s/%s" % (tag, f["gate_password"]["code"]))
    check_page(ctx, "expired", anon, "/%s/%s" % (tag, f["gate_expired"]["code"]))
    for path in ("/%s/nope" % tag, "/nobody/%s" % f["png"]["code"], "/x/y"):
        check_page(ctx, "missing %s" % path, anon, path)
    # The counters the page views left behind.
    for key in ("png", "txt", "mlenc"):
        a, b = seed.counters(f[key]["id"])
        ctx.compare("page counters %s" % key, a, b)


def area_bots(ctx, f, tag):
    for bot, ua in BOTS.items():
        pair = ctx.pair(ua=ua)
        for key in ("png", "wide", "mp4", "mp3", "txt", "zip", "pdf"):
            info = f[key]
            check_page(ctx, "%s %s" % (bot, key), pair, "/%s/%s.%s" % (tag, info["code"], info["ext"]))
    for key in ("png", "txt"):
        a, b = ctx.seed.counters(f[key]["id"])
        ctx.compare("bot counters %s" % key, a, b)


def area_tagless(ctx, f, tag, tagless_tag, tf):
    anon = ctx.pair()
    info = tf
    for path in ["/%s.%s" % (info["code"], info["ext"]), "/%s" % info["code"], "/%s/raw" % info["code"],
                 "/%s/raw.%s?w=64&w=128&b=x%%20y&10=a&2=b" % (info["code"], info["ext"]),
                 "/%s/download" % info["code"], "/%s/convert?to=jpg&check=1" % info["code"],
                 "/%s/timeline.vtt" % info["code"], "/%s/RAW" % info["code"], "/%s/render" % info["code"],
                 "/%s.%s" % (f["png"]["code"], f["png"]["ext"]), "/login", "/help", "/.hidden"]:
        a = anon.ref.request("GET", path, origin=False, headers={"Accept": "text/html"})
        b = anon.rust.request("GET", path, origin=False, headers={"Accept": "text/html"})
        ctx.compare("tagless %s (outcome)" % path, page_outcome(a), page_outcome(b))
    check_page(ctx, "tagless page", anon, "/%s.%s" % (info["code"], info["ext"]))


def area_public(ctx, tag, owner, album_slug):
    anon = ctx.pair()
    for path in ["/u/%s" % tag, "/a/%s/%s" % (tag, album_slug), "/u/nobody", "/a/%s/nope" % tag]:
        a = anon.ref.request("GET", path, origin=False, headers={"Accept": "text/html"})
        b = anon.rust.request("GET", path, origin=False, headers={"Accept": "text/html"})
        ctx.compare("public %s (outcome)" % path, page_outcome(a), page_outcome(b))
        if a.status == 200 and b.status == 200:
            ctx.compare("public %s (head)" % path, head_tags(a.text()), head_tags(b.text()))
            ref_gallery = Flight(a.text()).elements("PublicGallery")
            props = (rust_payload(b.text()) or {}).get("props") or {}
            ctx.compare("public %s (tiles)" % path, ref_gallery[0].get("items") if ref_gallery else None,
                        props.get("items") or props.get("tiles"))


def area_zip(ctx, tag, album_slug, f):
    """The reference cannot build a ZIP at all: `import archiver from
    "archiver"` against archiver 8, which has no default export, throws
    `TypeError: (0, a.default) is not a function` and every album download is
    a 500. The port serves the archive the route was written to produce; its
    entries are checked against the files the album holds."""
    anon = ctx.pair()
    path = "/a/%s/%s/zip" % (tag, album_slug)
    a, b = both(anon, "GET", path)
    ctx.expect("zip: the reference answers 500 (archiver 8 has no default export)", a.status == 500, a.status)
    hdrs = ("content-type", "content-disposition", "cache-control", "transfer-encoding")
    sb = shape(b, "len", hdrs)
    sb.pop("body")
    ctx.expect("zip: Rust streams the archive", sb == {
        "status": 200, "content-type": "application/zip", "cache-control": "no-store",
        "content-disposition": 'attachment; filename="%s.zip"' % album_slug, "transfer-encoding": "chunked",
        "set-cookie": []}, sb)
    if b.status == 200:
        entries = zip_entries(b.body)
        want = []
        for key in ("fix1_zip", "txt", "mp4", "png"):  # newest first
            data = f[key]["data"]
            want.append({"name": f[key]["filename"], "size": len(data), "method": 8, "flags": 8,
                         "made_by": (3, 45), "extract": 20, "attrs": 0x81a40020,
                         "sha": hashlib.sha256(data).hexdigest()[:16]})
        got = [dict((k, v) for k, v in e.items() if k != "crc") for e in entries]
        ctx.expect("zip: entries, order, method, attributes and bytes", got == want, {"got": got, "want": want})
    for p in ["/a/%s/nope/zip" % tag, "/a/nobody/x/zip"]:
        a, b = both(anon, "GET", p)
        ctx.compare("zip %s" % p, shape(a, "text", ("content-type",)), shape(b, "text", ("content-type",)))


def zip_entries(body):
    z = zipfile.ZipFile(io.BytesIO(body))
    out = []
    for i in z.infolist():
        out.append({"name": i.filename, "size": i.file_size, "crc": i.CRC, "method": i.compress_type,
                    "flags": i.flag_bits, "made_by": (i.create_system, i.create_version),
                    "extract": i.extract_version, "attrs": i.external_attr,
                    "sha": hashlib.sha256(z.read(i)).hexdigest()[:16]})
    return out


def area_oembed(ctx, f, tag):
    pair = ctx.pair()
    for key in ("png", "wide", "mp4", "mp3", "zip", "txt", "gate_password", "gate_private", "fix6_capped_png",
                "fix6_capped_mp4"):
        info = f[key]
        target = "http://localhost/%s/%s.%s" % (tag, info["code"], info["ext"])
        a, b = both(pair, "GET", "/oembed?url=%s&format=json" % urllib.parse.quote(target, safe=""))
        sa = shape(a, "json", ("content-type", "cache-control", "access-control-allow-origin"))
        sb = shape(b, "json", ("content-type", "cache-control", "access-control-allow-origin"))
        if key.startswith("fix6"):
            body = sb.get("body") or {}
            ctx.expect("fix6 oembed %s: Rust describes it without its bytes" % key,
                       body.get("type") == "link" and "url" not in body and "html" not in body and "width" not in body
                       and body.get("title") == info["filename"], body)
            rbody = sa.get("body") or {}
            ctx.expect("fix6 oembed %s: the reference hands out the raw URL" % key, "raw" in json.dumps(rbody), rbody)
            continue
        ctx.compare("oembed %s" % key, sa, sb)
    for q in ["", "url=", "format=xml&url=x", "url=nonsense", "url=http://localhost/onlyone",
              "url=http://localhost/%s/nope" % tag, "url=foo:%s/%s" % (tag, f["png"]["code"]),
              "url=http://localhost/%s/%s/raw/extra" % (tag, f["png"]["code"])]:
        a, b = both(pair, "GET", "/oembed?" + q)
        ctx.compare("oembed ?%s" % q, shape(a, "json", ("content-type", "cache-control")),
                    shape(b, "json", ("content-type", "cache-control")))


def area_delete(ctx, f, tag, owner):
    seed = ctx.seed
    pair = ctx.pair()
    write = seed.seed_token(owner, "write")
    read = seed.seed_token(owner, "read")
    info = f["del_token"]
    dt = seed.ref_db.sql("SELECT delete_token FROM uploads WHERE id = %s" % Q(info["id"])).strip()
    hdrs = ("content-type", "content-security-policy", "x-content-type-options", "referrer-policy", "cache-control")
    for label, token in (("wrong token", "nope"), ("read api token", read), ("prefix of the token", dt[:-1]),
                         ("empty-ish", "%20")):
        for method in ("GET", "POST"):
            a, b = both(pair, method, "/%s/%s/delete/%s" % (tag, info["code"], token))
            ctx.compare("delete %s %s" % (label, method), shape(a, "text", hdrs), shape(b, "text", hdrs))
    a, b = both(pair, "GET", "/%s/%s/delete/%s" % (tag, info["code"], dt))
    ctx.compare("delete page", shape(a, "text", hdrs), shape(b, "text", hdrs))
    a, b = both(pair, "POST", "/%s/%s/delete/%s" % (tag, info["code"], dt))
    ctx.compare("delete by token", shape(a, "text", hdrs), shape(b, "text", hdrs))
    a, b = both(pair, "GET", "/%s/%s/raw" % (tag, info["code"]))
    ctx.compare("delete: gone afterwards", shape(a, "text"), shape(b, "text"))
    a, b = seed.both_sql("SELECT count(*) FROM uploads WHERE id = %s" % Q(info["id"]))
    ctx.compare("delete: row removed", a, b)
    info = f["del_api"]
    a, b = both(pair, "POST", "/%s/%s/delete/%s" % (tag, info["code"], write))
    ctx.compare("delete by write api token", shape(a, "text", hdrs), shape(b, "text", hdrs))
    xss = f["del_xss"]
    seed.set_cols(xss["id"], filename="<img src=x onerror=\"alert('x')\">.png")
    dt = seed.ref_db.sql("SELECT delete_token FROM uploads WHERE id = %s" % Q(xss["id"])).strip()
    a, b = both(pair, "GET", "/%s/%s/delete/%s" % (tag, xss["code"], dt))
    ctx.compare("delete page escapes the filename", shape(a, "text", hdrs), shape(b, "text", hdrs))


def area_embed(ctx):
    pair = ctx.pair()
    for path in ["/embed/v1/archive-ZIP.png", "/embed/v1/binary-FILE.png", "/embed/v2/archive-ZIP.png",
                 "/embed/v1/code-ZIP.png", "/embed/v1/archive-zip.png", "/embed/v1/archive-ZIP.svg"]:
        a, b = both(pair, "GET", path)
        ok = path in ("/embed/v1/archive-ZIP.png", "/embed/v1/binary-FILE.png")
        ctx.compare("embed %s" % path, shape(a, "len", ("content-type", "cache-control", "content-disposition")),
                    shape(b, "len", ("content-type", "cache-control", "content-disposition")), pending_a=ok)


def area_extras(ctx, f, tag, owner, admin, dressed):
    """What depends on the owner's account and on who is looking: skin and
    wallpaper, copyRaw, the default password ("remember" across files), the
    playback note, the sign-in link of a secondary hostname, an oversized still
    handed to a media bot, and /_page agreeing with the document."""
    seed = ctx.seed
    d_owner, d_tag, d_files = dressed
    anon = ctx.pair()
    for key in ("png", "txt", "gate"):
        info = d_files[key]
        check_page(ctx, "dressed owner %s" % key, anon, "/%s/%s" % (d_tag, info["code"]))
    check_page(ctx, "dressed gallery", anon, "/u/%s" % d_tag, props=False)
    # The default password: a file stamped with the owner's current
    # generation offers "remember", and remembering opens the owner's other
    # stamped file too.
    visitor = ctx.pair(ip="192.0.2.%d" % (hash(d_tag) % 200 + 20))
    gate = d_files["gate"]
    ctx.actions.check(visitor, "unlock default password, remember", "unlock", "verifyMediaPassword",
                      [gate["id"], "defaultpw", True])
    other = d_files["gate2"]
    a, b = both(visitor, "GET", "/%s/%s/raw" % (d_tag, other["code"]))
    ctx.compare("default password opens the sibling file", shape(a), shape(b))
    # Playback note: an oversized, starved recording seen by its owner (and not
    # by an administrator).
    mp4 = f["playback"]
    seed.set_cols(mp4["id"], width=5056, height=1924, frames=403, duration_seconds=13.433, fps=30)
    for who, user in (("owner", owner), ("admin", admin)):
        pair = ctx.pair()
        parity.signed_in(pair, seed.ref_db, seed.rust_db, user)
        check_page(ctx, "playback note %s" % who, pair, "/%s/%s" % (tag, mp4["code"]))
    # A stranger on another hostname gets the sign-in link.
    for host in ("other.example", "localhost"):
        a = anon.ref.request("GET", "/%s/%s" % (tag, f["png"]["code"]), origin=False, headers={"Host": host})
        b = anon.rust.request("GET", "/%s/%s" % (tag, f["png"]["code"]), origin=False, headers={"Host": host})
        ra = (Flight(a.text()).one("MediaActions") or {}) if a.status == 200 else a.status
        rb = ((rust_payload(b.text()) or {}).get("props") or {}).get("viewer") or b.status
        ctx.compare("signInUrl on host %s" % host, undef(ra.get("signInUrl")) if isinstance(ra, dict) else ra,
                    rb.get("signInUrl") if isinstance(rb, dict) else rb)
    # An oversized still is handed to a media bot as a 1200px render.
    big = f["bigstill"]
    seed.set_cols(big["id"], size=9 * 1024 * 1024)
    bot = ctx.pair(ua=BOTS["discord"])
    check_page(ctx, "discord oversized still", bot, "/%s/%s.%s" % (tag, big["code"], big["ext"]))
    # /_page carries the same props as the document (the SPA's own path).
    for key in ("png", "txt", "mp4"):
        info = f[key]
        doc = anon.rust.request("GET", "/%s/%s" % (tag, info["code"]), origin=False).text()
        nav = anon.rust.request("GET", "/_page?path=" + urllib.parse.quote("/%s/%s" % (tag, info["code"]), safe=""),
                                origin=False).json()
        strip = lambda p: dict((k, v) for k, v in ((p or {}).get("props") or {}).items() if k != "viewer")
        ctx.compare("/_page and the document agree (%s)" % key, strip(rust_payload(doc)), strip(nav))


def main():
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--ref-db", default="modlight-srvmediab-ref-postgres-1:modlight")
    p.add_argument("--rust-db", default="modlight-devdb-postgres-1:modlight_srv_media_b")
    p.add_argument("--ref-app", default="modlight-srvmediab-ref-app-1")
    p.add_argument("--secrets", default=os.path.join(SCRATCH, "secrets.env"))
    p.add_argument("--only", default=None)
    a, _ = p.parse_known_args()
    run = parity.Run.from_args()
    secret = parity.read_env_file(a.secrets)["AUTH_SECRET"]
    rc, rd = a.ref_db.split(":")
    uc, ud = a.rust_db.split(":")
    seed = Seeder(run, parity.Db(rc, rd, secret), parity.Db(uc, ud, secret), run.ref, run.rust)
    ctx = Ctx(run, seed, parity.Actions(run, a.ref_app))

    owner, tag = seed.seed_user("o", public_profile=True)
    admin, _ = seed.seed_user("a", role="admin")
    token = seed.seed_token(owner)
    up = lambda key, label, **c: seed.upload(owner, token, key, label, **c)
    f = {}
    for key in ("png", "jpg", "wide", "gif", "webp", "heic", "mp4", "mp3", "txt", "bigtxt", "html", "svg", "json",
                "pdf", "zip", "docx", "mlenc"):
        f[key] = up(key, key)
    f["paste"] = up("txt", "paste")
    seed.set_cols(f["paste"]["id"], kind="paste", language="python", filename="snippet.py", ext="py")
    f["paste"].update(kind="paste", filename="snippet.py", ext="py")
    f["gate_private"] = up("png", "gpriv")
    f["gate_password"] = up("png", "gpw")
    f["gate_expired"] = up("png", "gexp")
    for i in range(9):
        f["fix1_%d" % i] = up("txt", "fx%d" % i)
    f["fix1_page_text"] = up("txt", "fxpt")
    f["fix1_page_image"] = up("png", "fxpi")
    f["fix1_encrypted"] = up("mlenc", "fxen")
    f["fix1_encrypted_ref"] = up("mlenc", "fxer")
    f["fix1_video"] = up("mp4", "fxvid")
    f["fix1_other"] = up("txt", "fxoth")
    f["fix1_zip"] = up("txt", "fxzip")
    f["fix6_capped_png"] = up("png", "f6png", max_views=3)
    f["fix6_capped_mp4"] = up("mp4", "f6mp4", max_views=3)
    f["del_token"] = up("png", "deltok")
    f["del_api"] = up("png", "delapi")
    f["del_xss"] = up("png", "delxss")

    # A public album holding a few kinds and the capped item of fix1.
    folder_id = seed.ref_db.sql(
        "INSERT INTO folders (owner_id, name, slug, is_public) VALUES (%s, 'Holiday', 'holiday-%s', true) RETURNING id"
        % (Q(owner), seed.suffix)).strip().splitlines()[0]
    seed.rust_db.sql("INSERT INTO folders (id, owner_id, name, slug, is_public) VALUES (%s, %s, 'Holiday', 'holiday-%s', true)"
                     % (Q(folder_id), Q(owner), seed.suffix))
    ctx.album_slug = "holiday-%s" % seed.suffix
    for key in ("png", "mp4", "txt", "fix1_zip"):
        seed.set_cols(f[key]["id"], folder_id=folder_id)
    # Tagless links: an admin who opted out of the tag segment.
    tl_owner, tl_tag = seed.seed_user("t", role="admin", skip_tag_in_url=True)
    tl_token = seed.seed_token(tl_owner)
    tf = seed.upload(tl_owner, tl_token, "png", "tagless")
    f["playback"] = up("mp4", "pback")
    f["bigstill"] = up("png", "bigst")
    # An owner who dressed their links: a skin, a wallpaper, copyRaw, and a
    # default file password stamped on two files.
    d_owner, d_tag = seed.seed_user("d", theme="glass", copy_raw=True, background_dim=35,
                                    background_key="backgrounds/msd%s-AbCdEf12.webp" % seed.suffix)
    d_token = seed.seed_token(d_owner)
    d_files = dict((k, seed.upload(d_owner, d_token, src, "d" + k)) for k, src in
                   (("png", "png"), ("txt", "txt"), ("gate", "png"), ("gate2", "jpg")))
    seed.both_sql('UPDATE "user" SET public_profile = true WHERE id = %s' % Q(d_owner))
    own_rust = parity.Browser(run.rust, real_ip=run.real_ip, user_agent=BROWSER_UA)
    own_rust.cookies[parity.SESSION_COOKIE] = seed.rust_db.mint_session(d_owner)
    own_rust.rpc("account", "updateDefaultFilePassword", ["defaultpw"])
    stored = seed.rust_db.sql('SELECT default_password_hash, default_password_version FROM "user" WHERE id = %s'
                              % Q(d_owner)).strip().split("\t")
    seed.ref_db.sql('UPDATE "user" SET default_password_hash = %s, default_password_version = %s WHERE id = %s'
                    % (Q(stored[0]), stored[1], Q(d_owner)))
    for k in ("gate", "gate2"):
        seed.set_cols(d_files[k]["id"], password_hash=stored[0], password_version=int(stored[1]))

    areas = [
        ("raw", lambda: area_raw(ctx, dict((k, f[k]) for k in ("png", "mp4", "txt", "html", "svg", "zip", "mlenc",
                                                                   "pdf", "paste")), tag)),
        ("thumbs", lambda: area_thumbs(ctx, f, tag)),
        ("gates", lambda: area_gates(ctx, f, tag, owner, admin)),
        ("render", lambda: area_render(ctx, f, tag, owner)),
        ("pages", lambda: area_pages(ctx, f, tag, owner, admin)),
        ("bots", lambda: area_bots(ctx, f, tag)),
        ("tagless", lambda: area_tagless(ctx, f, tag, tl_tag, tf)),
        ("public", lambda: area_public(ctx, tag, owner, ctx.album_slug)),
        ("zip", lambda: area_zip(ctx, tag, ctx.album_slug, f)),
        ("oembed", lambda: area_oembed(ctx, f, tag)),
        ("delete", lambda: area_delete(ctx, f, tag, owner)),
        ("embed", lambda: area_embed(ctx)),
        ("extras", lambda: area_extras(ctx, f, tag, owner, admin, (d_owner, d_tag, d_files))),
        ("fix1", lambda: area_fix1(ctx, f, tag)),
    ]
    for name, fn in areas:
        if a.only and a.only != name:
            continue
        fn()
    if ctx.pending:
        print("\npending-A (part A not merged; differ until it is): %d" % len(ctx.pending))
        if run.verbose:
            for n in ctx.pending:
                print("  " + n)
    run.finish()


if __name__ == "__main__":
    main()

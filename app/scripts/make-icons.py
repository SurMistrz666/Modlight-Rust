#!/usr/bin/env python3
"""Builds the Graphite icon set from assets/icons/Modlight.png.

Writes:
  app/assets/icons/Modlight.ico       256/128/64/48 resized from the 1024 px artwork,
                                      32/24/16 drawn on the pixel grid (simpler strokes)
  app/assets/icons/Modlight-<n>.png   the hand-tuned 16/24/32 frames, for review
  server/public/icon.svg              the browser tab icon (the 32 px drawing as vectors)
  server/public/apple-icon.png        180 px, opaque square (iOS rounds it itself)
  server/public/brand/email-logo.png  128 px, the rounded icon
  server/public/brand/audio-cover.png 1280x720 cover for audio turned into video

Needs rsvg-convert and ImageMagick 7 (`magick`). Run from anywhere:
  python3 app/scripts/make-icons.py
"""

import os
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
ICONS = os.path.join(ROOT, "app", "assets", "icons")
PUBLIC = os.path.join(ROOT, "server", "public")
ARTWORK = os.path.join(ICONS, "Modlight.png")

ACCENT = "#3b82f6"
TOP = "#1f1f1f"
BOTTOM = "#0a0a0a"
RIM = "#2e2e2e"


def tile(size, radius, rim=True):
    """The dark rounded square with its vertical gradient and a thin rim."""
    out = [
        "<defs><linearGradient id='g' x1='0' y1='0' x2='0' y2='1'>"
        f"<stop offset='0' stop-color='{TOP}'/><stop offset='1' stop-color='{BOTTOM}'/>"
        "</linearGradient></defs>",
        f"<rect width='{size}' height='{size}' rx='{radius}' fill='url(#g)'/>",
    ]
    if rim:
        out.append(
            f"<rect x='0.5' y='0.5' width='{size - 1}' height='{size - 1}' rx='{radius - 0.5}'"
            f" fill='none' stroke='{RIM}' stroke-width='1'/>"
        )
    return "".join(out)


def corners(lo, hi, stroke, arm):
    """The four capture corners as pixel-aligned rectangles: box [lo, hi)."""
    w = hi - lo
    rects = []
    for (x, y, sx, sy) in ((lo, lo, 1, 1), (hi, lo, -1, 1), (lo, hi, 1, -1), (hi, hi, -1, -1)):
        hx = x if sx > 0 else x - arm
        hy = y if sy > 0 else y - stroke
        vx = x if sx > 0 else x - stroke
        vy = y if sy > 0 else y - arm
        rects.append((hx, hy, arm, stroke))
        rects.append((vx, vy, stroke, arm))
    assert w > 2 * arm
    return "".join(f"<rect x='{x}' y='{y}' width='{a}' height='{b}' fill='#fff'/>" for x, y, a, b in rects)


def pixels(cells, colour):
    return "".join(f"<rect x='{x}' y='{y}' width='1' height='1' fill='{colour}'/>" for x, y in cells)


def frame16():
    """16 px: 1 px corners, a 2 px arrow drawn pixel by pixel."""
    cells = set()
    for y in range(5, 12):  # shaft
        cells |= {(7, y), (8, y)}
    cells |= {(6, 5), (9, 5)}
    cells |= {(5, 6), (6, 6), (9, 6), (10, 6)}
    cells |= {(4, 7), (5, 7), (10, 7), (11, 7)}
    cells |= {(7, 4), (8, 4)}
    body = tile(16, 3.5, rim=False) + corners(2, 14, 1, 4) + pixels(sorted(cells), ACCENT)
    return svg(16, body)


def frame24():
    """24 px: 2 px corners and shaft, a 45 degree chevron."""
    arrow = (
        f"<g fill='none' stroke='{ACCENT}' stroke-linecap='round' stroke-linejoin='round'>"
        "<path d='M12 8 V17' stroke-width='2'/>"
        "<path d='M8.25 11.5 L12 7.75 L15.75 11.5' stroke-width='2.1'/></g>"
    )
    body = tile(24, 5.25) + corners(3, 21, 2, 5) + arrow
    return svg(24, body)


def frame32():
    """32 px: 2 px corners, 2 px shaft, a 45 degree chevron with round caps."""
    arrow = (
        f"<g fill='none' stroke='{ACCENT}' stroke-linecap='round' stroke-linejoin='round'>"
        "<path d='M16 11 V22' stroke-width='2'/>"
        "<path d='M11 15.6 L16 10.6 L21 15.6' stroke-width='2.4'/></g>"
    )
    body = tile(32, 7.25) + corners(6, 26, 2, 7) + arrow
    return svg(32, body)


def svg(size, body):
    return (
        f"<svg xmlns='http://www.w3.org/2000/svg' width='{size}' height='{size}' "
        f"viewBox='0 0 {size} {size}' shape-rendering='geometricPrecision'>{body}</svg>"
    )


def favicon_svg():
    """The browser tab icon: the hand-tuned 32 px drawing, as vectors."""
    body = frame32().split(">", 1)[1].rsplit("</svg>", 1)[0]
    body = body.replace("><", ">\n  <").replace("'", '"')
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="32" height="32" '
        'role="img" aria-label="Modlight">\n  ' + body + "\n</svg>\n"
    )


def run(*args):
    subprocess.run(args, check=True)


def render(svg_text, out, size, work):
    src = os.path.join(work, os.path.basename(out) + ".svg")
    with open(src, "w") as f:
        f.write(svg_text)
    run("rsvg-convert", "-w", str(size), "-h", str(size), "-o", out, src)


def main():
    if not os.path.exists(ARTWORK):
        sys.exit(f"missing {ARTWORK}")
    with tempfile.TemporaryDirectory() as work:
        small = {}
        for size, make in ((16, frame16), (24, frame24), (32, frame32)):
            out = os.path.join(ICONS, f"Modlight-{size}.png")
            render(make(), out, size, work)
            small[size] = out
        big = []
        for size in (256, 128, 64, 48):
            out = os.path.join(work, f"big-{size}.png")
            run("magick", ARTWORK, "-filter", "Lanczos", "-resize", f"{size}x{size}", "-depth", "8", out)
            big.append(out)
        frames = big + [small[32], small[24], small[16]]
        # PNG-compressed 256 frame, BMP for the rest: what Windows expects.
        run("magick", *frames, os.path.join(ICONS, "Modlight.ico"))

        with open(os.path.join(PUBLIC, "icon.svg"), "w") as f:
            f.write(favicon_svg())
        # iOS masks the touch icon itself: an opaque square, no rounded corners.
        apple = svg(1024, tile(1024, 0, rim=False)).replace("</svg>", "")
        art = os.path.join(work, "art-square.png")
        render(apple + "</svg>", art, 1024, work)
        run(
            "magick", art, ARTWORK, "-compose", "over", "-composite",
            "-filter", "Lanczos", "-resize", "180x180", "-depth", "8", "-strip",
            # RGBA like the files they replace: the tests use them as sample PNGs.
            "PNG32:" + os.path.join(PUBLIC, "apple-icon.png"),
        )
        run(
            "magick", ARTWORK, "-filter", "Lanczos", "-resize", "128x128", "-depth", "8", "-strip",
            os.path.join(PUBLIC, "brand", "email-logo.png"),
        )
        run(
            "magick", "-size", "1280x720", f"xc:{BOTTOM}",
            "(", ARTWORK, "-filter", "Lanczos", "-resize", "240x240", ")",
            "-gravity", "center", "-compose", "over", "-composite", "-depth", "8", "-strip",
            "PNG32:" + os.path.join(PUBLIC, "brand", "audio-cover.png"),
        )


if __name__ == "__main__":
    main()

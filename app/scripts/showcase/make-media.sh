#!/bin/sh
# The README's screenshots and demo video, drawn from the app's QA fixtures: the
# "Alex Rivera" demo account on files.example.com, no server, no window, nobody's real
# account or files. The macOS pictures are rendered here at Retina scale
# (examples/showcase.rs); the Windows ones are the Windows renders the snapshot tests
# keep (crates/modlight-app/tests/snapshots/*/windows-*-en.png, made on a Windows PC).
#
#   app/scripts/showcase/make-media.sh [work-dir]
#
# Writes docs/media/: the screenshots framed as macOS and Windows 11 windows (lossless
# WebP) and the demo as MP4 and animated WebP. Needs the Rust toolchain, ImageMagick 7
# (magick), FFmpeg with libx264, and cwebp and img2webp (libwebp). Nothing outside the
# work directory and docs/media/ is written.
#
# A shell function has no variables of its own, so each one's names carry its prefix.

set -eu

here=$(cd "$(dirname "$0")" && pwd)
app_root=$(cd "$here/../.." && pwd)
repo=$(cd "$app_root/.." && pwd)
work=${1:-"$(mktemp -d "${TMPDIR:-/tmp}/modlight-showcase.XXXXXX")"}
mkdir -p "$work"
work=$(cd "$work" && pwd)
media="$repo/docs/media"
snapshots="$app_root/crates/modlight-app/tests/snapshots"
# The latin subsets: the latin-ext files hold only the extended letters.
bold="$app_root/assets/fonts/Inter-SemiBold-latin.ttf"
regular="$app_root/assets/fonts/Inter-Regular-latin.ttf"
icon="$app_root/assets/icons/Modlight.ico[0]"
raw="$work/raw"
tmp="$work/tmp"
rm -rf "$tmp"
mkdir -p "$tmp" "$media/macos" "$media/windows"

echo "showcase: rendering the QA screens"
(cd "$app_root" && cargo run --release -q -p modlight-app --example showcase -- "$raw" 2)

size() { magick identify -format "%w %h" "$1"; }
# calc <awk expression>: the value rounded to a whole number.
calc() { awk "BEGIN { v = $1; printf \"%d\", (v < 0 ? v - 0.5 : v + 0.5) }"; }
title_of() { awk -F '\t' -v s="$1" '$1 == s { print $2 }' "$raw/stills/titles.tsv"; }

# offset <x> <y>: an ImageMagick geometry offset, signs included (-geometry -40+12).
offset() { printf '%+d%+d' "$1" "$2"; }

# wallpaper <width> <height> <out> <mac|windows>: the soft gradient behind the windows: a
# diagonal gradient with three glows, each a radial gradient from a colour to the same
# colour fully transparent.
wallpaper() {
    wp_w=$1 wp_h=$2 wp_out=$3
    case $4 in
    windows) set -- '#071a3d-#0b3a8e' '#2563eb' '#38bdf8' '#1d4ed8' ;;
    *) set -- '#0c0a24-#2b1050' '#4f46e5' '#c026d3' '#0ea5e9' ;;
    esac
    wp_d1=$((wp_w * 9 / 10)) wp_d2=$((wp_w * 8 / 10)) wp_d3=$((wp_w * 45 / 100))
    magick -size "${wp_w}x${wp_h}" -define gradient:angle=135 "gradient:$1" \
        \( -size "${wp_d1}x${wp_d1}" "radial-gradient:$2-${2}00" \
            -channel A -evaluate multiply 0.55 +channel \) \
        -geometry "$(offset $((wp_w * 18 / 100 - wp_d1 / 2)) $((wp_h * 12 / 100 - wp_d1 / 2)))" \
        -composite \
        \( -size "${wp_d2}x${wp_d2}" "radial-gradient:$3-${3}00" \
            -channel A -evaluate multiply 0.45 +channel \) \
        -geometry "$(offset $((wp_w * 88 / 100 - wp_d2 / 2)) $((wp_h * 96 / 100 - wp_d2 / 2)))" \
        -composite \
        \( -size "${wp_d3}x${wp_d3}" "radial-gradient:$4-${4}00" \
            -channel A -evaluate multiply 0.35 +channel \) \
        -geometry "$(offset $((wp_w * 94 / 100 - wp_d3 / 2)) $((wp_h * 6 / 100 - wp_d3 / 2)))" \
        -composite "$wp_out"
}

# lights <png> <points to pixels> <left> <top>: the traffic lights drawn on <png> in
# place: 12 points across, 20 apart, centred 14 points in and 14 down from the window's
# corner at <left>,<top>.
lights() {
    lt_k=$2 lt_left=$3 lt_top=$4
    lt_y=$(calc "$lt_top + 14 * $lt_k") lt_r=$(calc "6 * $lt_k")
    lt_sw=$(awk "BEGIN { print 0.5 * $lt_k }")
    lt_x1=$(calc "$lt_left + 14 * $lt_k")
    lt_x2=$(calc "$lt_left + 34 * $lt_k")
    lt_x3=$(calc "$lt_left + 54 * $lt_k")
    magick "$1" -strokewidth "$lt_sw" \
        -fill '#ff5f57' -stroke '#e0443e' -draw "circle $lt_x1,$lt_y $((lt_x1 + lt_r)),$lt_y" \
        -fill '#febc2e' -stroke '#dea123' -draw "circle $lt_x2,$lt_y $((lt_x2 + lt_r)),$lt_y" \
        -fill '#28c840' -stroke '#1aab29' -draw "circle $lt_x3,$lt_y $((lt_x3 + lt_r)),$lt_y" \
        "$1"
}

# mac_window <content.png> <out.png> <points to pixels> <dark|light> <main|dialog|screen>
# [title]: the content as a macOS window with rounded corners and a hairline edge. A
# main window draws its own title band, so it only gets the traffic lights; a dialog
# gets the system's title bar with its title above the content; a screen (the region
# overlay, which covers the display) gets neither.
mac_window() {
    mw_in=$1 mw_out=$2 mw_k=$3 mw_theme=$4 mw_kind=$5 mw_title=${6:-}
    set -- $(size "$mw_in")
    mw_cw=$1 mw_ch=$2
    mw_bar=0
    [ "$mw_kind" = dialog ] && mw_bar=$(calc "28 * $mw_k")
    mw_h=$((mw_ch + mw_bar)) mw_r=$(calc "12 * $mw_k")
    if [ "$mw_theme" = dark ]; then
        mw_barbg='#2c2c2e' mw_text='#e5e5e7' mw_edge='#ffffff30' mw_sep='#000000'
    else
        mw_barbg='#ececec' mw_text='#3c3c43' mw_edge='#00000038' mw_sep='#0000002a'
    fi
    magick -size "${mw_cw}x${mw_h}" "xc:$mw_barbg" "$mw_in" -geometry "+0+$mw_bar" \
        -composite "$tmp/win.png"
    if [ "$mw_bar" -gt 0 ]; then
        magick "$tmp/win.png" -fill "$mw_sep" \
            -draw "rectangle 0,$((mw_bar - 1)) $mw_cw,$((mw_bar - 1))" \
            -font "$bold" -pointsize "$(calc "13 * $mw_k")" -fill "$mw_text" -gravity north \
            -annotate "+0+$(calc "6.5 * $mw_k")" "$mw_title" "$tmp/win.png"
    fi
    if [ "$mw_kind" != screen ]; then
        lights "$tmp/win.png" "$mw_k" 0 0
    fi
    magick "$tmp/win.png" \
        \( -size "${mw_cw}x${mw_h}" xc:black -fill white \
            -draw "roundrectangle 0,0 $((mw_cw - 1)),$((mw_h - 1)) $mw_r,$mw_r" \) \
        -alpha off -compose CopyOpacity -composite -compose over \
        -fill none -stroke "$mw_edge" -strokewidth 1 \
        -draw "roundrectangle 0.5,0.5 $((mw_cw - 1)).5,$((mw_h - 1)).5 $mw_r,$mw_r" \
        "$mw_out"
}

# win_window <content.png> <out.png> <dark|light>: a Windows render as a Windows 11
# window: 8 px corners and the system's hairline edge (the app draws its own caption
# buttons and title band).
win_window() {
    ww_in=$1 ww_out=$2 ww_edge='#ffffff1f'
    [ "$3" = light ] && ww_edge='#0000002e'
    set -- $(size "$ww_in")
    ww_w=$1 ww_h=$2
    magick "$ww_in" \
        \( -size "${ww_w}x${ww_h}" xc:black -fill white \
            -draw "roundrectangle 0,0 $((ww_w - 1)),$((ww_h - 1)) 8,8" \) \
        -alpha off -compose CopyOpacity -composite -compose over \
        -fill none -stroke "$ww_edge" -strokewidth 1 \
        -draw "roundrectangle 0.5,0.5 $((ww_w - 1)).5,$((ww_h - 1)).5 8,8" "$ww_out"
}

# shadow <window.png> <out.png> <sigma>: the window's soft shadow, blurred at a quarter
# of the size. It is 8 * sigma larger than the window on every side.
shadow() {
    magick "$1" -resize 25% -background black -shadow "60x$3+0+0" -resize 400% "$2"
}

# webp <in.png> <out.webp>: a lossless WebP (ImageMagick writes 16-bit PNGs, several
# times larger, from the same pixels).
webp() { cwebp -quiet -lossless -z 9 "$1" -o "$2"; }

# place <window.png> <out.webp> <margin> <mac|windows>: the window on a wallpaper, with a
# soft shadow under it.
place() {
    pl_in=$1 pl_out=$2 pl_m=$3 pl_kind=$4
    set -- $(size "$pl_in")
    pl_w=$(($1 + 2 * pl_m)) pl_h=$(($2 + 2 * pl_m))
    wallpaper "$pl_w" "$pl_h" "$tmp/wall.png" "$pl_kind"
    shadow "$pl_in" "$tmp/shadow.png" $((pl_m / 12 + 1))
    magick "$tmp/wall.png" "$tmp/shadow.png" -gravity center -geometry "+0+$((pl_m / 6))" \
        -composite "$pl_in" -gravity center -geometry +0+0 -composite -depth 8 "$tmp/placed.png"
    webp "$tmp/placed.png" "$pl_out"
}

# --- Screenshots ----------------------------------------------------------------
echo "showcase: framing the screenshots"
for name in my-space my-space-details editor region-capture-annotate tab-upload-busy \
    download-youtube tab-tools image-effects toasts history capture-menu color-picker ocr \
    qr-code; do
    for theme in dark light; do
        still="$raw/stills/$name-$theme.png"
        [ -f "$still" ] || continue
        title=$(title_of "$name")
        kind=main
        [ -n "$title" ] && kind=dialog
        [ "$name" = region-capture-annotate ] && kind=screen
        mac_window "$still" "$tmp/framed.png" 2 "$theme" "$kind" "$title"
        suffix=""
        [ "$theme" = light ] && suffix="-light"
        place "$tmp/framed.png" "$media/macos/$name$suffix.webp" 120 mac
    done
done
for name in my-space tab-upload-busy download-youtube tab-tools capture-menu; do
    for theme in dark light; do
        still="$snapshots/$name/windows-$theme-en.png"
        [ -f "$still" ] || continue
        if [ "$theme" = light ] && [ "$name" != my-space ]; then
            continue
        fi
        suffix=""
        [ "$theme" = light ] && suffix="-light"
        win_window "$still" "$tmp/framed.png" "$theme"
        place "$tmp/framed.png" "$media/windows/$name$suffix.webp" 60 windows
    done
done

# The hero: the Mac window in the dark palette and the Windows one in the light, side by
# side at the size the Windows render has (the shadows start 8 * 6 px before them).
magick "$raw/stills/my-space-dark.png" -resize 50% "$tmp/mac-1x.png"
mac_window "$tmp/mac-1x.png" "$tmp/hero-mac.png" 1 dark main
win_window "$snapshots/my-space/windows-light-en.png" "$tmp/hero-win.png" light
shadow "$tmp/hero-mac.png" "$tmp/hero-mac-shadow.png" 6
shadow "$tmp/hero-win.png" "$tmp/hero-win-shadow.png" 6
wallpaper 1960 1080 "$tmp/hero-wall.png" mac
magick "$tmp/hero-wall.png" \
    "$tmp/hero-win-shadow.png" -geometry +812+258 -composite \
    "$tmp/hero-win.png" -geometry +860+290 -composite \
    "$tmp/hero-mac-shadow.png" -geometry +12+38 -composite \
    "$tmp/hero-mac.png" -geometry +60+70 -composite -depth 8 "$tmp/hero.png"
webp "$tmp/hero.png" "$media/hero.webp"

# --- Demo video -----------------------------------------------------------------
echo "showcase: making the demo video"
VW=1920 VH=1080 TOP=212 MAXW=1680 MAXH=810 FADE=0.5

# card <out.png> <title> <subtitle>: the opening and closing card.
card() {
    wallpaper $VW $VH "$tmp/card-wall.png" mac
    magick "$tmp/card-wall.png" \
        \( "$icon" -resize 240x240 \) -gravity center -geometry +0-150 -composite \
        -font "$bold" -pointsize 96 -fill white -gravity center -annotate +0+40 "$2" \
        -font "$regular" -pointsize 38 -fill '#ffffffc0' -annotate +0+130 "$3" "$1"
}

# chrome <out.png> <content w> <content h> <dialog|main|screen> <title> <caption> <line>:
# the frame of one scene: wallpaper, caption, the window's shadow and decorations, and a
# hole where the content shows through. Prints "W Hc X Yc" of the content.
chrome() {
    ch_out=$1 ch_cw=$2 ch_ch=$3 ch_kind=$4 ch_title=$5
    ch_bar2=0
    [ "$ch_kind" = dialog ] && ch_bar2=56
    ch_k=$(awk "BEGIN { a = $MAXW / $ch_cw; b = $MAXH / ($ch_ch + $ch_bar2); print (a < b ? a : b) }")
    ch_w=$(calc "$ch_cw * $ch_k") ch_hc=$(calc "$ch_ch * $ch_k") ch_bar=$(calc "$ch_bar2 * $ch_k")
    ch_h=$((ch_hc + ch_bar)) ch_x=$(((VW - ch_w) / 2))
    ch_y=$((TOP + (MAXH - ch_h) / 2))
    ch_pk=$(awk "BEGIN { print 2 * $ch_k }")
    ch_r=$(calc "12 * $ch_pk")
    wallpaper $VW $VH "$tmp/scene-wall.png" mac
    magick "$tmp/scene-wall.png" \
        -font "$bold" -pointsize 56 -fill white -gravity north -annotate +0+56 "$6" \
        -font "$regular" -pointsize 30 -fill '#ffffffb8' -annotate +0+136 "$7" \
        \( -size "${VW}x${VH}" xc:none -fill black \
            -draw "roundrectangle $ch_x,$((ch_y + 18)) $((ch_x + ch_w - 1)),$((ch_y + ch_h + 17)) $ch_r,$ch_r" \
            -channel A -blur 0x28 -evaluate multiply 0.6 +channel \) -composite \
        -fill '#2c2c2e' \
        -draw "roundrectangle $ch_x,$ch_y $((ch_x + ch_w - 1)),$((ch_y + ch_h - 1)) $ch_r,$ch_r" \
        "$tmp/scene.png"
    if [ "$ch_bar" -gt 0 ]; then
        magick "$tmp/scene.png" -gravity north -font "$bold" -pointsize "$(calc "13 * $ch_pk")" \
            -fill '#e5e5e7' -annotate "+0+$((ch_y + $(calc "6.5 * $ch_pk")))" "$ch_title" \
            "$tmp/scene.png"
    fi
    # The hole: the content's rectangle, its bottom corners (and its top ones without a
    # title bar) rounded like the window's.
    magick -size "${VW}x${VH}" xc:white -fill black \
        -draw "roundrectangle $ch_x,$((ch_y + ch_bar)) $((ch_x + ch_w - 1)),$((ch_y + ch_h - 1)) $ch_r,$ch_r" \
        "$tmp/hole.png"
    if [ "$ch_bar" -gt 0 ]; then
        magick "$tmp/hole.png" -fill black \
            -draw "rectangle $ch_x,$((ch_y + ch_bar)) $((ch_x + ch_w - 1)),$((ch_y + ch_bar + ch_r))" \
            "$tmp/hole.png"
    fi
    magick "$tmp/scene.png" "$tmp/hole.png" -alpha off -compose CopyOpacity -composite \
        -compose over -fill none -stroke '#ffffff30' -strokewidth 1 \
        -draw "roundrectangle $ch_x.5,$ch_y.5 $((ch_x + ch_w - 1)).5,$((ch_y + ch_h - 1)).5 $ch_r,$ch_r" \
        "$ch_out"
    if [ "$ch_kind" != screen ]; then
        lights "$ch_out" "$ch_pk" "$ch_x" "$ch_y"
    fi
    echo "$ch_w $ch_hc $ch_x $((ch_y + ch_bar))"
}

# scene <n> <frames dir or still> <dialog|main|screen> <title> <caption> <line> [seconds]:
# one part of the video as $tmp/part<n>.mp4; a still is held for [seconds].
parts=""
scene() {
    sc_n=$1 sc_src=$2 sc_seconds=${7:-0}
    sc_first=$sc_src
    [ -d "$sc_src" ] && sc_first="$sc_src/0000.png"
    sc_size=$(size "$sc_first")
    sc_geometry=$(chrome "$tmp/chrome$sc_n.png" $sc_size "$3" "$4" "$5" "$6")
    set -- $sc_geometry
    sc_layout="scale=$1:$2:flags=lanczos,pad=$VW:$VH:$3:$4:color=black,format=rgba[c];[c][1:v]overlay=0:0"
    if [ -d "$sc_src" ]; then
        ffmpeg -y -loglevel error -framerate 30 -i "$sc_src/%04d.png" -i "$tmp/chrome$sc_n.png" \
            -filter_complex "[0:v]$sc_layout,tpad=start_mode=clone:start_duration=0.4:stop_mode=clone:stop_duration=0.9,format=yuv420p[v]" \
            -map "[v]" -r 30 -c:v libx264 -preset medium -crf 12 "$tmp/part$sc_n.mp4"
    else
        ffmpeg -y -loglevel error -loop 1 -framerate 30 -t "$sc_seconds" -i "$sc_src" \
            -i "$tmp/chrome$sc_n.png" -filter_complex "[0:v]$sc_layout,format=yuv420p[v]" \
            -map "[v]" -r 30 -c:v libx264 -preset medium -crf 12 "$tmp/part$sc_n.mp4"
    fi
    parts="$parts $sc_n"
}

card "$tmp/intro.png" "Modlight" "ShareX for macOS and Windows, rewritten in Rust"
ffmpeg -y -loglevel error -loop 1 -framerate 30 -t 2.6 -i "$tmp/intro.png" \
    -vf format=yuv420p -c:v libx264 -preset medium -crf 12 "$tmp/part0.mp4"
parts=0
scene 1 "$raw/scenes/capture" screen "" "Capture any region" \
    "Windows are picked out as you hover, and the magnifier shows the exact pixel"
scene 2 "$raw/scenes/annotate" screen "" "Annotate on the spot" \
    "Arrows, boxes and text right on the screen, before you even pick the area"
scene 3 "$raw/scenes/editor" dialog "$(cat "$raw/scenes/editor/title.txt")" \
    "The ShareX image editor" "Boxes, highlights, arrows, text and numbered steps, with undo"
scene 4 "$raw/scenes/upload" main "" "Upload to your own server" \
    "Live progress, uploads that resume, and the link on your clipboard"
scene 5 "$raw/stills/my-space-dark.png" main "" "Your files, your server" \
    "Thumbnails, animated previews, folders, share links and trash" 3.2
scene 6 "$raw/stills/download-youtube-dark.png" main "" "Download videos" \
    "YouTube, TikTok, X and around 1800 other sites, through yt-dlp" 2.8
scene 7 "$raw/stills/tab-tools-dark.png" main "" "Every ShareX tool" \
    "Colour picker, OCR, QR codes, pin to screen, image effects, video converter" 2.8
card "$tmp/outro.png" "Modlight" "Self-hosted  ·  open source  ·  macOS and Windows"
ffmpeg -y -loglevel error -loop 1 -framerate 30 -t 3.2 -i "$tmp/outro.png" \
    -vf format=yuv420p -c:v libx264 -preset medium -crf 12 "$tmp/part8.mp4"
parts="$parts 8"

# The parts joined with cross-fades, each brought to one frame rate and time base first.
inputs="" graph="" offset=0 last="s0" i=0
for n in $parts; do
    inputs="$inputs -i $tmp/part$n.mp4"
    graph="$graph[$i:v]fps=30,settb=AVTB,format=yuv420p[s$i];"
    i=$((i + 1))
done
i=0
for n in $parts; do
    length=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$tmp/part$n.mp4")
    if [ "$i" -gt 0 ]; then
        graph="$graph[$last][s$i]xfade=transition=fade:duration=$FADE:offset=$offset[x$i];"
        last="x$i"
    fi
    offset=$(awk "BEGIN { print $offset + $length - $FADE }")
    i=$((i + 1))
done
# shellcheck disable=SC2086 # the inputs are separate words
ffmpeg -y -loglevel error $inputs -filter_complex "${graph%;}" -map "[$last]" \
    -c:v libx264 -preset slow -crf 20 -pix_fmt yuv420p -movflags +faststart "$media/demo.mp4"

# The animated preview for the README: an animated WebP, half the size of a GIF at a
# third more pixels across.
mkdir -p "$tmp/webp"
ffmpeg -y -loglevel error -i "$media/demo.mp4" -vf "fps=15,scale=1280:-2:flags=lanczos" \
    "$tmp/webp/%04d.png"
# shellcheck disable=SC2046 # one argument per frame
img2webp -loop 0 -lossy -q 70 -m 6 -d 67 $(ls "$tmp/webp"/*.png) -o "$media/demo.webp" >/dev/null

echo "showcase: $media"
ls -l "$media" "$media/macos" "$media/windows" | awk 'NF > 5 { print $5, $9 }'

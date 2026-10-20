"""Build the media corpus for the media-processing parity checks.

    python3 corpus.py <dir>

Pictures come from the srv-library corpus (real files: EXIF-rotated, CMYK,
greyscale, alpha, animated GIF/WebP, AVIF still and `avis`, HEIC, TIFF);
everything with a timeline is generated here with the host's ffmpeg so the
set is reproducible: H.264/AAC, HEVC, VP9/Opus in WebM, H.264/Opus in MKV (an
audio track MP4 cannot take), a silent MP4, a variable frame rate MP4, a
3000 px wide recording (offered the web-playable rescue), a portrait MOV, a
long one for the timeline ceiling, an AVIF sequence, and audio in MP3, WAV,
FLAC, Ogg/Opus and M4A. Plus files that must be refused: a PDF, a text file,
an empty-looking binary.

Only the Python standard library and ffmpeg (Homebrew) on the host.
"""

import os
import shutil
import subprocess
import sys

LIBRARY_CORPUS = "/Users/marcinasdfg/Desktop/modlight/_scratch/srv-library/media/corpus"

PICTURES = [
    "anim.gif", "anim.webp", "eagle.jpg", "eagle_cmyk.jpg", "eagle_grey.jpg", "landscape_o6.jpg",
    "parrot.png", "parrot.avif", "parrot_avis.avif", "tiny.png", "one.png", "sharex.png", "earth.tiff",
    "cactus.heic", "macblue_2400.jpg",
]

SRC = ["-f", "lavfi", "-i"]


def ff(out, *args):
    if os.path.exists(out):
        return
    cmd = ["ffmpeg", "-nostdin", "-hide_banner", "-v", "error", *args, "-y", out]
    subprocess.run(cmd, check=True)


def main(dest):
    os.makedirs(dest, exist_ok=True)
    for name in PICTURES:
        target = os.path.join(dest, name)
        if not os.path.exists(target):
            shutil.copyfile(os.path.join(LIBRARY_CORPUS, name), target)

    p = lambda n: os.path.join(dest, n)
    video = "testsrc2=size=320x240:rate=25:duration=%s"
    tone = "sine=frequency=440:sample_rate=44100:duration=%s"

    ff(p("h264_aac.mp4"), *SRC, video % 12, *SRC, tone % 12, "-c:v", "libx264", "-pix_fmt", "yuv420p",
       "-c:a", "aac", "-shortest", "-movflags", "+faststart")
    ff(p("h264_opus.mkv"), *SRC, video % 6, *SRC, tone % 6, "-c:v", "libx264", "-pix_fmt", "yuv420p",
       "-c:a", "libopus", "-shortest")
    ff(p("hevc_aac.mp4"), *SRC, video % 5, *SRC, tone % 5, "-c:v", "libx265", "-tag:v", "hvc1",
       "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", "-x265-params", "log-level=error")
    ff(p("vp9_opus.webm"), *SRC, video % 5, *SRC, tone % 5, "-c:v", "libvpx-vp9", "-b:v", "300k",
       "-c:a", "libopus", "-shortest")
    ff(p("silent.mp4"), *SRC, video % 9, "-c:v", "libx264", "-pix_fmt", "yuv420p")
    # Variable frame rate: 50 frames at 25 fps, then the rest at 10 fps.
    ff(p("vfr.mp4"), *SRC, "testsrc2=size=320x240:rate=25:duration=10", "-vf",
       "setpts='if(lt(N,50),N/25,2+(N-50)/10)/TB'", "-fps_mode", "vfr", "-c:v", "libx264", "-pix_fmt", "yuv420p")
    ff(p("wide.mp4"), *SRC, "testsrc2=size=3000x400:rate=15:duration=3", *SRC, tone % 3,
       "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest")
    ff(p("portrait.mov"), *SRC, "testsrc2=size=240x426:rate=30:duration=4", *SRC, tone % 4,
       "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest")
    ff(p("long.mp4"), *SRC, "testsrc2=size=160x120:rate=10:duration=150", "-c:v", "libx264",
       "-pix_fmt", "yuv420p", "-g", "20")
    ff(p("sequence.avif"), *SRC, "testsrc2=size=128x96:rate=10:duration=2", "-c:v", "libsvtav1",
       "-pix_fmt", "yuv420p", "-f", "avif")

    ff(p("tone.mp3"), *SRC, tone % 3, "-c:a", "libmp3lame", "-q:a", "4")
    ff(p("tone.wav"), *SRC, tone % 2, "-c:a", "pcm_s16le")
    ff(p("tone.flac"), *SRC, tone % 2, "-c:a", "flac")
    ff(p("tone.ogg"), *SRC, tone % 2, "-c:a", "libopus")
    ff(p("tone.m4a"), *SRC, tone % 2, "-c:a", "aac")

    with open(p("notes.txt"), "w") as f:
        f.write("plain text that no engine converts\n")
    with open(p("blob.bin"), "wb") as f:
        f.write(bytes(range(256)) * 4)
    print("corpus ready: %d files in %s" % (len(os.listdir(dest)), dest))


if __name__ == "__main__":
    main(sys.argv[1])

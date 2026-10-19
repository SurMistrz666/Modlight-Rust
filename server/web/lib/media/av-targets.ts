/**
 * What a video, an audio file or an animation can be turned into, and what that
 * costs us.
 *
 * The sibling of convert-targets.ts, and it follows the same rule: this module
 * is the ONLY answer to "what may be asked for". The dashboard menu renders from
 * it and the route admits from it, so a target visible in the UI can always be
 * requested and one that is not visible cannot be. It runs in the browser, so it
 * knows nothing that requires opening the file.
 *
 * WHAT THIS MENU IS FOR
 *
 * People convert a file to make it open somewhere it currently does not, or to
 * make it smaller. Everything here has to answer one of those, and the previous
 * shape of the menu did not: a video offered PNG, JPG and M4A and no other video
 * format at all, so the one thing anybody wants from a .mov - an .mp4 - was the
 * one thing missing, while "turn this 4 minute film into a still picture" was
 * offered twice. Frame grabs are gone from the menu entirely; nothing else in
 * the app used them (video posters are drawn by the browser from a
 * `preload="metadata"` element, not by us), so the code went with them.
 *
 * The split that matters here does not exist on the image side at all.
 *
 *   STATIC  (this module)  - "does a .mkv offer MP4 at all?" A question about
 *                            the KIND of file, answerable from the name.
 *   DYNAMIC (av-probe.ts)  - "do THIS file's codecs fit in MP4 without
 *                            re-encoding?" A question about the bytes.
 *
 * The menu can only ask the static question: the dashboard renders 8000 rows
 * from the database, and spawning an ffprobe per tile to make the menu exact
 * would cost more than every conversion it advertises. So a container entry is a
 * genuine offer that a particular file can still decline, and when it does the
 * route says why in those words rather than answering a bare 415. The static
 * table below is chosen so that declining is rare - see CONTAINER_OFFERS.
 */

import { effectiveType, mediaKind } from "@/lib/media/mime";

/** Containers a video can be poured into. */
export type VideoContainer = "mp4" | "mov" | "mkv" | "webm";

export type AvTarget = VideoContainer | "webmp4" | "mp3" | "ogg" | "wav" | "gif" | "webp";

/**
 * How expensive the operation is, and therefore who may ask for it.
 *
 *  - `copy`   : no encoder runs. The streams are moved into another container
 *               untouched. The cost is bounded by input SIZE and is independent
 *               of what is in the file, which is what makes it safe to expose
 *               publicly.
 *  - `encode` : an encoder runs, and the caller chooses the workload (which
 *               seconds, how many). Owner only.
 *
 * This is the whole authorization model for /convert on media, and the boundary
 * is drawn from measurement rather than intuition. On the production container
 * (2 cores, 2 GiB, ffmpeg 5.1.9):
 *
 *     .mov -> .mp4, 184 kB  (copy)        128 ms    <- offered publicly
 *     .mov -> .mkv, 184 kB  (copy)        127 ms    <- offered publicly
 *     .mkv -> .mov, 37 MB   (copy)        269 ms    <- offered publicly
 *     5 MB MP3 -> MP4 still (encode)     2196 ms    <- owner only
 *     2.4 MB GIF -> MP4     (encode)      231 ms    <- owner only
 *     5 s of video -> GIF   (encode)      968 ms    <- owner only
 *     whole 32 s file -> 480p h264       5556 ms    <- NOT OFFERED AT ALL
 *
 * A stream copy costs about what serving the file costs, and serving the file is
 * already public, so it adds no amplification. An encode does: the requester
 * picks the duration and the frame rate, so the work per request is theirs to
 * choose, and that must be attributable to an account we can turn off.
 *
 * The last row is why no entry in this menu re-encodes a video's picture. At six
 * times realtime, a ten-minute upload is a hundred seconds of both cores - it
 * does not fit in an HTTP request, and a progress bar that lies for ninety
 * seconds before failing is worse than an honest absence. Every container entry
 * here is therefore a copy, never a transcode.
 */
export type AvCost = "copy" | "encode";

/**
 * What comes out, so the menu can label each entry truthfully.
 *
 *  - `video`       : the same picture and sound in another container;
 *  - `webVideo`    : the same recording re-encoded so a browser can actually
 *                    play it - see WEB_MAX_EDGE. The one entry in this module
 *                    that re-encodes a video's picture, and the one entry that
 *                    is offered only when the row's own dimensions say it would
 *                    help;
 *  - `clip`        : a short silent excerpt of something longer;
 *  - `animation`   : the whole moving picture, in another format;
 *  - `audio`       : the sound lifted out of a file that also has pictures;
 *  - `audioFormat` : sound re-encoded into another sound format;
 *  - `audioVideo`  : sound plus a still picture, so it can be posted where only
 *                    video is accepted.
 */
export type AvYield =
  | "video"
  | "webVideo"
  | "clip"
  | "animation"
  | "audio"
  | "audioFormat"
  | "audioVideo";

export interface AvOption {
  to: AvTarget;
  cost: AvCost;
  yields: AvYield;
}

/**
 * The picture size the ROW carries (uploads.width / uploads.height).
 *
 * Both null is the normal state for anything uploaded before the columns
 * existed, and it means "nobody has measured", never "small". Every decision
 * that reads it therefore fails CLOSED: an unmeasured video is offered no
 * rescue, which is the only safe answer when the whole point of the entry is
 * that it applies to a minority of files.
 */
export interface AvSourceSize {
  width?: number | null;
  height?: number | null;
}

/**
 * Longest edge a web-playable copy is scaled down to.
 *
 * NOT 4096, although 4096 is where the hardware decoders stop (Intel QuickSync,
 * NVDEC and AMD all refuse an H.264 picture wider than that and the browser
 * falls back to software, which is the whole reason the recording that started
 * this stutters). Two measurements argue against parking the output right under
 * that wall, both taken on the production container - two cores, 2 GiB,
 * ffmpeg 5.1.9 - against the reported file (5056x1924, 403 frames, 13.43 s):
 *
 *     scaled to 4096 wide     21867 ms    1353 kB    H.264 level 5.1
 *     scaled to 1920 wide      9459 ms     409 kB    H.264 level 4.0
 *     scaled to 1280 wide      8105 ms     212 kB    H.264 level 3.1
 *
 * The 4096 output costs 2.3x the encode for a picture that is still 6.4 Mpx and
 * still level 5.1 - a level plenty of phones and older laptops decline even
 * though the width is legal. 1920 lands on High@4.0, which is the single most
 * widely decodable H.264 configuration there is, and 1.4 Mpx at 30 fps is
 * 42 Mpx/s against the ~292 Mpx/s a software decoder manages, so there is a
 * seven-fold margin even where hardware decoding is unavailable.
 *
 * Nothing is lost that the source had: the reported file carries 0.0044 bits
 * per pixel (its owner's healthy recordings measure 0.050 to 0.098), so its
 * detail was destroyed by the encoder before it ever reached us. Two crops of
 * the same frame - the source downscaled for display, and the 1920 copy at
 * native size - are indistinguishable.
 *
 * It is also the OFFER threshold, and deliberately the same number: the entry
 * appears exactly when it would change the picture size, so a menu row can
 * never produce a file identical to the one the reader already has.
 */
export const WEB_MAX_EDGE = 1920;

/**
 * Would a web-playable copy be smaller than this file?
 *
 * The longest edge rather than the width, so a portrait recording and a rotated
 * one are judged the same way. Unknown dimensions answer false - see
 * AvSourceSize.
 */
export function needsWebRescue(size: AvSourceSize | null | undefined): boolean {
  const w = size?.width ?? 0;
  const h = size?.height ?? 0;
  return Math.max(w, h) > WEB_MAX_EDGE;
}

/**
 * What the SOURCE is, as the upload row already classifies it.
 *
 * Not inferred from the probe, and that matters: an animated AVIF and a short
 * silent MP4 are indistinguishable to ffprobe (both demux as `mov`, both have
 * one video stream and no audio), but they want opposite treatment. It lives
 * here rather than beside the planner because both the budget and the planner
 * need it, and neither may import the other.
 */
export type AvSourceKind = "video" | "audio" | "image-sequence";

// ---------------------------------------------------------------------------
// Which streams fit in which container
// ---------------------------------------------------------------------------

/**
 * Video codecs that may be placed in an MP4 (or a MOV, which is the same box
 * format) and played by every browser.
 *
 * VP8 and VP9 are absent on purpose, and not because ffmpeg refuses - measured,
 * it happily muxes VP9 into MP4 with the `vp09` tag. Chrome plays that; Safari
 * does not. A feature whose entire purpose is "make the file work somewhere"
 * must not produce a file that works in some browsers.
 */
const MP4_VIDEO_CODECS = new Set(["h264", "hevc", "av1", "mpeg4"]);

/** ...and the audio side of the same question. */
const MP4_AUDIO_CODECS = new Set(["aac", "mp3", "alac"]);

/** WebM is a profile of Matroska with a short, fixed codec list. */
const WEBM_VIDEO_CODECS = new Set(["vp8", "vp9", "av1"]);
const WEBM_AUDIO_CODECS = new Set(["opus", "vorbis"]);

/**
 * `null` means "anything": Matroska is a general-purpose container and takes
 * every codec we can produce or copy, which is what makes .mkv the entry that
 * never declines. It is the fallback offer on every video for exactly that
 * reason.
 */
const CONTAINER_CODECS: Record<
  VideoContainer,
  { video: Set<string> | null; audio: Set<string> | null }
> = {
  mp4: { video: MP4_VIDEO_CODECS, audio: MP4_AUDIO_CODECS },
  mov: { video: MP4_VIDEO_CODECS, audio: MP4_AUDIO_CODECS },
  mkv: { video: null, audio: null },
  webm: { video: WEBM_VIDEO_CODECS, audio: WEBM_AUDIO_CODECS },
};

/** The muxer name to force with `-f`. Never taken from a caller. */
export const CONTAINER_MUXER: Record<VideoContainer, string> = {
  mp4: "mp4",
  mov: "mov",
  mkv: "matroska",
  webm: "webm",
};

/** MP4 and MOV are both ISO base media format, and only they take faststart. */
export function isIsoBmff(to: VideoContainer): boolean {
  return to === "mp4" || to === "mov";
}

// ---------------------------------------------------------------------------
// The static offer
// ---------------------------------------------------------------------------

/** The container family a row is in, from the type we would reason with. */
type SourceContainer = VideoContainer | "other";

const SOURCE_CONTAINER: Record<string, SourceContainer> = {
  "video/mp4": "mp4",
  "video/x-m4v": "mp4",
  "video/m4v": "mp4",
  "video/3gpp": "mp4",
  "video/3gpp2": "mp4",
  "video/quicktime": "mov",
  "video/x-matroska": "mkv",
  "video/webm": "webm",
};

/**
 * Which containers each kind of source is offered, in menu order.
 *
 * Read this as "what will actually work", because every entry is a stream copy
 * and a stream copy either fits the container or does not.
 *
 *  - MP4 and MOV are the same box format, so swapping between them is the
 *    cheapest and most useful entry in the whole menu: measured at 128 ms for a
 *    184 kB clip and 269 ms for a 37 MB one, and the result is bit-identical
 *    video. Whichever of the two the file is NOT is listed first.
 *  - MKV is offered for everything, because Matroska takes every codec. It is
 *    the entry that cannot decline.
 *  - WebM is offered only from Matroska. Its codec list is VP8/VP9/AV1 plus
 *    Opus/Vorbis, and .mkv is the only source we see that routinely holds those
 *    - a .webm is already one, and everything else is H.264. Measured on a
 *    production .webm: `-c copy` into MP4 dies with "Could not find tag for
 *    codec vp8", which is precisely the menu entry we refuse to draw.
 *  - A .webm is therefore offered only MKV: it already plays in every browser,
 *    so nothing is unlocked by moving it, and its codecs do not fit MP4 or MOV.
 */
const CONTAINER_OFFERS: Record<SourceContainer, VideoContainer[]> = {
  mp4: ["mov", "mkv"],
  mov: ["mp4", "mkv"],
  // No webm: it accepts only VP8, VP9 and AV1, and a .mkv in circulation is
  // almost always H.264 or HEVC. Offering it meant a public, copy-priced entry
  // that answered 409 for every real file on this server - measured on the only
  // .mkv here (H.264 + AAC). A webm source can still go to mkv, which is the
  // direction that actually works.
  mkv: ["mp4", "mov"],
  webm: ["mkv"],
  other: ["mp4", "mov", "mkv"],
};

/**
 * Image containers libvips cannot animate for us, but ffmpeg can.
 *
 * HEIC and HEIF are listed for completeness; `mediaKind` classifies them as
 * "other" today, so they never reach the branch that reads this.
 */
const FFMPEG_ONLY_ANIMATION_EXT = new Set(["avif", "heic", "heif"]);

/** Sound formats we can encode into, and the types that already ARE each one. */
const AUDIO_TARGETS = ["mp3", "ogg", "wav"] as const;
const AUDIO_SELF: Record<string, (typeof AUDIO_TARGETS)[number]> = {
  "audio/mpeg": "mp3",
  "audio/mp3": "mp3",
  "audio/ogg": "ogg",
  "audio/vorbis": "ogg",
  "audio/wav": "wav",
  "audio/x-wav": "wav",
  "audio/wave": "wav",
};

const ALL_TARGETS: AvTarget[] = [
  "mp4",
  "mov",
  "mkv",
  "webm",
  "webmp4",
  "mp3",
  "ogg",
  "wav",
  "gif",
  "webp",
];

export function isAvTarget(v: string | null | undefined): v is AvTarget {
  return !!v && (ALL_TARGETS as string[]).includes(v);
}

export function isVideoContainer(v: string): v is VideoContainer {
  return v === "mp4" || v === "mov" || v === "mkv" || v === "webm";
}

function ext(name: string | null | undefined): string {
  return (name ?? "").replace(/^\./, "").toLowerCase();
}

/**
 * Everything this file offers, in menu order.
 *
 * `frames` is what the row knows (uploads.frames): 1 means a still, >1 an
 * animation, null "nobody has counted" - which is every imported row. Only the animated-image branch consults it, and only to avoid
 * offering "turn this into a moving picture" for a file we have already
 * established holds one frame.
 */
export function avTargetsFor(
  mime: string | null | undefined,
  extension?: string | null,
  frames?: number | null,
  size?: AvSourceSize | null,
): AvOption[] {
  const e = ext(extension);
  const type = effectiveType(mime, extension);
  const kind = mediaKind(mime, extension);

  if (kind === "video") {
    const source = SOURCE_CONTAINER[type] ?? "other";
    return [
      // First, when it is there at all: a recording too big for a hardware
      // decoder is the one fault on this menu that makes a file unwatchable
      // rather than merely inconvenient, and the container copies below cannot
      // touch it - they move the same samples into another box.
      ...(needsWebRescue(size)
        ? [{ to: "webmp4", cost: "encode", yields: "webVideo" } as AvOption]
        : []),
      ...CONTAINER_OFFERS[source].map<AvOption>((to) => ({
        to,
        cost: "copy",
        yields: "video",
      })),
      // A moving excerpt, and the sound. Both re-encode, so both are owner-only.
      // There is no still-frame entry any more, and no animated-WebP one either:
      // GIF is what people mean by "make me a moving snippet", and offering the
      // same thing twice under two names is how a menu stops being readable.
      { to: "gif", cost: "encode", yields: "clip" },
      { to: "mp3", cost: "encode", yields: "audio" },
    ];
  }

  if (kind === "audio") {
    return [
      // No entry for the format the file already is: "download this MP3 as MP3"
      // is a re-encode that can only ever lose quality.
      ...AUDIO_TARGETS.filter((to) => to !== AUDIO_SELF[type] && to !== e).map<AvOption>((to) => ({
        to,
        cost: "encode",
        yields: "audioFormat",
      })),
      // Sound on a still picture. Everywhere that accepts a video and refuses an
      // audio file - most social timelines, some chat clients - this is the only
      // way to post the thing. See planAudioVideo for what the picture is.
      { to: "mp4", cost: "encode", yields: "audioVideo" },
    ];
  }

  // Animations that libvips cannot help with, or cannot help with well.
  //
  // A file already known to hold one frame is not an animation and gets nothing
  // here; it belongs entirely to convert-targets.ts.
  if (kind === "image" && frames !== 1) {
    // The case this feature started from: an AVIF image sequence. libvips reads
    // only its cover frame (lib/media/probe.ts), so the image menu can offer
    // nothing but stills, and the 43 frames in the file are unreachable. ffmpeg
    // demuxes it through the mov family and gets all of them - measured on the
    // production file, 895 ms to an animated WebP and 1111 ms to a GIF.
    if (FFMPEG_ONLY_ANIMATION_EXT.has(e)) {
      return [
        { to: "gif", cost: "encode", yields: "animation" },
        { to: "webp", cost: "encode", yields: "animation" },
      ];
    }

    // A GIF's most valuable conversion by a wide margin, and libvips cannot do
    // it: measured on a production 2.39 MB GIF, the MP4 is 32 kB - seventy-four
    // times smaller, with the motion intact - and took 231 ms. Animated WebP is
    // left to sharp, which already keeps a GIF's animation.
    if (type === "image/gif") {
      return [{ to: "mp4", cost: "encode", yields: "animation" }];
    }
  }

  return [];
}

/** Is `to` something this file really offers? The route's admission test. */
export function avOptionFor(
  mime: string | null | undefined,
  extension: string | null | undefined,
  frames: number | null | undefined,
  to: string | null | undefined,
  size?: AvSourceSize | null,
): AvOption | null {
  if (!isAvTarget(to)) return null;
  return avTargetsFor(mime, extension, frames, size).find((o) => o.to === to) ?? null;
}

// ---------------------------------------------------------------------------
// The dynamic half
// ---------------------------------------------------------------------------

export type ContainerVerdict =
  | { ok: true }
  /** The bytes cannot go into this container as they are, and re-encoding is not on offer. */
  | { ok: false; reason: "video-codec" | "no-video" };

/**
 * Can these streams be poured into `to` without touching a single sample?
 *
 * Pure over the probe, so every combination that has ever surprised us is a unit
 * test. An audio stream we cannot copy is not fatal - the video is what the user
 * is trying to watch, so that track is simply dropped and the caller is told.
 */
export function containerVerdict(
  to: VideoContainer,
  probe: { video: { codec: string } | null; audio: { codec: string } | null },
): ContainerVerdict {
  if (!probe.video) return { ok: false, reason: "no-video" };
  const allowed = CONTAINER_CODECS[to].video;
  if (allowed && !allowed.has(probe.video.codec)) return { ok: false, reason: "video-codec" };
  return { ok: true };
}

/** May the audio track ride along into `to`, or must it be dropped? */
export function audioCopyableTo(to: VideoContainer, codec: string | null | undefined): boolean {
  if (!codec) return false;
  const allowed = CONTAINER_CODECS[to].audio;
  return allowed ? allowed.has(codec) : true;
}

/** MIME and extension a target produces. */
export const AV_OUTPUT: Record<AvTarget, { mime: string; ext: string }> = {
  mp4: { mime: "video/mp4", ext: "mp4" },
  // Same MIME and same extension as `mp4`, and that is the point: the result is
  // an ordinary MP4. Only the way it is produced differs, so the target needs
  // its own NAME (the menu and the route both key on it) and nothing else.
  webmp4: { mime: "video/mp4", ext: "mp4" },
  mov: { mime: "video/quicktime", ext: "mov" },
  mkv: { mime: "video/x-matroska", ext: "mkv" },
  webm: { mime: "video/webm", ext: "webm" },
  mp3: { mime: "audio/mpeg", ext: "mp3" },
  ogg: { mime: "audio/ogg", ext: "ogg" },
  wav: { mime: "audio/wav", ext: "wav" },
  gif: { mime: "image/gif", ext: "gif" },
  webp: { mime: "image/webp", ext: "webp" },
};

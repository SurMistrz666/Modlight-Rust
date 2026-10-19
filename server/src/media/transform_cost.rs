//! Port of `lib/media/transform-cost.ts`: what one media transform is
//! allowed to cost.
//!
//! The old budget was a single pixel ceiling (64 Mpx still / 150 Mpx animated)
//! applied to every target alike. That is a decompression-bomb guard, not a
//! resource budget: it measures the SOURCE and says nothing about the encoder the
//! request picked, and the encoders differ by two orders of magnitude.
//!
//! Measured, on incompressible noise (the worst case an uploader can hand us),
//! with libvips pinned to two threads to match the container's `cpus: 2.0`:
//!
//! ```text
//!   4000x3000 = 12 Mpx        time        peak RSS over baseline
//!     -> jpeg                 0.18 s      ~46 MB      (3.8 B per source pixel)
//!     -> png                  0.50 s      ~42 MB      (3.5 B/px)
//!     -> webp  (effort 4)     1.66 s      291 MB      (24.2 B/px)
//!     -> avif  (effort 4)    22.54 s      406 MB      (33.8 B/px)
//!     -> avif  (effort 2)     4.62 s      419 MB      (34.9 B/px, output +2%)
//! ```
//!
//! Two facts fall out of that table and they drive everything below.
//!
//! First, AVIF at effort 4 costs 45x what JPEG costs for the same picture, and
//! effort 2 gets 4.9x of that back for a 2% larger file. Effort 4 on a public
//! endpoint was never a defensible default; the encoder settings live in
//! convert.ts and are now chosen with this table in view.
//!
//! Second, memory is where the danger is. AVIF and WebP hold 24-35 bytes per
//! SOURCE pixel, so the old 64 Mpx ceiling authorised a 2.2 GB allocation inside
//! a 2 GiB container - twice, because the limiter allowed two at once.
//!
//! Everything here is pure arithmetic over those constants, so the budget is a
//! unit test instead of a production incident.

use crate::media::convert_targets::ConvertTarget;

// ---------------------------------------------------------------------------
// The pool
// ---------------------------------------------------------------------------

/// Memory the container is given (`docker inspect modlight` ->
/// HostConfig.Memory), mirrored from docker-compose.prod.yml `mem_limit: 2g`.
///
/// Not read from the cgroup at runtime on purpose: the budget has to be the same
/// number in a unit test, on a laptop and in production, or the ceilings the
/// tests assert are not the ceilings that ship. lib/runtime/limits.ts reads the
/// real cgroup and logs a warning when the two disagree.
pub const CONTAINER_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Held back for everything that is not a transform.
///
/// Measured on production (`/sys/fs/cgroup/memory.stat`): the running app's
/// anonymous memory sits at 348 MB, with another 1.7 GB of reclaimable page cache
/// on top - and `memory.events` already records 431 hits against `memory.max`,
/// every one of them absorbed by evicting that cache. Nothing has been OOM-killed
/// yet, but there is no free headroom either: an allocation that cannot be
/// satisfied by reclaim is the one that kills the container.
///
/// 1 GiB leaves room for the 348 MB baseline, SSR spikes, the upload paths and
/// enough page cache that reclaim stays cheap.
pub const RESERVED_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;

/// Everything the transform pool may hold at once, across all slots.
pub const TRANSFORM_POOL_BYTES: u64 = CONTAINER_MEMORY_BYTES - RESERVED_MEMORY_BYTES;

/// Capacity of the shared limiter, in units.
///
/// Denominated in units rather than jobs because the jobs are not comparable: a
/// conversion is seconds of CPU and hundreds of megabytes, a thumbnail is 39 ms
/// and essentially free (measured: 36 Mpx -> 256 px cover in 39 ms, with peak RSS
/// going DOWN, because libvips shrinks on load). One limiter with one queue, as
/// required - but a thumbnail must not have to wait behind two AVIF encodes for
/// a slot it barely uses.
///
/// Four units on a two-core box: two conversions, or four thumbnails, or one of
/// each kind. Two conversions is the memory worst case and it is what the
/// per-slot figure below is derived from.
pub const TRANSFORM_CAPACITY: u32 = 4;

/// A conversion takes half the pool.
pub const CONVERT_WEIGHT: u32 = 2;

/// A thumbnail takes a quarter.
pub const THUMBNAIL_WEIGHT: u32 = 1;

/// How many callers may wait for a slot before the rest are refused outright.
///
/// Waiting costs nothing but latency - a queued job holds no memory and no CPU -
/// and the deadline covers the wait as well as the work, so nobody can be stuck
/// here longer than CONVERT_TIMEOUT_MS. Twelve is sized for the honest burst: a
/// cold dashboard opens six connections per host and asks for a thumbnail on each
/// one, and those must not start answering 503 because two conversions happen to
/// be running.
pub const TRANSFORM_QUEUE: usize = 12;

/// Memory one unit of capacity is worth.
pub const BYTES_PER_UNIT: u64 = TRANSFORM_POOL_BYTES / TRANSFORM_CAPACITY as u64;

/// ...and what a whole conversion slot may therefore allocate: 512 MB.
pub const CONVERT_MEMORY_BYTES: u64 = BYTES_PER_UNIT * CONVERT_WEIGHT as u64;

// ---------------------------------------------------------------------------
// Deadlines
// ---------------------------------------------------------------------------

/// Wall clock a conversion may take before the caller is answered 503.
///
/// This bounds the RESPONSE, not always the work: libvips honours a timeout only
/// where it evaluates progressively. Measured both ways - an animated WebP encode
/// with `.timeout({seconds:1})` aborted after 1016 ms ("VipsImage: killed"),
/// while a single-frame AVIF encode ran its full 24.5 s with the same timeout
/// set, because the encoder never returns to the progress callback. So for a
/// still AVIF the only real bound is the pixel budget, which is why that one is
/// the tightest in the table.
pub const CONVERT_TIMEOUT_MS: u64 = 15_000;

/// A thumbnail that has not finished in 10 s is not going to be useful.
pub const THUMBNAIL_TIMEOUT_MS: u64 = 10_000;

/// The wall clock the pixel ceilings below were solved for, on the reference
/// machine. The production container is slower per core, which the timeout above
/// absorbs; the ceilings exist so it rarely comes to that.
const TARGET_SECONDS: f64 = 8.0;

// ---------------------------------------------------------------------------
// Per-target cost
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetCost {
    /// Peak bytes of RSS per SOURCE pixel, measured, with margin.
    pub bytes_per_pixel: f64,
    /// Source pixels per second, measured on the worst-case (incompressible) input.
    pub pixels_per_second: f64,
    /// Hard ceiling, when the arithmetic below would otherwise allow more than is
    /// sensible. PNG and JPEG are cheap enough that only the decompression-bomb
    /// guard binds them.
    pub hard_max_pixels: Option<f64>,
}

/// A pixel here is always a SOURCE pixel, and for an animation it is counted once
/// per frame - the distinction that let a 1654x762 GIF with 256 frames read as
/// 1.26 Mpx and cost 322.
///
/// `pdf` is not the odd one out it used to be. It was the most expensive target
/// we had (measured: 64 Mpx cost 9.4 s wall, of which 7.6 s was the event loop
/// BLOCKED inside pako, plus 1433 MB of RSS) because pdf-lib re-encoded the
/// picture in pure JavaScript. convert.ts now embeds a JPEG with DCTDecode, which
/// pdf-lib passes through untouched: the same 64 Mpx costs 0.96 s, 54 ms of block
/// and 623 MB. The number below is what that path measures.
fn costs(to: ConvertTarget) -> TargetCost {
    match to {
        // 3.5 B/px measured; 64 Mpx is the decompression-bomb ceiling, not a cost one.
        ConvertTarget::Png => {
            TargetCost { bytes_per_pixel: 6.0, pixels_per_second: 24_000_000.0, hard_max_pixels: Some(64_000_000.0) }
        }
        // 3.8 B/px measured.
        ConvertTarget::Jpg => {
            TargetCost { bytes_per_pixel: 6.0, pixels_per_second: 67_000_000.0, hard_max_pixels: Some(64_000_000.0) }
        }
        // 5.3 B/px measured for sharp->JPEG plus the copy pdf-lib embeds verbatim.
        ConvertTarget::Pdf => {
            TargetCost { bytes_per_pixel: 10.0, pixels_per_second: 67_000_000.0, hard_max_pixels: Some(40_000_000.0) }
        }
        // 24.2 B/px measured. Memory-bound long before the clock matters.
        ConvertTarget::Webp => {
            TargetCost { bytes_per_pixel: 30.0, pixels_per_second: 7_200_000.0, hard_max_pixels: None }
        }
        // 34.9 B/px measured, and 2.6 Mpx/s at effort 2. The tightest budget we have,
        // and the one target libvips cannot be made to abort part-way.
        ConvertTarget::Avif => {
            TargetCost { bytes_per_pixel: 40.0, pixels_per_second: 2_600_000.0, hard_max_pixels: None }
        }
    }
}

/// Re-encoding an animation is charged differently from a still.
///
/// libvips does NOT unroll the sequence into one tall image the way sharp's
/// `join` does; `webpsave` feeds the muxer a page at a time. Measured on a
/// 800x600x60 GIF of pure noise: 28.8 Mpx of source cost 41 MB of peak RSS, or
/// 1.42 bytes per source pixel - about a seventeenth of what the same pixels cost
/// on the still WebP path, and dominated by the output buffer rather than by the
/// frames.
///
/// So the animated path is bound by TIME (7.4 Mpx/s measured), not by memory.
const ANIMATED_WEBP: TargetCost =
    TargetCost { bytes_per_pixel: 4.0, pixels_per_second: 7_400_000.0, hard_max_pixels: None };

fn cost_of(to: ConvertTarget, animated: bool) -> TargetCost {
    if animated && to == ConvertTarget::Webp { ANIMATED_WEBP } else { costs(to) }
}

/// Round down to a whole million pixels, so the constants read like limits.
fn round_mpx(pixels: f64) -> f64 {
    ((pixels / 1_000_000.0).floor() * 1_000_000.0).max(1_000_000.0)
}

/// JavaScript's `Math.round`: nearest integer, ties toward +infinity.
pub(crate) fn js_round(x: f64) -> f64 {
    let r = x.round();
    // f64::round breaks ties away from zero; JS breaks them upwards.
    if x < 0.0 && r - x == -0.5 { r + 1.0 } else { r }
}

/// The most source pixels this target may be handed.
///
/// The smaller of what fits in a conversion slot's share of memory and what
/// finishes inside TARGET_SECONDS, then the target's own hard ceiling.
pub fn max_pixels_for(to: ConvertTarget, animated: bool) -> u64 {
    let cost = cost_of(to, animated);
    let by_memory = CONVERT_MEMORY_BYTES as f64 / cost.bytes_per_pixel;
    let by_time = cost.pixels_per_second * TARGET_SECONDS;
    let limit = round_mpx(by_memory.min(by_time));
    match cost.hard_max_pixels {
        Some(hard) => limit.min(hard) as u64,
        None => limit as u64,
    }
}

/// Peak memory this conversion is expected to need. For diagnostics and tests.
pub fn peak_bytes_for(to: ConvertTarget, animated: bool, pixels: u64) -> f64 {
    cost_of(to, animated).bytes_per_pixel * pixels as f64
}

/// Ceiling for anything that has to exist in memory as a whole: thumbnails and
/// every conversion the libvips engine runs.
///
/// `readBuffer()` does not stream, so without this a thumbnail of a multi-GB
/// upload takes the process down. It lives here, beside the other budgets, rather
/// than inside the route, because the in-place converter enforces the same
/// ceiling from a completely different entry point and the two must not drift.
/// The ffmpeg engine has its own, far larger limit (AV_MAX_INPUT_BYTES) precisely
/// because nothing on that path reads the file into memory.
pub const MAX_TRANSFORM_BYTES: u64 = 32 * 1024 * 1024;

/// A thumbnail's source is bounded by MAX_TRANSFORM_BYTES (32 MB) on the way in
/// and by libvips' shrink-on-load on the way through, so the only thing left to
/// guard is a header that DECLARES more pixels than the file could possibly hold
/// - the classic 40 kB PNG claiming 100000x100000.
pub const MAX_THUMBNAIL_PIXELS: u64 = 64_000_000;

// ---------------------------------------------------------------------------
// Motion preview
// ---------------------------------------------------------------------------
//
// A preview that MOVES (`/raw?w=…&motion=1`) is ONE CONTIGUOUS CLIP, and this
// section is the arithmetic that bounds it.
//
// IT USED TO BE EIGHT FRAMES SAMPLED ACROSS THE WHOLE FILE, and the complaint
// about that was exact: next to the real video playing in the panel, eight
// stills spread over a minute is a summary, not motion. The obvious fix - raise
// the sample count - was ruled out by the measurement, because the cost of that
// shape is in the SEEKS and not in the frames: on the 4K .mov on production, two
// frames cost 1.8 s, four cost 3.3 s and eight cost 6.8 s, so sixteen would have
// been about fourteen seconds for a tile.
//
// A contiguous clip pays for ONE seek and then decodes forward, which is the
// cheap direction. Measured in the production image (ffmpeg 5.1.9, `cpus: 2.0`),
// against real uploads, 8 sampled frames against a 2.5 s clip at 12 fps:
//
//   source                          8 seeks      one clip (30 frames)
//   3840x2160 .mov, 68 MB, 15 s     7045 ms      4325 ms
//   1920x1080 .mp4, 109 MB, 24 s    3825 ms      2655 ms
//   1920x1080 .mp4, 58 MB, 14 s     3935 ms      1741 ms
//   1920x1088 .mp4, 42 MB, 55 s     3972 ms       894 ms
//   1280x720  .mp4, 24 MB, 22 s     2006 ms       545 ms
//      48x96  .mp4, 400 MB, 42 s     471 ms       170 ms
//
// Four times the frames, and cheaper on every file we have. That is the whole
// argument for the shape; the constants below are what keep it bounded.

/// Frames per second the finished preview plays at, and how long it runs.
pub const MOTION_CLIP_FPS: f64 = 12.0;
pub const MOTION_CLIP_SECONDS: f64 = 2.5;

/// Most frames one preview may hold: the clip at its full length.
///
/// Thirty, against the eight this replaced. Twelve frames a second is about where
/// a sequence stops reading as a slideshow (silent film ran at 16-18, and every
/// measurement here is at 12), and two and a half seconds is long enough to show
/// a gesture finishing rather than a moment of one.
///
/// It is also the number of pages the ANIMATION path takes, for the same reason.
pub const MOTION_FRAMES: u32 = 30; // Math.round(MOTION_CLIP_SECONDS * MOTION_CLIP_FPS)

/// Shortest clip worth producing when the budget below cannot afford a full one.
///
/// Nine frames of real motion at 12 fps. Under that the artefact is back to being
/// a slideshow, and saying "this file is too large to preview" is more honest than
/// shipping one - so a source that cannot afford 0.75 s is refused rather than
/// quietly reduced to a flicker.
pub const MOTION_CLIP_MIN_SECONDS: f64 = 0.75;

/// Frame rate assumed when the container does not carry one.
///
/// `nb_frames / duration` answers this for four of the five production videos
/// checked; the fifth (the 48x96 file) omits `nb_frames` entirely. Assuming the
/// HIGH end costs nothing where it is wrong - a 1080p file charged at 60 fps still
/// affords the whole clip - and it is the only direction that cannot under-charge
/// a 4K source whose header is thin.
pub const MOTION_ASSUMED_FPS: f64 = 60.0;

/// Sanity bounds on a frame rate derived from a header.
///
/// `nb_frames / duration` is arithmetic over two numbers the file supplies, and a
/// variable-frame-rate container can make it nonsense in either direction. The low
/// clamp stops a bogus 0.01 fps from authorising an unbounded decode; the high one
/// stops a bogus 10000 fps from refusing a file that is perfectly fine.
pub const MOTION_MIN_SOURCE_FPS: f64 = 1.0;
pub const MOTION_MAX_SOURCE_FPS: f64 = 240.0;

/// Units of the shared limiter a motion preview holds.
///
/// A conversion's two, not a thumbnail's one, and it stays two under the new
/// shape: the worst measured job is 4325 ms of ffmpeg plus 1195 ms of sharp on a
/// two-core box, which is conversion-scale work and nothing like the 39 ms a still
/// thumbnail costs. Charging it a thumbnail's weight would let four run at once on
/// a box that has two cores.
pub const MOTION_WEIGHT: u32 = CONVERT_WEIGHT;

/// Deadline for one preview, queueing included.
///
/// The worst case measured on production is 5.5 s (a 4K60 .mov at the full clip
/// length, and the budget below trims that one to 1.8 s anyway). Twenty seconds is
/// that with room for a second job on the cores and for a wait in the queue, and
/// it is still short enough that a client drawing a grid is not left holding a
/// connection. The ffmpeg process has its own, smaller group timeout inside
/// lib/media/motion.ts, so the encoder dies before the response gives up rather
/// than after.
pub const MOTION_TIMEOUT_MS: u64 = 20_000;

/// Source pixels a VIDEO decode gets through per second, counted as
/// `width x height x frames decoded`.
///
/// Measured on production with the fixed cost of the open and the seek (0.2-0.7 s)
/// subtracted, over a 2.5 s window:
///
/// ```text
///   3840x2160 @60  150 frames = 1244 Mpx  ->  4325 ms   (~325 Mpx/s)
///   1920x1080 @60  150 frames =  311 Mpx  ->  2655 ms   (~144 Mpx/s)
///   1920x1080 @30   75 frames =  156 Mpx  ->  1741 ms   (~125 Mpx/s)
///   1920x1088 @30   75 frames =  169 Mpx  ->   894 ms   (~429 Mpx/s)
/// ```
///
/// The per-pixel rate RISES with frame size, because the per-frame overhead
/// (parsing, threading, filter setup) dominates for small pictures. That matters
/// for the ceiling: a job big enough to reach it is necessarily a big-frame job,
/// and those run at the fast end of the table. 150 Mpx/s is half the rate the
/// largest real file achieved, which is the same "two jobs may hold the cores"
/// discount av-cost.ts applies.
const MOTION_VIDEO_PIXELS_PER_SECOND: f64 = 150_000_000.0;

/// Source pixels the ANIMATION page reader gets through per second, counted the
/// same way.
///
/// This number went UP by more than three times when the sampler was replaced, and
/// the reason is the reason the whole shape changed. Reading page p of an
/// animation costs decoding pages 0..p, so eight pages spread evenly over P of
/// them was about four passes over the sequence. A contiguous run from page zero
/// is ONE pass, and libvips will do it in a single load.
///
/// Measured on production, 30 contiguous pages against the 8 sampled ones, whole
/// pipeline including the WebP encode:
///
/// ```text
///   1195x1169 GIF, 47 pages, 25 MB    2416 ms sampled  ->  1927 ms contiguous
///    624x765  GIF, 108 pages, 20 MB   2062 ms          ->  1426 ms
///   1654x762  GIF, 256 pages, 4 MB    2433 ms          ->  1283 ms
///    640x640  GIF, 135 pages, 7 MB    1468 ms          ->  1207 ms
/// ```
///
/// Netting out the ~900 ms the 30-frame WebP encode costs, the largest of those
/// decodes 41.9 Mpx in about 1.0 s. Halved, as above, for a second job.
const MOTION_PAGE_PIXELS_PER_SECOND: f64 = 20_000_000.0;

/// Wall clock the ceilings below are solved for.
const MOTION_TARGET_SECONDS: f64 = 6.0;

/// Most source pixels one VIDEO clip may decode, across every frame of it.
pub fn max_motion_decode_pixels() -> u64 {
    round_mpx(MOTION_VIDEO_PIXELS_PER_SECOND * MOTION_TARGET_SECONDS) as u64
}

/// Most source pixels one ANIMATION run may decode, across every page of it.
pub fn max_motion_source_pixels() -> u64 {
    round_mpx(MOTION_PAGE_PIXELS_PER_SECOND * MOTION_TARGET_SECONDS) as u64
}

/// What a budget answers: how much of the source this preview may touch.
///
/// Note what is NOT here: a way to say "too expensive, go away" as a first
/// resort. Both budgets SHORTEN before they refuse, because a large picture is
/// not a reason to have no preview - it is a reason to have a shorter one. The
/// refusal is left for the case where even the shortest clip is out of reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionBudget {
    pub ok: bool,
    /// Frames the engine may decode. Zero when refused.
    pub frames: u64,
    /// Source pixels across those frames - the number the ceiling is applied to.
    pub pixels: u64,
    pub limit: u64,
}

/// A video budget also carries the clip length its frame count came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionClipBudget {
    pub ok: bool,
    pub frames: u64,
    pub pixels: u64,
    pub limit: u64,
    pub seconds: f64,
}

/// How many contiguous pages of an animation may be decoded.
///
/// `framePixels x frames`, never `width x height`. That distinction is the whole
/// point of this function: a 1654x762 GIF with 256 frames reads as 1.26 Mpx to a
/// guard that multiplies two numbers, and the sampler that preceded this one
/// touched 322 Mpx of it. The multiplication survives the change of shape - what
/// changed is which frame count goes into it, because a run from page zero decodes
/// `frames` of them and not `pages`.
///
/// `pages` is the count in the FILE. Callers pass what the decoder reported, not
/// what the container hinted at, because a row that merely might be animated is
/// not evidence of anything.
pub fn motion_page_budget(frame_pixels: u64, pages: u64) -> MotionBudget {
    let limit = max_motion_source_pixels();
    // A single page still has to be materialised whole, so the per-frame ceiling
    // that guards every other libvips read applies here unchanged.
    if frame_pixels == 0 || pages <= 1 || frame_pixels > MAX_THUMBNAIL_PIXELS {
        return MotionBudget { ok: false, frames: 0, pixels: 0, limit };
    }
    let affordable = limit / frame_pixels;
    let frames = pages.min(MOTION_FRAMES as u64).min(affordable);
    if frames < 2 {
        return MotionBudget { ok: false, frames: 0, pixels: 0, limit };
    }
    MotionBudget { ok: true, frames, pixels: frame_pixels * frames, limit }
}

/// How long a contiguous clip of a video may be.
///
/// The frame-aware multiplication again, with the frame count coming from the clip
/// rather than from the file: decoding `seconds` of a source that runs at
/// `sourceFps` touches `framePixels x sourceFps x seconds`. Solving that for
/// `seconds` is what lets a 4K60 file have a SHORTER preview instead of no
/// preview - measured, that one is trimmed from 2.5 s to 1.8 s and nothing else on
/// this deployment is trimmed at all.
///
/// `source_fps` may be anything the header claims, including nothing.
pub fn motion_clip_budget(frame_pixels: u64, source_fps: Option<f64>) -> MotionClipBudget {
    let limit = max_motion_decode_pixels();
    if frame_pixels == 0 {
        return MotionClipBudget { ok: false, frames: 0, pixels: 0, limit, seconds: 0.0 };
    }

    let fps = match source_fps {
        Some(f) if f.is_finite() && f > 0.0 => f.clamp(MOTION_MIN_SOURCE_FPS, MOTION_MAX_SOURCE_FPS),
        _ => MOTION_ASSUMED_FPS,
    };

    let per_second = frame_pixels as f64 * fps;
    let seconds = MOTION_CLIP_SECONDS.min(limit as f64 / per_second);
    if seconds < MOTION_CLIP_MIN_SECONDS {
        return MotionClipBudget { ok: false, frames: 0, pixels: 0, limit, seconds: 0.0 };
    }
    MotionClipBudget {
        ok: true,
        seconds: js_round(seconds * 1000.0) / 1000.0,
        frames: js_round(seconds * MOTION_CLIP_FPS) as u64,
        pixels: js_round(per_second * seconds) as u64,
        limit,
    }
}

// ---------------------------------------------------------------------------
// Timeline preview (scrub-bar sprite sheet)
// ---------------------------------------------------------------------------
//
// What one sprite sheet is allowed to cost.
//
// The shape it is budgeting is in lib/media/timeline.ts: ONE ffmpeg pass over
// the file with the demuxer told to throw away every packet that is not a
// keyframe. The numbers that decided it, measured in the production image
// (ffmpeg 5.1.9, `cpus: 2.0`, `mem_limit: 2g`) against real uploads, all for a
// finished 100 tile sheet:
//
//   shape                                        wall      peak RSS
//   one pass, -discard nokey                     0.6-4.0 s  67-260 MB
//   one pass, -skip_frame nokey                  1.2-3.4 s  76-160 MB
//   one pass, full decode                        6.6-23.5 s 68-269 MB
//   N=40 separate seeking ffmpeg processes      13.6-36.2 s  ~50 MB each
//   ONE process holding N=40 seeking inputs      OOM-KILLED  1.93 GB
//
// Three things fall out of that table.
//
// First, THE SEEK IS THE WRONG TOOL HERE, which is the opposite of the lesson
// the poster and the motion clip learned. Those want one picture, or a run of
// pictures from one place, and a seek is how you avoid decoding the rest. A
// timeline wants a picture from EVERYWHERE, so the seeks stop being a shortcut
// past the file and become forty separate walks into it: 13.6 s to 36.2 s for
// forty tiles, against 0.6 s to 4.0 s for a hundred in one pass.
//
// Second, THE SEEKS MUST NOT BE BATCHED INTO ONE PROCESS. `-ss` before each of N
// `-i` flags is the obvious repair for the per-process overhead, and it is a
// memory bomb: each input carries its own decoder and its own filtergraph
// queues, and forty of them on a 1080p source peaked at 1.93 GB inside a 2 GiB
// container and were killed by the kernel. Measured, not feared - it happened on
// the box while this was being sized, twice.
//
// Third, LENGTH IS NOT THE COST. Two hours of recording cost 3.2 s and 80 MB;
// fifteen seconds of 4K cost 1.8 s and 260 MB. What the pass really pays for is
// the KEYFRAMES it decodes (measured counts: 13 to 157 for real clips, ~1000 for
// a two hour file) multiplied by the size of one picture. So there is no
// duration ceiling here. The ceilings are on bytes, which the audio/video path
// already sets, and on the size of a single frame, below.

/// Peak bytes of RSS per pixel of ONE source frame.
///
/// Measured over the baseline (~40 MB of ffmpeg with nothing to do): 720p 38 B,
/// 1080p 39 B, 4K 27 B - the rate FALLS as the picture grows, because a decoder's
/// fixed structures stop dominating. 32 is the 4K figure with margin, which is
/// the one that matters: the ceiling below only ever binds on a very large frame.
pub const TIMELINE_BYTES_PER_FRAME_PIXEL: u64 = 32;

/// Largest single frame a sheet may be built from.
///
/// A sprite pass holds one decoded picture and a few filter copies of it, so the
/// bound is what fits in a conversion slot's share of the pool: 512 MiB at 32
/// bytes a pixel. That allows 4K (8.3 Mpx, measured at 260 MB) and 5K, and
/// refuses 8K (33 Mpx), which would want about a gigabyte inside a 2 GiB
/// container that is also serving the site.
///
/// There is no matching ceiling on the frame COUNT, and that is deliberate rather
/// than forgotten: the number of keyframes in a file cannot be known without
/// reading it, an estimate from duration is wrong by more than an order of
/// magnitude in both directions on real files (0.12 to 2.1 per second measured),
/// and a guess that refuses a file this pass would have finished in three seconds
/// is worse than no guess. The deadline is the backstop for the shape no ceiling
/// predicts - an all-intra source, where "keyframe only" means every frame.
pub fn max_timeline_frame_pixels() -> u64 {
    round_mpx(CONVERT_MEMORY_BYTES as f64 / TIMELINE_BYTES_PER_FRAME_PIXEL as f64) as u64
}

/// Units of the shared limiter one sheet holds.
///
/// A conversion's two. The worst measured job is 4.0 s of ffmpeg and 260 MB on a
/// two core box, which is conversion-scale work; charging it a thumbnail's weight
/// would let four run at once on a box that has two cores.
pub const TIMELINE_WEIGHT: u32 = CONVERT_WEIGHT;

/// Deadline for one sheet, queueing included.
///
/// The worst case measured on production is 4.0 s (a 79 minute, 470 MB
/// recording). Twenty seconds is that with room for a second job on the cores and
/// for a wait in the queue. The ffmpeg process has its own, smaller group timeout
/// inside lib/media/timeline.ts, so the encoder dies before the response gives up
/// rather than after.
pub const TIMELINE_TIMEOUT_MS: u64 = 20_000;

/// Deadline for the TRACK, which is a probe and some string building.
///
/// It gets its own, much shorter number and a thumbnail's weight because it is
/// the request that happens on every video view, while the sheet is only fetched
/// once somebody actually reaches for the scrub bar. Making the cheap half wait
/// behind the expensive one is the mistake the weighted limiter exists to avoid.
pub const TIMELINE_TRACK_TIMEOUT_MS: u64 = 10_000;
pub const TIMELINE_TRACK_WEIGHT: u32 = THUMBNAIL_WEIGHT;

#[cfg(test)]
mod tests {
    //! tests/unit/transform-cost.test.ts
    //!
    //! The arithmetic these tests protect is the answer to "how much RAM does the
    //! worst permitted request take, times how many of them can run at once, against
    //! the container's real limit". Getting it wrong does not fail a request - it
    //! kills the container, taking the database and whatever else the OOM killer
    //! reaches with it.
    use super::*;
    use crate::media::convert_targets::IMAGE_TARGETS;

    // describe("the transform memory budget fits the container")

    #[test]
    fn is_sized_against_the_limit_the_container_actually_has() {
        // `docker inspect modlight` -> HostConfig.Memory = 2147483648, which is what
        // docker-compose.prod.yml asks for with `mem_limit: 2g`.
        assert_eq!(CONTAINER_MEMORY_BYTES, 2 * 1024 * 1024 * 1024);
        // The app's own anonymous memory measured 348 MB in production, with the
        // cgroup already having hit its ceiling 431 times on reclaimable page cache.
        const { assert!(RESERVED_MEMORY_BYTES >= 768 * 1024 * 1024) };
        assert_eq!(TRANSFORM_POOL_BYTES, CONTAINER_MEMORY_BYTES - RESERVED_MEMORY_BYTES);
    }

    #[test]
    fn never_lets_the_worst_permitted_case_exceed_the_pool() {
        let slots = TRANSFORM_CAPACITY / CONVERT_WEIGHT;
        assert_eq!(slots, 2);

        for to in IMAGE_TARGETS {
            for animated in [false, true] {
                let worst = peak_bytes_for(to, animated, max_pixels_for(to, animated));
                assert!(worst <= CONVERT_MEMORY_BYTES as f64, "{} (animated {animated}) in one slot", to.as_str());
                assert!(
                    worst * slots as f64 <= TRANSFORM_POOL_BYTES as f64,
                    "{} (animated {animated}) across {slots} slots",
                    to.as_str()
                );
            }
        }
    }

    #[test]
    fn keeps_the_whole_container_inside_its_limit_at_full_load() {
        let worst = IMAGE_TARGETS
            .iter()
            .map(|&to| peak_bytes_for(to, false, max_pixels_for(to, false)))
            .fold(f64::NEG_INFINITY, f64::max);
        let slots = (TRANSFORM_CAPACITY / CONVERT_WEIGHT) as f64;
        // 348 MB measured baseline, and a slice for everything else the process does.
        let at_full_load = worst * slots + RESERVED_MEMORY_BYTES as f64;
        assert!(at_full_load <= CONTAINER_MEMORY_BYTES as f64);
    }

    #[test]
    fn divides_the_pool_by_capacity_not_by_job_count() {
        assert!(BYTES_PER_UNIT * TRANSFORM_CAPACITY as u64 <= TRANSFORM_POOL_BYTES);
        assert_eq!(CONVERT_MEMORY_BYTES, BYTES_PER_UNIT * CONVERT_WEIGHT as u64);
    }

    // describe("the budget charges by target, not by source alone")

    #[test]
    fn gives_the_expensive_encoders_much_less_room_than_the_cheap_ones() {
        // Measured bytes of peak RSS per source pixel: png 3.5, webp 24.2, avif 34.9.
        // A single ceiling for all of them is what let a 36 Mpx AVIF request ask for
        // 1.4 GB inside a 2 GiB container.
        assert!(max_pixels_for(ConvertTarget::Png, false) > max_pixels_for(ConvertTarget::Webp, false));
        assert!(max_pixels_for(ConvertTarget::Webp, false) > max_pixels_for(ConvertTarget::Avif, false));
        assert!(max_pixels_for(ConvertTarget::Jpg, false) > max_pixels_for(ConvertTarget::Pdf, false));
    }

    #[test]
    fn still_allows_an_everyday_picture_through_every_target() {
        // 4000x3000 is a phone photo and a 4K screenshot is 8.3 Mpx; both have to
        // work, or the budget has broken the feature instead of protecting it.
        for to in IMAGE_TARGETS {
            assert!(max_pixels_for(to, false) >= 12_000_000, "{}", to.as_str());
        }
    }

    #[test]
    fn gives_an_animated_reencode_more_pixels_than_a_still_one_because_it_streams() {
        // libvips feeds the WebP muxer a page at a time (measured: 1.42 B per source
        // pixel over 28.8 Mpx), so an animation is bound by the clock, not by RAM.
        assert!(max_pixels_for(ConvertTarget::Webp, true) > max_pixels_for(ConvertTarget::Webp, false));
    }

    #[test]
    fn refuses_the_production_gif_that_started_this() {
        // 1654x762 with 256 frames = 322 Mpx of decoding, which read as 1.26 Mpx
        // while only one frame was being counted.
        assert!(1654u64 * 762 * 256 > max_pixels_for(ConvertTarget::Webp, true));
    }

    #[test]
    fn exact_ceilings_match_the_typescript_arithmetic() {
        // Computed with the upstream module's own formulas.
        assert_eq!(max_pixels_for(ConvertTarget::Png, false), 64_000_000);
        assert_eq!(max_pixels_for(ConvertTarget::Jpg, false), 64_000_000);
        assert_eq!(max_pixels_for(ConvertTarget::Pdf, false), 40_000_000);
        assert_eq!(max_pixels_for(ConvertTarget::Webp, false), 17_000_000);
        assert_eq!(max_pixels_for(ConvertTarget::Webp, true), 59_000_000);
        assert_eq!(max_pixels_for(ConvertTarget::Avif, false), 13_000_000);
        assert_eq!(max_motion_decode_pixels(), 900_000_000);
        assert_eq!(max_motion_source_pixels(), 120_000_000);
        assert_eq!(MOTION_FRAMES as f64, js_round(MOTION_CLIP_SECONDS * MOTION_CLIP_FPS));
    }

    // The sprite-sheet budget, which is a memory ceiling on ONE FRAME rather than on
    // a whole picture the way the conversion targets are.

    #[test]
    fn timeline_ceiling_is_a_conversion_slots_share_of_memory_at_the_measured_rate() {
        assert!(max_timeline_frame_pixels() * TIMELINE_BYTES_PER_FRAME_PIXEL <= CONVERT_MEMORY_BYTES);
    }

    #[test]
    fn timeline_allows_4k_which_is_what_people_actually_record_and_refuses_8k() {
        // Measured on the production 3840x2160 .mov: 260 MB peak, inside the slot.
        assert!(3840 * 2160 <= max_timeline_frame_pixels());
        assert!(5120 * 2880 <= max_timeline_frame_pixels());
        // 8K would want about a gigabyte inside a 2 GiB container that is also
        // serving the site, and two of them may run at once.
        assert!(7680 * 4320 > max_timeline_frame_pixels());
    }

    #[test]
    fn timeline_takes_a_conversions_weight_not_a_thumbnails() {
        // The worst job measured is 4.0 s of ffmpeg on a two core box. At a
        // thumbnail's weight four of those could run at once.
        assert_eq!(TIMELINE_WEIGHT, CONVERT_WEIGHT);
        // ...while the track is a probe, and must not queue behind the sheets.
        const { assert!(TIMELINE_TRACK_WEIGHT < TIMELINE_WEIGHT) };
        const { assert!(TIMELINE_TRACK_TIMEOUT_MS < TIMELINE_TIMEOUT_MS) };
    }

    // motion budgets (exercised by motion-preview.test.ts upstream; the pure
    // arithmetic is pinned here)

    #[test]
    fn motion_page_budget_charges_every_frame() {
        let b = motion_page_budget(1654 * 762, 256);
        assert!(b.ok);
        assert_eq!(b.frames, 30);
        assert_eq!(b.pixels, 1654 * 762 * 30);
        assert!(!motion_page_budget(0, 10).ok);
        assert!(!motion_page_budget(100, 1).ok);
        assert!(!motion_page_budget(MAX_THUMBNAIL_PIXELS + 1, 10).ok);
        // affordable < 2
        assert!(!motion_page_budget(61_000_000, 10).ok);
    }

    #[test]
    fn motion_clip_budget_shortens_before_it_refuses() {
        let full = motion_clip_budget(1920 * 1080, Some(30.0));
        assert_eq!((full.ok, full.seconds, full.frames), (true, 2.5, 30));
        let trimmed = motion_clip_budget(3840 * 2160, Some(60.0));
        assert!(trimmed.ok);
        assert_eq!(trimmed.seconds, 1.808);
        assert_eq!(trimmed.frames, 22);
        // No rate: assume 60.
        assert_eq!(motion_clip_budget(3840 * 2160, None), trimmed);
        assert_eq!(motion_clip_budget(3840 * 2160, Some(f64::NAN)), trimmed);
        // Too big for even the shortest clip.
        assert!(!motion_clip_budget(7680 * 4320, Some(240.0)).ok);
        assert!(!motion_clip_budget(0, Some(30.0)).ok);
    }

    #[test]
    fn js_round_breaks_ties_upwards() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(-2.6), -3.0);
        assert_eq!(js_round(0.49999999999999994), 0.0);
    }
}

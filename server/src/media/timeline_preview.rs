//! Port of `lib/media/timeline-preview.ts`: the policy half of the scrub-bar
//! preview - how many pictures a video gets, how far apart they sit, how big
//! each one is, where it lands on the sheet, and what the WebVTT that indexes
//! them says. Pure, so every decision here is a unit test, and so the route
//! can ask "would this row produce one?" without a media engine. The engine
//! that acts on these answers is `media::timeline`.
//!
//! WHAT THE PLAYER EXPECTS, AND WHY THE SHAPE IS NOT OURS TO CHOOSE.
//! media-chrome draws the picture over the scrub bar from a metadata text
//! track labelled `thumbnails` hanging off the media element: on a preview
//! request it finds the cue covering that moment, resolves the cue's TEXT as a
//! URL against the track's own src, and pulls `#xywh=x,y,w,h` out of the
//! fragment. So the deliverable is exactly two files - one sprite sheet and
//! one VTT whose cues address rectangles inside it.
//!
//! WHY A COUNT THAT SCALES WITH LENGTH RATHER THAN A FIXED ONE. A three second
//! clip and a two hour recording are not the same problem for the READER: one
//! wants half-second detail, the other wants to find a scene. They turned out
//! to be the same problem for the SERVER (see `media::timeline`): tiles are
//! nearly free in time and linear in BYTES, so the count is chosen for the
//! person scrubbing and bounded by what we are willing to send them.

use crate::media::mime::{MediaKind, media_kind};

/// `GET /{tag}/{code}/timeline.vtt`: the metadata track.
pub const TIMELINE_TRACK_ACTION: &str = "timeline.vtt";
/// `GET /{tag}/{code}/timeline.jpg`: the sprite sheet the track's cues address.
pub const TIMELINE_SHEET_ACTION: &str = "timeline.jpg";

/// Longest edge of one tile. media-chrome sizes its preview box between
/// 120x80 and 180x160 and SCALES the tile to fit; 160 on the long edge lands
/// inside that box at scale 1 for every shape, so nothing is ever upscaled.
pub const TIMELINE_TILE_BOX: f64 = 160.0;

/// No tile smaller than this on either edge, whatever the aspect ratio claims.
pub const TIMELINE_MIN_TILE_EDGE: f64 = 16.0;

/// How far apart the pictures sit when the length does not force otherwise:
/// one a second, chosen at the dense end of the keyframe rates measured on
/// production (0.12 to 2.02 per second), because over-sampling costs only
/// bytes on a cached artefact while under-sampling throws detail away.
pub const TIMELINE_INTERVAL_SECONDS: f64 = 1.0;

/// ...and the floor: the densest keyframe spacing measured is about one every
/// 0.5 s, so sampling finer is guaranteed to hand back the same picture twice.
pub const TIMELINE_MIN_INTERVAL_SECONDS: f64 = 0.5;

/// Fewest tiles worth making, and the most we are willing to send (a BYTE
/// budget wearing a count: 100 tiles at 160x90, JPEG q6, is about 153 kB).
pub const TIMELINE_MIN_TILES: f64 = 8.0;
pub const TIMELINE_MAX_TILES: f64 = 100.0;

/// Shortest video that gets a timeline at all - DERIVED, not chosen: the
/// fewest tiles worth making, at the closest spacing the source can supply.
pub const TIMELINE_MIN_SECONDS: f64 = TIMELINE_MIN_TILES * TIMELINE_MIN_INTERVAL_SECONDS;

/// Why there is no timeline for this file (`X-Timeline-Refused`). Deliberately
/// NOT in the `X-Convert-Refused` vocabulary: these tokens are read by a
/// player deciding whether to hang a `<track>` on its video, not by a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimelineRefusal {
    NotVideo,
    TooLarge,
    TooShort,
    TooManyPixels,
    UnknownDuration,
    Unreadable,
    NoEngine,
}

impl TimelineRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            TimelineRefusal::NotVideo => "not-video",
            TimelineRefusal::TooLarge => "too-large",
            TimelineRefusal::TooShort => "too-short",
            TimelineRefusal::TooManyPixels => "too-many-pixels",
            TimelineRefusal::UnknownDuration => "unknown-duration",
            TimelineRefusal::Unreadable => "unreadable",
            TimelineRefusal::NoEngine => "no-engine",
        }
    }
}

/// Everything the sheet and the VTT have to agree about, in one object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelinePlan {
    /// Length the plan was solved for, seconds.
    pub duration: f64,
    /// Pictures on the sheet.
    pub count: f64,
    /// Seconds one picture stands for. `duration / count`.
    pub interval: f64,
    pub tile_width: f64,
    pub tile_height: f64,
    pub columns: f64,
    pub rows: f64,
}

/// `canTimelinePreview(row)`: could this row have a timeline, judged from the
/// row alone? Deliberately weaker than the real answer: duration is decided
/// after the probe.
pub fn can_timeline_preview(mime_type: Option<&str>, ext: Option<&str>, size: i64) -> bool {
    if media_kind(mime_type, ext) != MediaKind::Video {
        return false;
    }
    size > 0 && size <= crate::media::av_cost::AV_MAX_INPUT_BYTES as i64
}

/// `Math.round(x)` (halves towards +Infinity).
fn js_round(x: f64) -> f64 {
    let r = x.round();
    if x < 0.0 && r - x == -0.5 { r + 1.0 } else { r }
}

fn even_edge(value: f64) -> f64 {
    let rounded = js_round(value / 2.0) * 2.0;
    TIMELINE_MIN_TILE_EDGE.max(rounded)
}

/// `tileGeometry(width, height)`: one tile's pixel size for a source of these
/// dimensions. Down only, and both edges even (an odd edge is silently
/// rounded by ffmpeg's yuv420p and would put every `#xywh` one pixel out of
/// step with the sheet).
pub fn tile_geometry(width: f64, height: f64) -> Option<(f64, f64)> {
    if width.is_nan() || width <= 0.0 || height.is_nan() || height <= 0.0 {
        return None;
    }
    let scale = 1f64.min(TIMELINE_TILE_BOX / width.max(height));
    Some((even_edge(width * scale), even_edge(height * scale)))
}

/// `timelinePlan(duration, width, height)`: how many pictures, how far apart,
/// and how they are laid out on the sheet. The keyframe floor is asked FIRST,
/// and it is the whole of the "too short" decision. The grid is as square as
/// the count allows: a 100x1 strip of 160px tiles is past the maximum texture
/// size of plenty of hardware.
pub fn timeline_plan(duration: f64, width: f64, height: f64) -> Result<TimelinePlan, TimelineRefusal> {
    if !duration.is_finite() || duration <= 0.0 {
        return Err(TimelineRefusal::UnknownDuration);
    }
    // What the source can supply: pictures no closer together than its keyframes.
    let affordable = (duration / TIMELINE_MIN_INTERVAL_SECONDS).floor();
    if affordable < TIMELINE_MIN_TILES {
        return Err(TimelineRefusal::TooShort);
    }
    let (tile_width, tile_height) = tile_geometry(width, height).ok_or(TimelineRefusal::Unreadable)?;
    let wanted = js_round(duration / TIMELINE_INTERVAL_SECONDS);
    let count = wanted.clamp(TIMELINE_MIN_TILES, TIMELINE_MAX_TILES).min(affordable);
    let columns = count.sqrt().ceil();
    let rows = (count / columns).ceil();
    Ok(TimelinePlan { duration, count, interval: duration / count, tile_width, tile_height, columns, rows })
}

/// `sheetSize(plan)`: pixel size of the finished sheet.
pub fn sheet_size(plan: &TimelinePlan) -> (f64, f64) {
    (plan.columns * plan.tile_width, plan.rows * plan.tile_height)
}

/// `tileTime(plan, index)`: the CENTRE of the slice a tile stands for.
pub fn tile_time(plan: &TimelinePlan, index: f64) -> f64 {
    plan.interval * (index + 0.5)
}

/// `tileRect(plan, index)`: where tile `index` sits on the sheet, in pixels,
/// as `(x, y, width, height)`.
pub fn tile_rect(plan: &TimelinePlan, index: f64) -> (f64, f64, f64, f64) {
    (
        (index % plan.columns) * plan.tile_width,
        (index / plan.columns).floor() * plan.tile_height,
        plan.tile_width,
        plan.tile_height,
    )
}

/// `timelineVtt(plan, sheetName)`: the metadata track, as WebVTT text.
/// `sheet_name` is RELATIVE on purpose: media-chrome resolves a cue's text
/// against the track element's own src, so it is right on every hostname the
/// file is served from. The last cue runs to the end of the file.
pub fn timeline_vtt(plan: &TimelinePlan, sheet_name: &str) -> String {
    let n = crate::i18n::format::js_number_to_string;
    let mut lines: Vec<String> = vec!["WEBVTT".into(), String::new()];
    let count = plan.count as u64;
    for i in 0..count {
        let i = i as f64;
        let start = plan.interval * i;
        let end = if i == plan.count - 1.0 { plan.duration } else { plan.interval * (i + 1.0) };
        let (x, y, w, h) = tile_rect(plan, i);
        lines.push(n(i + 1.0));
        lines.push(format!("{} --> {}", vtt_time(start), vtt_time(end)));
        lines.push(format!("{sheet_name}#xywh={},{},{},{}", n(x), n(y), n(w), n(h)));
        lines.push(String::new());
    }
    lines.join("\n")
}

/// `HH:MM:SS.mmm`. Truncated rather than rounded, so a cue can never begin a
/// millisecond after the cue before it ended.
pub fn vtt_time(seconds: f64) -> String {
    let total = (seconds * 1000.0).floor().max(0.0);
    let total = if total.is_nan() { 0.0 } else { total };
    let ms = total % 1000.0;
    let s = (total / 1000.0).floor() % 60.0;
    let m = (total / 60_000.0).floor() % 60.0;
    let h = (total / 3_600_000.0).floor();
    format!("{:02}:{:02}:{:02}.{:03}", h as u64, m as u64, s as u64, ms as u64)
}

#[cfg(test)]
mod tests {
    //! tests/unit/timeline-plan.test.ts
    use super::*;

    fn plan(duration: f64) -> TimelinePlan {
        plan_of(duration, 1920.0, 1080.0)
    }

    fn plan_of(duration: f64, w: f64, h: f64) -> TimelinePlan {
        timeline_plan(duration, w, h).unwrap_or_else(|r| panic!("refused: {}", r.as_str()))
    }

    #[test]
    fn how_many_pictures_a_video_gets() {
        assert_eq!(plan(30.0).count, 30.0);
        assert!((plan(30.0).interval - 1.0).abs() < 5e-7);
        assert_eq!(plan(90.0).count, 90.0);
        let long = plan(475.17);
        assert_eq!(long.count, TIMELINE_MAX_TILES);
        assert!((long.interval - 4.75).abs() < 0.005);
        let very_long = plan(7296.76);
        assert_eq!(very_long.count, TIMELINE_MAX_TILES);
        assert!((very_long.interval - 72.97).abs() < 0.05);
        let short = plan(5.0);
        assert_eq!(short.count, TIMELINE_MIN_TILES);
        assert!(short.interval >= TIMELINE_MIN_INTERVAL_SECONDS);
        let mut d = TIMELINE_MIN_SECONDS;
        while d <= 300.0 {
            assert!(plan(d).interval >= TIMELINE_MIN_INTERVAL_SECONDS - 1e-9, "{d}");
            d += 0.1;
        }
    }

    #[test]
    fn refuses_what_it_cannot_plan() {
        assert_eq!(TIMELINE_MIN_SECONDS, TIMELINE_MIN_TILES * TIMELINE_MIN_INTERVAL_SECONDS);
        assert_eq!(timeline_plan(3.0, 1920.0, 1080.0), Err(TimelineRefusal::TooShort));
        assert!(timeline_plan(TIMELINE_MIN_SECONDS - 0.001, 1920.0, 1080.0).is_err());
        let at = plan(TIMELINE_MIN_SECONDS);
        assert_eq!(at.count, TIMELINE_MIN_TILES);
        assert!((at.interval - TIMELINE_MIN_INTERVAL_SECONDS).abs() < 5e-10);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(timeline_plan(bad, 1920.0, 1080.0), Err(TimelineRefusal::UnknownDuration));
        }
        for d in [4.0, 7.0, 13.6, 27.1, 60.0, 90.1, 150.0, 293.5, 475.2, 3600.0, 7296.8] {
            let p = plan(d);
            assert!(p.count >= TIMELINE_MIN_TILES && p.count <= TIMELINE_MAX_TILES);
            assert!((p.interval * p.count - d).abs() < 5e-7);
        }
    }

    #[test]
    fn one_tile() {
        assert_eq!(tile_geometry(1920.0, 1080.0), Some((160.0, 90.0)));
        assert_eq!(tile_geometry(3840.0, 2160.0), Some((160.0, 90.0)));
        assert_eq!(tile_geometry(1440.0, 1080.0), Some((160.0, 120.0)));
        assert_eq!(tile_geometry(1080.0, 1080.0), Some((160.0, 160.0)));
        assert_eq!(tile_geometry(1080.0, 1920.0), Some((90.0, 160.0)));
        assert_eq!(tile_geometry(48.0, 96.0), Some((48.0, 96.0)));
        assert_eq!(tile_geometry(120.0, 68.0), Some((120.0, 68.0)));
        for (w, h) in
            [(1920.0, 1078.0), (1920.0, 1088.0), (1195.0, 1169.0), (1654.0, 762.0), (333.0, 777.0), (101.0, 99.0)]
        {
            let (tw, th) = tile_geometry(w, h).unwrap();
            assert_eq!(tw % 2.0, 0.0);
            assert_eq!(th % 2.0, 0.0);
            assert!(tw.max(th) <= TIMELINE_TILE_BOX);
        }
        assert_eq!(tile_geometry(0.0, 1080.0), None);
        assert_eq!(tile_geometry(1920.0, 0.0), None);
        assert_eq!(timeline_plan(30.0, 0.0, 0.0), Err(TimelineRefusal::Unreadable));
    }

    #[test]
    fn the_sheets_grid() {
        for d in [4.0, 20.0, 60.0, 150.0, 7296.8] {
            let p = plan(d);
            assert!(p.columns * p.rows >= p.count);
            assert!((p.columns - 1.0) * p.rows < p.count);
        }
        for d in [30.0, 150.0, 7296.8] {
            let (w, h) = sheet_size(&plan_of(d, 1080.0, 1080.0));
            assert!(w <= 4096.0 && h <= 4096.0);
        }
        let p = plan(150.0);
        let (w, h) = sheet_size(&p);
        let mut seen = std::collections::HashSet::new();
        for i in 0..p.count as u32 {
            let (x, y, rw, rh) = tile_rect(&p, f64::from(i));
            assert!(x + rw <= w && y + rh <= h);
            seen.insert((x as u64, y as u64));
        }
        assert_eq!(seen.len(), p.count as usize);
        let p = plan(30.0);
        assert!((tile_time(&p, 0.0) - 0.5).abs() < 5e-7);
        assert!((tile_time(&p, p.count - 1.0) - 29.5).abs() < 5e-7);
        assert!(tile_time(&p, p.count - 1.0) < p.duration);
    }

    fn to_seconds(stamp: &str) -> f64 {
        let parts: Vec<&str> = stamp.split(':').collect();
        parts[0].parse::<f64>().unwrap() * 3600.0
            + parts[1].parse::<f64>().unwrap() * 60.0
            + parts[2].parse::<f64>().unwrap()
    }

    fn parse_cues(vtt: &str) -> Vec<(f64, f64, String)> {
        let lines: Vec<&str> = vtt.split('\n').collect();
        let mut cues = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if let Some((a, b)) = line.split_once(" --> ") {
                cues.push((to_seconds(a), to_seconds(b), lines[i + 1].to_string()));
            }
        }
        cues
    }

    #[test]
    fn the_webvtt_the_player_reads() {
        let vtt = timeline_vtt(&plan(30.0), TIMELINE_SHEET_ACTION);
        let lines: Vec<&str> = vtt.split('\n').collect();
        assert_eq!(lines[0], "WEBVTT");
        assert_eq!(lines[1], "");
        assert_eq!(lines[4], format!("{TIMELINE_SHEET_ACTION}#xywh=0,0,160,90"));
        assert!(!vtt.contains("http"));

        let p = plan(150.0);
        let cues = parse_cues(&timeline_vtt(&p, TIMELINE_SHEET_ACTION));
        assert_eq!(cues.len(), p.count as usize);
        for (i, (start, end, payload)) in cues.iter().enumerate() {
            let (x, y, w, h) = tile_rect(&p, i as f64);
            assert_eq!(*payload, format!("{TIMELINE_SHEET_ACTION}#xywh={x},{y},{w},{h}"));
            let t = tile_time(&p, i as f64);
            assert!(t >= *start && t <= *end);
        }
        for d in [4.0, 13.6, 150.0, 7296.76] {
            let cues = parse_cues(&timeline_vtt(&plan(d), TIMELINE_SHEET_ACTION));
            assert_eq!(cues[0].0, 0.0);
            for i in 1..cues.len() {
                assert_eq!(cues[i].0, cues[i - 1].1);
            }
            assert!(cues.last().unwrap().1 >= (d * 1000.0).floor() / 1000.0 - 0.001);
        }
        assert_eq!(vtt_time(0.0), "00:00:00.000");
        assert_eq!(vtt_time(1.5), "00:00:01.500");
        assert_eq!(vtt_time(61.25), "00:01:01.250");
        assert_eq!(vtt_time(7296.756), "02:01:36.756");
        assert_eq!(vtt_time(1.0009), "00:00:01.000");
        assert_eq!(vtt_time(-5.0), "00:00:00.000");
    }

    #[test]
    fn which_rows_are_even_offered_one() {
        let row = |mime: &str, ext: &str, size: i64| can_timeline_preview(Some(mime), Some(ext), size);
        assert!(row("video/mp4", "mp4", 50_000_000));
        assert!(!row("image/gif", "gif", 50_000_000));
        assert!(!row("audio/mpeg", "mp3", 50_000_000));
        assert!(!row("application/pdf", "pdf", 50_000_000));
        assert!(row("application/octet-stream", "mp4", 50_000_000));
        assert!(!row("video/mp4", "mp4", 0));
        assert!(!row("video/mp4", "mp4", 600 * 1024 * 1024));
    }
}

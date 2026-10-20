//! Port of `lib/media/frame-step.ts`: how far the `,` and `.` keys move the
//! picture.
//!
//! The arithmetic runs in the browser (components/viewer/MediaPlayer.tsx, kept
//! verbatim in the SPA); this copy exists so the rules have a server-side twin
//! with the upstream tests, like the other pure media modules.
//!
//! WHY THIS IS NOT JUST `currentTime += 1/30`. A browser offers no way to ask
//! "which frame am I on" or "give me the next one": stepping is a seek by a
//! DURATION, so the whole feature turns on how long one frame is, and nothing
//! in the DOM will say. The two ways of being wrong are not symmetric:
//!
//!   - a step SHORTER than one frame seeks inside the frame already on screen,
//!     repaints the same picture, and reads as a dead key;
//!   - a step LONGER skips a frame or two and still visibly moves.
//!
//! So every fallback here leans long, and the exact value is measured off the
//! file itself as soon as it plays.

/// Sanity band for a frame duration, whatever produced it: 240 fps to 1 fps.
pub const MIN_FRAME_SECONDS: f64 = 1.0 / 240.0;
pub const MAX_FRAME_SECONDS: f64 = 1.0;

/// The step used until something better is known: 42 ms, one frame at 23.81
/// fps - a hair longer than the 41.71 ms of 23.976 fps, the slowest rate in
/// circulation, so the FIRST press always lands in a new picture.
pub const DEFAULT_FRAME_SECONDS: f64 = 0.042;

/// `usableFrameSeconds(value)`: is this a frame duration we are willing to
/// step by?
pub fn usable_frame_seconds(value: f64) -> bool {
    value.is_finite() && (MIN_FRAME_SECONDS..=MAX_FRAME_SECONDS).contains(&value)
}

/// `frameSecondsFromRow(frames, duration)`: frame duration implied by the
/// row, or `None` when the row cannot say.
pub fn frame_seconds_from_row(frames: Option<f64>, duration: f64) -> Option<f64> {
    let frames = frames.filter(|f| f.is_finite() && *f > 1.0)?;
    if !duration.is_finite() || duration <= 0.0 {
        return None;
    }
    let seconds = duration / frames;
    usable_frame_seconds(seconds).then_some(seconds)
}

/// Shortest stretch of playback worth deriving a rate from: the frame counter
/// is an integer, so a window is only as precise as the number of frames in it.
pub const MIN_MEASURE_WINDOW_SECONDS: f64 = 1.0;

/// `frameSecondsFromDecoded(mediaSeconds, decodedFrames)`: frame duration from
/// a stretch of playback - the DECODED count, which a compositor that cannot
/// keep up does not thin out.
pub fn frame_seconds_from_decoded(media_seconds: f64, decoded_frames: f64) -> Option<f64> {
    if !media_seconds.is_finite() || media_seconds < MIN_MEASURE_WINDOW_SECONDS {
        return None;
    }
    if !decoded_frames.is_finite() || decoded_frames < 1.0 {
        return None;
    }
    let seconds = media_seconds / decoded_frames;
    usable_frame_seconds(seconds).then_some(seconds)
}

/// `frameStepTarget({ from, frameSeconds, direction, snapped, duration })`:
/// where a step lands. Snapped (`from` is the START of the frame on screen),
/// it aims at the MIDDLE of the neighbouring frame (+1.5 / -0.5 frames), never
/// at a boundary; not snapped, one plain frame in the direction asked.
pub fn frame_step_target(from: f64, frame_seconds: f64, direction: i8, snapped: bool, duration: f64) -> f64 {
    let delta = if snapped {
        frame_seconds * if direction > 0 { 1.5 } else { -0.5 }
    } else {
        frame_seconds * f64::from(direction)
    };
    // Half a frame short of the end, never the end itself: seeking to
    // `duration` is the one seek a browser answers by ending the video.
    let ceiling =
        if duration.is_finite() && duration > 0.0 { (duration - frame_seconds / 2.0).max(0.0) } else { f64::INFINITY };
    (from + delta).max(0.0).min(ceiling)
}

/// `sitsInFrame(currentTime, frameStart, frameSeconds)`: is `current_time`
/// still inside the frame that starts at `frame_start`?
pub fn sits_in_frame(current_time: f64, frame_start: Option<f64>, frame_seconds: f64) -> bool {
    let Some(start) = frame_start else { return false };
    if !current_time.is_finite() {
        return false;
    }
    let offset = current_time - start;
    offset >= 0.0 && offset < frame_seconds
}

#[cfg(test)]
mod tests {
    //! tests/unit/frame-step.test.ts
    use super::*;

    fn frame_at(time: f64, frame: f64) -> f64 {
        (time / frame).floor()
    }

    const REAL_RATES: [f64; 8] = [23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0];

    #[test]
    fn the_default_step() {
        const { assert!(DEFAULT_FRAME_SECONDS > 1.0 / 23.976) };
        for fps in REAL_RATES {
            let frame = 1.0 / fps;
            for offset in [0.0, 0.5, 0.99] {
                let from = 10.0 * frame + offset * frame;
                let target = frame_step_target(from, DEFAULT_FRAME_SECONDS, 1, false, 600.0);
                assert!(frame_at(target, frame) > frame_at(from, frame));
            }
        }
        const { assert!(DEFAULT_FRAME_SECONDS / (1.0 / 60.0) < 3.0) };
    }

    #[test]
    fn a_step_measured_from_the_frame_on_screen() {
        for fps in REAL_RATES {
            let frame = 1.0 / fps;
            let start = 40.0 * frame;
            let forward = frame_step_target(start, frame, 1, true, 600.0);
            let back = frame_step_target(start, frame, -1, true, 600.0);
            assert_eq!(frame_at(forward, frame), 41.0);
            assert_eq!(frame_at(back, frame), 39.0);
            assert!(forward > 41.0 * frame && forward < 42.0 * frame);
        }
        let frame = 1.0 / 30.0;
        let mut showing = 100.0 * frame;
        for _ in 0..10 {
            let target = frame_step_target(showing, frame, 1, true, 600.0);
            showing = frame_at(target, frame) * frame;
        }
        assert_eq!((showing / frame).round(), 110.0);
    }

    #[test]
    fn the_edges_of_the_file() {
        assert_eq!(frame_step_target(0.0, 0.04, -1, false, 10.0), 0.0);
        let end = frame_step_target(9.99, 0.04, 1, false, 10.0);
        assert!((end - 9.98).abs() < 5e-11 && end < 10.0);
        assert!((frame_step_target(5.0, 0.04, 1, false, f64::NAN) - 5.04).abs() < 5e-11);
    }

    #[test]
    fn what_the_row_can_say() {
        assert!((frame_seconds_from_row(Some(300.0), 10.0).unwrap() - 1.0 / 30.0).abs() < 5e-11);
        assert_eq!(frame_seconds_from_row(None, 10.0), None);
        assert_eq!(frame_seconds_from_row(Some(1.0), 10.0), None);
        assert_eq!(frame_seconds_from_row(Some(300.0), 0.0), None);
        assert_eq!(frame_seconds_from_row(Some(300.0), f64::NAN), None);
        assert_eq!(frame_seconds_from_row(Some(10_000.0), 10.0), None);
        assert_eq!(frame_seconds_from_row(Some(5.0), 10.0), None);
    }

    #[test]
    fn learning_the_rate_from_playback() {
        let close = |a: Option<f64>, b: f64| assert!((a.unwrap() - b).abs() < 5e-11);
        close(frame_seconds_from_decoded(2.0, 60.0), 1.0 / 30.0);
        close(frame_seconds_from_decoded(4.0, 96.0), 1.0 / 24.0);
        close(frame_seconds_from_decoded(1.5, 90.0), 1.0 / 60.0);
        assert_eq!(frame_seconds_from_decoded(0.26, 8.0), None);
        assert_eq!(frame_seconds_from_decoded(MIN_MEASURE_WINDOW_SECONDS - 0.001, 30.0), None);
        close(frame_seconds_from_decoded(MIN_MEASURE_WINDOW_SECONDS, 30.0), 1.0 / 30.0);
        assert_eq!(frame_seconds_from_decoded(2.0, 0.0), None);
        assert_eq!(frame_seconds_from_decoded(2.0, -5.0), None);
        assert_eq!(frame_seconds_from_decoded(f64::NAN, 30.0), None);
        assert_eq!(frame_seconds_from_decoded(1.0, 600.0), None);
        assert_eq!(frame_seconds_from_decoded(10.0, 1.0), None);
        for frames in [29.0, 30.0, 31.0] {
            let measured = frame_seconds_from_decoded(1.0, frames).unwrap();
            assert!((measured - 1.0 / 30.0).abs() / (1.0 / 30.0) < 0.05);
        }
        assert!(usable_frame_seconds(MIN_FRAME_SECONDS));
        assert!(usable_frame_seconds(MAX_FRAME_SECONDS));
        assert!(!usable_frame_seconds(MIN_FRAME_SECONDS / 2.0));
        assert!(!usable_frame_seconds(MAX_FRAME_SECONDS * 2.0));
        assert!(!usable_frame_seconds(f64::INFINITY));
    }

    #[test]
    fn whether_the_remembered_frame_start_still_holds() {
        let frame = 1.0 / 30.0;
        assert!(sits_in_frame(2.0, Some(2.0), frame));
        assert!(sits_in_frame(2.0 + frame * 0.9, Some(2.0), frame));
        assert!(!sits_in_frame(2.0 + frame * 1.5, Some(2.0), frame));
        assert!(!sits_in_frame(1.0, Some(2.0), frame));
        assert!(!sits_in_frame(2.0, None, frame));
        let presented = 5.0;
        let first = frame_step_target(presented, frame, 1, true, 60.0);
        let stale = sits_in_frame(first, Some(presented), frame);
        assert!(!stale);
        let second = frame_step_target(first, frame, 1, stale, 60.0);
        assert_eq!(frame_at(first, frame), 151.0);
        assert_eq!(frame_at(second, frame), 152.0);
    }
}

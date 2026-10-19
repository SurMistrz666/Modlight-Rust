/**
 * Frame stepping: how far the `,` and `.` keys move the picture.
 *
 * Pure, so the arithmetic that decides whether a keypress changes the frame is
 * a unit test rather than something only a real video can prove. The wiring -
 * the keys, the pause, the measurement - is components/viewer/MediaPlayer.tsx.
 *
 * WHY THIS IS NOT JUST `currentTime += 1/30`. A browser offers no way to ask
 * "which frame am I on" or "give me the next one": stepping is a seek by a
 * DURATION, so the whole feature turns on how long one frame is, and nothing in
 * the DOM will say. The two ways of being wrong are not symmetric:
 *
 *   - a step SHORTER than one frame seeks inside the frame already on screen,
 *     repaints the same picture, and reads as a dead key;
 *   - a step LONGER skips a frame or two and still visibly moves.
 *
 * So every fallback here leans long, and the exact value is measured off the
 * file itself as soon as it plays (`requestVideoFrameCallback` in the player).
 */

/**
 * Sanity band for a frame duration, whatever produced it.
 *
 * 240 fps is past every camera that lands here (a phone's slow-motion mode
 * records at 240 and stores it at 30), and 1 fps is slower than any real
 * recording - a value outside this band is a broken `nb_frames`, a seek
 * mistaken for a frame gap, or a duration of zero, and none of those should be
 * allowed to set the step.
 */
export const MIN_FRAME_SECONDS = 1 / 240;
export const MAX_FRAME_SECONDS = 1;

/**
 * The step used until something better is known: 42 ms.
 *
 * That is one frame at 23.81 fps, a hair longer than the 41.71 ms of 23.976 fps
 * - the slowest rate anything in circulation actually carries (cinema, and the
 * films and anime yt-dlp brings back). Longer than the true frame on every real
 * file, so the FIRST press always lands in a new picture; on a 60 fps screen
 * recording it covers two and a half frames, which is still frame-scale motion
 * and is corrected the moment the clip plays.
 *
 * Deliberately not 1/30: for 23.976 fps content 33 ms is four fifths of a
 * frame, so the first press would repaint the same picture and the key would
 * look broken - the one failure worth designing against.
 */
export const DEFAULT_FRAME_SECONDS = 0.042;

/** Is this a frame duration we are willing to step by? */
export function usableFrameSeconds(value: number): boolean {
  return (
    Number.isFinite(value) && value >= MIN_FRAME_SECONDS && value <= MAX_FRAME_SECONDS
  );
}

/**
 * Frame duration implied by the row, or null when the row cannot say.
 *
 * `uploads.frames` is written by the IMAGE path only (lib/upload/finalize.ts
 * counts pages of a WebP/GIF; the convert route backfills the same column), so
 * for a video it is null on every row this deployment holds today. Four lines
 * that cost nothing and are correct the day something fills it - and until
 * then the measurement below and the default above do all the work.
 */
export function frameSecondsFromRow(
  frames: number | null | undefined,
  duration: number,
): number | null {
  if (frames == null || !Number.isFinite(frames) || frames <= 1) return null;
  if (!Number.isFinite(duration) || duration <= 0) return null;
  const seconds = duration / frames;
  return usableFrameSeconds(seconds) ? seconds : null;
}

/**
 * Shortest stretch of playback worth deriving a rate from.
 *
 * The frame counter is an integer, so a window is only as precise as the number
 * of frames in it: a quarter second of 30 fps content is 8 frames and one frame
 * of rounding is 12%, while a full second is 30 and the same rounding is 3%.
 * Measured in the browser, quarter-second windows on a 30 fps clip read between
 * 26.5 and 30.7 fps; a second settles it.
 */
export const MIN_MEASURE_WINDOW_SECONDS = 1;

/**
 * Frame duration from a stretch of playback: how much media time passed, and
 * how many frames the DECODER produced while it did.
 *
 * The decoded count rather than the presented one, and that is the whole point.
 * `requestVideoFrameCallback` reports `presentedFrames`, which counts what
 * reached the compositor, and a compositor that cannot keep up - a background
 * tab, a 120 fps file on a 60 Hz screen, an automated browser - presents a
 * SUBSET at a perfectly steady rhythm. There is nothing in those numbers that
 * says so, so the rate comes out an exact multiple of the truth and the step
 * skips that many frames. Measured in this project's own harness: six seconds
 * of a 30 fps clip in which the decoder produced 176 frames and
 * `requestVideoFrameCallback` fired NOT ONCE.
 * `getVideoPlaybackQuality().totalVideoFrames` counts frames the decoder made,
 * dropped ones included, so it is immune to that - it read 29.3 fps across the
 * same six seconds.
 *
 * The window must not contain a seek: seeking decodes from the preceding
 * keyframe forward, and any of those frames that land in the count would make
 * the file look faster than it is - the one direction that hurts, because a
 * step shorter than a frame repaints the picture already on screen.
 */
export function frameSecondsFromDecoded(
  mediaSeconds: number,
  decodedFrames: number,
): number | null {
  if (!Number.isFinite(mediaSeconds) || mediaSeconds < MIN_MEASURE_WINDOW_SECONDS) return null;
  if (!Number.isFinite(decodedFrames) || decodedFrames < 1) return null;
  const seconds = mediaSeconds / decodedFrames;
  return usableFrameSeconds(seconds) ? seconds : null;
}

/**
 * Where a step lands.
 *
 * `snapped` says whether `from` is the START of the frame on screen (which
 * `requestVideoFrameCallback` gives exactly) or just some point inside it:
 *
 *   - snapped, we aim at the MIDDLE of the neighbouring frame (+1.5 / -0.5
 *     frames). Aiming at its edge instead would put the seek on the boundary
 *     between two frames, where a rounding error of a microsecond decides which
 *     one is painted - and half the time that is the frame already on screen.
 *   - not snapped, one plain frame in the direction asked. `from` is then mid
 *     frame, so a whole frame lands mid the next one just the same.
 *
 * Because the snapped case re-reads the frame start every time, repeated
 * presses cannot drift: each one is measured from the picture actually on
 * screen rather than from the last number we asked for.
 */
export function frameStepTarget(opts: {
  from: number;
  frameSeconds: number;
  direction: 1 | -1;
  snapped: boolean;
  duration: number;
}): number {
  const { from, frameSeconds, direction, snapped, duration } = opts;
  const delta = snapped
    ? frameSeconds * (direction > 0 ? 1.5 : -0.5)
    : frameSeconds * direction;

  // Half a frame short of the end, never the end itself: seeking to `duration`
  // is the one seek a browser answers by ending the video, so the last frame
  // would be unreachable by stepping forward.
  const ceiling =
    Number.isFinite(duration) && duration > 0
      ? Math.max(0, duration - frameSeconds / 2)
      : Number.POSITIVE_INFINITY;

  return Math.min(Math.max(0, from + delta), ceiling);
}

/**
 * Is `currentTime` still inside the frame that starts at `frameStart`?
 *
 * This is what keeps a remembered frame start honest. It goes stale in two
 * ordinary ways - the visitor drags the scrub bar, or presses the step key
 * again before the seek has landed - and in both the remembered value describes
 * a frame we have already left. Answering "no" costs only the boundary
 * precision above; trusting a stale start costs the keypress entirely, because
 * it recomputes the same target twice.
 */
export function sitsInFrame(
  currentTime: number,
  frameStart: number | null,
  frameSeconds: number,
): frameStart is number {
  if (frameStart == null || !Number.isFinite(currentTime)) return false;
  const offset = currentTime - frameStart;
  return offset >= 0 && offset < frameSeconds;
}

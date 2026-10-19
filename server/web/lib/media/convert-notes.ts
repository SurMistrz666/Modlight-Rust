/**
 * The vocabulary the conversion endpoints speak, and its translation keys.
 *
 * Pure and client-importable on purpose: the same two lists are needed by the
 * server (which emits the tokens as `X-Convert-Refused` / `X-Convert-Note`
 * headers) and by the UI (which turns them into a sentence). Keeping one copy is
 * what stops a token from being emitted that nothing can translate.
 */

/** Header token -> the key under `files.convert.note`. */
const NOTE_KEYS: Record<string, string> = {
  "audio-dropped": "audioDropped",
  "audio-reencoded": "audioReencoded",
  clipped: "clipped",
};

/**
 * The message key for a note, or null when the server said something this build
 * has no sentence for.
 *
 * Returning null matters. The previous version of this mapper ended in
 * `default: return "clipped"`, so an unrecognised note claimed the file had been
 * cut short - a confident, specific and false statement about someone's
 * download. Silence is the better failure.
 */
export function noteKey(note: string | null | undefined): string | null {
  if (!note) return null;
  return NOTE_KEYS[note] ?? null;
}

/**
 * Refusal tokens with a translation under `files.convert.refused`.
 *
 * Everything the two engines can refuse with: lib/media/av-action.ts for audio
 * and video, the image branch of the /convert route for the rest, plus the two
 * the shared transform limiter raises.
 */
export const REFUSAL_KEYS = new Set([
  "encode-requires-owner",
  "ffmpeg-unavailable",
  "video-codec",
  "audio-codec",
  "no-video",
  "no-audio",
  "unreadable",
  "too-large",
  "too-long",
  "too-many-pixels",
  "wrong-target",
  "empty-output",
  "busy",
  "timeout",
  "missing",
]);

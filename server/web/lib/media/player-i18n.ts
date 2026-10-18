/**
 * Polish (and English) labels for the media-chrome player controls.
 *
 * media-chrome ships de/en/es/fr/pt/zh-CN/zh-TW and no Polish, and its
 * dictionary is keyed by the English SENTENCE rather than by an id. Both facts
 * shape this module:
 *
 *  - the table below is the bridge between our message ids (which live in
 *    messages/{en,pl}/viewer.json like every other visible string) and the exact
 *    lookup string the library will ask for. `tests/unit/player-i18n.test.ts`
 *    asserts every right-hand value still exists in the library's own English
 *    dictionary, so an upstream rename fails a test instead of silently
 *    reverting one control to English on a Polish page.
 *  - because the key IS the English text, registering an incomplete dictionary
 *    is safe: anything we do not translate resolves to the key itself.
 *
 * English is registered through the same path as Polish rather than left to the
 * library default, so the two languages cannot drift: if a string is wrong in
 * messages/en/viewer.json, it is wrong on screen and someone notices.
 */

/** our message id (inside the `viewer.player` namespace) -> media-chrome key */
export const PLAYER_LABELS = {
  "tooltip.play": "Play",
  "tooltip.pause": "Pause",
  "tooltip.mute": "Mute",
  "tooltip.unmute": "Unmute",
  "tooltip.enterFullscreen": "Enter fullscreen mode",
  "tooltip.exitFullscreen": "Exit fullscreen mode",
  "tooltip.enterPip": "Enter picture in picture mode",
  "tooltip.exitPip": "Exit picture in picture mode",
  "tooltip.playbackRate": "Playback rate",
  "tooltip.seekBackward": "Seek backward",
  "tooltip.seekForward": "Seek forward",

  "aria.play": "play",
  "aria.pause": "pause",
  "aria.mute": "mute",
  "aria.unmute": "unmute",
  "aria.enterFullscreen": "enter fullscreen mode",
  "aria.exitFullscreen": "exit fullscreen mode",
  "aria.enterPip": "enter picture in picture mode",
  "aria.exitPip": "exit picture in picture mode",
  "aria.playbackRate": "Playback rate {playbackRate}",
  "aria.seekBackward": "seek back {seekOffset} seconds",
  "aria.seekForward": "seek forward {seekOffset} seconds",
  "aria.seek": "seek",
  "aria.volume": "volume",
  "aria.playbackTime": "playback time",
  "aria.loading": "media loading",
  "aria.videoPlayer": "video player",
  "aria.audioPlayer": "audio player",
  "aria.timeOf": "{currentTime} of {totalTime}",
  "aria.timeUnknown": "video not loaded, unknown time.",

  "unit.hour": "hour",
  "unit.hours": "hours",
  "unit.minute": "minute",
  "unit.minutes": "minutes",
  "unit.second": "second",
  "unit.seconds": "seconds",

  "error.network": "Network Error",
  "error.decode": "Decode Error",
  "error.unsupported": "Source Not Supported",
  "error.encrypted": "Encryption Error",
  "error.networkMessage": "A network error caused the media download to fail.",
  "error.decodeMessage":
    "A media error caused playback to be aborted. The media could be corrupt or your browser does not support this format.",
  "error.unsupportedMessage":
    "An unsupported error occurred. The server or network failed, or your browser does not support this format.",
  "error.encryptedMessage": "The media is encrypted and there are no keys to decrypt it.",
} as const satisfies Record<string, string>;

export type PlayerLabelId = keyof typeof PLAYER_LABELS;

/**
 * Labels whose text carries a placeholder that media-chrome substitutes at
 * runtime (the current time, the seek offset, the playback rate).
 *
 * ICU would reject a message with an argument nobody supplied, so we hand the
 * placeholder back to next-intl as its own value: `{playbackRate}` in, the
 * literal `{playbackRate}` out, and the library fills it in later.
 */
export const PLAYER_LABEL_VARS = {
  "aria.playbackRate": ["playbackRate"],
  "aria.seekBackward": ["seekOffset"],
  "aria.seekForward": ["seekOffset"],
  "aria.timeOf": ["currentTime", "totalTime"],
} as const satisfies Partial<Record<PlayerLabelId, readonly string[]>>;

export type PlayerTranslate = (id: PlayerLabelId, values?: Record<string, string>) => string;

/**
 * Turn our messages into the shape `addTranslation()` wants: keyed by the
 * library's own English sentence.
 */
export function buildPlayerDictionary(translate: PlayerTranslate): Record<string, string> {
  const dictionary: Record<string, string> = {};
  for (const [id, libraryKey] of Object.entries(PLAYER_LABELS) as [PlayerLabelId, string][]) {
    const names = (PLAYER_LABEL_VARS as Partial<Record<PlayerLabelId, readonly string[]>>)[id];
    const values = names
      ? Object.fromEntries(names.map((name) => [name, `{${name}}`]))
      : undefined;
    dictionary[libraryKey] = translate(id, values);
  }
  return dictionary;
}

/** The languages we have player labels for. Everything else reads English. */
export type PlayerLang = "en" | "pl";

/**
 * next-intl hands us a locale, media-chrome wants a language tag. It resolves
 * an exact tag first and the base language second, so "pl-PL" and "pl" have to
 * land on the same dictionary.
 */
export function playerLang(locale: string | null | undefined): PlayerLang {
  const base = (locale ?? "").split("-")[0].trim().toLowerCase();
  return base === "pl" ? "pl" : "en";
}

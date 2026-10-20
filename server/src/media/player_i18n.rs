//! Port of `lib/media/player-i18n.ts`: Polish (and English) labels for the
//! media-chrome player controls.
//!
//! media-chrome ships de/en/es/fr/pt/zh-CN/zh-TW and no Polish, and its
//! dictionary is keyed by the English SENTENCE rather than by an id. Both facts
//! shape this module:
//!
//!  - the table below is the bridge between our message ids (which live in
//!    messages/{en,pl}/viewer.json like every other visible string) and the
//!    exact lookup string the library will ask for. The test asserts every
//!    right-hand value still exists in the library's own English dictionary
//!    (media-chrome 4.19.2, the version the SPA ships), so an upstream rename
//!    fails a test instead of silently reverting one control to English.
//!  - because the key IS the English text, registering an incomplete
//!    dictionary is safe: anything we do not translate resolves to the key.
//!
//! The browser builds the dictionary itself (the SPA keeps the upstream
//! module); the server side is here for page data and parity tests.

use indexmap::IndexMap;

/// our message id (inside the `viewer.player` namespace) -> media-chrome key
pub const PLAYER_LABELS: [(&str, &str); 44] = [
    ("tooltip.play", "Play"),
    ("tooltip.pause", "Pause"),
    ("tooltip.mute", "Mute"),
    ("tooltip.unmute", "Unmute"),
    ("tooltip.enterFullscreen", "Enter fullscreen mode"),
    ("tooltip.exitFullscreen", "Exit fullscreen mode"),
    ("tooltip.enterPip", "Enter picture in picture mode"),
    ("tooltip.exitPip", "Exit picture in picture mode"),
    ("tooltip.playbackRate", "Playback rate"),
    ("tooltip.seekBackward", "Seek backward"),
    ("tooltip.seekForward", "Seek forward"),
    ("aria.play", "play"),
    ("aria.pause", "pause"),
    ("aria.mute", "mute"),
    ("aria.unmute", "unmute"),
    ("aria.enterFullscreen", "enter fullscreen mode"),
    ("aria.exitFullscreen", "exit fullscreen mode"),
    ("aria.enterPip", "enter picture in picture mode"),
    ("aria.exitPip", "exit picture in picture mode"),
    ("aria.playbackRate", "Playback rate {playbackRate}"),
    ("aria.seekBackward", "seek back {seekOffset} seconds"),
    ("aria.seekForward", "seek forward {seekOffset} seconds"),
    ("aria.seek", "seek"),
    ("aria.volume", "volume"),
    ("aria.playbackTime", "playback time"),
    ("aria.loading", "media loading"),
    ("aria.videoPlayer", "video player"),
    ("aria.audioPlayer", "audio player"),
    ("aria.timeOf", "{currentTime} of {totalTime}"),
    ("aria.timeUnknown", "video not loaded, unknown time."),
    ("unit.hour", "hour"),
    ("unit.hours", "hours"),
    ("unit.minute", "minute"),
    ("unit.minutes", "minutes"),
    ("unit.second", "second"),
    ("unit.seconds", "seconds"),
    ("error.network", "Network Error"),
    ("error.decode", "Decode Error"),
    ("error.unsupported", "Source Not Supported"),
    ("error.encrypted", "Encryption Error"),
    ("error.networkMessage", "A network error caused the media download to fail."),
    (
        "error.decodeMessage",
        "A media error caused playback to be aborted. The media could be corrupt or your browser does not support this format.",
    ),
    (
        "error.unsupportedMessage",
        "An unsupported error occurred. The server or network failed, or your browser does not support this format.",
    ),
    ("error.encryptedMessage", "The media is encrypted and there are no keys to decrypt it."),
];

/// Labels whose text carries a placeholder that media-chrome substitutes at
/// runtime. ICU would reject a message with an argument nobody supplied, so
/// the placeholder is handed back to the translator as its own value:
/// `{playbackRate}` in, the literal `{playbackRate}` out.
pub const PLAYER_LABEL_VARS: [(&str, &[&str]); 4] = [
    ("aria.playbackRate", &["playbackRate"]),
    ("aria.seekBackward", &["seekOffset"]),
    ("aria.seekForward", &["seekOffset"]),
    ("aria.timeOf", &["currentTime", "totalTime"]),
];

/// `buildPlayerDictionary(translate)`: our messages in the shape
/// `addTranslation()` wants, keyed by the library's own English sentence.
pub fn build_player_dictionary(
    translate: impl Fn(&str, Option<&IndexMap<String, String>>) -> String,
) -> IndexMap<String, String> {
    let mut dictionary = IndexMap::new();
    for (id, library_key) in PLAYER_LABELS {
        let values = PLAYER_LABEL_VARS
            .iter()
            .find(|(v, _)| *v == id)
            .map(|(_, names)| names.iter().map(|n| (n.to_string(), format!("{{{n}}}"))).collect::<IndexMap<_, _>>());
        dictionary.insert(library_key.to_string(), translate(id, values.as_ref()));
    }
    dictionary
}

/// The languages we have player labels for. Everything else reads English.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerLang {
    En,
    Pl,
}

impl PlayerLang {
    pub fn as_str(self) -> &'static str {
        match self {
            PlayerLang::En => "en",
            PlayerLang::Pl => "pl",
        }
    }
}

/// `playerLang(locale)`: next-intl hands us a locale, media-chrome wants a
/// language tag; "pl-PL" and "pl" land on the same dictionary.
pub fn player_lang(locale: Option<&str>) -> PlayerLang {
    let base = locale.unwrap_or("").split('-').next().unwrap_or("").trim().to_lowercase();
    if base == "pl" { PlayerLang::Pl } else { PlayerLang::En }
}

#[cfg(test)]
mod tests {
    //! tests/unit/player-i18n.test.ts (media-chrome's English keys come from
    //! tests/fixtures/media_chrome_4.19.2_en_keys.json, read out of the
    //! reference image's node_modules).
    use super::*;

    fn read_json(rel: &str) -> serde_json::Value {
        let p = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn lookup<'a>(dict: &'a serde_json::Value, path: &str) -> Option<&'a str> {
        let mut node = dict;
        for part in path.split('.') {
            node = node.get(part)?;
        }
        node.as_str()
    }

    #[test]
    fn labels_are_wired_to_strings_media_chrome_asks_for() {
        let en_keys = read_json("tests/fixtures/media_chrome_4.19.2_en_keys.json");
        let en_keys: Vec<&str> = en_keys.as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        let unknown: Vec<&str> = PLAYER_LABELS.iter().map(|(_, k)| *k).filter(|k| !en_keys.contains(k)).collect();
        assert!(unknown.is_empty(), "{unknown:?}");
        let keys: std::collections::HashSet<&str> = PLAYER_LABELS.iter().map(|(_, k)| *k).collect();
        assert_eq!(keys.len(), PLAYER_LABELS.len());
        for key in ["Play", "Pause", "Mute", "Playback rate", "seek", "volume"] {
            assert!(keys.contains(key));
        }
    }

    #[test]
    fn both_dictionaries_carry_every_player_label() {
        let en = read_json("messages/en/viewer.json");
        let pl = read_json("messages/pl/viewer.json");
        for (id, _) in PLAYER_LABELS {
            assert!(lookup(&en, &format!("player.{id}")).is_some(), "en player.{id}");
            assert!(lookup(&pl, &format!("player.{id}")).is_some(), "pl player.{id}");
        }
        for (id, names) in PLAYER_LABEL_VARS {
            for locale in [&en, &pl] {
                let text = lookup(locale, &format!("player.{id}")).unwrap_or("");
                for name in names {
                    assert!(text.contains(&format!("{{{name}}}")), "{id} / {name}");
                }
            }
        }
    }

    #[test]
    fn build_player_dictionary_keys_by_library_sentence() {
        let dictionary = build_player_dictionary(|id, _| format!("<{id}>"));
        assert_eq!(dictionary["Play"], "<tooltip.play>");
        assert_eq!(dictionary["seek"], "<aria.seek>");
        assert_eq!(dictionary.len(), PLAYER_LABELS.len());

        let seen = std::cell::RefCell::new(IndexMap::<String, Option<IndexMap<String, String>>>::new());
        build_player_dictionary(|id, values| {
            seen.borrow_mut().insert(id.to_string(), values.cloned());
            id.to_string()
        });
        let seen = seen.into_inner();
        let time_of = seen["aria.timeOf"].clone().unwrap();
        assert_eq!(time_of["currentTime"], "{currentTime}");
        assert_eq!(time_of["totalTime"], "{totalTime}");
        assert_eq!(seen["aria.seekBackward"].clone().unwrap()["seekOffset"], "{seekOffset}");
        assert!(seen["tooltip.play"].is_none());
    }

    #[test]
    fn player_lang_resolves_the_base_language() {
        assert_eq!(player_lang(Some("pl")), PlayerLang::Pl);
        assert_eq!(player_lang(Some("pl-PL")), PlayerLang::Pl);
        assert_eq!(player_lang(Some("PL")), PlayerLang::Pl);
        for l in [Some("en-GB"), Some("de"), None, Some("")] {
            assert_eq!(player_lang(l), PlayerLang::En);
        }
    }
}

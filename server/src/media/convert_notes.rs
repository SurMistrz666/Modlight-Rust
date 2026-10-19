//! Port of `lib/media/convert-notes.ts`: the vocabulary the conversion
//! endpoints speak, and its translation keys.
//!
//! Pure and client-importable on purpose upstream: the same two lists are
//! needed by the server (which emits the tokens as `X-Convert-Refused` /
//! `X-Convert-Note` headers) and by the UI (which turns them into a sentence).
//! Keeping one copy is what stops a token from being emitted that nothing can
//! translate.

/// Header token -> the key under `files.convert.note`.
const NOTE_KEYS: [(&str, &str); 3] =
    [("audio-dropped", "audioDropped"), ("audio-reencoded", "audioReencoded"), ("clipped", "clipped")];

/// The message key for a note, or `None` when the server said something this
/// build has no sentence for.
///
/// Returning `None` matters. The previous version of this mapper ended in
/// `default: return "clipped"`, so an unrecognised note claimed the file had
/// been cut short - a confident, specific and false statement about someone's
/// download. Silence is the better failure.
pub fn note_key(note: Option<&str>) -> Option<&'static str> {
    let note = note.filter(|n| !n.is_empty())?;
    NOTE_KEYS.iter().find(|(token, _)| *token == note).map(|(_, key)| *key)
}

/// Refusal tokens with a translation under `files.convert.refused`.
///
/// Everything the two engines can refuse with: `media::av_action` for audio and
/// video, the image branch of the /convert route for the rest, plus the two the
/// shared transform limiter raises.
pub const REFUSAL_KEYS: [&str; 15] = [
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
];

/// `REFUSAL_KEYS.has(token)`
pub fn is_refusal_key(token: &str) -> bool {
    REFUSAL_KEYS.contains(&token)
}

#[cfg(test)]
mod tests {
    //! tests/unit/convert-vocabulary.test.ts
    //!
    //! Upstream scraped the tokens out of the TypeScript sources of the two
    //! emitters; here the emitters' own tables are asked directly
    //! (`av_action::REASON_TEXT`), plus the tokens the image branch of the
    //! /convert route writes (the route is the media serving port's; its
    //! tokens are listed upstream as `wrong-target`, `too-large`,
    //! `too-many-pixels`, `unreadable`, `busy`, `timeout`).
    use super::*;
    use crate::media::av_action;

    fn files_json(locale: &str) -> serde_json::Value {
        let path = format!("{}/messages/{locale}/files.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The route's own refusal table upstream (`REASON_TEXT` in
    /// app/[userCode]/[mediaCode]/[action]/route.ts) plus the limiter's
    /// header tokens.
    const ROUTE_TOKENS: [&str; 6] = ["wrong-target", "too-large", "too-many-pixels", "unreadable", "busy", "timeout"];

    fn emitted() -> std::collections::BTreeSet<&'static str> {
        let mut found: std::collections::BTreeSet<&'static str> =
            av_action::REASON_TEXT.iter().map(|(k, _)| *k).collect();
        found.extend(ROUTE_TOKENS);
        found
    }

    #[test]
    fn every_refusal_has_a_sentence_in_both_languages() {
        let emitted = emitted();
        assert!(emitted.len() > 10);
        let (en, pl) = (files_json("en"), files_json("pl"));
        for token in &emitted {
            assert!(is_refusal_key(token), "{token} is not in REFUSAL_KEYS");
            assert!(en["convert"]["refused"][token].as_str().is_some_and(|s| !s.is_empty()), "en {token}");
            assert!(pl["convert"]["refused"][token].as_str().is_some_and(|s| !s.is_empty()), "pl {token}");
        }
    }

    #[test]
    fn no_client_side_reason_the_server_cannot_send() {
        let emitted = emitted();
        for token in REFUSAL_KEYS {
            assert!(emitted.contains(token), "{token} is claimed by the UI but never sent");
        }
    }

    #[test]
    fn translates_every_note_and_stays_silent_about_unknown_ones() {
        let (en, pl) = (files_json("en"), files_json("pl"));
        for token in ["audio-dropped", "audio-reencoded", "clipped"] {
            let key = note_key(Some(token)).unwrap_or_else(|| panic!("no key for {token}"));
            assert!(en["convert"]["note"][key].as_str().is_some_and(|s| !s.is_empty()));
            assert!(pl["convert"]["note"][key].as_str().is_some_and(|s| !s.is_empty()));
        }
        assert_eq!(note_key(Some("something-new")), None);
        assert_eq!(note_key(None), None);
        assert_eq!(note_key(Some("")), None);
    }
}

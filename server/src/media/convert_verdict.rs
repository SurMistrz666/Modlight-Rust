//! Port of `lib/media/convert-verdict.ts`: remembers whether a given
//! conversion can happen, so asking costs nothing the second time.
//!
//! The reason this exists: the menu asks before it downloads (`?check=1`),
//! and the answer for the interesting refusals - "this video has no audio
//! track", "these codecs will not go into that container" - is only knowable
//! by probing the file. Without a memo, every menu click would re-probe, and
//! the click that follows a successful check would probe the same bytes a
//! second time.
//!
//! Only two things are stored, and the asymmetry is deliberate:
//!
//!  - a REFUSAL, which is a property of these bytes and this target and will
//!    not change on its own. Serving it from memory also means a refused
//!    conversion never takes a limiter slot again, which matters because
//!    /convert is public.
//!  - an OK, which is used ONLY to answer a check. It is never allowed to skip
//!    work on the real request; it means "the gates passed a moment ago", not
//!    "the output exists". The derived-file cache is what avoids repeated work.
//!
//! The key carries the source identity (the row's checksum, or its size when
//! it has none), so converting a file in place - which rewrites the bytes
//! under the same code - retires every memo about the old ones instead of
//! answering for them.
//!
//! One process-wide map, like the module-level `Map` upstream; insertion
//! order is iteration order (`IndexMap` with order-preserving removal, as a
//! JavaScript `Map`), so the oldest key is the one the cap drops.

use std::sync::{LazyLock, Mutex};

use indexmap::IndexMap;

use crate::i18n::format::js_number_to_string;

/// `ConvertVerdict`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertVerdict {
    /// `{ ok: true, note? }`. The note is something true about the result that
    /// was not asked for - the sound could not come along, or this is the
    /// beginning of a longer file. Remembered WITH the yes, because it belongs
    /// to the result rather than to the run: without it, only the first caller
    /// inside the TTL was warned and everybody after them got a silent 204 for
    /// the very same conversion.
    Ok { note: Option<String> },
    /// `{ ok: false, status, reason, limit? }`
    Refused { status: u16, reason: String, limit: Option<String> },
}

impl ConvertVerdict {
    pub fn ok(&self) -> bool {
        matches!(self, ConvertVerdict::Ok { .. })
    }

    pub fn refused(status: u16, reason: impl Into<String>) -> ConvertVerdict {
        ConvertVerdict::Refused { status, reason: reason.into(), limit: None }
    }
}

struct Entry {
    verdict: ConvertVerdict,
    expires: i64,
}

/// Long enough to cover a menu click and the download that follows it, short
/// enough that a server-side change - ffmpeg installed, a quota lifted, a file
/// re-encoded without its row's checksum changing - is not remembered for long.
pub const TTL_MS: i64 = 10 * 60 * 1000;

/// A cap, not just a sweep. /convert is reachable with a public link and the
/// key includes a caller-chosen target, clip length and offset, so a flood of
/// distinct keys inside one TTL window would otherwise grow this map without
/// limit - exactly the shape of the rate-limiter bucket leak.
pub const MAX_ENTRIES: usize = 5_000;
const SWEEP_INTERVAL_MS: i64 = 60 * 1000;

struct Memo {
    map: IndexMap<String, Entry>,
    last_sweep: i64,
}

static MEMO: LazyLock<Mutex<Memo>> = LazyLock::new(|| Mutex::new(Memo { map: IndexMap::new(), last_sweep: 0 }));

fn memo() -> std::sync::MutexGuard<'static, Memo> {
    MEMO.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `Date.now()`
fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

/// The parts of a verdict key (`verdictKey({...})`).
#[derive(Debug, Clone)]
pub struct VerdictKeyParts<'a> {
    /// `mediaId`: the row id (a uuid string upstream).
    pub media_id: &'a str,
    pub source_tag: &'a str,
    pub to: &'a str,
    /// Privileged callers can reach encoders, so their verdicts are not shared.
    pub privileged: bool,
    pub clip_seconds: Option<f64>,
    pub offset_seconds: Option<f64>,
}

/// `verdictKey(parts)`: the parts joined with a space, numbers spelled the
/// way JavaScript prints them and a missing clip/offset as an empty string.
pub fn verdict_key(parts: &VerdictKeyParts<'_>) -> String {
    let num = |n: Option<f64>| n.map(js_number_to_string).unwrap_or_default();
    [
        parts.media_id.to_string(),
        parts.source_tag.to_string(),
        parts.to.to_string(),
        if parts.privileged { "p" } else { "a" }.to_string(),
        num(parts.clip_seconds),
        num(parts.offset_seconds),
    ]
    .join(" ")
}

fn maintain(m: &mut Memo, now: i64) {
    if now - m.last_sweep >= SWEEP_INTERVAL_MS {
        m.last_sweep = now;
        m.map.retain(|_, entry| entry.expires > now);
    }
    // Sweeping only removes EXPIRED entries, so a burst of unique keys inside
    // one window frees nothing. The cap is what actually bounds this.
    while m.map.len() > MAX_ENTRIES {
        if m.map.shift_remove_index(0).is_none() {
            break;
        }
    }
}

/// `readVerdict(key)`
pub fn read_verdict(key: &str) -> Option<ConvertVerdict> {
    read_verdict_at(key, now_ms())
}

/// `readVerdict(key, now)`
pub fn read_verdict_at(key: &str, now: i64) -> Option<ConvertVerdict> {
    let mut m = memo();
    let expires = m.map.get(key)?.expires;
    if expires <= now {
        m.map.shift_remove(key);
        return None;
    }
    m.map.get(key).map(|e| e.verdict.clone())
}

/// `rememberVerdict(key, verdict)`
pub fn remember_verdict(key: &str, verdict: ConvertVerdict) {
    remember_verdict_at(key, verdict, now_ms());
}

/// `rememberVerdict(key, verdict, now)`. Setting an existing key keeps its
/// place in the order, as `Map.prototype.set` does.
pub fn remember_verdict_at(key: &str, verdict: ConvertVerdict, now: i64) {
    let mut m = memo();
    m.map.insert(key.to_string(), Entry { verdict, expires: now + TTL_MS });
    maintain(&mut m, now);
}

/// Test seam. Never called by the app.
pub fn reset_verdicts() {
    let mut m = memo();
    m.map.clear();
    m.last_sweep = 0;
}

/// Test seam: proves the cap actually bounds the map.
pub fn verdict_count() -> usize {
    memo().map.len()
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    //! tests/unit/convert-verdict.test.ts
    use super::*;

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        let g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset_verdicts();
        g
    }

    fn key(media: &str, tag: &str, to: &str, privileged: bool) -> String {
        verdict_key(&VerdictKeyParts {
            media_id: media,
            source_tag: tag,
            to,
            privileged,
            clip_seconds: None,
            offset_seconds: None,
        })
    }

    #[test]
    fn returns_what_was_remembered_until_it_expires() {
        let _g = guard();
        let k = key("1", "abc", "mp3", false);
        remember_verdict_at(&k, ConvertVerdict::refused(409, "no-audio"), 0);
        assert_eq!(read_verdict_at(&k, 60_000), Some(ConvertVerdict::refused(409, "no-audio")));
        assert_eq!(read_verdict_at(&k, 10 * 60 * 1000 + 1), None);
    }

    #[test]
    fn keeps_a_privileged_verdict_apart_from_a_public_one() {
        assert_ne!(key("1", "abc", "mp4", true), key("1", "abc", "mp4", false));
    }

    #[test]
    fn forgets_everything_about_a_file_whose_bytes_were_replaced() {
        let _g = guard();
        let before = key("7", "old", "webm", true);
        let after = key("7", "new", "webm", true);
        remember_verdict_at(&before, ConvertVerdict::refused(409, "video-codec"), 0);
        assert_eq!(read_verdict_at(&after, 1_000), None);
    }

    #[test]
    fn stays_bounded_when_flooded_with_keys_inside_one_window() {
        let _g = guard();
        let now = 1_000;
        for i in 0..20_000 {
            remember_verdict_at(&format!("flood {i}"), ConvertVerdict::Ok { note: None }, now);
        }
        assert!(verdict_count() <= 5_000);
        assert_eq!(read_verdict_at("flood 19999", now), Some(ConvertVerdict::Ok { note: None }));
        assert_eq!(read_verdict_at("flood 0", now), None);
    }

    #[test]
    fn numbers_in_the_key_are_spelled_like_javascript() {
        let k = verdict_key(&VerdictKeyParts {
            media_id: "m",
            source_tag: "s12",
            to: "gif",
            privileged: true,
            clip_seconds: Some(6.0),
            offset_seconds: Some(2.5),
        });
        assert_eq!(k, "m s12 gif p 6 2.5");
        assert_eq!(key("m", "s", "mp4", false), "m s mp4 a  ");
    }
}

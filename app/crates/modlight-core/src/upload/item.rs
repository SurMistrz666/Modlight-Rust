//! One upload that has not happened yet (`ModlightQueueItem`), its state and the reason
//! it is in it. Shaped exactly like upstream's members so `Queue.json` is the same file
//! on both platforms: PascalCase names, enums as their numbers, dates as Newtonsoft
//! writes them.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::dotnet;
use crate::upload::progress::ProgressManager;

/// Where one queued upload stands (`ModlightQueueState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum QueueState {
    /// Not being sent right now, and will be tried again by itself.
    #[default]
    Waiting = 0,
    /// Bytes are moving.
    Sending = 1,
    /// Stopped trying. Nothing will happen to this item until a person acts on it:
    /// reached only for refusals repeating cannot fix, and the item and its bytes are
    /// kept, because every one of those causes is something the owner can remove.
    Blocked = 2,
}

/// Why a queued upload is where it is (`ModlightQueueReason`).
///
/// A code rather than a sentence, and stored as one: the queue outlives the process, so
/// a message frozen into the file would still be in last month's language after the
/// person switched to Polish. The sentence is built when it is drawn (`text.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum QueueReason {
    #[default]
    None = 0,
    /// The system says this machine has no network at all.
    NoNetwork = 1,
    /// There is a network, but nothing answered: DNS, TLS, a socket that died, a stall.
    Unreachable = 2,
    /// The server answered, with something that may well not happen next time.
    ServerError = 3,
    /// Nobody is signed in, so there is no account to send to.
    SignedOut = 4,
    /// Signed in, but as somebody else than the account this was queued for.
    AccountChanged = 5,
    /// The file this item points at is gone.
    SourceMissing = 6,
    /// 507: the account has no room.
    OutOfSpace = 7,
    /// 401: the server would not take this token.
    TokenRejected = 8,
    /// 413: the server will not accept a file this size.
    TooLarge = 9,
    /// Refused for a reason that will not change by itself.
    Refused = 10,
    /// Sat in the queue past its deadline.
    TooOld = 11,
    /// The server kept answering with errors until the attempts ran out.
    GaveUp = 12,
}

const STATES: [(QueueState, &str); 3] = [
    (QueueState::Waiting, "Waiting"),
    (QueueState::Sending, "Sending"),
    (QueueState::Blocked, "Blocked"),
];

const REASONS: [(QueueReason, &str); 13] = [
    (QueueReason::None, "None"),
    (QueueReason::NoNetwork, "NoNetwork"),
    (QueueReason::Unreachable, "Unreachable"),
    (QueueReason::ServerError, "ServerError"),
    (QueueReason::SignedOut, "SignedOut"),
    (QueueReason::AccountChanged, "AccountChanged"),
    (QueueReason::SourceMissing, "SourceMissing"),
    (QueueReason::OutOfSpace, "OutOfSpace"),
    (QueueReason::TokenRejected, "TokenRejected"),
    (QueueReason::TooLarge, "TooLarge"),
    (QueueReason::Refused, "Refused"),
    (QueueReason::TooOld, "TooOld"),
    (QueueReason::GaveUp, "GaveUp"),
];

/// Newtonsoft writes an enum as its number and reads either the number or the name, so
/// both are accepted here; an unknown value reads as the default rather than failing
/// the whole file.
struct EnumVisitor<T: 'static + Copy + Default>(&'static [(T, &'static str)]);

impl<T: Copy + Default> Visitor<'_> for EnumVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an enum number or name")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<T, E> {
        Ok(self
            .0
            .get(value as usize)
            .filter(|_| value >= 0)
            .map(|(v, _)| *v)
            .unwrap_or_default())
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<T, E> {
        self.visit_i64(value as i64)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<T, E> {
        Ok(self
            .0
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(value))
            .map(|(v, _)| *v)
            .unwrap_or_default())
    }

    fn visit_unit<E: de::Error>(self) -> Result<T, E> {
        Ok(T::default())
    }
}

impl Serialize for QueueState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(*self as i64)
    }
}

impl<'de> Deserialize<'de> for QueueState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(EnumVisitor(&STATES))
    }
}

impl Serialize for QueueReason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(*self as i64)
    }
}

impl<'de> Deserialize<'de> for QueueReason {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(EnumVisitor(&REASONS))
    }
}

/// One upload that has not happened yet, described well enough to happen later,
/// including after the app has been closed and reopened.
///
/// What is kept is the intent, not the bytes: a path, a name, a destination folder and
/// the account it was meant for. Copying the file would double the disk cost of the very
/// case this exists for. The one exception is content that exists nowhere else (a
/// capture not saved, a clipboard or text upload): that is spooled into the queue's own
/// folder and `owns_source` says the queue must delete it when it is done.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QueueItem {
    /// Identifies this item across restarts, and names its spool file.
    #[serde(deserialize_with = "dotnet::nullable_string::deserialize")]
    pub id: String,
    /// The name the file is uploaded under, which need not be the name on disk.
    #[serde(deserialize_with = "dotnet::nullable_string::deserialize")]
    pub file_name: String,
    /// Absolute path of the bytes to send.
    #[serde(deserialize_with = "dotnet::nullable_string::deserialize")]
    pub source_path: String,
    /// The queue created this file and must delete it when it is done with it.
    pub owns_source: bool,
    /// Size when it was queued, compared with the file on every attempt.
    pub length: i64,
    /// Last write time when it was queued; same purpose as `length`.
    #[serde(with = "dotnet::utc")]
    pub source_written_utc: DateTime<Utc>,
    /// Where it should land, or `None` for the account's own default folder.
    pub folder_id: Option<String>,
    /// Origin of the server it was queued for.
    #[serde(deserialize_with = "dotnet::nullable_string::deserialize")]
    pub host: String,
    /// Short code of the account it was queued for. Compared before every attempt: a
    /// sign-out and a sign-in as somebody else must not post one person's captures into
    /// the other's library hours later.
    #[serde(deserialize_with = "dotnet::nullable_string::deserialize")]
    pub account_tag: String,
    #[serde(with = "dotnet::utc")]
    pub queued_at_utc: DateTime<Utc>,
    /// After this, waiting stops being patience and starts being a leak.
    #[serde(with = "dotnet::utc")]
    pub deadline_utc: DateTime<Utc>,
    /// How many times the SERVER answered with an error. Counted apart from waiting for
    /// a network: a missing network will change by itself, a server that answered 500
    /// eight times running will not answer differently on the ninth.
    pub server_attempts: i32,
    /// Not before this. Set by the backoff.
    #[serde(with = "dotnet::utc")]
    pub next_attempt_utc: DateTime<Utc>,
    /// How long the last wait was, which is what the next one steps up from. Persisted
    /// so a machine restarted every ten minutes does not get a fresh 15 s retry loop.
    pub backoff_seconds: i32,
    /// The resumable session on the server, once there is one. Holding it is what lets a
    /// transfer that was 90% sent resume at 90% the next morning; it is a capability for
    /// that one upload, written in clear on purpose (worth far less than the token).
    pub tus_url: Option<String>,
    /// Last offset the server confirmed. Advisory: the server is asked before every try.
    pub tus_offset: i64,
    pub state: QueueState,
    pub reason: QueueReason,
    /// Whatever the server said, or a number for the sentence. Never the sentence itself.
    #[serde(deserialize_with = "dotnet::nullable_string::deserialize")]
    pub detail: String,
    /// Bytes moved so far in the attempt happening now. Not persisted.
    #[serde(skip)]
    pub progress: Option<ProgressManager>,
}

impl Default for QueueItem {
    fn default() -> Self {
        QueueItem {
            id: dotnet::new_guid_n(),
            file_name: String::new(),
            source_path: String::new(),
            owns_source: false,
            length: 0,
            source_written_utc: dotnet::min_value(),
            folder_id: None,
            host: String::new(),
            account_tag: String::new(),
            queued_at_utc: Utc::now(),
            deadline_utc: dotnet::min_value(),
            server_attempts: 0,
            next_attempt_utc: dotnet::min_value(),
            backoff_seconds: 0,
            tus_url: None,
            tus_offset: 0,
            state: QueueState::Waiting,
            reason: QueueReason::None,
            detail: String::new(),
            progress: None,
        }
    }
}

impl QueueItem {
    /// Put the item back into a state a fresh process can act on (`Revive`).
    ///
    /// `Sending` describes a socket, and after a restart there is none, so it comes back
    /// as waiting; its offset is still on the server. The wait is not carried over
    /// either: a restart is a change in the world and deserves an attempt now.
    pub fn revive(&mut self, now: DateTime<Utc>) {
        self.progress = None;
        if self.state == QueueState::Sending {
            self.state = QueueState::Waiting;
            self.reason = QueueReason::None;
        }
        if self.state == QueueState::Waiting {
            self.next_attempt_utc = now;
        }
    }

    /// Whether the values read back off the disk describe an item at all (`IsUsable`).
    pub fn is_usable(&self) -> bool {
        !self.id.is_empty()
            && !self.file_name.is_empty()
            && !self.source_path.is_empty()
            && !self.host.is_empty()
            && self.length >= 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn an_item_round_trips_in_upstreams_shape() {
        let when = Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap();
        let item = QueueItem {
            id: "0123456789abcdef0123456789abcdef".into(),
            file_name: "capture.png".into(),
            source_path: "/tmp/capture.png".into(),
            length: 42,
            source_written_utc: when,
            host: "https://files.example.com".into(),
            account_tag: "alex".into(),
            queued_at_utc: when,
            deadline_utc: when,
            next_attempt_utc: when,
            state: QueueState::Blocked,
            reason: QueueReason::OutOfSpace,
            detail: "1.0 MB|0 B".into(),
            ..QueueItem::default()
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["Id"], "0123456789abcdef0123456789abcdef");
        assert_eq!(json["State"], 2);
        assert_eq!(json["Reason"], 7);
        assert_eq!(json["SourceWrittenUtc"], "2026-10-19T12:00:00Z");
        assert_eq!(json["FolderId"], serde_json::Value::Null);
        assert!(json.get("Progress").is_none());
        let back: QueueItem = serde_json::from_value(json).unwrap();
        assert_eq!(back.state, QueueState::Blocked);
        assert_eq!(back.reason, QueueReason::OutOfSpace);
        assert_eq!(back.deadline_utc, when);
    }

    #[test]
    fn what_newtonsoft_wrote_reads_back() {
        // Names instead of numbers, nulls for strings, a missing member, a date with
        // seven digits of fraction: all things a Windows install can leave behind.
        let json = r#"{
            "Id": "ab", "FileName": "a.txt", "SourcePath": "C:\\a.txt", "OwnsSource": true,
            "Length": 3, "SourceWrittenUtc": "2026-10-19T12:00:00.1234567Z",
            "FolderId": "f1", "Host": "https://h", "AccountTag": null,
            "QueuedAtUtc": "2026-10-19T12:00:00Z", "DeadlineUtc": "0001-01-01T00:00:00",
            "ServerAttempts": 2, "NextAttemptUtc": "2026-10-19T12:05:00Z",
            "TusUrl": null, "TusOffset": 0, "State": "Sending", "Reason": 99, "Detail": null
        }"#;
        let item: QueueItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.account_tag, "");
        assert_eq!(item.detail, "");
        assert_eq!(item.state, QueueState::Sending);
        assert_eq!(item.reason, QueueReason::None);
        assert!(dotnet::is_min(&item.deadline_utc));
        assert_eq!(item.backoff_seconds, 0);
        assert!(item.is_usable());
    }

    #[test]
    fn revive_brings_a_sending_item_back_to_waiting() {
        let now = Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap();
        let mut item = QueueItem {
            state: QueueState::Sending,
            reason: QueueReason::Unreachable,
            next_attempt_utc: now + chrono::Duration::minutes(10),
            ..QueueItem::default()
        };
        item.revive(now);
        assert_eq!(item.state, QueueState::Waiting);
        assert_eq!(item.reason, QueueReason::None);
        assert_eq!(item.next_attempt_utc, now);

        let mut blocked = QueueItem {
            state: QueueState::Blocked,
            reason: QueueReason::OutOfSpace,
            next_attempt_utc: now + chrono::Duration::minutes(10),
            ..QueueItem::default()
        };
        blocked.revive(now);
        assert_eq!(blocked.state, QueueState::Blocked);
        assert_eq!(
            blocked.next_attempt_utc,
            now + chrono::Duration::minutes(10)
        );
    }

    #[test]
    fn unusable_items_are_recognised() {
        let item = QueueItem::default();
        assert!(!item.is_usable(), "no name, no path, no host");
        let item = QueueItem {
            file_name: "a".into(),
            source_path: "/a".into(),
            host: "https://h".into(),
            length: -1,
            ..QueueItem::default()
        };
        assert!(!item.is_usable());
    }
}

//! Turning a queue reason into a sentence at the moment it is read
//! (`ModlightQueueText`). One place, because the row in the Upload tab and the notice
//! that goes up when an item stops must give the same reason for the same halt.

use chrono::{DateTime, Utc};

use crate::l10n::{Arg, format, s, t};
use crate::upload::item::QueueReason;

/// Separates the two figures a quota message needs.
pub const DETAIL_SEPARATOR: char = '|';

/// The sentence for `reason`, with `detail` (a number, a path, the server's own words)
/// put wherever the language's grammar wants it.
pub fn sentence(reason: QueueReason, detail: &str) -> String {
    match reason {
        QueueReason::NoNetwork => t(s::UploadQueue_Reason_NoNetwork).to_string(),
        QueueReason::Unreachable => t(s::UploadQueue_Reason_Unreachable).to_string(),
        QueueReason::ServerError => {
            if detail.is_empty() {
                t(s::UploadQueue_Reason_Unreachable).to_string()
            } else {
                format(t(s::UploadQueue_Reason_ServerError), &[Arg::from(detail)])
            }
        }
        QueueReason::SignedOut => t(s::UploadQueue_Reason_SignedOut).to_string(),
        QueueReason::AccountChanged => format(
            t(s::UploadQueue_Reason_AccountChanged),
            &[Arg::from(detail)],
        ),
        QueueReason::SourceMissing => t(s::UploadQueue_Reason_SourceMissing).to_string(),
        QueueReason::OutOfSpace => quota(detail),
        QueueReason::TokenRejected => t(s::UploadQueue_Reason_TokenRejected).to_string(),
        QueueReason::TooLarge => t(s::UploadQueue_Reason_TooLarge).to_string(),
        QueueReason::Refused => {
            if detail.is_empty() {
                t(s::UploadQueue_Reason_Refused).to_string()
            } else {
                format!("{} {detail}", t(s::UploadQueue_Reason_Refused))
            }
        }
        QueueReason::TooOld => t(s::UploadQueue_Reason_TooOld).to_string(),
        QueueReason::GaveUp => format(t(s::UploadQueue_Reason_GaveUp), &[Arg::from(detail)]),
        QueueReason::None => String::new(),
    }
}

/// "There is no room: 1.4 GB needed, 300 MB free." Both figures, because only together
/// do they say whether deleting one screenshot will do.
fn quota(detail: &str) -> String {
    match detail.find(DETAIL_SEPARATOR) {
        Some(split) if split > 0 && split < detail.len() - 1 => format(
            t(s::UploadQueue_Reason_OutOfSpace),
            &[
                Arg::from(&detail[..split]),
                Arg::from(&detail[split + DETAIL_SEPARATOR.len_utf8()..]),
            ],
        ),
        _ => t(s::UploadQueue_Reason_OutOfSpaceShort).to_string(),
    }
}

/// How long until the next attempt, in the words the transfer rows use: seconds up to
/// ninety ("1 min" for something 61 seconds away rounds the wrong way), minutes after.
pub fn countdown(next_attempt: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let left = next_attempt - now;
    if left <= chrono::Duration::zero() {
        return t(s::UploadQueue_RetryImminent).to_string();
    }
    let seconds = left.num_milliseconds() as f64 / 1000.0;
    let when = if seconds < 90.0 {
        format(
            t(s::UploadQueue_InSeconds),
            &[Arg::Int(seconds.ceil() as i64)],
        )
    } else {
        format(
            t(s::UploadQueue_InMinutes),
            &[Arg::Int((seconds / 60.0).ceil() as i64)],
        )
    };
    format(t(s::UploadQueue_RetryIn), &[Arg::from(when)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::{Language, set_thread_language};
    use chrono::TimeZone;

    #[test]
    fn every_reason_has_its_sentence() {
        set_thread_language(Some(Language::English));
        assert_eq!(sentence(QueueReason::None, "x"), "");
        assert_eq!(
            sentence(QueueReason::ServerError, ""),
            t(s::UploadQueue_Reason_Unreachable)
        );
        assert!(sentence(QueueReason::ServerError, "503").contains("503"));
        assert!(sentence(QueueReason::AccountChanged, "Alex").contains("Alex"));
        assert!(sentence(QueueReason::GaveUp, "8").contains('8'));
        assert_eq!(
            sentence(QueueReason::Refused, "nope"),
            format!("{} nope", t(s::UploadQueue_Reason_Refused))
        );
        let full = sentence(QueueReason::OutOfSpace, "1.4 GB|300 MB");
        assert!(full.contains("1.4 GB") && full.contains("300 MB"), "{full}");
        assert_eq!(
            sentence(QueueReason::OutOfSpace, "|300 MB"),
            t(s::UploadQueue_Reason_OutOfSpaceShort)
        );
        assert_eq!(
            sentence(QueueReason::OutOfSpace, "1.4 GB|"),
            t(s::UploadQueue_Reason_OutOfSpaceShort)
        );
        for reason in [
            QueueReason::NoNetwork,
            QueueReason::Unreachable,
            QueueReason::SignedOut,
            QueueReason::SourceMissing,
            QueueReason::TokenRejected,
            QueueReason::TooLarge,
            QueueReason::TooOld,
        ] {
            assert!(!sentence(reason, "").is_empty(), "{reason:?}");
        }
        set_thread_language(None);
    }

    #[test]
    fn the_countdown_switches_to_minutes_after_ninety_seconds() {
        set_thread_language(Some(Language::English));
        let now = Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap();
        assert_eq!(countdown(now, now), t(s::UploadQueue_RetryImminent));
        let in_61 = countdown(now + chrono::Duration::seconds(61), now);
        assert!(in_61.contains("61"), "{in_61}");
        let in_91 = countdown(now + chrono::Duration::seconds(91), now);
        assert!(in_91.contains('2'), "{in_91}");
        set_thread_language(None);
    }
}

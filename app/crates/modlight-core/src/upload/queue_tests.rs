//! The queue's rules, against a transport that answers what each test scripts.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::upload::item::{QueueReason, QueueState};
use crate::upload::store::QueueStore;

type Script = Box<dyn FnOnce(&mut TransferItem) -> TransferResult + Send>;

#[derive(Default)]
struct Fake {
    sends: Mutex<VecDeque<Script>>,
    quota: Mutex<VecDeque<Result<QuotaAnswer, String>>>,
    abandoned: Mutex<Vec<String>>,
    sent_names: Mutex<Vec<String>>,
    /// When set, a send waits for its cancellation instead of answering.
    hang: AtomicBool,
}

impl Fake {
    fn answer(&self, script: impl FnOnce(&mut TransferItem) -> TransferResult + Send + 'static) {
        self.sends.lock().unwrap().push_back(Box::new(script));
    }

    fn quota_answer(&self, answer: Result<QuotaAnswer, String>) {
        self.quota.lock().unwrap().push_back(answer);
    }
}

impl QueueTransport for Fake {
    fn send<'a>(
        &'a self,
        _account: &'a QueueAccount,
        item: &'a mut TransferItem,
        cancel: CancellationToken,
        mut progress: ProgressFn<'a>,
    ) -> BoxFuture<'a, TransferResult> {
        Box::pin(async move {
            self.sent_names.lock().unwrap().push(item.file_name.clone());
            if self.hang.load(Ordering::SeqCst) {
                cancel.cancelled().await;
                return TransferResult::failure(TransferOutcome::Canceled, QueueReason::None, "");
            }
            progress(item.length / 2, item.length);
            let script = self.sends.lock().unwrap().pop_front();
            match script {
                Some(script) => script(item),
                None => TransferResult::failure(
                    TransferOutcome::Retry,
                    QueueReason::Unreachable,
                    "no script",
                ),
            }
        })
    }

    fn abandon(&self, _account: QueueAccount, url: String) -> BoxFuture<'static, ()> {
        self.abandoned.lock().unwrap().push(url);
        Box::pin(async {})
    }

    fn quota(&self, _account: QueueAccount) -> BoxFuture<'static, Result<QuotaAnswer, String>> {
        let answer = self
            .quota
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(QuotaAnswer::Remaining(None)));
        Box::pin(async move { answer })
    }
}

fn success(item: &mut TransferItem) -> TransferResult {
    item.tus_url = None;
    item.tus_offset = item.length;
    TransferResult {
        outcome: TransferOutcome::Success,
        reason: QueueReason::None,
        detail: String::new(),
        copy_url: "https://files.example.com/alex/abc".into(),
        view_url: "https://files.example.com/alex/abc".into(),
        raw_url: "https://files.example.com/alex/abc.png".into(),
        retry_after: None,
    }
}

struct Bench {
    queue: UploadQueue,
    fake: Arc<Fake>,
    account: AccountSlot,
    network: Arc<AtomicBool>,
    clock: Arc<Mutex<DateTime<Utc>>>,
    dir: tempfile::TempDir,
}

fn alex() -> QueueAccount {
    QueueAccount {
        host: "https://files.example.com".into(),
        token: "t".into(),
        tag: "alex".into(),
        display_name: "Alex Rivera".into(),
    }
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let account: AccountSlot = Arc::new(RwLock::new(Some(alex())));
    let network = Arc::new(AtomicBool::new(true));
    let clock = Arc::new(Mutex::new(
        Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap(),
    ));
    let net = network.clone();
    let time = clock.clone();
    let queue = UploadQueue::new(
        QueueStore::new(dir.path()),
        account.clone(),
        fake.clone(),
        Arc::new(move || net.load(Ordering::SeqCst)),
    )
    .with_clock(Arc::new(move || *time.lock().unwrap()));
    Bench {
        queue,
        fake,
        account,
        network,
        clock,
        dir,
    }
}

impl Bench {
    fn file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn take_over(&self, name: &str, answer: Option<u16>) -> bool {
        let path = self.file(name, b"0123456789");
        self.queue.try_take_over(TakeOver {
            file_name: name.into(),
            folder_id: Some("f1".into()),
            file_path: Some(path),
            data: None,
            answer,
        })
    }

    fn advance(&self, seconds: i64) {
        *self.clock.lock().unwrap() += chrono::Duration::seconds(seconds);
    }

    fn item(&self) -> QueueItem {
        self.queue.snapshot().into_iter().next().expect("one item")
    }

    fn finished(&self) -> Vec<QueueOutcome> {
        self.queue
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                QueueEvent::Finished(o) => Some(*o),
                _ => None,
            })
            .collect()
    }

    fn notices(&self) -> Vec<String> {
        self.queue
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                QueueEvent::Notice(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    /// Make the item due and run one pass.
    async fn pump_now(&self) {
        self.advance(3600);
        self.queue.pump().await;
    }
}

#[test]
fn only_unanswered_attempts_are_taken() {
    let b = bench();
    assert!(
        !b.take_over("full.png", Some(507)),
        "507 is an answer about this upload"
    );
    assert!(!b.take_over("bad.png", Some(400)));
    for status in [408, 429, 502, 503, 504] {
        assert!(b.take_over(&format!("{status}.png"), Some(status)));
    }
    assert!(b.take_over("offline.png", None));
    assert_eq!(b.queue.count(), 6);
    let item = b.item();
    assert_eq!(item.host, "https://files.example.com");
    assert_eq!(item.account_tag, "alex");
    assert_eq!(item.folder_id.as_deref(), Some("f1"));
    assert!(
        !item.owns_source,
        "a file of its own is referenced, not copied"
    );
    assert_eq!(item.length, 10);
    let queued = *b.clock.lock().unwrap();
    assert_eq!(
        item.next_attempt_utc,
        queued + chrono::Duration::seconds(15)
    );
    assert_eq!(item.deadline_utc, queued + chrono::Duration::days(14));
    assert!(b.dir.path().join("Queue.json").is_file(), "written at once");
}

#[test]
fn content_without_a_file_is_spooled_and_nothing_without_an_account() {
    let b = bench();
    assert!(b.queue.try_take_over(TakeOver {
        file_name: "clipboard.png".into(),
        data: Some(b"png bytes".to_vec()),
        ..TakeOver::default()
    }));
    let item = b.item();
    assert!(item.owns_source);
    assert!(item.source_path.ends_with(".png"));
    assert!(std::path::Path::new(&item.source_path).is_file());

    assert!(
        !b.queue.try_take_over(TakeOver {
            file_name: "empty.png".into(),
            data: Some(Vec::new()),
            ..TakeOver::default()
        }),
        "nothing to send"
    );
    *b.account.write().unwrap() = None;
    assert!(!b.take_over("nobody.png", None));
}

#[tokio::test]
async fn a_successful_send_reports_and_cleans_up() {
    let b = bench();
    assert!(b.queue.try_take_over(TakeOver {
        file_name: "note.txt".into(),
        data: Some(b"hello".to_vec()),
        ..TakeOver::default()
    }));
    let spooled = b.item().source_path.clone();
    b.queue.drain_events();
    b.fake.answer(success);
    b.pump_now().await;
    assert_eq!(b.queue.count(), 0);
    assert!(
        !std::path::Path::new(&spooled).exists(),
        "the spool copy goes"
    );
    let events = b.queue.drain_events();
    let finished: Vec<&QueueOutcome> = events
        .iter()
        .filter_map(|e| match e {
            QueueEvent::Finished(o) => Some(o.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(finished.len(), 1);
    assert!(finished[0].succeeded);
    assert_eq!(finished[0].link, "https://files.example.com/alex/abc");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, QueueEvent::Notice(n) if n.contains("note.txt")))
    );
    assert!(events.iter().any(|e| matches!(e, QueueEvent::Progressed)));
    let saved = std::fs::read_to_string(b.dir.path().join("Queue.json")).unwrap();
    assert!(saved.contains("\"Items\": []"), "{saved}");
}

#[tokio::test]
async fn no_network_waits_without_spending_attempts() {
    let b = bench();
    b.take_over("a.png", None);
    b.network.store(false, Ordering::SeqCst);
    b.pump_now().await;
    assert_eq!(b.item().reason, QueueReason::NoNetwork);
    assert_eq!(b.item().server_attempts, 0);
    assert!(b.fake.sent_names.lock().unwrap().is_empty());

    b.network.store(true, Ordering::SeqCst);
    b.fake.answer(success);
    b.queue.pump().await;
    assert_eq!(
        b.queue.count(),
        0,
        "the row's reason was taken back and it went"
    );
}

#[tokio::test]
async fn signed_out_and_somebody_else() {
    let b = bench();
    b.take_over("a.png", None);
    *b.account.write().unwrap() = None;
    b.pump_now().await;
    assert_eq!(b.item().reason, QueueReason::SignedOut);

    *b.account.write().unwrap() = Some(QueueAccount {
        tag: "sam".into(),
        display_name: "Sam".into(),
        ..alex()
    });
    b.queue.pump().await;
    assert_eq!(b.item().reason, QueueReason::AccountChanged);
    assert_eq!(b.item().detail, "Sam");
    assert!(
        b.fake.sent_names.lock().unwrap().is_empty(),
        "never sent to Sam"
    );

    *b.account.write().unwrap() = Some(QueueAccount {
        host: "https://FILES.example.com/".into(),
        ..alex()
    });
    b.fake.answer(success);
    b.queue.pump().await;
    assert_eq!(b.queue.count(), 0, "signing back in makes it go");
}

#[tokio::test]
async fn server_errors_spend_the_budget_and_then_stop() {
    let b = bench();
    b.take_over("a.png", None);
    for attempt in 1..=MAX_SERVER_ATTEMPTS {
        b.fake.answer(|_| {
            TransferResult::failure(TransferOutcome::Retry, QueueReason::ServerError, "500")
        });
        b.pump_now().await;
        let item = b.item();
        assert_eq!(item.state, QueueState::Waiting);
        assert_eq!(item.server_attempts, attempt);
        assert_eq!(item.reason, QueueReason::ServerError);
    }
    b.queue.drain_events();
    b.fake.answer(|_| {
        TransferResult::failure(TransferOutcome::Retry, QueueReason::ServerError, "500")
    });
    b.pump_now().await;
    let item = b.item();
    assert_eq!(item.state, QueueState::Blocked);
    assert_eq!(item.reason, QueueReason::GaveUp);
    assert_eq!(item.detail, "8");
    assert_eq!(b.notices().len(), 1, "told once");

    // Blocked items are not retried by the clock.
    b.pump_now().await;
    assert_eq!(b.fake.sent_names.lock().unwrap().len(), 9);

    // Send now forgives the history.
    b.queue.send_now(&item.id);
    let item = b.item();
    assert_eq!(item.state, QueueState::Waiting);
    assert_eq!(item.server_attempts, 0);
    assert_eq!(item.backoff_seconds, 0);
}

#[tokio::test]
async fn the_backoff_climbs_and_progress_resets_it() {
    let b = bench();
    b.take_over("a.png", None);
    let mut expected = 0;
    for _ in 0..8 {
        b.fake.answer(|_| {
            TransferResult::failure(TransferOutcome::Retry, QueueReason::Unreachable, "")
        });
        b.pump_now().await;
        expected = next_backoff(expected);
        let item = b.item();
        assert_eq!(item.backoff_seconds, expected);
        assert_eq!(item.server_attempts, 0, "the network never costs attempts");
        let wait = (item.next_attempt_utc - *b.clock.lock().unwrap()).num_seconds();
        let spread = (expected / 5).max(1) as i64;
        assert!(
            (expected as i64 - spread..=expected as i64 + spread).contains(&wait),
            "{wait}"
        );
    }
    assert_eq!(expected, 900, "capped at the last step");

    // A transfer that got further proves the link works: the next wait starts low.
    b.fake.answer(|item| {
        item.tus_url = Some("https://files.example.com/api/files/s1".into());
        item.tus_offset = 6;
        TransferResult::failure(TransferOutcome::Retry, QueueReason::Unreachable, "")
    });
    b.pump_now().await;
    let item = b.item();
    assert_eq!(item.backoff_seconds, 15);
    assert_eq!(item.tus_offset, 6, "the offset is kept");
    assert_eq!(
        item.tus_url.as_deref(),
        Some("https://files.example.com/api/files/s1")
    );

    // Retry-After wins over the ladder, clamped to an hour.
    b.fake.answer(|_| TransferResult {
        retry_after: Some(Duration::from_secs(7200)),
        ..TransferResult::failure(TransferOutcome::Retry, QueueReason::ServerError, "429")
    });
    b.pump_now().await;
    let wait = (b.item().next_attempt_utc - *b.clock.lock().unwrap()).num_seconds();
    assert_eq!(wait, 3600);
}

#[tokio::test]
async fn refusals_block_and_say_so_once() {
    let b = bench();
    b.take_over("a.png", None);
    b.queue.drain_events();
    b.fake.answer(|_| {
        TransferResult::failure(TransferOutcome::Blocked, QueueReason::TooLarge, "nope")
    });
    b.pump_now().await;
    assert_eq!(b.item().state, QueueState::Blocked);
    assert_eq!(b.item().reason, QueueReason::TooLarge);
    let notices = b.notices();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].starts_with("a.png\n"), "{}", notices[0]);
}

#[tokio::test]
async fn the_quota_is_checked_before_a_byte_moves() {
    let b = bench();
    b.take_over("a.png", None);
    b.fake.quota_answer(Ok(QuotaAnswer::Remaining(Some(4))));
    b.pump_now().await;
    let item = b.item();
    assert_eq!(item.state, QueueState::Blocked);
    assert_eq!(item.reason, QueueReason::OutOfSpace);
    assert_eq!(item.detail, format!("{}|{}", cloud_size(10), cloud_size(4)));
    assert!(b.fake.sent_names.lock().unwrap().is_empty());

    let b = bench();
    b.take_over("a.png", None);
    b.fake.quota_answer(Ok(QuotaAnswer::Refused));
    b.pump_now().await;
    assert_eq!(b.item().reason, QueueReason::TokenRejected);

    let b = bench();
    b.take_over("a.png", None);
    b.fake.quota_answer(Err("connection refused".into()));
    b.pump_now().await;
    let item = b.item();
    assert_eq!(item.state, QueueState::Waiting);
    assert_eq!(item.reason, QueueReason::Unreachable);
    assert_eq!(item.server_attempts, 0);

    // The answer is trusted for a minute.
    let b = bench();
    b.take_over("a.png", None);
    b.take_over("b.png", None);
    b.fake.quota_answer(Ok(QuotaAnswer::Remaining(Some(1_000))));
    b.fake.quota_answer(Ok(QuotaAnswer::Refused));
    b.fake.answer(success);
    b.fake.answer(success);
    b.pump_now().await;
    b.queue.pump().await;
    assert_eq!(b.queue.count(), 0);
}

#[tokio::test]
async fn a_vanished_or_changed_file() {
    let b = bench();
    b.take_over("a.png", None);
    let path = b.item().source_path.clone();
    std::fs::remove_file(&path).unwrap();
    b.pump_now().await;
    assert_eq!(b.queue.count(), 0);
    let finished = b.finished();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].reason, QueueReason::SourceMissing);
    assert_eq!(finished[0].detail, path);

    let b = bench();
    b.take_over("a.png", None);
    b.fake.answer(|item| {
        item.tus_url = Some("https://files.example.com/api/files/s1".into());
        item.tus_offset = 4;
        TransferResult::failure(TransferOutcome::Retry, QueueReason::Unreachable, "")
    });
    b.pump_now().await;
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&b.item().source_path, b"a longer file than before").unwrap();
    b.fake.answer(|item| {
        assert_eq!(
            item.tus_url, None,
            "the old session belongs to the old bytes"
        );
        assert_eq!(item.tus_offset, 0);
        assert_eq!(item.length, 25);
        success(item)
    });
    b.pump_now().await;
    assert_eq!(b.queue.count(), 0);
}

#[tokio::test]
async fn old_items_are_given_up_on_with_a_row() {
    let b = bench();
    b.take_over("a.png", None);
    b.advance(15 * 24 * 3600);
    b.queue.pump().await;
    assert_eq!(b.queue.count(), 0);
    let finished = b.finished();
    assert_eq!(finished[0].reason, QueueReason::TooOld);
    assert!(!finished[0].succeeded && !finished[0].canceled);
}

#[tokio::test]
async fn cancel_leaves_a_mark_and_abandons_the_session() {
    let b = bench();
    b.take_over("a.png", None);
    b.fake.answer(|item| {
        item.tus_url = Some("https://files.example.com/api/files/s9".into());
        item.tus_offset = 3;
        TransferResult::failure(TransferOutcome::Retry, QueueReason::Unreachable, "")
    });
    b.pump_now().await;
    b.queue.drain_events();
    let id = b.item().id;
    b.queue.cancel(&id);
    assert_eq!(b.queue.count(), 0);
    let finished = b.finished();
    assert!(finished[0].canceled);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        b.fake.abandoned.lock().unwrap().as_slice(),
        ["https://files.example.com/api/files/s9"]
    );

    // In flight: the cancel stops the transfer, the worker reports it.
    let b = bench();
    b.take_over("b.png", None);
    b.fake.hang.store(true, Ordering::SeqCst);
    b.advance(3600);
    let queue = b.queue.clone();
    let worker = tokio::spawn(async move { queue.pump().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let item = b.item();
    assert_eq!(item.state, QueueState::Sending);
    b.queue.send_now(&item.id);
    assert_eq!(
        b.item().state,
        QueueState::Sending,
        "send now does nothing in flight"
    );
    b.queue.cancel(&item.id);
    worker.await.unwrap();
    assert_eq!(b.queue.count(), 0);
    assert!(b.finished().iter().any(|o| o.canceled));
}

#[tokio::test]
async fn the_queue_comes_back_after_a_restart() {
    let b = bench();
    b.take_over("a.png", None);
    b.fake.answer(|item| {
        item.tus_url = Some("https://files.example.com/api/files/s2".into());
        item.tus_offset = 7;
        TransferResult::failure(TransferOutcome::Retry, QueueReason::Unreachable, "")
    });
    b.pump_now().await;

    // A new process: same folder, new queue.
    let fake = Arc::new(Fake::default());
    let restarted = UploadQueue::new(
        QueueStore::new(b.dir.path()),
        b.account.clone(),
        fake.clone(),
        Arc::new(|| true),
    );
    restarted.restore();
    let items = restarted.snapshot();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].tus_offset, 7);
    assert!(
        items[0].next_attempt_utc <= Utc::now(),
        "a restart deserves an attempt now"
    );
    fake.answer(|item| {
        assert_eq!(item.tus_offset, 7, "resumed where it stopped");
        success(item)
    });
    restarted.pump().await;
    assert_eq!(restarted.count(), 0);
}

#[test]
fn the_ladder_and_the_jitter() {
    assert_eq!(next_backoff(0), 15);
    assert_eq!(next_backoff(15), 30);
    assert_eq!(next_backoff(600), 900);
    assert_eq!(next_backoff(900), 900);
    for _ in 0..200 {
        let j = jitter(15);
        assert!((12..=18).contains(&j), "{j}");
        assert!(jitter(1) >= 1);
    }
}

#[test]
fn matching_ignores_case_and_a_trailing_slash() {
    let item = QueueItem {
        host: "https://Files.Example.com/".into(),
        account_tag: String::new(),
        ..QueueItem::default()
    };
    assert!(matches(&item, &alex()), "an item older than the tag field");
    let item = QueueItem {
        host: "https://files.example.com".into(),
        account_tag: "ALEX".into(),
        ..QueueItem::default()
    };
    assert!(!matches(&item, &alex()), "tags are compared exactly");
}

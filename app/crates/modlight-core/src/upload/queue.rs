//! Uploads that could not go out when they were made, kept until they can
//! (`ModlightUploadQueue`).
//!
//! A failure with no answer from the server is not a failure, it is a postponement: the
//! task hands the upload over here, ends quietly, and this drains the backlog in the
//! background, across network changes and restarts, resuming a part-sent file where it
//! stopped (tus) rather than at zero.
//!
//! Three decisions, as upstream states them:
//! 1. Only unanswered attempts are taken over ([`is_worth_queuing`]); a 507 fails now.
//! 2. Waiting for a network costs no attempts; a server that answers with errors gets
//!    [`MAX_SERVER_ATTEMPTS`] tries and then stops and says so.
//! 3. Cancel means cancel, and it leaves a mark in the history.
//!
//! One item at a time, on purpose. One worker task owns every transfer; the list is
//! guarded by one lock and handed out only as copies. Events go to whoever drains them
//! (the window), which marshals for itself.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use modlight_api::tus::TusSession;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::format::cloud_size;
use crate::l10n::{Arg, format, s, t};
use crate::upload::item::{QueueItem, QueueReason, QueueState};
use crate::upload::progress::ProgressManager;
use crate::upload::store::QueueStore;
use crate::upload::text::{DETAIL_SEPARATOR, sentence};

/// How many times a server that answers with errors is asked again: spread over the
/// backoff, a little over an hour of asking.
pub const MAX_SERVER_ATTEMPTS: i32 = 8;

/// How long an item may sit here before waiting stops being patience: two weeks.
pub const MAXIMUM_AGE: chrono::Duration = chrono::Duration::days(14);

/// The backoff ladder, in seconds, capped at the last step.
pub const BACKOFF: [i32; 7] = [15, 30, 60, 120, 300, 600, 900];

/// How often the worker looks even when nothing woke it.
pub const IDLE_POLL: Duration = Duration::from_secs(15);

/// How long a quota answer is trusted before it is asked for again.
pub const QUOTA_FRESHNESS: chrono::Duration = chrono::Duration::seconds(60);

/// Bytes that may be parked when the content exists nowhere else.
pub const MAX_SPOOL_BYTES: u64 = 512 * 1024 * 1024;

/// The account the queue sends to: whoever is signed in now.
#[derive(Clone, PartialEq, Eq)]
pub struct QueueAccount {
    /// Origin without a trailing slash.
    pub host: String,
    pub token: String,
    pub tag: String,
    /// `ModlightSignIn.DisplayName`: what the "signed in as somebody else" row names.
    pub display_name: String,
}

impl std::fmt::Debug for QueueAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueAccount")
            .field("host", &self.host)
            .field("tag", &self.tag)
            .finish_non_exhaustive()
    }
}

/// Where the queue reads the current account from; the window writes it on sign-in and
/// sign-out.
pub type AccountSlot = Arc<RwLock<Option<QueueAccount>>>;

/// What became of one item that left the queue (`ModlightQueueOutcome`).
#[derive(Clone, Debug)]
pub struct QueueOutcome {
    pub item: QueueItem,
    pub succeeded: bool,
    pub canceled: bool,
    pub reason: QueueReason,
    pub detail: String,
    /// The link to hand to a person: the account's preferred one (`copy_url`).
    pub link: String,
    pub view_url: String,
    pub raw_url: String,
    pub when: DateTime<Utc>,
}

impl QueueOutcome {
    fn new(item: QueueItem) -> QueueOutcome {
        QueueOutcome {
            item,
            succeeded: false,
            canceled: false,
            reason: QueueReason::None,
            detail: String::new(),
            link: String::new(),
            view_url: String::new(),
            raw_url: String::new(),
            when: Utc::now(),
        }
    }
}

/// What the queue tells the window.
#[derive(Clone, Debug)]
pub enum QueueEvent {
    /// The list or one of its items changed: redraw from [`UploadQueue::snapshot`].
    Changed,
    /// Bytes moved on the item being sent.
    Progressed,
    /// One item left the queue.
    Finished(Box<QueueOutcome>),
    /// A line for the desktop notice (`TaskHelpers.ShowNotificationTip`); the window adds
    /// the title.
    Notice(String),
}

/// How one transfer attempt ended (`ModlightTransferOutcome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOutcome {
    Success,
    /// Did not work this time; trying again later is reasonable.
    Retry,
    /// Will not work by repetition; somebody has to do something.
    Blocked,
    /// The queue asked for it to stop.
    Canceled,
}

/// `ModlightTransferResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferResult {
    pub outcome: TransferOutcome,
    pub reason: QueueReason,
    pub detail: String,
    pub copy_url: String,
    pub view_url: String,
    pub raw_url: String,
    /// What the server asked us to wait, when it asked.
    pub retry_after: Option<Duration>,
}

impl TransferResult {
    pub fn failure(outcome: TransferOutcome, reason: QueueReason, detail: &str) -> TransferResult {
        TransferResult {
            outcome,
            reason,
            detail: detail.to_string(),
            copy_url: String::new(),
            view_url: String::new(),
            raw_url: String::new(),
            retry_after: None,
        }
    }
}

/// The resumable part of an item a transfer reads and updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferItem {
    pub file_name: String,
    pub source_path: PathBuf,
    pub length: i64,
    pub folder_id: Option<String>,
    pub tus_url: Option<String>,
    pub tus_offset: i64,
}

/// What `/api/me` said about room, for the check before a transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuotaAnswer {
    /// The server refused the token (401/403).
    Refused,
    /// Bytes left, or `None` for an unlimited (or unreported) quota.
    Remaining(Option<i64>),
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Progress callback of a transfer: bytes confirmed so far, and the total.
pub type ProgressFn<'a> = Box<dyn FnMut(i64, i64) + Send + 'a>;

/// Called by a transfer with the item as the server has just confirmed it (the session
/// made or found, then each chunk the server took whole), for the queue to save at once.
pub type CheckpointFn<'a> = Box<dyn FnMut(&TransferItem) + Send + 'a>;

/// The network side of the queue: the tus client and `/api/me`. A trait so the queue's
/// rules can be tested without a server.
pub trait QueueTransport: Send + Sync + 'static {
    /// Send one item as far as it will go this time. Updates the session URL, offset
    /// and folder on `item` as they are learned, however it returns, and hands the item
    /// to `checkpoint` whenever the server confirmed a new point to resume from.
    fn send<'a>(
        &'a self,
        account: &'a QueueAccount,
        item: &'a mut TransferItem,
        cancel: CancellationToken,
        progress: ProgressFn<'a>,
        checkpoint: CheckpointFn<'a>,
    ) -> BoxFuture<'a, TransferResult>;

    /// Tell the server to forget a half-finished upload. Best effort.
    fn abandon(&self, account: QueueAccount, url: String) -> BoxFuture<'static, ()>;

    /// Ask the account how much room it has left. `Err` when nothing answered.
    fn quota(&self, account: QueueAccount) -> BoxFuture<'static, Result<QuotaAnswer, String>>;
}

/// What a failed live upload hands over (`TryTakeOver`'s view of the task).
#[derive(Clone, Debug, Default)]
pub struct TakeOver {
    /// The name to upload under, already treated as the live path treats it.
    pub file_name: String,
    pub folder_id: Option<String>,
    /// The file on disk, when there is one.
    pub file_path: Option<PathBuf>,
    /// The bytes, when there is no file (a clipboard image, a text).
    pub data: Option<Vec<u8>>,
    /// The HTTP status the server answered with, or `None` when it never answered.
    pub answer: Option<u16>,
    /// The resumable session a live upload of these same bytes left on the server, to
    /// carry on from rather than send everything again.
    pub session: Option<LiveSession>,
}

/// A resumable session a live upload left on the server, and the length of the file it
/// was made for: a session is only any use for exactly those bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveSession {
    pub length: u64,
    pub session: TusSession,
}

/// Whether a failure is one the queue should take: no answer at all, or the four codes
/// that mean the request never reached the application (408, 429, 502, 503, 504).
pub fn is_worth_queuing(answer: Option<u16>) -> bool {
    modlight_api::is_worth_queuing(answer)
}

/// The next step up the backoff ladder.
pub fn next_backoff(current: i32) -> i32 {
    BACKOFF
        .iter()
        .copied()
        .find(|step| *step > current)
        .unwrap_or(BACKOFF[BACKOFF.len() - 1])
}

/// Spread the retries out: ten files queued during one outage must not all wake at the
/// same instant and knock the link over again.
pub fn jitter(seconds: i32) -> i32 {
    let spread = (seconds / 5).max(1);
    let offset = rand::random_range(0..=spread * 2);
    (seconds - spread + offset).max(1)
}

/// Whether this item belongs to whoever is signed in now. An item older than the tag
/// field is matched on the host alone rather than stranded for ever.
pub fn matches(item: &QueueItem, account: &QueueAccount) -> bool {
    let a = item.host.trim_end_matches('/');
    let b = account.host.trim_end_matches('/');
    if !a.eq_ignore_ascii_case(b) {
        return false;
    }
    item.account_tag.is_empty() || item.account_tag == account.tag
}

struct Inner {
    items: Vec<QueueItem>,
    sending: Option<String>,
    send_cancel: Option<CancellationToken>,
    quota_asked: Option<DateTime<Utc>>,
    quota_remaining: Option<i64>,
    last_progress_event: Option<Instant>,
}

struct Shared {
    inner: Mutex<Inner>,
    store: QueueStore,
    wake: Notify,
    events: Mutex<Sender<QueueEvent>>,
    receiver: Mutex<Receiver<QueueEvent>>,
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    account: AccountSlot,
    transport: Arc<dyn QueueTransport>,
    network: Arc<dyn Fn() -> bool + Send + Sync>,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    started: Mutex<bool>,
}

/// The offline queue. Cheap to clone; every clone is the same queue.
#[derive(Clone)]
pub struct UploadQueue {
    shared: Arc<Shared>,
}

impl UploadQueue {
    pub fn new(
        store: QueueStore,
        account: AccountSlot,
        transport: Arc<dyn QueueTransport>,
        network: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> UploadQueue {
        let (sender, receiver) = channel();
        UploadQueue {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    items: Vec::new(),
                    sending: None,
                    send_cancel: None,
                    quota_asked: None,
                    quota_remaining: None,
                    last_progress_event: None,
                }),
                store,
                wake: Notify::new(),
                events: Mutex::new(sender),
                receiver: Mutex::new(receiver),
                waker: Mutex::new(None),
                account,
                transport,
                network,
                clock: Arc::new(Utc::now),
                started: Mutex::new(false),
            }),
        }
    }

    /// A queue whose clock is `clock` (tests).
    pub fn with_clock(self, clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>) -> UploadQueue {
        let shared = Arc::try_unwrap(self.shared)
            .ok()
            .expect("with_clock is called before the queue is shared");
        UploadQueue {
            shared: Arc::new(Shared { clock, ..shared }),
        }
    }

    /// Something to call after events were sent (the window's repaint).
    pub fn set_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        *self.shared.waker.lock().unwrap() = Some(waker);
    }

    fn now(&self) -> DateTime<Utc> {
        (self.shared.clock)()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.shared.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: QueueEvent) {
        let _ = self.shared.events.lock().unwrap().send(event);
        if let Some(waker) = self.shared.waker.lock().unwrap().clone() {
            waker();
        }
    }

    /// The events since the last call, oldest first.
    pub fn drain_events(&self) -> Vec<QueueEvent> {
        self.shared.receiver.lock().unwrap().try_iter().collect()
    }

    /// Start draining, and pick up whatever the last run left behind. The file is read
    /// on the worker, not here: start-up must not wait on a disk.
    pub fn initialize(&self, runtime: &tokio::runtime::Handle) {
        {
            let mut started = self.shared.started.lock().unwrap();
            if *started {
                return;
            }
            *started = true;
        }
        let queue = self.clone();
        runtime.spawn(async move { queue.run().await });
    }

    /// Read the file back without starting the worker (tests, and the first pump).
    pub fn restore(&self) {
        let restored = self.shared.store.load(self.now());
        self.shared.store.sweep_spool(&restored);
        let count = restored.len();
        self.lock().items.extend(restored);
        if count > 0 {
            log::info!("Modlight queue: {count} upload(s) left over from the last run.");
            self.emit(QueueEvent::Changed);
        }
    }

    async fn run(&self) {
        self.restore();
        loop {
            self.pump().await;
            tokio::select! {
                _ = self.shared.wake.notified() => {}
                _ = tokio::time::sleep(IDLE_POLL) => {}
            }
        }
    }

    /// Look again now.
    pub fn wake(&self) {
        self.shared.wake.notify_one();
    }

    /// How many uploads are waiting.
    pub fn count(&self) -> usize {
        self.lock().items.len()
    }

    /// A copy of the queue, oldest first, for drawing.
    pub fn snapshot(&self) -> Vec<QueueItem> {
        self.lock().items.clone()
    }

    fn persist(&self) {
        let copy = self.snapshot();
        self.shared.store.save(&copy);
    }

    fn notice(&self, text: String) {
        self.emit(QueueEvent::Notice(text));
    }

    /// Take an upload that could not go out, and promise to send it later
    /// (`TryTakeOver`). True when the queue has taken responsibility for it; the caller
    /// then ends its task quietly and must not delete the file.
    pub fn try_take_over(&self, request: TakeOver) -> bool {
        if !is_worth_queuing(request.answer) {
            return false;
        }
        let Some(account) = self.shared.account.read().unwrap().clone() else {
            // Nothing to queue this against: an item with no account could never be
            // matched to one later without guessing.
            return false;
        };
        let now = self.now();
        let mut item = QueueItem {
            file_name: if request.file_name.is_empty() {
                "file".into()
            } else {
                request.file_name.clone()
            },
            folder_id: request.folder_id.clone().filter(|f| !f.is_empty()),
            host: account.host.clone(),
            account_tag: account.tag.clone(),
            queued_at_utc: now,
            deadline_utc: now + MAXIMUM_AGE,
            next_attempt_utc: now + chrono::Duration::seconds(BACKOFF[0] as i64),
            ..QueueItem::default()
        };
        if !self.attach(&mut item, &request) {
            return false;
        }
        if let Some(live) = request.session.filter(|live| {
            live.session.url.is_some() && i64::try_from(live.length) == Ok(item.length)
        }) {
            // The live upload's own session: the queue carries on from the server's
            // offset (asked again first) instead of sending the file a second time.
            item.tus_url = live.session.url;
            item.tus_offset = live.session.offset as i64;
            item.folder_id = live.session.folder_id;
        }
        let name = item.file_name.clone();
        let length = item.length;
        self.lock().items.push(item);
        self.persist();
        self.emit(QueueEvent::Changed);
        self.wake();
        log::info!(
            "Modlight queue: {name} could not go out now, so it is queued ({length} bytes)."
        );
        self.notice(format(t(s::UploadQueue_Notice_Queued), &[Arg::from(&name)]));
        true
    }

    /// Point the item at bytes it can read later: the file where it lies, or, for
    /// content that has no file, a copy in the queue's own folder.
    fn attach(&self, item: &mut QueueItem, request: &TakeOver) -> bool {
        if let Some(path) = request.file_path.as_ref().filter(|p| p.is_file())
            && let Ok(meta) = std::fs::metadata(path)
        {
            item.source_path = path.to_string_lossy().into_owned();
            item.owns_source = false;
            item.length = meta.len() as i64;
            item.source_written_utc = written_utc(&meta);
            return true;
        }
        let Some(data) = request.data.as_ref() else {
            return false;
        };
        if data.is_empty() || data.len() as u64 > MAX_SPOOL_BYTES {
            return false;
        }
        match self
            .shared
            .store
            .spool(&item.id, &item.file_name, &mut data.as_slice())
        {
            Ok(path) => {
                let Ok(meta) = std::fs::metadata(&path) else {
                    return false;
                };
                item.source_path = path.to_string_lossy().into_owned();
                item.owns_source = true;
                item.length = meta.len() as i64;
                item.source_written_utc = written_utc(&meta);
                true
            }
            Err(error) => {
                log::warn!("Modlight queue could not take over an upload: {error}");
                false
            }
        }
    }

    /// Try this one right now, and forgive it its history (`SendNow`): a person pressing
    /// this is asserting that something has changed.
    pub fn send_now(&self, id: &str) {
        {
            let now = self.now();
            let mut inner = self.lock();
            let Some(item) = inner.items.iter_mut().find(|i| i.id == id) else {
                return;
            };
            if item.state == QueueState::Sending {
                return;
            }
            item.state = QueueState::Waiting;
            item.reason = QueueReason::None;
            item.detail.clear();
            item.server_attempts = 0;
            item.backoff_seconds = 0;
            item.next_attempt_utc = now;
        }
        self.persist();
        self.emit(QueueEvent::Changed);
        self.wake();
    }

    /// Stop one queued upload for good (`Cancel`). An item in flight is left to the
    /// worker, which reports the cancellation when the transfer stops; any other is
    /// reported cancelled at once.
    pub fn cancel(&self, id: &str) {
        let item = {
            let mut inner = self.lock();
            let Some(index) = inner.items.iter().position(|i| i.id == id) else {
                return;
            };
            if inner.items[index].state == QueueState::Sending {
                if let Some(cancel) = &inner.send_cancel {
                    cancel.cancel();
                }
                return;
            }
            inner.items.remove(index)
        };
        let mut item = item;
        self.release(&mut item, true);
        self.persist();
        let mut outcome = QueueOutcome::new(item);
        outcome.canceled = true;
        outcome.when = self.now();
        self.emit(QueueEvent::Finished(Box::new(outcome)));
        self.emit(QueueEvent::Changed);
    }

    /// One pass of the worker (`Pump`), public for tests.
    pub async fn pump(&self) {
        if self.count() == 0 {
            return;
        }
        self.expire();
        if self.count() == 0 {
            return;
        }
        if !(self.shared.network)() {
            self.stamp(QueueReason::NoNetwork);
            return;
        }
        let Some(account) = self.shared.account.read().unwrap().clone() else {
            self.stamp(QueueReason::SignedOut);
            return;
        };
        // These two describe the machine, not the upload: take them back off the moment
        // they stop being true, or a row reads "no connection" minutes after it returned.
        self.clear(&[QueueReason::NoNetwork, QueueReason::SignedOut]);
        let Some(id) = self.due(&account) else {
            return;
        };
        self.send(&id, &account).await;
    }

    /// The next item allowed to go. One queued for another account is skipped (and says
    /// so), not stopped: signing back in as the right person makes it go by itself.
    fn due(&self, account: &QueueAccount) -> Option<String> {
        let now = self.now();
        let mut changed = false;
        let mut pick = None;
        {
            let mut inner = self.lock();
            for candidate in inner.items.iter_mut() {
                if candidate.state != QueueState::Waiting || candidate.next_attempt_utc > now {
                    continue;
                }
                if !matches(candidate, account) {
                    if candidate.reason != QueueReason::AccountChanged {
                        candidate.reason = QueueReason::AccountChanged;
                        candidate.detail = account.display_name.clone();
                        changed = true;
                    }
                    continue;
                }
                pick = Some(candidate.id.clone());
                break;
            }
        }
        if changed {
            self.emit(QueueEvent::Changed);
        }
        pick
    }

    fn clear(&self, reasons: &[QueueReason]) {
        let mut changed = false;
        {
            let mut inner = self.lock();
            for item in inner.items.iter_mut() {
                if item.state == QueueState::Waiting && reasons.contains(&item.reason) {
                    item.reason = QueueReason::None;
                    item.detail.clear();
                    changed = true;
                }
            }
        }
        if changed {
            self.emit(QueueEvent::Changed);
        }
    }

    /// Say the same thing on every waiting row when the reason is the machine's. Not
    /// persisted: it is recomputed within a second of the next start.
    fn stamp(&self, reason: QueueReason) {
        let mut changed = false;
        {
            let mut inner = self.lock();
            for item in inner.items.iter_mut() {
                if item.state == QueueState::Waiting && item.reason != reason {
                    item.reason = reason;
                    item.detail.clear();
                    changed = true;
                }
            }
        }
        if changed {
            self.emit(QueueEvent::Changed);
        }
    }

    fn take(&self, id: &str) -> Option<QueueItem> {
        let mut inner = self.lock();
        let index = inner.items.iter().position(|i| i.id == id)?;
        Some(inner.items.remove(index))
    }

    fn with_item<R>(&self, id: &str, f: impl FnOnce(&mut QueueItem) -> R) -> Option<R> {
        let mut inner = self.lock();
        inner.items.iter_mut().find(|i| i.id == id).map(f)
    }

    async fn send(&self, id: &str, account: &QueueAccount) {
        if !self.verify(id) {
            return;
        }
        if !self.has_room(id, account).await {
            return;
        }
        let cancel = CancellationToken::new();
        let Some(mut transfer) = ({
            let mut inner = self.lock();
            inner.sending = Some(id.to_string());
            inner.send_cancel = Some(cancel.clone());
            inner.items.iter_mut().find(|i| i.id == id).map(|item| {
                item.state = QueueState::Sending;
                item.reason = QueueReason::None;
                item.detail.clear();
                TransferItem {
                    file_name: item.file_name.clone(),
                    source_path: PathBuf::from(&item.source_path),
                    length: item.length,
                    folder_id: item.folder_id.clone(),
                    tus_url: item.tus_url.clone(),
                    tus_offset: item.tus_offset,
                }
            })
        }) else {
            return;
        };
        self.persist();
        self.emit(QueueEvent::Changed);

        let started_at = transfer.tus_offset;
        let mut measured = ProgressManager::new(transfer.length.max(1), started_at);
        self.with_item(id, |item| item.progress = Some(measured.clone()));
        let mut last_seen = started_at;
        let queue = self.clone();
        let id_owned = id.to_string();
        let progress: ProgressFn<'_> = Box::new(move |sent: i64, _total: i64| {
            if sent > last_seen {
                measured.update_progress(sent - last_seen);
                last_seen = sent;
                let snapshot = measured.clone();
                queue.with_item(&id_owned, |item| item.progress = Some(snapshot));
                queue.progressed();
            }
        });

        // A crash mid-file must find the last confirmed chunk in Queue.json, not the
        // offset the attempt started from.
        let queue = self.clone();
        let id_owned = id.to_string();
        let checkpoint: CheckpointFn<'_> = Box::new(move |confirmed: &TransferItem| {
            queue.with_item(&id_owned, |item| {
                item.tus_url = confirmed.tus_url.clone();
                item.tus_offset = confirmed.tus_offset;
                item.folder_id = confirmed.folder_id.clone();
            });
            queue.persist();
        });

        let result = self
            .shared
            .transport
            .send(account, &mut transfer, cancel.clone(), progress, checkpoint)
            .await;

        // Whatever happened, the offset and the session URL are worth keeping: they are
        // what the next attempt starts from.
        let moved = transfer.tus_offset > started_at;
        self.with_item(id, |item| {
            item.progress = None;
            item.tus_url = transfer.tus_url.clone();
            item.tus_offset = transfer.tus_offset;
            item.folder_id = transfer.folder_id.clone();
        });

        // The claim is held until the ending is recorded, so a Cancel landing in the
        // gap is a no-op rather than a second, false "cancelled" row.
        match result.outcome {
            TransferOutcome::Success => self.complete(id, &result),
            TransferOutcome::Canceled => self.abandon(id),
            TransferOutcome::Blocked => self.block(id, result.reason, &result.detail),
            TransferOutcome::Retry => self.reschedule(id, &result, moved),
        }
        let mut inner = self.lock();
        inner.sending = None;
        inner.send_cancel = None;
    }

    fn progressed(&self) {
        // The transfer reports every chunk; the window needs a few frames a second.
        let due = {
            let mut inner = self.lock();
            let now = Instant::now();
            let due = inner
                .last_progress_event
                .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(100));
            if due {
                inner.last_progress_event = Some(now);
            }
            due
        };
        if due {
            self.emit(QueueEvent::Progressed);
        }
    }

    /// Make sure the bytes are still there and still the same bytes (`Verify`).
    fn verify(&self, id: &str) -> bool {
        let Some(path) = self.with_item(id, |i| i.source_path.clone()) else {
            return false;
        };
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => {
                self.with_item(id, |item| {
                    let written = written_utc(&meta);
                    if meta.len() as i64 != item.length || written != item.source_written_utc {
                        // The file changed while it waited: the server's session belongs
                        // to the old contents, so the transfer starts again.
                        log::info!(
                            "Modlight queue: {} changed on disk, so the transfer starts again.",
                            item.file_name
                        );
                        item.length = meta.len() as i64;
                        item.source_written_utc = written;
                        item.tus_url = None;
                        item.tus_offset = 0;
                    }
                });
                true
            }
            _ => {
                // The price of not copying every upload: a path can stop pointing at a
                // file. It fails, and says so about a named file.
                self.fail(id, QueueReason::SourceMissing, &path);
                false
            }
        }
    }

    /// Ask whether the account still has room before sending anything (`HasRoom`):
    /// without it a full account would read as "the server will not take a file this
    /// size", true and useless.
    async fn has_room(&self, id: &str, account: &QueueAccount) -> bool {
        let now = self.now();
        let stale = self
            .lock()
            .quota_asked
            .is_none_or(|asked| now - asked > QUOTA_FRESHNESS);
        if stale {
            let answer = tokio::time::timeout(
                Duration::from_secs(30),
                self.shared.transport.quota(account.clone()),
            )
            .await
            .unwrap_or_else(|_| Err("The request timed out.".into()));
            match answer {
                Ok(QuotaAnswer::Refused) => {
                    self.block(id, QueueReason::TokenRejected, "");
                    return false;
                }
                Ok(QuotaAnswer::Remaining(remaining)) => {
                    let mut inner = self.lock();
                    inner.quota_asked = Some(now);
                    inner.quota_remaining = remaining;
                }
                Err(error) => {
                    log::warn!("Modlight queue could not read the account's quota: {error}");
                    self.reschedule(
                        id,
                        &TransferResult::failure(
                            TransferOutcome::Retry,
                            QueueReason::Unreachable,
                            &error,
                        ),
                        false,
                    );
                    return false;
                }
            }
        }
        let remaining = self.lock().quota_remaining;
        let length = self.with_item(id, |i| i.length).unwrap_or(0);
        match remaining {
            Some(remaining) if length > remaining => {
                self.block(
                    id,
                    QueueReason::OutOfSpace,
                    &format!(
                        "{}{DETAIL_SEPARATOR}{}",
                        cloud_size(length),
                        cloud_size(remaining)
                    ),
                );
                false
            }
            _ => true,
        }
    }

    fn complete(&self, id: &str, result: &TransferResult) {
        let Some(mut item) = self.take(id) else {
            return;
        };
        // The session is finished, so there is nothing on the server to abandon.
        self.release(&mut item, false);
        self.persist();
        // Deliberately NOT put on the clipboard: this can land hours after the file was
        // made, and the clipboard then holds whatever the person is doing right now.
        log::info!("Modlight queue: {} went up.", item.file_name);
        self.notice(format(
            t(s::UploadQueue_Notice_Sent),
            &[Arg::from(&item.file_name)],
        ));
        let mut outcome = QueueOutcome::new(item);
        outcome.succeeded = true;
        outcome.link = result.copy_url.clone();
        outcome.view_url = result.view_url.clone();
        outcome.raw_url = result.raw_url.clone();
        outcome.when = self.now();
        self.emit(QueueEvent::Finished(Box::new(outcome)));
        self.emit(QueueEvent::Changed);
        self.wake();
    }

    /// The person cancelled the item that was in flight.
    fn abandon(&self, id: &str) {
        let Some(mut item) = self.take(id) else {
            return;
        };
        self.release(&mut item, true);
        self.persist();
        let mut outcome = QueueOutcome::new(item);
        outcome.canceled = true;
        outcome.when = self.now();
        self.emit(QueueEvent::Finished(Box::new(outcome)));
        self.emit(QueueEvent::Changed);
        self.wake();
    }

    /// Stopped, and waiting for a person rather than a clock. Announced once per
    /// reason, because the point of stopping rather than looping is that somebody is told.
    fn block(&self, id: &str, reason: QueueReason, detail: &str) {
        let Some((announce, name, detail)) = self.with_item(id, |item| {
            let announce = item.state != QueueState::Blocked || item.reason != reason;
            item.state = QueueState::Blocked;
            item.reason = reason;
            item.detail = detail.to_string();
            item.progress = None;
            (announce, item.file_name.clone(), item.detail.clone())
        }) else {
            return;
        };
        self.persist();
        self.emit(QueueEvent::Changed);
        if announce {
            log::info!("Modlight queue: {name} stopped - {reason:?} {detail}");
            self.notice(format!("{name}\n{}", sentence(reason, &detail)));
        }
    }

    /// Off the queue for good, with a reason.
    fn fail(&self, id: &str, reason: QueueReason, detail: &str) {
        let Some(mut item) = self.take(id) else {
            return;
        };
        self.release(&mut item, true);
        self.persist();
        log::info!(
            "Modlight queue: {} given up on - {reason:?} {detail}",
            item.file_name
        );
        let mut outcome = QueueOutcome::new(item);
        outcome.reason = reason;
        outcome.detail = detail.to_string();
        outcome.when = self.now();
        self.emit(QueueEvent::Finished(Box::new(outcome)));
        self.emit(QueueEvent::Changed);
        self.wake();
    }

    /// Put a failed attempt back on the clock. The attempt budget is spent ONLY on
    /// `ServerError`: anything the network did leaves the count where it was, so a laptop
    /// off the network for a week comes back with its budget intact.
    fn reschedule(&self, id: &str, result: &TransferResult, moved: bool) {
        let now = self.now();
        let Some(exhausted) = self.with_item(id, |item| {
            if result.reason == QueueReason::ServerError {
                item.server_attempts += 1;
            }
            let exhausted = item.server_attempts > MAX_SERVER_ATTEMPTS;
            if !exhausted {
                if moved {
                    // Progress is evidence about the link, and it outweighs the failure
                    // that came after it.
                    item.backoff_seconds = 0;
                }
                let wait = match result.retry_after {
                    // The server named a time; second-guessing it earns a longer one.
                    Some(after) => (after.as_secs_f64() as i64).clamp(1, 3600),
                    None => {
                        item.backoff_seconds = next_backoff(item.backoff_seconds);
                        jitter(item.backoff_seconds) as i64
                    }
                };
                item.state = QueueState::Waiting;
                item.reason = result.reason;
                item.detail = result.detail.clone();
                item.next_attempt_utc = now + chrono::Duration::seconds(wait);
            }
            item.progress = None;
            exhausted
        }) else {
            return;
        };
        if exhausted {
            self.block(id, QueueReason::GaveUp, &MAX_SERVER_ATTEMPTS.to_string());
            return;
        }
        self.persist();
        self.emit(QueueEvent::Changed);
    }

    /// Drop everything that has been waiting past its deadline: a row in the history
    /// saying it was given up on, never a silent deletion.
    fn expire(&self) {
        let now = self.now();
        let overdue: Vec<String> = self
            .lock()
            .items
            .iter()
            .filter(|i| {
                !crate::dotnet::is_min(&i.deadline_utc)
                    && i.deadline_utc <= now
                    && i.state != QueueState::Sending
            })
            .map(|i| i.id.clone())
            .collect();
        for id in overdue {
            self.fail(&id, QueueReason::TooOld, "");
        }
    }

    /// Let go of everything the item was holding on to. Abandoning the server's session
    /// is best effort and off this task: the server reclaims sessions within a day.
    fn release(&self, item: &mut QueueItem, abandon_session: bool) {
        item.progress = None;
        self.shared.store.discard_spool(item);
        if !abandon_session {
            return;
        }
        let Some(url) = item.tus_url.take() else {
            return;
        };
        let Some(account) = self.shared.account.read().unwrap().clone() else {
            return;
        };
        let transport = self.shared.transport.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { transport.abandon(account, url).await });
        }
    }
}

/// A file's last write time at .NET's resolution.
fn written_utc(meta: &std::fs::Metadata) -> DateTime<Utc> {
    meta.modified()
        .map(|t| crate::dotnet::to_ticks_precision(DateTime::<Utc>::from(t)))
        .unwrap_or_else(|_| crate::dotnet::min_value())
}

/// The last write time of `path`, as the queue records it.
pub fn file_written_utc(path: &Path) -> Option<DateTime<Utc>> {
    std::fs::metadata(path).ok().map(|m| written_utc(&m))
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;

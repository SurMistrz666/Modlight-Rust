//! Watch folders: a folder whose new files are handed to a task as they arrive
//! (`ShareX/WatchFolder.cs`, `WatchFolderSettings.cs`, and the wait for a finished file
//! made of `Helpers.WaitWhileAsync`, `FileHelpers.IsFileLocked` and `GetFileSize`).
//!
//! A [`WatchFolder`] reports every file that is created in its folder, or moved into it,
//! once, when the file has stopped changing: upstream's `FileSystemWatcher.Created` and
//! the wait its handler runs. What happens next (moving the file to the screenshots
//! folder, the upload) belongs to the task flow.
//!
//! Paths are delivered to a callback, on the watch folder's own thread, one at a time,
//! in the order the files became ready. The callback should pass the path on (a channel,
//! the task queue) and return. No callback starts once the [`WatchFolder`] is being
//! dropped, and the drop waits for one already running, so the callback must never wait
//! on the thread that drops the watcher.
//!
//! The events come from FSEvents (through `notify`), which says less than
//! FileSystemWatcher and repeats itself; this module makes up the difference:
//!
//! * FSEvents marks every event of a young file "created", for about half a minute after
//!   its creation, even when that was before the watch began, and the new name of a
//!   rename "renamed" on its later changes. A file is therefore known by its identity
//!   (device, inode, birth time): a repeated flag on a file already handled at that path
//!   is not news. Files that changed shortly before the watch began are known from the
//!   start.
//! * FSEvents does not say which side of a rename an event is, nor whether the other
//!   side is in the folder. The side that no longer exists is the old name; when the new
//!   name follows at once, the file was renamed inside the watched tree. Windows reports
//!   a rename within one folder as `Renamed`, which upstream does not listen to, and a
//!   move from one folder of the tree to another as `Deleted` and `Created`; a new name
//!   with no old name was moved in from outside, which is `Created` too.
//! * "Locked" on Windows means another process opened the file without sharing it,
//!   which macOS has no equivalent of. The closest honest check is that the file can be
//!   opened for reading. What really waits for a writer to finish, upstream and here, is
//!   the size test: bigger than zero and unchanged since the previous check, four times.

use std::collections::HashMap;
use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

/// `WatchFolderDuplicateEventTimer.expireTime`.
const DUPLICATE_EVENT_EXPIRY: Duration = Duration::from_millis(1000);

/// The arguments upstream passes to `Helpers.WaitWhileAsync`: a second before the first
/// check, a check every quarter of a second, and five seconds of checks at most.
const WAIT_START: Duration = Duration::from_millis(1000);
const WAIT_INTERVAL: Duration = Duration::from_millis(250);
const WAIT_TIMEOUT: Duration = Duration::from_millis(5000);

/// Checks that must find the size unchanged (`successCount < 4`).
const UNCHANGED_CHECKS: u32 = 4;

/// The two halves of one rename arrive in the same FSEvents callback, microseconds apart;
/// a move out followed by an unrelated move in comes much later than this.
const RENAME_PAIR_WINDOW: Duration = Duration::from_millis(50);

/// How long a handled file is remembered. FSEvents repeats a file's flags for about 30 s,
/// so this is far more than needed, and it keeps the memory of a busy folder bounded.
const REMEMBER_FOR: Duration = Duration::from_secs(600);

/// The fewest remembered files worth sweeping for old ones.
const REMEMBER_SWEEP_MIN: usize = 256;

/// One watch folder in the task settings (`WatchFolderSettings`), with upstream's member
/// names. The strings are null in JSON when upstream never set them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchFolderSettings {
    /// The folder, with `%variables%` not yet expanded.
    #[serde(rename = "FolderPath")]
    pub folder_path: Option<String>,
    /// FileSystemWatcher's `Filter`, see [`matches_filter`].
    #[serde(rename = "Filter")]
    pub filter: Option<String>,
    #[serde(rename = "IncludeSubdirectories")]
    pub include_subdirectories: bool,
    /// Read by the task flow, which moves a reported file there before uploading it.
    #[serde(rename = "MoveFilesToScreenshotsFolder")]
    pub move_files_to_screenshots_folder: bool,
}

/// FileSystemWatcher's `Filter` test for one file name: `*` is any run of characters,
/// `?` exactly one, anything else itself, ignoring case as on Windows
/// (`FileSystemName.MatchesSimpleExpression`, one UTF-16 unit at a time). An empty filter
/// and `*.*` match every name, names without a dot included, because the watcher's
/// filter collection turns both into `*`. Only the name is matched, never the folders
/// above it, and `;` or `|` separate nothing.
pub fn matches_filter(filter: &str, file_name: &str) -> bool {
    if file_name.is_empty() {
        return false;
    }
    let filter = if filter.is_empty() || filter == "*.*" {
        "*"
    } else {
        filter
    };
    let pattern: Vec<u16> = filter.encode_utf16().map(fold_case).collect();
    let name: Vec<u16> = file_name.encode_utf16().map(fold_case).collect();
    wildcard_match(&pattern, &name)
}

/// OrdinalIgnoreCase's view of one UTF-16 unit: its simple uppercase form. Letters whose
/// uppercase is longer than one unit (ß) and surrogate halves stay as they are.
fn fold_case(unit: u16) -> u16 {
    let Some(c) = char::from_u32(u32::from(unit)) else {
        return unit;
    };
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(u), None) => u16::try_from(u32::from(u)).unwrap_or(unit),
        _ => unit,
    }
}

/// `*` and `?` matching with backtracking to the last `*`, which is linear enough for
/// file names.
fn wildcard_match(pattern: &[u16], name: &[u16]) -> bool {
    const STAR: u16 = b'*' as u16;
    const ONE: u16 = b'?' as u16;
    let (mut p, mut n) = (0, 0);
    // After the last star seen: where the pattern resumes, and how much of the name the
    // star has taken so far.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some(&STAR) => {
                star = Some((p + 1, n));
                p += 1;
            }
            Some(&unit) if unit == ONE || unit == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((resume, taken)) => {
                    star = Some((resume, taken + 1));
                    p = resume;
                    n = taken + 1;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&unit| unit == STAR)
}

/// Why a folder could not be watched. Upstream's `Enable()` silently watches nothing in
/// the first case; the caller decides whether to say so.
#[derive(Debug)]
pub enum WatchError {
    /// The path is empty, missing, or not a folder.
    FolderNotFound(PathBuf),
    /// FSEvents refused the folder.
    Watch(notify::Error),
    /// The watcher's thread could not be started.
    Thread(std::io::Error),
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchError::FolderNotFound(path) => {
                write!(f, "the folder does not exist: {}", path.display())
            }
            WatchError::Watch(error) => write!(f, "the folder cannot be watched: {error}"),
            WatchError::Thread(error) => write!(f, "the folder watcher cannot start: {error}"),
        }
    }
}

impl std::error::Error for WatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WatchError::FolderNotFound(_) => None,
            WatchError::Watch(error) => Some(error),
            WatchError::Thread(error) => Some(error),
        }
    }
}

/// What the worker thread is told.
enum Message {
    /// An FSEvents event and when it reached us.
    Event(Instant, notify::Result<notify::Event>),
    Stop,
}

/// One folder being watched (`WatchFolder`). Dropping it stops the watch.
#[derive(Debug)]
pub struct WatchFolder {
    folder: PathBuf,
    watcher: Option<notify::RecommendedWatcher>,
    control: Sender<Message>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl WatchFolder {
    /// `Enable()`: watch `folder`, which is the settings' `FolderPath` with its
    /// `%variables%` already expanded, with the settings' filter and subdirectory choice.
    /// Each new file that settles is passed to `on_file` (see the module documentation
    /// for the thread and the order), as a path under `folder` as given, even when that
    /// path goes through a symbolic link.
    pub fn start<F>(
        folder: &Path,
        settings: &WatchFolderSettings,
        on_file: F,
    ) -> Result<WatchFolder, WatchError>
    where
        F: FnMut(PathBuf) + Send + 'static,
    {
        if folder.as_os_str().is_empty() || !folder.is_dir() {
            return Err(WatchError::FolderNotFound(folder.to_path_buf()));
        }
        // FSEvents names files by their real path (/private/var/... for /var/...).
        let real_folder = folder
            .canonicalize()
            .map_err(|_| WatchError::FolderNotFound(folder.to_path_buf()))?;
        // Taken before the stream starts, so that what changes later is never taken for
        // a file that was already there.
        let started = SystemTime::now();
        let tracker = Tracker::new(folder.to_path_buf(), real_folder.clone(), settings);

        let (control, receiver) = mpsc::channel();
        let events = control.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let _ = events.send(Message::Event(Instant::now(), event));
            })
            .map_err(WatchError::Watch)?;
        let mode = if settings.include_subdirectories {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher
            .watch(&real_folder, mode)
            .map_err(WatchError::Watch)?;

        let stop = Arc::new(AtomicBool::new(false));
        let worker = thread::Builder::new()
            .name("modlight-watch-folder".to_string())
            .spawn({
                let stop = stop.clone();
                move || run(receiver, tracker, started, &stop, on_file)
            })
            .map_err(WatchError::Thread)?;
        Ok(WatchFolder {
            folder: folder.to_path_buf(),
            watcher: Some(watcher),
            control,
            stop,
            worker: Some(worker),
        })
    }

    /// The folder as it was given to [`WatchFolder::start`].
    pub fn folder(&self) -> &Path {
        &self.folder
    }
}

impl Drop for WatchFolder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Stops the FSEvents stream and waits for its thread.
        drop(self.watcher.take());
        let _ = self.control.send(Message::Stop);
        if let Some(worker) = self.worker.take() {
            // The callback itself may drop the watcher; that thread cannot wait for itself
            // and ends on its own once the callback returns.
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

/// The worker: events in, ready files out, the waits stepped in between.
fn run(
    receiver: Receiver<Message>,
    mut tracker: Tracker,
    started: SystemTime,
    stop: &AtomicBool,
    mut on_file: impl FnMut(PathBuf),
) {
    tracker.remember_existing(started, Instant::now());
    while !stop.load(Ordering::SeqCst) {
        let message = match tracker.next_due() {
            Some(due) => receiver.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match message {
            Ok(Message::Event(at, Ok(event))) => {
                let change = Change::of(&event.kind);
                for path in &event.paths {
                    tracker.observe(at, change, path, &Disk);
                }
            }
            Ok(Message::Event(_, Err(error))) => log::debug!("watch folder: {error}"),
            Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        for path in tracker.poll(Instant::now(), &Disk) {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            on_file(path);
        }
    }
}

/// What an FSEvents event says about its path, as far as watching for new files goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Created,
    Removed,
    Renamed,
    Other,
}

impl Change {
    fn of(kind: &EventKind) -> Change {
        match kind {
            EventKind::Create(_) => Change::Created,
            EventKind::Remove(_) => Change::Removed,
            EventKind::Modify(ModifyKind::Name(_)) => Change::Renamed,
            _ => Change::Other,
        }
    }
}

/// Which file is at a path: a new file at the same path is another identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileId {
    device: u64,
    inode: u64,
    /// Volumes that make up inode numbers (FAT, exFAT) can give a new file the old one's.
    born: Option<SystemTime>,
}

fn file_id(metadata: &std::fs::Metadata) -> FileId {
    FileId {
        device: metadata.dev(),
        inode: metadata.ino(),
        born: metadata.created().ok(),
    }
}

/// What is at a path, without following a symbolic link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Missing,
    File(FileId),
    /// A folder, a symbolic link, a device...: never reported. Upstream would not either:
    /// a folder fails its file checks, and a link's own size on Windows is 0.
    Other,
}

/// The file system as the waits see it; a trait so the timing can be tested without one.
trait Probe {
    fn entry(&self, path: &Path) -> Entry;
    /// `!FileHelpers.IsFileLocked(path)`: here, whether the file opens for reading.
    fn can_read(&self, path: &Path) -> bool;
    /// `FileHelpers.GetFileSize(path)`: -1 when there is no file to measure.
    fn size(&self, path: &Path) -> i64;
}

struct Disk;

impl Probe for Disk {
    fn entry(&self, path: &Path) -> Entry {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => Entry::File(file_id(&metadata)),
            Ok(_) => Entry::Other,
            Err(_) => Entry::Missing,
        }
    }

    fn can_read(&self, path: &Path) -> bool {
        std::fs::File::open(path).is_ok()
    }

    fn size(&self, path: &Path) -> i64 {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            _ => -1,
        }
    }
}

/// `WatchFolderDuplicateEventTimer`: an event for the same path within a second of the
/// last one is the same event, and it starts the second again.
struct DuplicateEventTimer {
    path: PathBuf,
    since: Instant,
}

impl DuplicateEventTimer {
    fn is_elapsed(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= DUPLICATE_EVENT_EXPIRY
    }

    fn is_duplicate_event(&mut self, path: &Path, now: Instant) -> bool {
        let duplicate = path == self.path && !self.is_elapsed(now);
        if duplicate {
            self.since = now;
        }
        duplicate
    }
}

/// Where one file's wait stands after a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Waiting,
    Ready,
    GaveUp,
}

/// One file waiting to settle: upstream's `WaitWhileAsync` call, kept as state the
/// worker steps, rather than a sleeping thread per file.
struct FileWait {
    due: Instant,
    /// When the checks began (`WaitWhile`'s stopwatch), after the first second.
    checks_since: Option<Instant>,
    /// `successCount`: checks that found the size unchanged. Upstream never resets it,
    /// so the four need not be in a row.
    unchanged: u32,
    previous_size: i64,
}

impl FileWait {
    fn new(at: Instant) -> FileWait {
        FileWait {
            due: at + WAIT_START,
            checks_since: None,
            unchanged: 0,
            previous_size: -1,
        }
    }

    /// One turn of `Helpers.WaitWhile`: the check, then the timeout, then the next check
    /// a quarter of a second later. A file that settles at the last check is ready.
    fn check(&mut self, now: Instant, path: &Path, probe: &impl Probe) -> Step {
        let since = *self.checks_since.get_or_insert(now);
        let waiting = if probe.can_read(path) {
            let size = probe.size(path);
            if size > 0 && size == self.previous_size {
                self.unchanged += 1;
            }
            self.previous_size = size;
            self.unchanged < UNCHANGED_CHECKS
        } else {
            self.previous_size = -1;
            true
        };
        if !waiting {
            Step::Ready
        } else if now.saturating_duration_since(since) >= WAIT_TIMEOUT {
            Step::GaveUp
        } else {
            self.due = now + WAIT_INTERVAL;
            Step::Waiting
        }
    }
}

/// Everything the worker knows: upstream's duplicate timers and pending waits, and what
/// it takes to read FSEvents like FileSystemWatcher.
struct Tracker {
    /// The folder as given: reported paths start with it.
    folder: PathBuf,
    /// The folder as FSEvents names it.
    real_folder: PathBuf,
    filter: String,
    include_subdirectories: bool,
    timers: Vec<DuplicateEventTimer>,
    waits: HashMap<PathBuf, FileWait>,
    /// The files handled at each path, and when; see the module documentation.
    known: HashMap<PathBuf, (FileId, Instant)>,
    sweep_at: usize,
    /// The old name of a rename whose new name may come next, and when it came.
    rename_source: Option<(PathBuf, Instant)>,
}

impl Tracker {
    fn new(folder: PathBuf, real_folder: PathBuf, settings: &WatchFolderSettings) -> Tracker {
        Tracker {
            folder,
            real_folder,
            filter: settings.filter.clone().unwrap_or_default(),
            include_subdirectories: settings.include_subdirectories,
            timers: Vec::new(),
            waits: HashMap::new(),
            known: HashMap::new(),
            sweep_at: REMEMBER_SWEEP_MIN,
            rename_source: None,
        }
    }

    /// An event path as the path to report (`e.FullPath`), when it is inside the watch.
    fn full_path(&self, event_path: &Path) -> Option<PathBuf> {
        let relative = event_path
            .strip_prefix(&self.real_folder)
            .or_else(|_| event_path.strip_prefix(&self.folder))
            .ok()?;
        let depth = relative.components().count();
        if depth == 0 || (!self.include_subdirectories && depth > 1) {
            return None;
        }
        Some(self.folder.join(relative))
    }

    fn matches(&self, path: &Path) -> bool {
        path.file_name()
            .is_some_and(|name| matches_filter(&self.filter, &name.to_string_lossy()))
    }

    fn next_due(&self) -> Option<Instant> {
        self.waits.values().map(|wait| wait.due).min()
    }

    fn observe(&mut self, at: Instant, change: Change, event_path: &Path, probe: &impl Probe) {
        let Some(path) = self.full_path(event_path) else {
            return;
        };
        // The two halves of a rename are neighbours; an event about a third path in
        // between means they are not a pair. (FSEvents sends the other flags of the old
        // name right after its "renamed", under the same path.)
        if change != Change::Renamed
            && self
                .rename_source
                .as_ref()
                .is_some_and(|(source, _)| *source != path)
        {
            self.rename_source = None;
        }
        match change {
            Change::Created => {
                let entry = probe.entry(&path);
                if entry != Entry::Other {
                    self.created(at, path, entry);
                }
            }
            Change::Renamed => self.renamed(at, path, probe),
            Change::Removed => {
                if probe.entry(&path) == Entry::Missing {
                    self.known.remove(&path);
                }
            }
            Change::Other => {}
        }
    }

    fn renamed(&mut self, at: Instant, path: PathBuf, probe: &impl Probe) {
        let entry = probe.entry(&path);
        if entry == Entry::Missing {
            // The old name of a rename, or a file moved out of the folder.
            self.known.remove(&path);
            self.rename_source = Some((path, at));
            return;
        }
        let source = self.rename_source.take().filter(|(source, since)| {
            *source != path && at.saturating_duration_since(*since) <= RENAME_PAIR_WINDOW
        });
        match source {
            // Renamed within one folder: FileSystemWatcher raises Renamed, which upstream
            // does not listen to. Remembered, so its repeated flags are not news later.
            Some((source, _)) if source.parent() == path.parent() => {
                if let Entry::File(id) = entry
                    && self.matches(&path)
                {
                    self.remember(path, id, at);
                }
            }
            // Moved in from outside, or from another folder of the watched tree.
            _ => {
                if entry != Entry::Other {
                    self.created(at, path, entry);
                }
            }
        }
    }

    /// `fileWatcher_Created`, for a path FileSystemWatcher raises `Created` for.
    fn created(&mut self, at: Instant, path: PathBuf, entry: Entry) {
        if !self.matches(&path) {
            return;
        }
        self.timers.retain(|timer| !timer.is_elapsed(at));
        if self
            .timers
            .iter_mut()
            .any(|timer| timer.is_duplicate_event(&path, at))
        {
            return;
        }
        self.timers.push(DuplicateEventTimer {
            path: path.clone(),
            since: at,
        });
        // Not upstream: FSEvents repeats "created" on later events of the same file. One
        // wait per path at a time, and a file already handled is not new.
        if self.waits.contains_key(&path) {
            return;
        }
        if let Entry::File(id) = entry
            && self.known.get(&path).is_some_and(|(known, _)| *known == id)
        {
            return;
        }
        // A file that is already gone still gets its wait, as upstream gives it one: a
        // file that comes back at that path in time is reported.
        self.waits.insert(path, FileWait::new(at));
    }

    /// Step the waits that are due; returns the files that are ready, in path order.
    fn poll(&mut self, now: Instant, probe: &impl Probe) -> Vec<PathBuf> {
        let mut due: Vec<PathBuf> = self
            .waits
            .iter()
            .filter(|(_, wait)| wait.due <= now)
            .map(|(path, _)| path.clone())
            .collect();
        due.sort();
        let mut ready = Vec::new();
        for path in due {
            let Some(wait) = self.waits.get_mut(&path) else {
                continue;
            };
            let step = wait.check(now, &path, probe);
            if step == Step::Waiting {
                continue;
            }
            self.waits.remove(&path);
            match probe.entry(&path) {
                Entry::File(id) => self.remember(path.clone(), id, now),
                _ => {
                    self.known.remove(&path);
                }
            }
            if step == Step::Ready {
                ready.push(path);
            } else {
                log::debug!("watch folder: {} did not settle in time", path.display());
            }
        }
        ready
    }

    fn remember(&mut self, path: PathBuf, id: FileId, now: Instant) {
        self.known.insert(path, (id, now));
        if self.known.len() > self.sweep_at {
            self.known
                .retain(|_, (_, since)| now.saturating_duration_since(*since) < REMEMBER_FOR);
            self.sweep_at = (self.known.len() * 2).max(REMEMBER_SWEEP_MIN);
        }
    }

    /// The files already in the watch that changed shortly before it began: FSEvents
    /// would flag them "created" at their next change, and FileSystemWatcher never
    /// reports them. A file whose status changed after the start (one arriving while
    /// this runs) is left out, so it is still news.
    fn remember_existing(&mut self, started: SystemTime, now: Instant) {
        let mut folders = vec![self.folder.clone()];
        while let Some(folder) = folders.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                // A directory entry's metadata does not follow symbolic links.
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let path = entry.path();
                if metadata.is_dir() {
                    if self.include_subdirectories {
                        folders.push(path);
                    }
                    continue;
                }
                if !metadata.is_file() || !self.matches(&path) {
                    continue;
                }
                let changed = u64::try_from(metadata.ctime()).ok().map(|seconds| {
                    UNIX_EPOCH + Duration::new(seconds, metadata.ctime_nsec() as u32)
                });
                let recent = changed
                    .and_then(|changed| started.duration_since(changed).ok())
                    .is_some_and(|age| age < REMEMBER_FOR);
                if recent {
                    self.remember(path, file_id(&metadata), now);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn filters_match_like_file_system_watcher() {
        let table: &[(&str, &str, bool)] = &[
            ("", "anything.png", true),
            ("", "no-extension", true),
            ("*", ".hidden", true),
            ("*.*", "no-extension", true),
            ("*.png", "shot.png", true),
            ("*.png", "SHOT.PNG", true),
            ("*.png", ".png", true),
            ("*.png", "shot.png.txt", false),
            ("*.png", "shotpng", false),
            ("*.PNG", "zdjęcie.png", true),
            ("?.txt", "a.txt", true),
            ("?.txt", "ab.txt", false),
            ("?.txt", ".txt", false),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "abc", true),
            ("a*b*c", "acb", false),
            ("a*b", "abab", true),
            ("*.", "file.", true),
            ("*.", "file", false),
            ("report", "REPORT", true),
            ("report", "report2", false),
            ("Ż*", "żółw.jpg", true),
            ("**.jpg", "x.jpg", true),
            ("*.png;*.jpg", "x.png", false),
            ("*", "", false),
            ("??", "😀", true),
        ];
        for &(filter, name, expected) in table {
            assert_eq!(
                matches_filter(filter, name),
                expected,
                "{filter:?} on {name:?}"
            );
        }
    }

    #[test]
    fn settings_keep_upstream_member_names() {
        let json = r#"{"FolderPath":"%UserProfile%\\Shots","Filter":null,"IncludeSubdirectories":true,"MoveFilesToScreenshotsFolder":false}"#;
        let settings: WatchFolderSettings = serde_json::from_str(json).unwrap();
        assert_eq!(
            settings.folder_path.as_deref(),
            Some("%UserProfile%\\Shots")
        );
        assert_eq!(settings.filter, None);
        assert!(settings.include_subdirectories);
        assert_eq!(serde_json::to_string(&settings).unwrap(), json);
        let empty: WatchFolderSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, WatchFolderSettings::default());
    }

    /// A file system in memory: paths are the reported ones.
    #[derive(Default)]
    struct FakeDisk {
        files: HashMap<PathBuf, FakeFile>,
    }

    #[derive(Clone, Copy)]
    struct FakeFile {
        inode: u64,
        size: i64,
        readable: bool,
        folder: bool,
    }

    impl FakeDisk {
        fn put(&mut self, path: &str, inode: u64, size: i64) {
            let file = FakeFile {
                inode,
                size,
                readable: true,
                folder: false,
            };
            self.files.insert(PathBuf::from(path), file);
        }

        fn file(&mut self, path: &str) -> &mut FakeFile {
            self.files.get_mut(Path::new(path)).unwrap()
        }

        fn rename(&mut self, from: &str, to: &str) {
            let file = self.files.remove(Path::new(from)).unwrap();
            self.files.insert(PathBuf::from(to), file);
        }
    }

    impl Probe for FakeDisk {
        fn entry(&self, path: &Path) -> Entry {
            match self.files.get(path) {
                None => Entry::Missing,
                Some(file) if file.folder => Entry::Other,
                Some(file) => Entry::File(FileId {
                    device: 1,
                    inode: file.inode,
                    born: None,
                }),
            }
        }

        fn can_read(&self, path: &Path) -> bool {
            self.files.get(path).is_some_and(|file| file.readable)
        }

        fn size(&self, path: &Path) -> i64 {
            match self.files.get(path) {
                Some(file) if !file.folder => file.size,
                _ => -1,
            }
        }
    }

    /// A watch of `/w`, which FSEvents knows as `/private/w`.
    fn tracker(filter: &str, include_subdirectories: bool) -> Tracker {
        let settings = WatchFolderSettings {
            filter: Some(filter.to_string()),
            include_subdirectories,
            ..WatchFolderSettings::default()
        };
        Tracker::new(PathBuf::from("/w"), PathBuf::from("/private/w"), &settings)
    }

    /// Drives a tracker on a fake clock: events at given milliseconds, checks whenever a
    /// wait is due, and a hook to change the disk before each check.
    struct Run {
        tracker: Tracker,
        disk: FakeDisk,
        start: Instant,
        reports: Vec<(u64, String)>,
    }

    impl Run {
        fn new(tracker: Tracker) -> Run {
            Run {
                tracker,
                disk: FakeDisk::default(),
                start: Instant::now(),
                reports: Vec::new(),
            }
        }

        fn at(&self, ms: u64) -> Instant {
            self.start + Duration::from_millis(ms)
        }

        /// An FSEvents event for `/private/w/{name}` at `ms`, after the checks due by then.
        fn event(&mut self, ms: u64, change: Change, name: &str) {
            self.until(ms, |_, _| {});
            let path = Path::new("/private/w").join(name);
            let at = self.at(ms);
            self.tracker.observe(at, change, &path, &self.disk);
        }

        /// Run the checks due up to `ms`, calling `change(ms, disk)` before each.
        fn until(&mut self, ms: u64, mut change: impl FnMut(u64, &mut FakeDisk)) {
            while let Some(due) = self.tracker.next_due() {
                let due_ms = due.duration_since(self.start).as_millis() as u64;
                if due_ms > ms {
                    break;
                }
                change(due_ms, &mut self.disk);
                for path in self.tracker.poll(due, &self.disk) {
                    self.reports.push((due_ms, path.display().to_string()));
                }
            }
        }

        fn reports(&self) -> Vec<(u64, &str)> {
            self.reports
                .iter()
                .map(|(ms, path)| (*ms, path.as_str()))
                .collect()
        }
    }

    #[test]
    fn a_settled_file_is_reported_two_seconds_after_it_appears() {
        let mut run = Run::new(tracker("*.png", false));
        run.disk.put("/w/a.png", 1, 10);
        run.event(0, Change::Created, "a.png");
        run.until(10_000, |_, _| {});
        // One second, then the first size, then four unchanged sizes a quarter apart.
        assert_eq!(run.reports(), [(2000, "/w/a.png")]);
        assert!(run.tracker.waits.is_empty());
    }

    #[test]
    fn unchanged_checks_add_up_and_need_a_size() {
        let mut run = Run::new(tracker("", false));
        run.disk.put("/w/a.bin", 1, 0);
        run.event(0, Change::Created, "a.bin");
        // Empty until 1750; 5 bytes for two checks (one unchanged), then 9 bytes.
        run.until(10_000, |ms, disk| {
            disk.file("/w/a.bin").size = match ms {
                0..=1500 => 0,
                1501..=2000 => 5,
                _ => 9,
            };
        });
        // 2000 counts one, 2250 sees a new size, 2500/2750/3000 count three more.
        assert_eq!(run.reports(), [(3000, "/w/a.bin")]);
    }

    #[test]
    fn a_lock_only_forgets_the_previous_size() {
        let mut run = Run::new(tracker("", false));
        run.disk.put("/w/a.bin", 1, 10);
        run.event(0, Change::Created, "a.bin");
        // Two unchanged checks (1250, 1500), unreadable at 1750, readable again.
        run.until(10_000, |ms, disk| {
            disk.file("/w/a.bin").readable = ms != 1750
        });
        // 2000 reads the size afresh, 2250 and 2500 make the count four.
        assert_eq!(run.reports(), [(2500, "/w/a.bin")]);

        let mut run = Run::new(tracker("", false));
        run.disk.put("/w/b.bin", 1, 10);
        run.event(0, Change::Created, "b.bin");
        run.until(10_000, |ms, disk| {
            disk.file("/w/b.bin").readable = ms >= 3000
        });
        assert_eq!(run.reports(), [(4000, "/w/b.bin")]);
    }

    #[test]
    fn a_file_that_never_settles_is_dropped_after_the_timeout() {
        let mut run = Run::new(tracker("", false));
        run.disk.put("/w/grows.bin", 1, 1);
        run.event(0, Change::Created, "grows.bin");
        run.until(20_000, |ms, disk| {
            disk.file("/w/grows.bin").size = ms as i64
        });
        assert!(run.reports().is_empty());
        assert!(run.tracker.waits.is_empty());
        // Its later "created" flags are not a new file.
        run.event(20_000, Change::Created, "grows.bin");
        assert!(run.tracker.waits.is_empty());

        // Settling exactly at the last check (five seconds into the checks) still counts.
        let mut run = Run::new(tracker("", false));
        run.disk.put("/w/late.bin", 1, 1);
        run.event(0, Change::Created, "late.bin");
        run.until(20_000, |ms, disk| {
            disk.file("/w/late.bin").size = ms.min(5000) as i64
        });
        assert_eq!(run.reports(), [(6000, "/w/late.bin")]);

        let mut run = Run::new(tracker("", false));
        run.disk.put("/w/later.bin", 1, 1);
        run.event(0, Change::Created, "later.bin");
        run.until(20_000, |ms, disk| {
            disk.file("/w/later.bin").size = ms.min(5250) as i64
        });
        assert!(run.reports().is_empty());
    }

    #[test]
    fn repeated_created_flags_are_one_file() {
        let mut run = Run::new(tracker("*.png", false));
        run.disk.put("/w/a.png", 1, 10);
        // FSEvents flags every write of a young file "created".
        for ms in [0, 600, 1500, 2700, 4000] {
            run.event(ms, Change::Created, "a.png");
        }
        run.until(10_000, |_, _| {});
        assert_eq!(run.reports(), [(2000, "/w/a.png")]);

        // A new file at the same path is news, once the duplicate timer has run out.
        run.disk.put("/w/a.png", 2, 20);
        run.event(10_000, Change::Created, "a.png");
        run.until(20_000, |_, _| {});
        assert_eq!(run.reports(), [(2000, "/w/a.png"), (12_000, "/w/a.png")]);

        // Within a second of the last event for that path, upstream's timer swallows it.
        run.disk.put("/w/a.png", 3, 30);
        run.event(20_000, Change::Created, "a.png");
        run.disk.put("/w/a.png", 4, 40);
        run.event(20_500, Change::Created, "a.png");
        run.until(30_000, |_, _| {});
        assert_eq!(run.reports().len(), 3);
    }

    #[test]
    fn a_file_gone_before_its_wait_ends_is_not_reported_unless_it_returns() {
        let mut run = Run::new(tracker("", false));
        run.event(0, Change::Created, "temp.bin");
        run.until(10_000, |_, _| {});
        assert!(run.reports().is_empty());

        run.event(20_000, Change::Created, "back.bin");
        run.until(30_000, |ms, disk| {
            if ms >= 23_000 && !disk.files.contains_key(Path::new("/w/back.bin")) {
                disk.put("/w/back.bin", 7, 5);
            }
        });
        assert_eq!(run.reports(), [(24_000, "/w/back.bin")]);
    }

    #[test]
    fn renames_inside_a_folder_are_not_new_files_but_moves_in_are() {
        let mut run = Run::new(tracker("*.png", true));
        run.disk.put("/w/a.png", 1, 10);
        run.event(0, Change::Created, "a.png");
        run.until(5000, |_, _| {});
        assert_eq!(run.reports(), [(2000, "/w/a.png")]);

        // a.png -> b.png in the same folder: the old name (gone, with its old flags), then
        // the new one; later changes of b.png repeat "renamed".
        run.disk.rename("/w/a.png", "/w/b.png");
        run.event(5000, Change::Created, "a.png");
        run.event(5000, Change::Renamed, "a.png");
        run.event(5000, Change::Other, "a.png");
        run.event(5001, Change::Renamed, "b.png");
        run.event(7000, Change::Renamed, "b.png");

        // An editor's safe save: a temporary name renamed over the file.
        run.disk.put("/w/b.png", 2, 12);
        run.event(9000, Change::Created, ".b.png.tmp");
        run.event(9000, Change::Renamed, ".b.png.tmp");
        run.event(9000, Change::Renamed, "b.png");

        // Moved in from outside: a new name with no old name before it.
        run.disk.put("/w/c.png", 3, 10);
        run.event(11_000, Change::Renamed, "c.png");

        // Moved between two folders of the tree: Deleted + Created on Windows.
        run.disk.put("/w/two/d.png", 4, 10);
        run.event(11_000, Change::Renamed, "one/d.png");
        run.event(11_000, Change::Renamed, "two/d.png");

        // A move out, and a move in long after it, are not one rename.
        run.disk.put("/w/f.png", 5, 10);
        run.event(11_000, Change::Renamed, "e.png");
        run.event(11_500, Change::Renamed, "f.png");

        run.until(30_000, |_, _| {});
        assert_eq!(
            run.reports(),
            [
                (2000, "/w/a.png"),
                (13_000, "/w/c.png"),
                (13_000, "/w/two/d.png"),
                (13_500, "/w/f.png"),
            ]
        );
    }

    #[test]
    fn only_matching_files_inside_the_watch_are_reported() {
        let mut run = Run::new(tracker("*.png", false));
        run.disk.put("/w/A.PNG", 1, 10);
        run.disk.put("/w/note.txt", 2, 10);
        run.disk.put("/w/sub/deep.png", 3, 10);
        run.disk.files.insert(
            PathBuf::from("/w/folder.png"),
            FakeFile {
                inode: 4,
                size: 0,
                readable: true,
                folder: true,
            },
        );
        run.event(0, Change::Created, "A.PNG");
        run.event(0, Change::Created, "note.txt");
        run.event(0, Change::Created, "sub/deep.png");
        run.event(0, Change::Created, "folder.png");
        run.event(0, Change::Created, "");
        // An event named by the folder as given, not as FSEvents names it.
        run.disk.put("/w/given.png", 5, 10);
        let at = run.at(0);
        run.tracker
            .observe(at, Change::Created, Path::new("/w/given.png"), &run.disk);
        run.until(10_000, |_, _| {});
        assert_eq!(run.reports(), [(2000, "/w/A.PNG"), (2000, "/w/given.png")]);
    }

    fn watch(
        folder: &Path,
        filter: &str,
        subdirectories: bool,
    ) -> (WatchFolder, Receiver<(PathBuf, u64)>) {
        let settings = WatchFolderSettings {
            folder_path: Some(folder.display().to_string()),
            filter: Some(filter.to_string()),
            include_subdirectories: subdirectories,
            move_files_to_screenshots_folder: false,
        };
        let (sender, receiver) = mpsc::channel();
        let watch = WatchFolder::start(folder, &settings, move |path| {
            let size = std::fs::metadata(&path).map_or(0, |m| m.len());
            let _ = sender.send((path, size));
        })
        .unwrap();
        // Let the FSEvents stream and the first look at the folder settle.
        thread::sleep(Duration::from_millis(300));
        (watch, receiver)
    }

    /// A settled file is reported two seconds after it appears, give or take the load.
    const QUIET: Duration = Duration::from_millis(2500);

    /// `count` reports (or whatever came within a generous deadline), and then anything
    /// else that arrives in a `quiet` spell after them: a file that should not have been
    /// reported would have been by then.
    fn reports(
        receiver: &Receiver<(PathBuf, u64)>,
        count: usize,
        quiet: Duration,
    ) -> Vec<(PathBuf, u64)> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut reports = Vec::new();
        while reports.len() < count {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(report) => reports.push(report),
                Err(_) => break,
            }
        }
        while let Ok(report) = receiver.recv_timeout(quiet) {
            reports.push(report);
        }
        reports.sort();
        reports
    }

    #[test]
    fn fsevents_new_and_moved_in_files_are_reported_once() {
        let base = tempfile::tempdir().unwrap();
        let folder = base.path().join("watch");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // Made just before the watch: FSEvents still calls its next change "created".
        std::fs::write(folder.join("old.png"), b"old").unwrap();
        let (watch, receiver) = watch(&folder, "*.png", false);

        std::fs::write(folder.join("created.png"), [1u8; 100]).unwrap();
        std::fs::write(outside.join("Moved.PNG"), [2u8; 50]).unwrap();
        std::fs::rename(outside.join("Moved.PNG"), folder.join("Moved.PNG")).unwrap();
        std::fs::write(folder.join("note.txt"), b"not a png").unwrap();
        std::fs::write(folder.join("sub").join("deep.png"), b"too deep").unwrap();
        let mut old = std::fs::OpenOptions::new()
            .append(true)
            .open(folder.join("old.png"))
            .unwrap();
        old.write_all(b" and changed").unwrap();
        drop(old);
        let mut chunked = std::fs::File::create(folder.join("chunked.png")).unwrap();
        for chunk in 0..3u8 {
            chunked.write_all(&[chunk; 1000]).unwrap();
            chunked.flush().unwrap();
            thread::sleep(Duration::from_millis(150));
        }
        drop(chunked);

        let reports = reports(&receiver, 3, QUIET);
        drop(watch);
        assert_eq!(
            reports,
            [
                (folder.join("Moved.PNG"), 50),
                (folder.join("chunked.png"), 3000),
                (folder.join("created.png"), 100),
            ]
        );
    }

    #[test]
    fn fsevents_subdirectories_count_and_renames_within_a_folder_do_not() {
        let base = tempfile::tempdir().unwrap();
        let folder = base.path().join("watch");
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::create_dir_all(folder.join("sub2")).unwrap();
        let (watch, receiver) = watch(&folder, "", true);

        std::fs::write(folder.join("sub").join("deep.png"), b"deep").unwrap();
        std::fs::write(folder.join("plain"), b"no extension").unwrap();
        std::fs::create_dir(folder.join("new folder")).unwrap();
        // No quiet spell here: the one after the renames also catches a stray report.
        let first = reports(&receiver, 2, Duration::ZERO);
        assert_eq!(
            first,
            [
                (folder.join("plain"), 12),
                (folder.join("sub").join("deep.png"), 4),
            ]
        );

        std::fs::rename(folder.join("plain"), folder.join("renamed")).unwrap();
        std::fs::rename(
            folder.join("sub").join("deep.png"),
            folder.join("sub2").join("deep.png"),
        )
        .unwrap();
        let second = reports(&receiver, 1, QUIET);
        drop(watch);
        assert_eq!(second, [(folder.join("sub2").join("deep.png"), 4)]);
    }

    #[test]
    fn dropping_the_watch_folder_stops_it() {
        let base = tempfile::tempdir().unwrap();
        let (watch, receiver) = watch(base.path(), "", false);
        std::fs::write(base.path().join("late.png"), b"late").unwrap();
        thread::sleep(Duration::from_millis(200));
        drop(watch);
        assert!(receiver.recv_timeout(QUIET).is_err());

        let missing = base.path().join("missing");
        let error = WatchFolder::start(&missing, &WatchFolderSettings::default(), |_| {});
        assert!(matches!(error, Err(WatchError::FolderNotFound(path)) if path == missing));
        let error = WatchFolder::start(Path::new(""), &WatchFolderSettings::default(), |_| {});
        assert!(matches!(error, Err(WatchError::FolderNotFound(_))));
    }
}

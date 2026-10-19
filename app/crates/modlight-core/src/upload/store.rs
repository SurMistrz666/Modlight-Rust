//! Where the queue lives between runs (`ModlightQueueStore`): `Queue.json` in the
//! personal folder, and `Queue/` beside it for content that exists nowhere else.
//!
//! A file of its own rather than a member of `ApplicationConfig.json`: the settings are
//! written on a schedule, the queue has to be right after a power cut. It is written the
//! instant anything changes, whole, to a temporary file that is then moved over the old
//! one, so a process killed halfway leaves the previous queue, never a truncated one.
//! Every failure to read answers "nothing queued": a damaged bookkeeping file must not be
//! able to take the app down with it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::upload::item::QueueItem;

pub const QUEUE_FILE_NAME: &str = "Queue.json";

/// Where content that exists nowhere else is parked.
pub const SPOOL_FOLDER_NAME: &str = "Queue";

/// Bumped only when the shape changes in a way an older build cannot read. An unknown
/// version reads as an empty queue: a downgrade must not wedge the app.
pub const CURRENT_VERSION: i32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct QueueFile {
    #[serde(default)]
    version: i32,
    #[serde(default)]
    items: Option<Vec<Option<QueueItem>>>,
}

#[derive(Debug)]
pub struct QueueStore {
    /// `None` in sandbox mode, where nothing is written to disk at all.
    folder: Option<PathBuf>,
    lock: Mutex<()>,
}

impl QueueStore {
    /// A store in `personal_folder` (the profile).
    pub fn new(personal_folder: &Path) -> QueueStore {
        QueueStore {
            folder: Some(personal_folder.to_path_buf()),
            lock: Mutex::new(()),
        }
    }

    /// A store that keeps nothing (`Program.Sandbox`).
    pub fn sandbox() -> QueueStore {
        QueueStore {
            folder: None,
            lock: Mutex::new(()),
        }
    }

    pub fn queue_file_path(&self) -> Option<PathBuf> {
        self.folder.as_ref().map(|f| f.join(QUEUE_FILE_NAME))
    }

    pub fn spool_folder(&self) -> Option<PathBuf> {
        self.folder.as_ref().map(|f| f.join(SPOOL_FOLDER_NAME))
    }

    /// Read the queue back, revived for a fresh process.
    pub fn load(&self, now: DateTime<Utc>) -> Vec<QueueItem> {
        let Some(path) = self.queue_file_path() else {
            return Vec::new();
        };
        if !path.is_file() {
            return Vec::new();
        }
        let text = {
            let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    log::warn!(
                        "Modlight queue could not be read; starting with an empty one: {error}"
                    );
                    return Vec::new();
                }
            }
        };
        // Newtonsoft writes a byte order mark only when asked; tolerate one anyway.
        let text = text.trim_start_matches('\u{feff}');
        let file: QueueFile = match serde_json::from_str(text) {
            Ok(file) => file,
            Err(error) => {
                log::warn!("Modlight queue could not be read; starting with an empty one: {error}");
                return Vec::new();
            }
        };
        if file.version != CURRENT_VERSION {
            return Vec::new();
        }
        let mut items: Vec<QueueItem> = file
            .items
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .filter(QueueItem::is_usable)
            .collect();
        for item in &mut items {
            item.revive(now);
        }
        items
    }

    /// Write the queue out, whole. A failure is logged and otherwise ignored: the queue
    /// is still right in memory and is written again on the next change.
    pub fn save(&self, items: &[QueueItem]) {
        let Some(path) = self.queue_file_path() else {
            return;
        };
        let file = QueueFile {
            version: CURRENT_VERSION,
            items: Some(items.iter().cloned().map(Some).collect()),
        };
        let text = match serde_json::to_string_pretty(&file) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("Modlight queue could not be written: {error}");
                return;
            }
        };
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let result = (|| -> std::io::Result<()> {
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder)?;
            }
            let temporary = path.with_file_name(format!("{QUEUE_FILE_NAME}.tmp"));
            std::fs::write(&temporary, text.as_bytes())?;
            // Move over the top rather than delete-then-write: between a delete and a
            // write the queue does not exist, and a power cut there loses it.
            std::fs::rename(&temporary, &path)
        })();
        if let Err(error) = result {
            log::warn!("Modlight queue could not be written: {error}");
        }
    }

    /// Park bytes that exist nowhere else; answers the path they now live at. The id
    /// names the file (collision-proof) and the original extension is kept on the end
    /// (readable by a person who finds the folder).
    pub fn spool(
        &self,
        item_id: &str,
        file_name: &str,
        data: &mut dyn Read,
    ) -> std::io::Result<PathBuf> {
        let folder = self
            .spool_folder()
            .ok_or_else(|| std::io::Error::other("No spool folder in sandbox mode."))?;
        std::fs::create_dir_all(&folder)?;
        let extension = Path::new(file_name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let path = folder.join(format!("{item_id}{extension}"));
        let mut target = std::fs::File::create(&path)?;
        std::io::copy(data, &mut target)?;
        Ok(path)
    }

    /// Delete one spooled file. Only ever touches files the queue created.
    pub fn discard_spool(&self, item: &QueueItem) {
        if !item.owns_source || item.source_path.is_empty() {
            return;
        }
        let path = Path::new(&item.source_path);
        if path.exists()
            && let Err(error) = std::fs::remove_file(path)
        {
            log::warn!("Modlight queue could not delete a spooled file: {error}");
        }
    }

    /// Delete spooled files no item points at any more. Run once at start, against the
    /// queue as just read: a crash between spooling and writing the queue leaves a file
    /// nothing will look at, and only at start can that be told from one being spooled.
    pub fn sweep_spool(&self, items: &[QueueItem]) {
        let Some(folder) = self.spool_folder() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return;
        };
        let keep: Vec<String> = items
            .iter()
            .filter(|i| i.owns_source && !i.source_path.is_empty())
            .map(|i| i.source_path.to_lowercase())
            .collect();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if keep.contains(&path.to_string_lossy().to_lowercase()) {
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => log::info!("Modlight queue: removed an orphaned spooled file."),
                Err(error) => log::warn!("Modlight queue: {error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upload::item::{QueueReason, QueueState};
    use chrono::TimeZone;

    fn item(name: &str, path: &str) -> QueueItem {
        QueueItem {
            file_name: name.into(),
            source_path: path.into(),
            host: "https://files.example.com".into(),
            length: 3,
            ..QueueItem::default()
        }
    }

    #[test]
    fn the_queue_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::new(dir.path());
        let mut sending = item("b.png", "/tmp/b.png");
        sending.state = QueueState::Sending;
        sending.reason = QueueReason::Unreachable;
        sending.tus_url = Some("https://files.example.com/api/files/abc".into());
        sending.tus_offset = 2;
        store.save(&[item("a.png", "/tmp/a.png"), sending]);

        let text = std::fs::read_to_string(dir.path().join(QUEUE_FILE_NAME)).unwrap();
        assert!(text.contains("\"Version\": 1"), "{text}");
        assert!(!dir.path().join("Queue.json.tmp").exists());

        let now = Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap();
        let items = QueueStore::new(dir.path()).load(now);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[1].state,
            QueueState::Waiting,
            "a socket does not survive"
        );
        assert_eq!(items[1].tus_offset, 2, "the offset does");
        assert_eq!(items[1].next_attempt_utc, now);
    }

    #[test]
    fn damage_reads_as_an_empty_queue() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::new(dir.path());
        let now = Utc::now();
        assert!(store.load(now).is_empty(), "no file");
        std::fs::write(dir.path().join(QUEUE_FILE_NAME), "{ not json").unwrap();
        assert!(store.load(now).is_empty());
        std::fs::write(
            dir.path().join(QUEUE_FILE_NAME),
            r#"{"Version":2,"Items":[]}"#,
        )
        .unwrap();
        assert!(store.load(now).is_empty(), "a newer version");
        std::fs::write(
            dir.path().join(QUEUE_FILE_NAME),
            r#"{"Version":1,"Items":[null,{"Id":"x"},{"Id":"y","FileName":"a","SourcePath":"/a","Host":"https://h","Length":1}]}"#,
        )
        .unwrap();
        let items = store.load(now);
        assert_eq!(items.len(), 1, "unusable rows are dropped");
        assert_eq!(items[0].id, "y");
    }

    #[test]
    fn spooled_files_are_kept_discarded_and_swept() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::new(dir.path());
        let path = store
            .spool("abc", "clip board.png", &mut &b"png"[..])
            .unwrap();
        assert_eq!(path.file_name().unwrap(), "abc.png");
        assert_eq!(std::fs::read(&path).unwrap(), b"png");

        let mut owned = item("clip board.png", path.to_str().unwrap());
        owned.owns_source = true;
        let orphan = store.spool("zzz", "lost.txt", &mut &b"x"[..]).unwrap();
        store.sweep_spool(std::slice::from_ref(&owned));
        assert!(path.exists(), "still wanted");
        assert!(!orphan.exists(), "nobody points at it");

        store.discard_spool(&owned);
        assert!(!path.exists());

        // Never a file the queue did not create.
        let mine = dir.path().join("mine.txt");
        std::fs::write(&mine, "keep").unwrap();
        store.discard_spool(&item("mine.txt", mine.to_str().unwrap()));
        assert!(mine.exists());
    }

    #[test]
    fn a_sandbox_writes_nothing() {
        let store = QueueStore::sandbox();
        store.save(&[item("a", "/a")]);
        assert!(store.load(Utc::now()).is_empty());
        assert!(store.spool("a", "a.txt", &mut &b"x"[..]).is_err());
    }
}

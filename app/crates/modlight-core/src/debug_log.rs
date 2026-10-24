//! `DebugHelper.Logger`: every `log` record of this app, kept in memory for the Debug
//! log window and appended to `Logs/Modlight-Log-yyyy-MM.txt` in the personal folder,
//! one line each as `yyyy-MM-dd HH:mm:ss.fff - message`.
//!
//! Records of the app's own crates are kept from Info up, everything else from Warn up
//! (the graphics and network crates are chatty at Info).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// `Program.LogsFolderName`.
pub const LOGS_FOLDER: &str = "Logs";

struct DebugLog {
    text: Mutex<String>,
    file: Option<PathBuf>,
    /// Bumped on every line, so a window knows to redraw.
    count: AtomicU64,
    echo: bool,
}

static LOG: OnceLock<DebugLog> = OnceLock::new();

/// `Program.LogsFilePath` for `now`.
pub fn log_file_path(personal_folder: &Path, now: chrono::DateTime<chrono::Local>) -> PathBuf {
    personal_folder
        .join(LOGS_FOLDER)
        .join(format!("Modlight-Log-{}.txt", now.format("%Y-%m")))
}

/// `Logger.MessageFormat` for one line.
pub fn format_line(at: chrono::DateTime<chrono::Local>, message: &str) -> String {
    format!("{} - {message}\n", at.format("%Y-%m-%d %H:%M:%S%.3f"))
}

struct Sink;

impl log::Log for Sink {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let ours = metadata.target().starts_with("modlight");
        metadata.level()
            <= if ours {
                log::Level::Info
            } else {
                log::Level::Warn
            }
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            write(&record.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// Install the logger once. `personal_folder` is `None` when nothing may be written to
/// disk (QA shots); `echo` copies each line to standard error (`Debug.Write`).
pub fn init(personal_folder: Option<&Path>, echo: bool) {
    let file = personal_folder.map(|folder| log_file_path(folder, chrono::Local::now()));
    if let Some(parent) = file.as_ref().and_then(|f| f.parent()) {
        let _ = std::fs::create_dir_all(parent);
    }
    if LOG
        .set(DebugLog {
            text: Mutex::new(String::new()),
            file,
            count: AtomicU64::new(0),
            echo,
        })
        .is_ok()
    {
        let _ = log::set_logger(&Sink);
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// `Logger.WriteLine`.
pub fn write(message: &str) {
    let Some(log) = LOG.get() else {
        return;
    };
    let line = format_line(chrono::Local::now(), message);
    if log.echo {
        eprint!("{line}");
    }
    if let Ok(mut text) = log.text.lock() {
        text.push_str(&line);
    }
    if let Some(path) = &log.file
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
    log.count.fetch_add(1, Ordering::Relaxed);
}

/// `Logger.ToString`: everything written this session.
pub fn text() -> String {
    LOG.get()
        .and_then(|log| log.text.lock().ok().map(|t| t.clone()))
        .unwrap_or_default()
}

/// How many lines were written (changes when there is something new to show).
pub fn count() -> u64 {
    LOG.get()
        .map(|log| log.count.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// `Logger.LogFilePath`.
pub fn file_path() -> Option<PathBuf> {
    LOG.get().and_then(|log| log.file.clone())
}

/// `DebugLogWindow.GetUrlAtCaret`: the http(s) address around `index` (a char index).
pub fn url_at(text: &str, index: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let index = index.min(chars.len() - 1);
    let mut start = index;
    let mut end = index;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let candidate: String = chars[start..end].iter().collect();
    let candidate = candidate.trim_matches(|c| {
        matches!(
            c,
            '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
        )
    });
    let url = url::Url::parse(candidate).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| candidate.to_string())
}

/// `CleanupManager.Cleanup` at start: with "Automatically cleanup old backup files" the
/// weekly settings copies in `Backup`, with "... old log files" the monthly logs, each
/// down to the newest `CleanupKeepFileCount`. Returns how many files went.
pub fn cleanup(personal_folder: &Path, config: &crate::config::AppConfig) -> usize {
    let on = |member: &str| config.get(member).and_then(|v| v.as_bool()) == Some(true);
    let keep = config
        .get("CleanupKeepFileCount")
        .and_then(|v| v.as_i64())
        .unwrap_or(10)
        .max(0) as usize;
    let mut removed = 0;
    if on("AutoCleanupBackupFiles") {
        let folder = personal_folder.join(crate::config::BACKUP_FOLDER);
        for prefix in [
            "ApplicationConfig-",
            "HotkeysConfig-",
            "UploadersConfig-",
            "History-",
        ] {
            removed += cleanup_folder(&folder, prefix, ".json", keep);
        }
    }
    if on("AutoCleanupLogFiles") {
        removed += cleanup_folder(
            &personal_folder.join(LOGS_FOLDER),
            "Modlight-Log-",
            ".txt",
            keep,
        );
    }
    removed
}

/// `CleanupManager.CleanupFolder`: keep the newest `keep` files of `folder` whose names
/// start with `prefix` and end with `suffix`; the older ones go.
pub fn cleanup_folder(folder: &Path, prefix: &str, suffix: &str, keep: usize) -> usize {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with(prefix) && name.ends_with(suffix)
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut removed = 0;
    for (_, path) in files.into_iter().skip(keep) {
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn lines_and_paths_read_like_upstream() {
        let at = chrono::Local
            .with_ymd_and_hms(2026, 10, 20, 9, 41, 5)
            .unwrap();
        assert_eq!(
            format_line(at, "Started"),
            "2026-10-20 09:41:05.000 - Started\n"
        );
        assert_eq!(
            log_file_path(Path::new("/p"), at),
            PathBuf::from("/p/Logs/Modlight-Log-2026-10.txt")
        );
    }

    #[test]
    fn a_url_is_found_around_the_caret() {
        let text = "Upload done: \"https://files.example.com/q8\", next";
        assert_eq!(
            url_at(text, 20).as_deref(),
            Some("https://files.example.com/q8")
        );
        assert_eq!(url_at(text, 2), None);
        assert_eq!(url_at("ftp://x", 3), None);
        assert_eq!(url_at("", 0), None);
    }

    #[test]
    fn cleanup_keeps_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        for (i, name) in [
            "Modlight-Log-2026-07.txt",
            "Modlight-Log-2026-08.txt",
            "Modlight-Log-2026-09.txt",
            "other.txt",
        ]
        .iter()
        .enumerate()
        {
            let path = dir.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            let time =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        assert_eq!(cleanup_folder(dir.path(), "Modlight-Log-", ".txt", 2), 1);
        assert!(!dir.path().join("Modlight-Log-2026-07.txt").exists());
        assert!(dir.path().join("other.txt").exists());
    }

    /// "Automatically cleanup old backup files" thins the weekly settings copies the
    /// saves leave, never the copy of the previous file or anything else in `Backup`.
    #[test]
    fn the_start_cleans_the_weekly_backups_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join(crate::config::BACKUP_FOLDER);
        std::fs::create_dir_all(&backup).unwrap();
        let names = [
            "ApplicationConfig-2026-09-W39.json",
            "ApplicationConfig-2026-10-W41.json",
            "ApplicationConfig-2026-10-W43.json",
            "HotkeysConfig-2026-09-W39.json",
            "HotkeysConfig-2026-10-W43.json",
            "ApplicationConfig.json",
            "HotkeysConfig.json",
            "CloudSettings-superseded-local.json",
        ];
        for (i, name) in names.iter().enumerate() {
            let path = backup.join(name);
            std::fs::write(&path, b"{}").unwrap();
            let time =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        let mut config = crate::config::AppConfig::in_memory();
        config.set("CleanupKeepFileCount", serde_json::json!(1));
        assert_eq!(cleanup(dir.path(), &config), 0, "off by default");
        config.set("AutoCleanupBackupFiles", serde_json::json!(true));
        assert_eq!(cleanup(dir.path(), &config), 3);
        let left: std::collections::BTreeSet<String> = std::fs::read_dir(&backup)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let expected: std::collections::BTreeSet<String> = [
            "ApplicationConfig-2026-10-W43.json",
            "HotkeysConfig-2026-10-W43.json",
            "ApplicationConfig.json",
            "HotkeysConfig.json",
            "CloudSettings-superseded-local.json",
        ]
        .iter()
        .map(|n| n.to_string())
        .collect();
        assert_eq!(left, expected);
    }
}

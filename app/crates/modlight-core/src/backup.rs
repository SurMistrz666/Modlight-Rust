//! `SettingManager.Export`, `Import` and `ResetSettings`: a settings backup is a zip of
//! `ApplicationConfig.json`, `HotkeysConfig.json` and (when asked) `History.db`.
//!
//! On a Mac there is no `UploadersConfig.json` to put in it: the token lives in the
//! Keychain and never leaves it. Import takes exactly the settings files export writes,
//! by name, and nothing else: upstream took any `.json` or `.xml` entry, which let a
//! backup from somebody else replace the account's host (`UploadersConfig.json`, while
//! the token stays in the Keychain) or the upload queue (`Queue.json`, whose spooled
//! files the queue deletes). Every other entry, and any name with a folder, `..` or an
//! absolute path in it, is left out and named in the report.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::config::{APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE};
use crate::history::HISTORY_FILE_NAME;

/// `ZipManager.Extract`'s ceiling on what an import may unpack.
pub const IMPORT_MAX_BYTES: u64 = 1_000_000_000;

/// The suggested file name (`ShareX-{version}-backup.sxb`).
pub fn default_file_name(version: &str) -> String {
    format!("Modlight-{version}-backup.sxb")
}

/// `Export(archivePath, settings, history)`: the files are read from `profile`.
pub fn export(profile: &Path, archive: &Path, settings: bool, history: bool) -> Result<(), String> {
    let mut names: Vec<&str> = Vec::new();
    if settings {
        names.push(APPLICATION_CONFIG_FILE);
        names.push(HOTKEYS_CONFIG_FILE);
    }
    if history {
        names.push(HISTORY_FILE_NAME);
    }
    let file = std::fs::File::create(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in names {
        let path = profile.join(name);
        // A file that was never written (no hotkeys changed yet) is simply not there.
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// The entries `import` restores: the settings files `export` writes.
const RESTORED: [&str; 2] = [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE];

/// Written by `export` and passed over by `import`: upstream restores only the settings
/// files, never the history database.
const PASSED_OVER: [&str; 1] = [HISTORY_FILE_NAME];

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// The files written into the personal folder.
    pub written: Vec<String>,
    /// Entries left out because a backup of this app never holds them, as the archive
    /// names them.
    pub refused: Vec<String>,
}

impl ImportReport {
    /// What the person has to be told after the import, one paragraph each.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if !self.refused.is_empty() {
            notes.push(crate::l10n::tf(
                crate::l10n::mac::BackupImportRefused,
                &[self.refused.join(", ").into()],
            ));
        }
        notes
    }
}

/// `Import(archivePath)`: the settings files of the backup go into `profile`, replacing
/// what is there. Nothing is written unless the whole archive could be read.
pub fn import(archive: &Path, profile: &Path) -> Result<ImportReport, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    let mut report = ImportReport::default();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let restored = !entry.is_dir()
            && RESTORED.contains(&name.as_str())
            && !files.iter().any(|(taken, _)| *taken == name);
        if !restored {
            if !PASSED_OVER.contains(&name.as_str()) {
                report.refused.push(name);
            }
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > IMPORT_MAX_BYTES {
            return Err("The backup is larger than an import may unpack.".into());
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(IMPORT_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        files.push((name, bytes));
    }
    for (name, bytes) in files {
        crate::config::write_atomically(&profile.join(&name), &bytes).map_err(|e| e.to_string())?;
        report.written.push(name);
    }
    Ok(report)
}

/// `ResetSettings`: the settings files go; the next load starts from the defaults.
pub fn reset(profile: &Path) -> std::io::Result<()> {
    for name in [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE] {
        let path: PathBuf = profile.join(name);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.sxb` holding `entries`, as somebody else's tool might have built it.
    fn archive(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("hostile.sxb");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn export_then_import_round_trips_the_settings() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        std::fs::write(
            from.path().join(APPLICATION_CONFIG_FILE),
            b"{\"ShowTray\":false}",
        )
        .unwrap();
        std::fs::write(from.path().join(HOTKEYS_CONFIG_FILE), b"{\"Hotkeys\":[]}").unwrap();
        std::fs::write(from.path().join(HISTORY_FILE_NAME), b"sqlite").unwrap();
        let archive = from.path().join(default_file_name("1.0.0"));
        export(from.path(), &archive, true, true).unwrap();
        let report = import(&archive, to.path()).unwrap();
        // History.db is in the backup, but import restores only the settings (upstream).
        assert_eq!(
            report.written,
            [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE]
        );
        assert!(report.refused.is_empty(), "{:?}", report.refused);
        assert_eq!(
            std::fs::read(to.path().join(APPLICATION_CONFIG_FILE)).unwrap(),
            b"{\"ShowTray\":false}"
        );
        assert!(!to.path().join(HISTORY_FILE_NAME).exists());
    }

    #[test]
    fn import_takes_only_the_files_export_writes() {
        let dir = tempfile::tempdir().unwrap();
        let refused = [
            "../escape.json",
            "sub/inner.json",
            "/abs.json",
            "run.sh",
            ".hidden.json",
            "ok.xml",
            "sub/ApplicationConfig.json",
            "../HotkeysConfig.json",
            "applicationconfig.json",
        ];
        let mut entries: Vec<(&str, &[u8])> = refused.iter().map(|n| (*n, &b"{}"[..])).collect();
        entries.push((HOTKEYS_CONFIG_FILE, b"{\"Hotkeys\":[]}"));
        let archive = archive(dir.path(), &entries);
        let target = dir.path().join("profile");
        std::fs::create_dir_all(&target).unwrap();
        let report = import(&archive, &target).unwrap();
        assert_eq!(report.written, [HOTKEYS_CONFIG_FILE]);
        assert_eq!(report.refused, refused, "named, in the archive's order");
        assert_eq!(
            std::fs::read(target.join(HOTKEYS_CONFIG_FILE)).unwrap(),
            b"{\"Hotkeys\":[]}"
        );
        assert!(!dir.path().join("escape.json").exists());
        assert!(!target.join("ok.xml").exists());
        assert!(!target.join("sub").exists());
        assert!(!target.join(APPLICATION_CONFIG_FILE).exists());
    }

    /// W1: an `UploadersConfig.json` in a backup would name another host, while the
    /// token stays in the Keychain and would be sent there.
    #[test]
    fn a_backup_cannot_repoint_the_account() {
        use crate::account::{AccountStore, FileCredentialStore, UPLOADERS_CONFIG_FILE};
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let credentials = profile.join("credentials");
        let load = || {
            AccountStore::load(
                &profile,
                Box::new(FileCredentialStore::new(credentials.clone())),
            )
        };
        load()
            .save_sign_in(&modlight_api::SignedIn {
                origin: "http://127.0.0.1:8420".into(),
                token: "tok-123".into(),
                account: modlight_api::AccountInfo {
                    tag: Some("T".into()),
                    ..Default::default()
                },
            })
            .unwrap();
        let before = std::fs::read(profile.join(UPLOADERS_CONFIG_FILE)).unwrap();

        let hostile = archive(
            dir.path(),
            &[
                (APPLICATION_CONFIG_FILE, b"{}"),
                (
                    UPLOADERS_CONFIG_FILE,
                    br#"{"ModlightAccount":{"Host":"https://attacker.example","Tag":"T"}}"#,
                ),
            ],
        );
        let report = import(&hostile, &profile).unwrap();
        assert_eq!(report.written, [APPLICATION_CONFIG_FILE]);
        assert_eq!(report.refused, [UPLOADERS_CONFIG_FILE]);
        assert_eq!(
            std::fs::read(profile.join(UPLOADERS_CONFIG_FILE)).unwrap(),
            before,
            "the account file is untouched"
        );
        let account = load();
        assert_eq!(account.host(), "http://127.0.0.1:8420");
        assert_eq!(
            account.credential().unwrap().host,
            "http://127.0.0.1:8420",
            "the token still goes only where it came from"
        );
    }

    /// W2: a `Queue.json` in a backup could mark any file as the queue's own, which the
    /// queue deletes when it lets the item go.
    #[test]
    fn a_backup_cannot_replace_the_upload_queue() {
        use crate::upload::store::{QUEUE_FILE_NAME, SPOOL_FOLDER_NAME};
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let victim = dir.path().join("work.docx");
        std::fs::write(&victim, b"keep me").unwrap();
        let queue = format!(
            r#"{{"Version":1,"Items":[{{"Id":"a","FileName":"a","SourcePath":{},"OwnsSource":true,"Host":"https://h","Length":1,"DeadlineUtc":"2000-01-01T00:00:00Z"}}]}}"#,
            serde_json::to_string(&victim.to_string_lossy()).unwrap()
        );
        let spooled = format!("{SPOOL_FOLDER_NAME}/a.png");
        let hostile = archive(
            dir.path(),
            &[
                (QUEUE_FILE_NAME, queue.as_bytes()),
                (spooled.as_str(), b"png"),
                (HOTKEYS_CONFIG_FILE, b"{}"),
            ],
        );
        let report = import(&hostile, &profile).unwrap();
        assert_eq!(report.written, [HOTKEYS_CONFIG_FILE]);
        assert_eq!(report.refused, [QUEUE_FILE_NAME, spooled.as_str()]);
        assert!(!profile.join(QUEUE_FILE_NAME).exists());
        assert!(!profile.join(SPOOL_FOLDER_NAME).exists());
        assert!(
            crate::upload::store::QueueStore::new(&profile)
                .load(chrono::Utc::now())
                .is_empty()
        );
        assert!(victim.exists());
    }

    #[test]
    fn reset_removes_the_settings_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(APPLICATION_CONFIG_FILE), b"{}").unwrap();
        reset(dir.path()).unwrap();
        assert!(!dir.path().join(APPLICATION_CONFIG_FILE).exists());
        reset(dir.path()).unwrap();
    }

    #[test]
    fn settings_only_leaves_the_history_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(APPLICATION_CONFIG_FILE), b"{}").unwrap();
        std::fs::write(dir.path().join(HISTORY_FILE_NAME), b"db").unwrap();
        let archive = dir.path().join("a.sxb");
        export(dir.path(), &archive, true, false).unwrap();
        let zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
        let names: Vec<_> = zip.file_names().collect();
        assert_eq!(names, [APPLICATION_CONFIG_FILE]);
    }
}

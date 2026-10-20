//! `SettingManager.Export`, `Import` and `ResetSettings`: a settings backup is a zip of
//! `ApplicationConfig.json`, `HotkeysConfig.json` and (when asked) `History.db`.
//!
//! On a Mac there is no `UploadersConfig.json` to put in it: the token lives in the
//! Keychain and never leaves it. Import takes only `.json` and `.xml` entries, as
//! upstream does, and only plain names: an entry with a folder in its name, `..`, or an
//! absolute path is skipped instead of being written somewhere else.

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

/// `Import(archivePath)`: the `.json` and `.xml` entries go into `profile`, replacing
/// what is there. Returns the names written.
pub fn import(archive: &Path, profile: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    let mut written = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !is_plain_name(&name) || !has_extension(&name, &["json", "xml"]) {
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
        crate::config::write_atomically(&profile.join(&name), &bytes).map_err(|e| e.to_string())?;
        written.push(name);
    }
    Ok(written)
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

fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
        && !name.starts_with('.')
}

fn has_extension(name: &str, extensions: &[&str]) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let written = import(&archive, to.path()).unwrap();
        // History.db is in the backup, but import takes only json and xml (upstream).
        assert_eq!(written, [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE]);
        assert_eq!(
            std::fs::read(to.path().join(APPLICATION_CONFIG_FILE)).unwrap(),
            b"{\"ShowTray\":false}"
        );
        assert!(!to.path().join(HISTORY_FILE_NAME).exists());
    }

    #[test]
    fn import_skips_paths_and_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.sxb");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for name in [
            "../escape.json",
            "sub/inner.json",
            "/abs.json",
            "run.sh",
            ".hidden.json",
            "ok.xml",
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
        let target = dir.path().join("profile");
        std::fs::create_dir_all(&target).unwrap();
        let written = import(&archive, &target).unwrap();
        assert_eq!(written, ["ok.xml"]);
        assert!(!dir.path().join("escape.json").exists());
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

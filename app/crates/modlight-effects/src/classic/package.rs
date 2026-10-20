//! `ImageEffectPackager`: a preset and the pictures it uses in one `.sxie` (zip) file.
//!
//! Import is written for a hostile archive: only `Config.json` at the root and picture
//! files are taken, an entry whose path could leave the destination (absolute, `..`, a
//! drive or a backslash) is refused, links are never created, and both the declared and
//! the written sizes count against upstream's 100 MB ceiling.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use modlight_core::components::archive::{MAX_ENTRIES, safe_relative, too_large_message};
use modlight_core::names::file_helpers::is_image_file;

/// The preset's entry.
pub const CONFIG_FILE_NAME: &str = "Config.json";

/// `ExtractPackage`'s `maxUncompressedSize`.
pub const MAX_UNCOMPRESSED_SIZE: u64 = 100_000_000;

/// `Package`: `Config.json` and every picture under `assets_folder` (entries named from
/// the folder's parent, so they keep the folder's name). Returns the written file.
pub fn package(
    output_file: &Path,
    config_json: &str,
    assets_folder: Option<&Path>,
) -> Result<PathBuf, String> {
    if output_file.as_os_str().is_empty() {
        return Err("no output file".to_string());
    }
    if let Some(parent) = output_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut assets: Vec<(PathBuf, String)> = Vec::new();
    if let Some(folder) = assets_folder.filter(|f| f.is_dir()) {
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        collect_images(folder, &name, &mut assets);
    }
    assets.sort_by(|a, b| a.1.cmp(&b.1));
    let tmp = output_file.with_extension("sxie.tmp");
    let result = (|| -> Result<(), String> {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(CONFIG_FILE_NAME, options)
            .map_err(|e| e.to_string())?;
        zip.write_all(config_json.as_bytes())
            .map_err(|e| e.to_string())?;
        for (path, entry) in &assets {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            zip.start_file(entry.as_str(), options)
                .map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&tmp, output_file).map_err(|e| e.to_string())?;
            Ok(output_file.to_path_buf())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Pictures under `folder`, recursively; links are not followed.
fn collect_images(folder: &Path, prefix: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let entry_name = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if kind.is_dir() {
            collect_images(&entry.path(), &entry_name, out);
        } else if kind.is_file() && is_image_file(&name) {
            out.push((entry.path(), entry_name));
        }
    }
}

/// Why an entry was left out of an import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skipped {
    /// Absolute, `..`, a drive, a backslash or empty.
    UnsafePath(String),
    /// A link, a device or another non-file entry.
    NotAFile(String),
    /// Neither `Config.json` nor a picture.
    NotAPicture(String),
}

/// What `extract_package` found.
#[derive(Clone, Debug, Default)]
pub struct Extracted {
    /// The preset's JSON, when the package had one.
    pub config_json: Option<String>,
    /// The pictures written, inside the destination.
    pub files: Vec<PathBuf>,
    /// The entries refused.
    pub skipped: Vec<Skipped>,
}

/// `ExtractPackage`: the pictures into `destination` (their folders kept) and the text of
/// `Config.json`. Errors when the archive cannot be read or is larger than 100 MB.
pub fn extract_package(package_file: &Path, destination: &Path) -> Result<Extracted, String> {
    let file = std::fs::File::open(package_file).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{} entries", zip.len()));
    }
    let mut total: u64 = 0;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|e| e.to_string())?;
        total = total.saturating_add(entry.size());
    }
    if total > MAX_UNCOMPRESSED_SIZE {
        return Err(too_large_message(total, MAX_UNCOMPRESSED_SIZE));
    }
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let mut out = Extracted::default();
    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let base = name.rsplit('/').next().unwrap_or("").to_string();
        let is_config = name.eq_ignore_ascii_case(CONFIG_FILE_NAME);
        if !is_config && !is_image_file(&base) {
            if !entry.is_dir() {
                out.skipped.push(Skipped::NotAPicture(name));
            }
            continue;
        }
        if entry.is_dir() || entry.is_symlink() || !entry.is_file() {
            out.skipped.push(Skipped::NotAFile(name));
            continue;
        }
        let declared = entry.size();
        if is_config {
            if out.config_json.is_some() {
                continue;
            }
            let mut bytes = Vec::new();
            let read = (&mut entry)
                .take(declared.min(MAX_UNCOMPRESSED_SIZE) + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())? as u64;
            if read > declared {
                return Err(format!("{name} is larger than it says"));
            }
            written = written.saturating_add(read);
            out.config_json = Some(
                String::from_utf8_lossy(&bytes)
                    .trim_start_matches('\u{feff}')
                    .to_string(),
            );
            continue;
        }
        let Some(relative) = safe_relative(&name) else {
            out.skipped.push(Skipped::UnsafePath(name));
            continue;
        };
        let target = destination.join(&relative);
        if !lands_inside(destination, &target) {
            out.skipped.push(Skipped::UnsafePath(name));
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // A link already sitting where the picture goes is replaced, never followed.
        if std::fs::symlink_metadata(&target).is_ok_and(|m| !m.is_file()) {
            out.skipped.push(Skipped::NotAFile(name));
            continue;
        }
        let mut bytes = Vec::new();
        let read = (&mut entry)
            .take(declared + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())? as u64;
        if read > declared {
            return Err(format!("{name} is larger than it says"));
        }
        written = written.saturating_add(read);
        if written > MAX_UNCOMPRESSED_SIZE {
            return Err(too_large_message(written, MAX_UNCOMPRESSED_SIZE));
        }
        std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
        out.files.push(target);
    }
    Ok(out)
}

/// No folder between `destination` and `target` is a link (a planted link could carry a
/// later entry out of the destination).
fn lands_inside(destination: &Path, target: &Path) -> bool {
    let Ok(rest) = target.strip_prefix(destination) else {
        return false;
    };
    let mut at = destination.to_path_buf();
    let parts: Vec<_> = rest.components().collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        at.push(part);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("modlight-sxie-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn package_round_trips_config_and_pictures() {
        let dir = scratch("roundtrip");
        let assets = dir.join("My preset");
        std::fs::create_dir_all(assets.join("sub")).unwrap();
        std::fs::write(assets.join("a.png"), b"png").unwrap();
        std::fs::write(assets.join("sub/b.jpg"), b"jpg").unwrap();
        std::fs::write(assets.join("notes.txt"), b"text").unwrap();
        let file = dir.join("out/My preset.sxie");
        package(&file, "{\"Name\":\"x\"}", Some(&assets)).unwrap();
        let dest = dir.join("dest");
        let got = extract_package(&file, &dest).unwrap();
        assert_eq!(got.config_json.as_deref(), Some("{\"Name\":\"x\"}"));
        assert!(dest.join("My preset/a.png").is_file());
        assert!(dest.join("My preset/sub/b.jpg").is_file());
        assert!(!dest.join("My preset/notes.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_with_escaping_paths_are_refused() {
        let dir = scratch("escape");
        let file = dir.join("evil.sxie");
        write_zip(
            &file,
            &[
                ("Config.json", b"{}"),
                ("../outside.png", b"x"),
                ("/abs.png", b"x"),
                ("a\\..\\..\\win.png", b"x"),
                ("C:/drive.png", b"x"),
                ("ok/fine.png", b"x"),
                ("run.sh", b"x"),
            ],
        );
        let dest = dir.join("dest");
        let got = extract_package(&file, &dest).unwrap();
        assert!(!dir.join("outside.png").exists());
        assert!(dest.join("ok/fine.png").is_file());
        assert_eq!(got.files.len(), 2, "{:?}", got.files);
        assert!(
            got.skipped
                .contains(&Skipped::UnsafePath("../outside.png".into()))
        );
        assert!(
            got.skipped
                .contains(&Skipped::UnsafePath("/abs.png".into()))
        );
        assert!(
            got.skipped
                .contains(&Skipped::UnsafePath("a\\..\\..\\win.png".into()))
        );
        assert!(got.skipped.contains(&Skipped::NotAPicture("run.sh".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_only_counts_at_the_root() {
        let dir = scratch("config");
        let file = dir.join("p.sxie");
        write_zip(&file, &[("sub/Config.json", b"{\"a\":1}")]);
        let got = extract_package(&file, &dir.join("dest")).unwrap();
        assert!(got.config_json.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn planted_links_are_not_followed() {
        let dir = scratch("link");
        let dest = dir.join("dest");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dest.join("sub")).unwrap();
        let file = dir.join("p.sxie");
        write_zip(&file, &[("sub/x.png", b"x")]);
        let got = extract_package(&file, &dest).unwrap();
        assert!(!elsewhere.join("x.png").exists());
        assert_eq!(got.skipped, vec![Skipped::UnsafePath("sub/x.png".into())]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

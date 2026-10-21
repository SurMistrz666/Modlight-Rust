//! Unpacking a verified archive (`ZipManager.Extract` with its size ceiling).
//!
//! Only ever run on bytes whose SHA-256 matched a pinned value, so none of this is the
//! line of defence against a hostile archive - the digest is. It is still written as if
//! it were: an entry that would land outside the destination is skipped (upstream's
//! rule), links and devices are never created, the declared size of the whole archive
//! must stay under the component's ceiling, and the bytes actually written are counted
//! against the same ceiling, because a zip can lie about its sizes.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::format::local_size;
use crate::l10n::{Arg, format, helpers, t};

/// More entries than any real component has (ExifTool's tree is about 1500).
pub const MAX_ENTRIES: usize = 20_000;

/// "The uncompressed size of this archive exceeds ..." with both numbers, as upstream.
pub fn too_large_message(total: u64, limit: u64) -> String {
    format!(
        "{}\n\n{}\n{}",
        t(helpers::ZipManager_Archive_too_large),
        format(
            t(helpers::ZipManager_Archive_uncompressed_file_size),
            &[Arg::from(local_size(total as i64, false, 2))]
        ),
        format(
            t(helpers::ZipManager_Maximum_allowed_file_size),
            &[Arg::from(local_size(limit as i64, false, 2))]
        ),
    )
}

/// A relative path made only of ordinary names, or `None` for anything that could
/// leave the destination (absolute, `..`, a drive prefix; on Windows also a name with a
/// colon or a DOS device name) or that is empty.
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\0') || name.contains('\\') {
        return None;
    }
    let mut out = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            #[cfg(windows)]
            Component::Normal(part) if !is_plain_windows_name(&part.to_string_lossy()) => {
                return None;
            }
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Whether a name is an ordinary file name on Windows: no colon (a drive or another
/// file's stream) and no DOS device name (`CON`, `NUL`, `COM1`..., with or without an
/// extension), which Windows opens as the device wherever the name sits. Upstream's
/// `Path.GetFullPath` turns such a name into `\\.\CON`, which then fails its "inside the
/// destination" test.
#[cfg(windows)]
pub fn is_plain_windows_name(name: &str) -> bool {
    if name.contains(':') {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    // COM0-9 and LPT0-9, the superscript digits included.
    let port = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    !port.is_some_and(|number| {
        let mut chars = number.chars();
        matches!(
            (chars.next(), chars.next()),
            (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
        )
    })
}

/// Copy at most `limit` bytes; more than that is an error, not a truncation.
fn copy_limited(reader: &mut dyn Read, writer: &mut dyn Write, limit: u64) -> Result<u64, String> {
    let mut limited = reader.take(limit + 1);
    let copied = std::io::copy(&mut limited, writer).map_err(|e| e.to_string())?;
    if copied > limit {
        return Err(too_large_message(copied, limit));
    }
    Ok(copied)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// Extract the one entry of a zip whose file name is `file_name` (wherever it sits) into
/// `destination/file_name`, made executable. `ZipSingleFile`.
pub fn extract_zip_single(
    archive: &Path,
    file_name: &str,
    destination: &Path,
    max_uncompressed: u64,
) -> Result<PathBuf, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{} entries", zip.len()));
    }
    // The whole archive's declared size, as upstream measures it.
    let mut total: u64 = 0;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|e| e.to_string())?;
        total = total.saturating_add(entry.size());
    }
    if max_uncompressed > 0 && total > max_uncompressed {
        return Err(too_large_message(total, max_uncompressed));
    }
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let target = destination.join(file_name);
    let mut found = false;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        if entry.is_dir() || entry.is_symlink() {
            continue;
        }
        let name = entry.name().to_string();
        let base = name.rsplit('/').next().unwrap_or("");
        if !base.eq_ignore_ascii_case(file_name) {
            continue;
        }
        if found {
            return Err(format!("{file_name} is in the archive more than once"));
        }
        found = true;
        let declared = entry.size();
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        let limit = if max_uncompressed > 0 {
            declared.min(max_uncompressed)
        } else {
            declared
        };
        if let Err(error) = copy_limited(&mut entry, &mut out, limit) {
            drop(out);
            let _ = std::fs::remove_file(&target);
            return Err(error);
        }
        set_mode(&target, 0o755);
    }
    if !found {
        return Err(format!("{file_name} was not in the archive"));
    }
    Ok(target)
}

/// Extract a gzip'd tarball whose entries all sit under one top-level folder, keeping
/// only the names in `keep` (first level inside that folder) and writing them into
/// `destination`. Regular files and folders only; `executables` get mode 755.
pub fn extract_tar_gz_folder(
    archive: &Path,
    keep: &[&str],
    executables: &[&str],
    destination: &Path,
    max_uncompressed: u64,
) -> Result<(), String> {
    // First pass: the declared sizes, the entry count and the shared top-level folder.
    let mut top: Option<String> = None;
    let mut total: u64 = 0;
    let mut count = 0usize;
    {
        let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries().map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            count += 1;
            if count > MAX_ENTRIES {
                return Err(format!("more than {MAX_ENTRIES} entries"));
            }
            if matches!(
                entry.header().entry_type(),
                tar::EntryType::XGlobalHeader | tar::EntryType::XHeader
            ) {
                continue;
            }
            total = total.saturating_add(entry.header().size().unwrap_or(0));
            let path = entry.path().map_err(|e| e.to_string())?;
            let name = path.to_string_lossy().to_string();
            if let Some(relative) = safe_relative(&name)
                && let Some(first) = relative.components().next()
            {
                let first = first.as_os_str().to_string_lossy().to_string();
                match &top {
                    None => top = Some(first),
                    Some(existing) if *existing != first => {
                        return Err("the archive has more than one top-level folder".into());
                    }
                    _ => {}
                }
            }
        }
    }
    if max_uncompressed > 0 && total > max_uncompressed {
        return Err(too_large_message(total, max_uncompressed));
    }
    let Some(top) = top else {
        return Err("the archive is empty".into());
    };

    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut written: u64 = 0;
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        let path = entry.path().map_err(|e| e.to_string())?;
        let name = path.to_string_lossy().to_string();
        let Some(relative) = safe_relative(&name) else {
            // Upstream's rule: an entry that would land outside is skipped.
            continue;
        };
        let mut parts = relative.components();
        if parts
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            != Some(top.clone())
        {
            continue;
        }
        let inner: PathBuf = parts.collect();
        let Some(first) = inner.components().next() else {
            continue;
        };
        let first = first.as_os_str().to_string_lossy().to_string();
        if !keep.contains(&first.as_str()) {
            continue;
        }
        let target = destination.join(&inner);
        match kind {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let declared = entry.header().size().unwrap_or(0);
                let remaining = if max_uncompressed > 0 {
                    max_uncompressed.saturating_sub(written)
                } else {
                    u64::MAX - 1
                };
                let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
                let copied = copy_limited(&mut entry, &mut out, declared.min(remaining))?;
                written += copied;
                let executable = executables.contains(&inner.to_string_lossy().as_ref());
                set_mode(&target, if executable { 0o755 } else { 0o644 });
            }
            // Links, devices and FIFOs are never created: a link is how an archive
            // writes outside the folder it was unpacked into.
            _ => continue,
        }
    }
    Ok(())
}

/// Extract a zip whose entries all sit under one top-level folder (ExifTool's Windows
/// build, `exiftool-13.59_64/`), taking out only the names in `keep`: each pair is a
/// file or folder directly inside that top-level folder and the name it gets in
/// `destination` (`exiftool(-k).exe` becomes `exiftool.exe`: the "(-k)" in the name is
/// what makes ExifTool wait for a key press before it exits). Files and folders only;
/// every name must be in the archive.
pub fn extract_zip_folder(
    archive: &Path,
    keep: &[(&str, &str)],
    destination: &Path,
    max_uncompressed: u64,
) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{} entries", zip.len()));
    }
    // The declared sizes and the one top-level folder.
    let mut total: u64 = 0;
    let mut top: Option<String> = None;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|e| e.to_string())?;
        total = total.saturating_add(entry.size());
        let Some(relative) = safe_relative(entry.name()) else {
            continue;
        };
        let Some(first) = relative.components().next() else {
            continue;
        };
        let first = first.as_os_str().to_string_lossy().to_string();
        match &top {
            None => top = Some(first),
            Some(existing) if *existing != first => {
                return Err("the archive has more than one top-level folder".into());
            }
            _ => {}
        }
    }
    if max_uncompressed > 0 && total > max_uncompressed {
        return Err(too_large_message(total, max_uncompressed));
    }
    let Some(top) = top else {
        return Err("the archive is empty".into());
    };

    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let mut found = vec![false; keep.len()];
    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some(relative) = safe_relative(entry.name()) else {
            // Upstream's rule: an entry that would land outside is skipped.
            continue;
        };
        let mut parts = relative.components();
        if parts
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            != Some(top.clone())
        {
            continue;
        }
        let inner: PathBuf = parts.collect();
        let mut inner_parts = inner.components();
        let Some(first) = inner_parts.next() else {
            continue;
        };
        let first = first.as_os_str().to_string_lossy().to_string();
        let Some(slot) = keep.iter().position(|(from, _)| *from == first) else {
            continue;
        };
        found[slot] = true;
        let rest = inner_parts.as_path();
        let target = if rest.as_os_str().is_empty() {
            destination.join(keep[slot].1)
        } else {
            destination.join(keep[slot].1).join(rest)
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        // Links are never created: a link is how an archive writes outside the folder
        // it was unpacked into.
        if entry.is_symlink() || !entry.is_file() {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let declared = entry.size();
        let remaining = if max_uncompressed > 0 {
            max_uncompressed.saturating_sub(written)
        } else {
            u64::MAX - 1
        };
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        written += copy_limited(&mut entry, &mut out, declared.min(remaining))?;
        set_mode(&target, 0o644);
    }
    if let Some(missing) = keep
        .iter()
        .zip(&found)
        .find_map(|((from, _), found)| (!found).then_some(from))
    {
        return Err(format!("{missing} was not in the archive"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_with(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut writer = zip::ZipWriter::new(file.reopen().unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        file
    }

    fn tar_gz_with(entries: &[(&str, &[u8], tar::EntryType)]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let encoder =
            flate2::write::GzEncoder::new(file.reopen().unwrap(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes, kind) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            // Written raw so names the builder would refuse (`..`, absolute) still go in.
            let raw = header.as_old_mut();
            raw.name = [0; 100];
            raw.name[..name.len()].copy_from_slice(name.as_bytes());
            if *kind == tar::EntryType::Symlink {
                raw.linkname[..2].copy_from_slice(b"..");
            }
            header.set_cksum();
            builder.append(&header, *bytes).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        file
    }

    #[test]
    fn the_single_file_comes_out_executable_wherever_it_sat() {
        let archive = zip_with(&[("readme.txt", b"hi"), ("bin/ffmpeg", b"\x7fELF-ish")]);
        let dir = tempfile::tempdir().unwrap();
        let out = extract_zip_single(archive.path(), "ffmpeg", dir.path(), 1024).unwrap();
        assert_eq!(out, dir.path().join("ffmpeg"));
        assert_eq!(std::fs::read(&out).unwrap(), b"\x7fELF-ish");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        assert!(!dir.path().join("readme.txt").exists());
    }

    #[test]
    fn a_missing_or_doubled_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let none = zip_with(&[("readme.txt", b"hi")]);
        assert!(extract_zip_single(none.path(), "ffmpeg", dir.path(), 1024).is_err());
        let twice = zip_with(&[("a/ffmpeg", b"1"), ("b/ffmpeg", b"2")]);
        assert!(extract_zip_single(twice.path(), "ffmpeg", dir.path(), 1024).is_err());
    }

    #[test]
    fn a_zip_over_the_ceiling_is_refused_before_anything_is_written() {
        let archive = zip_with(&[("ffmpeg", &[0u8; 4096])]);
        let dir = tempfile::tempdir().unwrap();
        let error = extract_zip_single(archive.path(), "ffmpeg", dir.path(), 1000).unwrap_err();
        assert!(error.contains(t(helpers::ZipManager_Archive_too_large)));
        assert!(!dir.path().join("ffmpeg").exists());
    }

    #[test]
    fn a_traversing_zip_entry_lands_inside_the_destination() {
        let archive = zip_with(&[("../../ffmpeg", b"x")]);
        let outer = tempfile::tempdir().unwrap();
        let dir = outer.path().join("a").join("b");
        let out = extract_zip_single(archive.path(), "ffmpeg", &dir, 1024).unwrap();
        assert_eq!(out, dir.join("ffmpeg"));
        assert!(!outer.path().join("ffmpeg").exists());
    }

    #[test]
    fn unsafe_names_are_recognised() {
        assert!(safe_relative("a/b/c").is_some());
        assert!(safe_relative("./a").is_some());
        assert!(safe_relative("../a").is_none());
        assert!(safe_relative("a/../../b").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("a\\..\\b").is_none());
        assert!(safe_relative("").is_none());
        #[cfg(windows)]
        {
            assert!(safe_relative("C:/x").is_none());
            assert!(safe_relative("a/b.png:stream").is_none());
            assert!(safe_relative("CON").is_none());
            assert!(safe_relative("tools/nul.txt").is_none());
            assert!(safe_relative("com1.png").is_none());
            assert!(safe_relative("LPT9").is_none());
            assert!(safe_relative("console.png").is_some());
            assert!(safe_relative("com10.png").is_some());
            assert!(safe_relative("exiftool_files/perl.exe").is_some());
        }
        assert!(safe_relative(".").is_none());
    }

    #[test]
    fn the_tarball_keeps_only_what_was_asked_for() {
        use tar::EntryType::{Directory, Regular};
        let archive = tar_gz_with(&[
            ("Image-ExifTool-13.0/", b"", Directory),
            (
                "Image-ExifTool-13.0/exiftool",
                b"#!/usr/bin/perl\n",
                Regular,
            ),
            ("Image-ExifTool-13.0/lib/", b"", Directory),
            (
                "Image-ExifTool-13.0/lib/Image/ExifTool.pm",
                b"1;\n",
                Regular,
            ),
            ("Image-ExifTool-13.0/t/test.t", b"tests", Regular),
            ("Image-ExifTool-13.0/README", b"read me", Regular),
        ]);
        let dir = tempfile::tempdir().unwrap();
        extract_tar_gz_folder(
            archive.path(),
            &["exiftool", "lib"],
            &["exiftool"],
            dir.path(),
            1 << 20,
        )
        .unwrap();
        assert!(dir.path().join("exiftool").is_file());
        assert!(dir.path().join("lib/Image/ExifTool.pm").is_file());
        assert!(!dir.path().join("t").exists());
        assert!(!dir.path().join("README").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &str| {
                std::fs::metadata(dir.path().join(p))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777
            };
            assert_eq!(mode("exiftool"), 0o755);
            assert_eq!(mode("lib/Image/ExifTool.pm"), 0o644);
        }
    }

    #[test]
    fn traversal_and_links_in_a_tarball_never_leave_the_folder() {
        use tar::EntryType::{Regular, Symlink};
        let archive = tar_gz_with(&[
            ("top/exiftool", b"ok", Regular),
            ("top/../../evil", b"bad", Regular),
            ("/abs/evil", b"bad", Regular),
            ("top/lib/link", b"", Symlink),
        ]);
        let outer = tempfile::tempdir().unwrap();
        let dir = outer.path().join("x").join("y");
        extract_tar_gz_folder(
            archive.path(),
            &["exiftool", "lib"],
            &["exiftool"],
            &dir,
            1 << 20,
        )
        .unwrap();
        assert!(dir.join("exiftool").is_file());
        assert!(!outer.path().join("evil").exists());
        assert!(!outer.path().join("x").join("evil").exists());
        assert!(std::fs::symlink_metadata(dir.join("lib/link")).is_err());
    }

    #[test]
    fn a_tarball_over_the_ceiling_is_refused() {
        use tar::EntryType::Regular;
        let archive = tar_gz_with(&[("top/exiftool", &[1u8; 5000], Regular)]);
        let dir = tempfile::tempdir().unwrap();
        let error = extract_tar_gz_folder(archive.path(), &["exiftool"], &[], dir.path(), 1000)
            .unwrap_err();
        assert!(error.contains(t(helpers::ZipManager_Archive_too_large)));
        assert!(!dir.path().join("exiftool").exists());
    }

    #[test]
    fn two_top_level_folders_are_refused() {
        use tar::EntryType::Regular;
        let archive = tar_gz_with(&[("a/exiftool", b"1", Regular), ("b/exiftool", b"2", Regular)]);
        let dir = tempfile::tempdir().unwrap();
        assert!(
            extract_tar_gz_folder(archive.path(), &["exiftool"], &[], dir.path(), 1 << 20).is_err()
        );
    }

    #[test]
    fn a_zip_folder_gives_only_what_is_kept_under_its_new_names() {
        let archive = zip_with(&[
            ("exiftool-13.59_64/README.txt", b"read me"),
            ("exiftool-13.59_64/exiftool(-k).exe", b"MZ launcher"),
            ("exiftool-13.59_64/exiftool_files/perl.exe", b"MZ perl"),
            (
                "exiftool-13.59_64/exiftool_files/lib/Image/ExifTool.pm",
                b"1;",
            ),
            ("exiftool-13.59_64/../escape.txt", b"x"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let keep = [
            ("exiftool_files", "exiftool_files"),
            ("exiftool(-k).exe", "exiftool.exe"),
        ];
        extract_zip_folder(archive.path(), &keep, dir.path(), 1 << 20).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("exiftool.exe")).unwrap(),
            b"MZ launcher"
        );
        assert!(
            dir.path()
                .join("exiftool_files")
                .join("lib")
                .join("Image")
                .join("ExifTool.pm")
                .is_file()
        );
        assert!(!dir.path().join("README.txt").exists());
        assert!(!dir.path().join("exiftool(-k).exe").exists());
        assert!(!dir.path().parent().unwrap().join("escape.txt").exists());

        // Every kept name must be there.
        let missing = zip_with(&[("top/exiftool_files/perl.exe", b"x")]);
        let error = extract_zip_folder(missing.path(), &keep, dir.path(), 1 << 20).unwrap_err();
        assert!(error.contains("exiftool(-k).exe"), "{error}");
        // One top-level folder, and the size ceiling.
        let two = zip_with(&[("a/x", b"x"), ("b/y", b"y")]);
        assert!(extract_zip_folder(two.path(), &keep, dir.path(), 1 << 20).is_err());
        assert!(extract_zip_folder(archive.path(), &keep, dir.path(), 8).is_err());
    }
}

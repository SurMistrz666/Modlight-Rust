//! Every interface text the code uses has its own Polish translation.
//!
//! A key the code names always exists in English (the tables are generated from the
//! English `.resx` files, and a missing name does not compile), but the Polish table
//! falls back to the English text when it lacks a key, silently. This test reads the
//! sources of every crate, finds each `<table>::<Key>` (with the module aliases each file
//! gives the tables, `helpers as h`, `editor as s`...), and fails on a key whose Polish
//! text is missing or empty.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use modlight_core::l10n;

/// The generated table modules and the `.resx` project each one comes from.
const TABLES: &[(&str, &str)] = &[
    ("s", "ShareX"),
    ("avalonia", "ShareX.Avalonia"),
    ("uploaders", "ShareX.UploadersLib"),
    ("helpers", "ShareX.HelpersLib"),
    ("capture", "ShareX.ScreenCaptureLib"),
    ("editor", "ShareX.ImageEditor"),
    ("effects", "ShareX.ImageEffectsLib"),
    ("tools", "ShareX.Tools"),
    ("history", "ShareX.HistoryLib"),
    ("mac", "Mac"),
    ("windows", "Windows"),
    ("port", "Port"),
];

fn rust_sources(folder: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// What each name before `::` means in this file: the tables by their own names, then
/// the aliases the file's `use ...l10n::{x as y}` gives them.
fn aliases(source: &str) -> Vec<(String, &'static str)> {
    let mut map: Vec<(String, &'static str)> = TABLES
        .iter()
        .map(|(module, project)| (module.to_string(), *project))
        .collect();
    let group = regex::Regex::new(r"l10n::\{([^}]*)\}").unwrap();
    let single = regex::Regex::new(r"l10n::([a-z_]+) as ([a-z_]+)").unwrap();
    let renamed = regex::Regex::new(r"^([a-z_]+) as ([a-z_]+)$").unwrap();
    let mut add = |module: &str, alias: &str| {
        if let Some((_, project)) = TABLES.iter().find(|(m, _)| *m == module) {
            map.retain(|(name, _)| name != alias);
            map.push((alias.to_string(), project));
        }
    };
    for captures in group.captures_iter(source) {
        for part in captures[1].split(',').map(str::trim) {
            if let Some(parts) = renamed.captures(part) {
                add(&parts[1], &parts[2]);
            }
        }
    }
    for captures in single.captures_iter(source) {
        add(&captures[1], &captures[2]);
    }
    map
}

#[test]
fn every_text_the_code_uses_is_translated() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        rust_sources(&entry.path().join("src"), &mut files);
    }
    assert!(files.len() > 50, "the sources were not found: {crates:?}");
    let usage = regex::Regex::new(r"\b([a-z_]+)::([A-Z][A-Za-z0-9_]*)\b").unwrap();
    let mut used = BTreeSet::new();
    let mut untranslated = BTreeSet::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        let aliases = aliases(&source);
        for captures in usage.captures_iter(&source) {
            let Some((_, project)) = aliases.iter().find(|(name, _)| *name == captures[1]) else {
                continue;
            };
            // `capture::CaptureKind` and the like are types, not texts.
            let Some(key) = l10n::lookup(&format!("{project}:{}", &captures[2])) else {
                continue;
            };
            used.insert(l10n::name_of(key));
            if !l10n::has_polish(key) {
                untranslated.insert(format!(
                    "{} ({})",
                    l10n::name_of(key),
                    file.strip_prefix(&crates).unwrap_or(file).display()
                ));
            }
        }
    }
    assert!(used.len() > 1500, "only {} keys found in use", used.len());
    assert!(
        untranslated.is_empty(),
        "texts the code uses without a Polish translation:\n{}",
        untranslated.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn a_missing_polish_text_is_told_apart_from_the_fallback() {
    use l10n::{Language, text_in};
    let trash = l10n::lookup("ShareX:MySpace_Trash").unwrap();
    assert!(l10n::has_polish(trash));
    assert_ne!(
        text_in(trash, Language::Polish),
        text_in(trash, Language::English)
    );
}

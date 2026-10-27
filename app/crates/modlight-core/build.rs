//! Generates the string tables, the skins and the icon constants from the files copied
//! out of upstream (`app/assets/upstream`). Nothing user-visible is typed by hand: a
//! change to a `.resx`, to `Skins.axaml` or to the Lucide map is a rebuild away.
//!
//! Also gives the crate this build's identity (`build_info`): the commit and the time it
//! was built, from `MODLIGHT_BUILD` and `MODLIGHT_BUILT` when the build scripts set them
//! (so the app knows it is exactly the build they publish), else from git and the clock.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let assets = manifest.join("../../assets/upstream");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed={}", assets.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("../../assets/mac").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("../../assets/windows").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("../../assets/port").display()
    );

    build_identity(&manifest);

    std::fs::write(out.join("strings.rs"), strings(&assets.join("l10n"))).unwrap();
    std::fs::write(
        out.join("skins.rs"),
        skins(
            &assets.join("theming"),
            &manifest.join("../../assets/mac/theming/Skins.axaml"),
        ),
    )
    .unwrap();
    std::fs::write(
        out.join("lucide.rs"),
        lucide(&assets.join("icons/lucide-codepoints.json")),
    )
    .unwrap();
}

/// `MODLIGHT_BUILD_ID` and `MODLIGHT_BUILT_AT` for `build_info`. A value the build scripts
/// give is taken as it is (an empty `MODLIGHT_BUILD` included: built outside git); without
/// them the commit comes from git (and the build reruns when HEAD moves) and the time is
/// now. A value that would not fit a file name or is not a UTC time fails the build.
fn build_identity(manifest: &Path) {
    println!("cargo:rerun-if-env-changed=MODLIGHT_BUILD");
    println!("cargo:rerun-if-env-changed=MODLIGHT_BUILT");
    let build = match std::env::var("MODLIGHT_BUILD") {
        Ok(value) => value.trim().to_string(),
        Err(_) => git_commit(manifest),
    };
    assert!(
        build.len() <= 40
            && build
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')),
        "MODLIGHT_BUILD must be at most 40 letters, digits, dots, dashes or underscores, not {build:?}"
    );
    let built = match std::env::var("MODLIGHT_BUILT") {
        Ok(value) => value.trim().to_string(),
        Err(_) => utc_stamp(SystemTime::now()),
    };
    assert!(
        is_utc_stamp(&built),
        "MODLIGHT_BUILT must be a UTC time written as 2026-10-27T14:03:05Z, not {built:?}"
    );
    println!("cargo:rustc-env=MODLIGHT_BUILD_ID={build}");
    println!("cargo:rustc-env=MODLIGHT_BUILT_AT={built}");
}

/// `git rev-parse --short=8 HEAD` in the crate's checkout, or empty outside git. The files
/// that say where HEAD is are watched, so a commit or a checkout gives the next build the
/// new commit (only files that exist: a missing one would rerun this script every time).
fn git_commit(manifest: &Path) -> String {
    let git = |args: &[&str]| -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(manifest)
            .args(args)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (output.status.success() && !text.is_empty()).then_some(text)
    };
    let Some(commit) = git(&["rev-parse", "--short=8", "HEAD"]) else {
        return String::new();
    };
    let mut watched = vec![git(&["rev-parse", "--git-path", "HEAD"])];
    watched.push(git(&["rev-parse", "--git-path", "packed-refs"]));
    if let Some(branch) = git(&["rev-parse", "--symbolic-full-name", "HEAD"])
        && branch.starts_with("refs/")
    {
        watched.push(git(&["rev-parse", "--git-path", &branch]));
    }
    for path in watched.into_iter().flatten() {
        let path = manifest.join(path);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    commit
}

/// A moment as `2026-10-27T14:03:05Z` (UTC, whole seconds).
fn utc_stamp(at: SystemTime) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let second_of_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60
    )
}

/// The proleptic Gregorian date of a day counted from 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Whether a text has the shape `utc_stamp` writes (the app parses it as RFC 3339).
fn is_utc_stamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 20
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            10 => *b == b'T',
            13 | 16 => *b == b':',
            19 => *b == b'Z',
            _ => b.is_ascii_digit(),
        })
}

/// The projects whose tables are used, with the module each one is generated into.
const PROJECTS: &[(&str, &str)] = &[
    ("ShareX", "s"),
    ("ShareX.Avalonia", "avalonia"),
    ("ShareX.UploadersLib", "uploaders"),
    ("ShareX.HelpersLib", "helpers"),
    ("ShareX.ScreenCaptureLib", "capture"),
    ("ShareX.ImageEditor", "editor"),
    ("ShareX.ImageEffectsLib", "effects"),
    // Phase 5, tools: the tool windows and the history windows.
    ("ShareX.Tools", "tools"),
    ("ShareX.HistoryLib", "history"),
    // The macOS port's own texts, where upstream's name Windows (assets/mac/l10n).
    ("Mac", "mac"),
    // The port's own texts on Windows, where upstream has none and the Mac's name macOS
    // (assets/windows/l10n).
    ("Windows", "windows"),
    // The port's own texts on both systems, for what upstream never had (assets/port/l10n):
    // the build line, the app's updates from the server's Downloads.
    ("Port", "port"),
];

fn read_resx(path: &Path) -> BTreeMap<String, String> {
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let document = roxmltree::Document::parse(&text).unwrap();
    let mut table = BTreeMap::new();
    for data in document
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("data"))
    {
        // Binary resources (icons, bitmaps) carry a type or a mime type; only text is used.
        if data.attribute("type").is_some() || data.attribute("mimetype").is_some() {
            continue;
        }
        let Some(name) = data.attribute("name") else {
            continue;
        };
        let value = data
            .children()
            .find(|n| n.has_tag_name("value"))
            .and_then(|n| n.text())
            .unwrap_or("")
            .to_string();
        table.insert(name.to_string(), value);
    }
    table
}

fn strings(dir: &Path) -> String {
    let mut names = Vec::new();
    let mut english = Vec::new();
    let mut polish = Vec::new();
    // Whether the Polish table really has the text (not the English fallback below).
    let mut translated = Vec::new();
    let mut modules = String::new();

    let mac_dir = dir.join("../../mac/l10n");
    let windows_dir = dir.join("../../windows/l10n");
    let port_dir = dir.join("../../port/l10n");
    for (project, module) in PROJECTS {
        let dir = match *project {
            "Mac" => mac_dir.as_path(),
            "Windows" => windows_dir.as_path(),
            "Port" => port_dir.as_path(),
            _ => dir,
        };
        let en = read_resx(&dir.join(format!("{project}.resx")));
        let pl = read_resx(&dir.join(format!("{project}.pl.resx")));
        writeln!(
            modules,
            "/// Strings of upstream `{project}/Localization/Strings.resx`."
        )
        .unwrap();
        writeln!(modules, "#[allow(non_upper_case_globals, dead_code)]").unwrap();
        writeln!(modules, "pub mod {module} {{").unwrap();
        writeln!(modules, "    use super::Key;").unwrap();
        for (name, value) in &en {
            let index = names.len();
            names.push(format!("{project}:{name}"));
            english.push(value.clone());
            // A key the Polish table lacks falls back to English, like .NET does.
            polish.push(pl.get(name).cloned().unwrap_or_else(|| value.clone()));
            translated.push(
                pl.get(name)
                    .is_some_and(|text| !text.trim().is_empty() || value.trim().is_empty()),
            );
            writeln!(modules, "    pub const {name}: Key = Key({index});").unwrap();
        }
        writeln!(modules, "}}").unwrap();
    }

    let mut out = String::new();
    writeln!(
        out,
        "// Generated by modlight-core/build.rs from assets/upstream/l10n. Do not edit."
    )
    .unwrap();
    writeln!(out, "pub(crate) const COUNT: usize = {};", names.len()).unwrap();
    writeln!(out, "pub(crate) static NAMES: [&str; COUNT] = [").unwrap();
    for name in &names {
        writeln!(out, "    {name:?},").unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out, "pub(crate) static TRANSLATED: [bool; COUNT] = [").unwrap();
    for present in &translated {
        writeln!(out, "    {present},").unwrap();
    }
    writeln!(out, "];").unwrap();
    for (table, values) in [("ENGLISH", &english), ("POLISH", &polish)] {
        writeln!(out, "pub(crate) static {table}: [&str; COUNT] = [").unwrap();
        for value in values.iter() {
            writeln!(out, "    {value:?},").unwrap();
        }
        writeln!(out, "];").unwrap();
    }
    out.push_str(&modules);
    out
}

/// The colour keys every skin dictionary carries, in the order of `Palette`'s fields.
const COLOR_KEYS: &[(&str, &str)] = &[
    ("ShareX.Color.Accent.Start", "accent_start"),
    ("ShareX.Color.Accent.End", "accent_end"),
    ("ShareX.Color.Accent.Foreground", "accent_foreground"),
    ("ShareX.Color.Skin.Accent", "skin_accent"),
    (
        "ShareX.Color.Skin.Accent.Foreground",
        "skin_accent_foreground",
    ),
    ("ShareX.Color.Text", "text"),
    ("ShareX.Color.Text.Secondary", "text_secondary"),
    ("ShareX.Color.Text.OnPanel", "text_on_panel"),
    (
        "ShareX.Color.Text.Secondary.OnPanel",
        "text_secondary_on_panel",
    ),
    ("ShareX.Color.Focus", "focus"),
    ("ShareX.Color.Focus.OnPanel", "focus_on_panel"),
    ("ShareX.Color.Background.Main", "background_main"),
    ("ShareX.Color.Background.Panel", "background_panel"),
    ("ShareX.Color.Background.Popup", "background_popup"),
    ("ShareX.Color.Background.Menu", "background_menu"),
    ("ShareX.Color.Background.Toolbar", "background_toolbar"),
    ("ShareX.Color.Border", "border"),
    ("ShareX.Color.Control.Background", "control_background"),
    (
        "ShareX.Color.Control.Background.Hover",
        "control_background_hover",
    ),
    (
        "ShareX.Color.Control.Background.Disabled",
        "control_background_disabled",
    ),
    ("ShareX.Color.Control.Border", "control_border"),
    ("ShareX.Color.Separator", "separator"),
    (
        "ShareX.Color.TaskMode.Cancel.Background",
        "cancel_background",
    ),
    ("ShareX.Color.TaskMode.Cancel.Border", "cancel_border"),
    (
        "ShareX.Color.TaskMode.Continue.Background",
        "continue_background",
    ),
    (
        "ShareX.Color.TaskMode.Continue.Background.Hover",
        "continue_background_hover",
    ),
    ("ShareX.Color.TaskMode.Continue.Border", "continue_border"),
    ("ShareX.Color.TaskMode.Foreground", "task_foreground"),
    ("ShareX.Color.Checker.A", "checker_a"),
    ("ShareX.Color.Checker.B", "checker_b"),
];

fn parse_color(hex: &str) -> [u8; 4] {
    let hex = hex.trim().trim_start_matches('#');
    let value = u32::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad colour {hex}"));
    match hex.len() {
        6 => [(value >> 16) as u8, (value >> 8) as u8, value as u8, 255],
        8 => [
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
            (value >> 24) as u8,
        ],
        _ => panic!("bad colour {hex}"),
    }
}

/// `"Mgławica"` as C# wrote it, decoded.
fn unescape_csharp(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'u') {
            chars.next();
            let hex: String = (0..4).filter_map(|_| chars.next()).collect();
            out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

/// The skins: upstream's, after the app's own (`extra`, the same format), which comes
/// first and is the default.
fn skins(dir: &Path, extra: &Path) -> String {
    let axaml_path = dir.join("Skins.axaml");
    let variants_path = dir.join("SkinVariants.g.cs");
    println!("cargo:rerun-if-changed={}", axaml_path.display());
    println!("cargo:rerun-if-changed={}", variants_path.display());

    // Palettes, keyed by the SkinVariants member name (e.g. "Default_Dark").
    println!("cargo:rerun-if-changed={}", extra.display());
    let mut palettes: BTreeMap<String, BTreeMap<String, [u8; 4]>> = BTreeMap::new();
    for path in [axaml_path.as_path(), extra] {
        let text = std::fs::read_to_string(path).unwrap();
        let document = roxmltree::Document::parse(&text).unwrap();
        for dictionary in document
            .descendants()
            .filter(|n| n.has_tag_name("ResourceDictionary"))
        {
            let Some(key) = dictionary
                .attributes()
                .find(|a| a.name() == "Key")
                .map(|a| a.value().to_string())
            else {
                continue;
            };
            let Some(variant) = key
                .strip_prefix("{x:Static helpers:SkinVariants.")
                .and_then(|k| k.strip_suffix('}'))
            else {
                continue;
            };
            let mut colors = BTreeMap::new();
            for color in dictionary.children().filter(|n| n.has_tag_name("Color")) {
                let name = color
                    .attributes()
                    .find(|a| a.name() == "Key")
                    .unwrap()
                    .value();
                colors.insert(name.to_string(), parse_color(color.text().unwrap_or("")));
            }
            palettes.insert(variant.to_string(), colors);
        }
    }

    // Ids and names, in the picker's order, from SkinVariants.g.cs.
    let variants = std::fs::read_to_string(&variants_path).unwrap();
    let mut skins = Vec::new();
    let mut default_id = String::new();
    for line in variants.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("public const string DefaultSkinId = \"") {
            default_id = rest.trim_end_matches("\";").to_string();
        }
        if let Some(rest) = line.strip_prefix("new(\"") {
            let parts: Vec<&str> = rest.trim_end_matches("),").split(", ").collect();
            let field = |i: usize| unescape_csharp(parts[i].trim_matches('"'));
            skins.push((
                field(0),
                field(1),
                field(2),
                parts[3].to_string(),
                parts[4].to_string(),
            ));
        }
    }
    assert!(
        !skins.is_empty() && !default_id.is_empty(),
        "SkinVariants.g.cs not understood"
    );
    // The app's own skin leads the list and is what an account starts with.
    skins.insert(
        0,
        (
            "graphite".to_string(),
            "Graphite".to_string(),
            "Grafit".to_string(),
            "Graphite_Light".to_string(),
            "Graphite_Dark".to_string(),
        ),
    );
    default_id = "graphite".to_string();

    let palette = |variant: &str| -> String {
        let colors = palettes
            .get(variant)
            .unwrap_or_else(|| panic!("no palette {variant}"));
        let mut out = String::from("Palette {\n");
        for (key, field) in COLOR_KEYS {
            let c = colors
                .get(*key)
                .unwrap_or_else(|| panic!("{variant} lacks {key}"));
            writeln!(
                out,
                "            {field}: Rgba([{}, {}, {}, {}]),",
                c[0], c[1], c[2], c[3]
            )
            .unwrap();
        }
        out.push_str("        }");
        out
    };

    let mut out = String::new();
    writeln!(
        out,
        "// Generated by modlight-core/build.rs from assets/upstream/theming. Do not edit."
    )
    .unwrap();
    writeln!(out, "pub const DEFAULT_SKIN_ID: &str = {default_id:?};").unwrap();
    writeln!(out, "pub static SKINS: &[Skin] = &[").unwrap();
    for (id, english, polish, light, dark) in &skins {
        writeln!(out, "    Skin {{").unwrap();
        writeln!(out, "        id: {id:?},").unwrap();
        writeln!(out, "        name_english: {english:?},").unwrap();
        writeln!(out, "        name_polish: {polish:?},").unwrap();
        writeln!(out, "        light: {},", palette(light)).unwrap();
        writeln!(out, "        dark: {},", palette(dark)).unwrap();
        writeln!(out, "    }},").unwrap();
    }
    writeln!(out, "];").unwrap();
    out
}

fn lucide(path: &Path) -> String {
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(path).unwrap();
    let map: BTreeMap<String, u32> = serde_json::from_str(&text).unwrap();
    let mut out = String::new();
    writeln!(
        out,
        "// Generated by modlight-core/build.rs from lucide-codepoints.json. Do not edit."
    )
    .unwrap();
    writeln!(
        out,
        "// Names follow upstream LucideIcons.cs (kebab-case to SCREAMING_SNAKE_CASE)."
    )
    .unwrap();
    for (name, code) in &map {
        let ident = name.replace('-', "_").to_ascii_uppercase();
        let ident = if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            format!("_{ident}")
        } else {
            ident
        };
        writeln!(out, "pub const {ident}: char = '\\u{{{code:x}}}';").unwrap();
    }
    out
}

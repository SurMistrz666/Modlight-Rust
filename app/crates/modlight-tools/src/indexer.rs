//! The directory indexer (`Indexer`, `IndexerText`, `IndexerHtml`, `IndexerXml`,
//! `IndexerJson`, `FolderInfo`): a folder tree as text, HTML, XML or JSON, with sizes,
//! counts and a footer, as upstream writes them. Line breaks are the system's
//! (`Environment.NewLine`: CRLF on Windows, `\n` on a Mac).
//!
//! A hidden file or folder is what .NET reports as `FileAttributes.Hidden`: on Windows
//! the hidden attribute, on a Mac a name that starts with a dot or the `UF_HIDDEN` flag.
//! Symbolic links to folders are listed but not followed.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use modlight_core::format::local_size;
use modlight_core::l10n::{Arg, format, t, tools as tl};
use modlight_core::names::url_helpers::html_encode_numeric;

use crate::dotnet;
use crate::settings::{IndexerOutput, IndexerSettings};

/// Where "Latest version can be downloaded from" points: the Modlight App's own page
/// (upstream's `Links.Website` is ShareX's site, which the port leaves out).
pub const PROJECT_PAGE: &str = "https://github.com/Nimfek/Modlight-App";

/// The default style sheet (`IndexerDefault.css`).
pub const DEFAULT_CSS: &str = include_str!("../../../assets/upstream/tools/IndexerDefault.css");

/// A file of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
}

/// `FolderInfo`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderInfo {
    pub path: PathBuf,
    pub files: Vec<FileEntry>,
    pub folders: Vec<FolderInfo>,
    pub size: u64,
    pub total_file_count: u64,
    pub total_folder_count: u64,
}

impl FolderInfo {
    /// `FolderName`: `Path.GetFileName`.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.total_file_count == 0 && self.total_folder_count == 0
    }

    /// `Update`: sizes and counts from the leaves up, sub-folders sorted by name.
    pub fn update(&mut self) {
        for folder in &mut self.folders {
            folder.update();
        }
        self.folders
            .sort_by(|a, b| culture_compare(&a.name(), &b.name()));
        self.size = self.folders.iter().map(|f| f.size).sum::<u64>()
            + self.files.iter().map(|f| f.size).sum::<u64>();
        self.total_file_count =
            self.files.len() as u64 + self.folders.iter().map(|f| f.total_file_count).sum::<u64>();
        self.total_folder_count = self.folders.len() as u64
            + self
                .folders
                .iter()
                .map(|f| f.total_folder_count)
                .sum::<u64>();
    }
}

/// `string.CompareTo` (culture-aware): letters compared without case first, then lower
/// case before upper case.
pub fn culture_compare(a: &str, b: &str) -> Ordering {
    let fold = |s: &str| s.to_lowercase();
    fold(a).cmp(&fold(b)).then_with(|| {
        // On a tie, the lower-case spelling first (ICU's tertiary order).
        b.cmp(a)
    })
}

/// `FileAttributes.Hidden` on Windows: the entry's own hidden attribute.
#[cfg(windows)]
pub fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

/// Hidden on a Mac: a dot first, or `UF_HIDDEN`.
#[cfg(not(windows))]
pub fn is_hidden(path: &Path) -> bool {
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            return meta.st_flags() & UF_HIDDEN != 0;
        }
    }
    false
}

/// `GetFolderInfo`: the tree under `folder` down to `MaxDepthLevel` (0: unlimited).
/// A folder that cannot be read is listed empty (`UnauthorizedAccessException`).
pub fn folder_info(folder: &Path, settings: &IndexerSettings, level: i64) -> FolderInfo {
    let mut info = FolderInfo {
        path: folder.to_path_buf(),
        ..FolderInfo::default()
    };
    if settings.max_depth_level != 0 && level >= settings.max_depth_level {
        return info;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return info;
    };
    let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    for entry in &entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // A link to a folder is a folder (listed, not followed).
        let is_dir = kind.is_dir() || (kind.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }
        if settings.skip_hidden_folders && is_hidden(&path) {
            continue;
        }
        if kind.is_symlink() {
            info.folders.push(FolderInfo {
                path,
                ..FolderInfo::default()
            });
        } else {
            info.folders.push(folder_info(&path, settings, level + 1));
        }
    }
    if !settings.skip_files {
        for entry in &entries {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let is_file = kind.is_file() || (kind.is_symlink() && path.is_file());
            if !is_file {
                continue;
            }
            if settings.skip_hidden_files && is_hidden(&path) {
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            info.files.push(FileEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                size,
            });
        }
        info.files.sort_by(|a, b| culture_compare(&a.name, &b.name));
    }
    info
}

fn size_text(size: u64, settings: &IndexerSettings) -> String {
    local_size(size as i64, settings.binary_units, 2)
}

/// `Indexer.Index(folderPath, settings)`.
pub fn index(folder: &Path, settings: &IndexerSettings, now: DateTime<Utc>) -> String {
    let mut info = folder_info(folder, settings, 0);
    info.update();
    let out = match settings.output {
        IndexerOutput::Txt => text(&info, settings, now),
        IndexerOutput::Html => html(folder, &info, settings, now),
        IndexerOutput::Xml => xml(&info, settings),
        IndexerOutput::Json => json(&info, settings),
    };
    system_line_breaks(out)
}

/// The writers build their lines with `\n`; on Windows every line break is CRLF, as
/// `AppendLine`, `XmlWriter` and Json.NET write them there (a CRLF already in a custom
/// style sheet stays one).
fn system_line_breaks(text: String) -> String {
    if !cfg!(windows) {
        return text;
    }
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut previous = '\0';
    for c in text.chars() {
        if c == '\n' && previous != '\r' {
            out.push('\r');
        }
        out.push(c);
        previous = c;
    }
    out
}

// ------------------------------------------------------------------ text

/// `IndexerText`.
pub fn text(info: &FolderInfo, settings: &IndexerSettings, now: DateTime<Utc>) -> String {
    let mut content = String::new();
    text_folder(&mut content, info, settings, 0);
    let mut out = String::new();
    out.push_str(content.trim());
    out.push('\n');
    if settings.add_footer {
        let footer = dotnet::format_with_date(
            t(tl::IndexerText_Generated_by),
            0,
            now,
            &[Arg::from(""), Arg::from(PROJECT_PAGE)],
        );
        out.push_str(&"_".repeat(footer.encode_utf16().count()));
        out.push('\n');
        out.push_str(&footer);
        out.push('\n');
    }
    out.trim().to_string()
}

fn text_folder(out: &mut String, dir: &FolderInfo, settings: &IndexerSettings, level: usize) {
    let mut row = format!("{}{}", settings.indentation_text.repeat(level), dir.name());
    if settings.show_size_info && dir.size > 0 {
        row.push_str(&format!(" [{}]", size_text(dir.size, settings)));
    }
    out.push_str(&row);
    out.push('\n');
    for sub in &dir.folders {
        if settings.add_empty_line_after_folders {
            out.push('\n');
        }
        text_folder(out, sub, settings, level + 1);
    }
    if !dir.files.is_empty() {
        if settings.add_empty_line_after_folders {
            out.push('\n');
        }
        for file in &dir.files {
            let mut row = format!(
                "{}{}",
                settings.indentation_text.repeat(level + 1),
                file.name
            );
            if settings.show_size_info {
                row.push_str(&format!(" [{}]", size_text(file.size, settings)));
            }
            out.push_str(&row);
            out.push('\n');
        }
    }
}

// ------------------------------------------------------------------ HTML

fn start_tag(tag: &str, fields: &str) -> String {
    if fields.is_empty() {
        format!("<{tag}>")
    } else {
        format!("<{tag} {fields}>")
    }
}

fn tag(name: &str, content: &str, fields: &str) -> String {
    format!(
        "{}{}</{name}>",
        start_tag(name, fields),
        html_encode_numeric(content)
    )
}

/// `IndexerHtml`.
pub fn html(
    folder: &Path,
    info: &FolderInfo,
    settings: &IndexerSettings,
    now: DateTime<Utc>,
) -> String {
    let mut out = String::new();
    let mut line = |s: &str| {
        out.push_str(s);
        out.push('\n');
    };
    line("<!DOCTYPE html>");
    line("<html>");
    line("<head>");
    line("<meta charset=\"UTF-8\">");
    line("<meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\">");
    let folder_name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    line(&tag(
        "title",
        &format(t(tl::IndexerHtml_Index_for), &[Arg::from(folder_name)]),
        "",
    ));
    line(&css_style(settings));
    line("</head>");
    line("<body>");
    // `TrimEnd('\\')` and `LastIndexOf(@"\")` upstream: the system's separator.
    let separator = std::path::MAIN_SEPARATOR;
    let full = folder.to_string_lossy();
    let full = full.trim_end_matches(separator);
    let pre_trim = full.rfind(separator).map(|i| i + 1).unwrap_or(0);
    let mut content = String::new();
    html_folder(&mut content, info, settings, 0, pre_trim);
    line(content.trim());
    if settings.add_footer {
        let link = format!("<a href=\"{PROJECT_PAGE}\">Modlight Directory Indexer</a>");
        let footer = dotnet::format_with_date(
            t(tl::IndexerHtml_Generated_by_on),
            1,
            now,
            &[Arg::from(link)],
        );
        line(&format!("<div>{footer}</div>"));
    }
    line("</body>");
    line("</html>");
    out.trim().to_string()
}

fn css_style(settings: &IndexerSettings) -> String {
    let custom = settings.use_custom_css_file
        && !settings.custom_css_file_path.is_empty()
        && Path::new(&settings.custom_css_file_path).is_file();
    let css = if custom {
        std::fs::read_to_string(&settings.custom_css_file_path).unwrap_or_default()
    } else {
        DEFAULT_CSS.to_string()
    };
    let css = css.trim_start_matches('\u{feff}');
    format!("<style type=\"text/css\">\n{css}\n</style>")
}

fn html_folder(
    out: &mut String,
    dir: &FolderInfo,
    settings: &IndexerSettings,
    level: usize,
    pre_trim: usize,
) {
    out.push_str(&html_folder_row(dir, settings, level, pre_trim));
    out.push('\n');
    let class = if level > 0 {
        "FolderBorder"
    } else {
        "MainFolderBorder"
    };
    out.push_str(&start_tag("div", &format!("class=\"{class}\"")));
    out.push('\n');
    if !dir.files.is_empty() {
        out.push_str("<ul>\n");
        for file in &dir.files {
            let mut row = format!("<li>{}", html_encode_numeric(&file.name));
            if settings.show_size_info {
                row.push(' ');
                row.push_str(&tag(
                    "span",
                    &size_text(file.size, settings),
                    "class=\"FileSize\"",
                ));
            }
            row.push_str("</li>");
            out.push_str(&row);
            out.push('\n');
        }
        out.push_str("</ul>\n");
    }
    for sub in &dir.folders {
        html_folder(out, sub, settings, level + 1, pre_trim);
    }
    out.push_str("</div>\n");
}

fn html_folder_row(
    dir: &FolderInfo,
    settings: &IndexerSettings,
    level: usize,
    pre_trim: usize,
) -> String {
    let mut info = String::new();
    if !dir.is_empty() {
        if settings.show_size_info {
            info.push_str(&size_text(dir.size, settings));
            info.push(' ');
        }
        info.push('(');
        if dir.total_file_count > 0 {
            let key = if dir.total_file_count == 1 {
                tl::IndexerHtml_One_file
            } else {
                tl::IndexerHtml_File_count
            };
            info.push_str(&format(t(key), &[Arg::from(dir.total_file_count as i64)]));
        }
        if dir.total_folder_count > 0 {
            if dir.total_file_count > 0 {
                info.push_str(", ");
            }
            let key = if dir.total_folder_count == 1 {
                tl::IndexerHtml_One_folder
            } else {
                tl::IndexerHtml_Folder_count
            };
            info.push_str(&format(t(key), &[Arg::from(dir.total_folder_count as i64)]));
        }
        info.push(')');
        info = format!(" {}", tag("span", &info, "class=\"FolderInfo\""));
    }
    let path = dir.path.to_string_lossy().into_owned();
    let title = if settings.display_path {
        if settings.display_path_limited {
            path.get(pre_trim.min(path.len())..)
                .unwrap_or("")
                .to_string()
        } else {
            path
        }
    } else {
        dir.name()
    };
    let heading = (level + 1).clamp(1, 6);
    format!(
        "<h{heading}>{}{info}</h{heading}>",
        html_encode_numeric(&title)
    )
}

// ------------------------------------------------------------------ XML

fn xml_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            '\t' => out.push_str("&#x9;"),
            c => out.push(c),
        }
    }
    out
}

fn xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// `IndexerXml`: `XmlWriter` with `Indent = true` (two spaces, `" />"` for an empty
/// element, the UTF-8 declaration).
pub fn xml(info: &FolderInfo, settings: &IndexerSettings) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>");
    xml_folder(&mut out, info, settings, 0);
    out
}

fn indent(out: &mut String, depth: usize) {
    out.push('\n');
    out.push_str(&"  ".repeat(depth));
}

fn xml_item(
    out: &mut String,
    name: &str,
    size: Option<String>,
    element: &str,
    settings: &IndexerSettings,
    depth: usize,
    children: impl FnOnce(&mut String),
) {
    indent(out, depth);
    out.push('<');
    out.push_str(element);
    let mut has_children = false;
    let mut body = String::new();
    if settings.use_attribute {
        out.push_str(&format!(" Name=\"{}\"", xml_attr(name)));
        if let Some(size) = &size {
            out.push_str(&format!(" Size=\"{}\"", xml_attr(size)));
        }
    } else {
        indent(&mut body, depth + 1);
        body.push_str(&format!("<Name>{}</Name>", xml_text(name)));
        if let Some(size) = &size {
            indent(&mut body, depth + 1);
            body.push_str(&format!("<Size>{}</Size>", xml_text(size)));
        }
        has_children = true;
    }
    let before = body.len();
    children(&mut body);
    if body.len() > before {
        has_children = true;
    }
    if has_children {
        out.push('>');
        out.push_str(&body);
        indent(out, depth);
        out.push_str(&format!("</{element}>"));
    } else {
        out.push_str(" />");
    }
}

fn xml_folder(out: &mut String, dir: &FolderInfo, settings: &IndexerSettings, depth: usize) {
    let size = (settings.show_size_info && !dir.is_empty()).then(|| size_text(dir.size, settings));
    xml_item(out, &dir.name(), size, "Folder", settings, depth, |body| {
        if !dir.files.is_empty() {
            indent(body, depth + 1);
            body.push_str("<Files>");
            for file in &dir.files {
                let size = settings
                    .show_size_info
                    .then(|| size_text(file.size, settings));
                xml_item(body, &file.name, size, "File", settings, depth + 2, |_| {});
            }
            indent(body, depth + 1);
            body.push_str("</Files>");
        }
        if !dir.folders.is_empty() {
            indent(body, depth + 1);
            body.push_str("<Folders>");
            for sub in &dir.folders {
                xml_folder(body, sub, settings, depth + 2);
            }
            indent(body, depth + 1);
            body.push_str("</Folders>");
        }
    });
}

// ------------------------------------------------------------------ JSON

/// `IndexerJson`: Newtonsoft's indented writer (two spaces).
pub fn json(info: &FolderInfo, settings: &IndexerSettings) -> String {
    use serde_json::{Map, Value};
    fn parseable(dir: &FolderInfo, settings: &IndexerSettings, object: &mut Map<String, Value>) {
        object.insert("Name".into(), Value::String(dir.name()));
        if settings.show_size_info {
            object.insert("Size".into(), Value::String(size_text(dir.size, settings)));
        }
        if !dir.folders.is_empty() {
            let folders = dir
                .folders
                .iter()
                .map(|sub| {
                    let mut child = Map::new();
                    parseable(sub, settings, &mut child);
                    Value::Object(child)
                })
                .collect();
            object.insert("Folders".into(), Value::Array(folders));
        }
        if !dir.files.is_empty() {
            let files = dir
                .files
                .iter()
                .map(|file| {
                    let mut child = Map::new();
                    child.insert("Name".into(), Value::String(file.name.clone()));
                    if settings.show_size_info {
                        child.insert("Size".into(), Value::String(size_text(file.size, settings)));
                    }
                    Value::Object(child)
                })
                .collect();
            object.insert("Files".into(), Value::Array(files));
        }
    }
    fn simple(dir: &FolderInfo, object: &mut Map<String, Value>) {
        let mut items: Vec<Value> = dir
            .folders
            .iter()
            .map(|sub| {
                let mut child = Map::new();
                simple(sub, &mut child);
                Value::Object(child)
            })
            .collect();
        items.extend(dir.files.iter().map(|f| Value::String(f.name.clone())));
        object.insert(dir.name(), Value::Array(items));
    }
    let mut root = Map::new();
    if settings.create_parseable_json {
        parseable(info, settings, &mut root);
    } else {
        simple(info, &mut root);
    }
    serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use modlight_core::culture::{Culture, set_thread_culture};
    use modlight_core::l10n::{Language, set_thread_language};

    /// The odd file name: `<>` cannot be in a Windows file name, braces can.
    const ODD: &str = if cfg!(windows) {
        "c & {d}.bin"
    } else {
        "c & <d>.bin"
    };

    /// An expectation written for macOS, with this system's odd name.
    fn here(expected: &str) -> String {
        expected.replace("c & <d>.bin", ODD).replace(
            "c &amp; &lt;d&gt;.bin",
            if cfg!(windows) {
                "c &amp; {d}.bin"
            } else {
                "c &amp; &lt;d&gt;.bin"
            },
        )
    }

    /// The output with `\n` line breaks; on Windows it must have had CRLF only.
    fn lf(text: String) -> String {
        if cfg!(windows) {
            let without = text.replace("\r\n", "");
            assert!(!without.contains('\n'), "a line break without CR");
            text.replace("\r\n", "\n")
        } else {
            text
        }
    }

    /// docs/ (a.txt 1500 B, B.md 10 B, .hidden), docs/sub/ (c.bin 2048 B), docs/empty/.
    /// On Windows `.hidden` and `.git` carry the hidden attribute, which is what makes
    /// them hidden there.
    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("docs");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("a.txt"), vec![b'a'; 1500]).unwrap();
        std::fs::write(root.join("B.md"), vec![b'b'; 10]).unwrap();
        std::fs::write(root.join(".hidden"), b"h").unwrap();
        std::fs::write(root.join("sub").join(ODD), vec![0u8; 2048]).unwrap();
        #[cfg(windows)]
        for name in [".hidden", ".git"] {
            let made = std::process::Command::new("attrib.exe")
                .arg("+h")
                .arg(root.join(name))
                .status()
                .unwrap();
            assert!(made.success());
        }
        dir
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 20, 14, 5, 9).unwrap()
    }

    fn setup() {
        set_thread_language(Some(Language::English));
        set_thread_culture(Some(Culture::english()));
    }

    #[test]
    fn the_tree() {
        let dir = tree();
        let mut info = folder_info(&dir.path().join("docs"), &IndexerSettings::default(), 0);
        info.update();
        let names: Vec<String> = info.folders.iter().map(|f| f.name()).collect();
        assert_eq!(names, ["empty", "sub"]);
        let files: Vec<&str> = info.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(files, ["a.txt", "B.md"]);
        assert_eq!(
            (info.size, info.total_file_count, info.total_folder_count),
            (3558, 3, 2)
        );
        let settings = IndexerSettings {
            max_depth_level: 1,
            skip_hidden_folders: false,
            skip_files: true,
            ..IndexerSettings::default()
        };
        let mut info = folder_info(&dir.path().join("docs"), &settings, 0);
        info.update();
        assert_eq!(info.folders.len(), 3);
        assert!(info.files.is_empty());
        assert!(
            info.folders
                .iter()
                .all(|f| f.files.is_empty() && f.folders.is_empty())
        );
    }

    #[test]
    fn text_output() {
        setup();
        let dir = tree();
        let settings = IndexerSettings {
            output: IndexerOutput::Txt,
            ..IndexerSettings::default()
        };
        let text = lf(index(&dir.path().join("docs"), &settings, now()));
        let expected = &here(
            "docs [3.56 KB]\n|___empty\n|___sub [2.05 KB]\n|___|___c & <d>.bin [2.05 KB]\n|___a.txt [1.50 KB]\n|___B.md [10 B]\n",
        );
        assert!(text.starts_with(expected), "{text}");
        let footer = "Generated by Modlight Directory Indexer on 2026-10-20 at 14:05:09 UTC. Latest version can be downloaded from: https://github.com/Nimfek/Modlight-App";
        assert!(
            text.ends_with(&format!("{}\n{footer}", "_".repeat(footer.len()))),
            "{text}"
        );
    }

    #[test]
    fn text_with_empty_lines_and_no_sizes() {
        setup();
        let dir = tree();
        let settings = IndexerSettings {
            output: IndexerOutput::Txt,
            show_size_info: false,
            add_footer: false,
            add_empty_line_after_folders: true,
            indentation_text: "  ".into(),
            ..IndexerSettings::default()
        };
        let text = lf(index(&dir.path().join("docs"), &settings, now()));
        assert_eq!(
            text,
            here("docs\n\n  empty\n\n  sub\n\n    c & <d>.bin\n\n  a.txt\n  B.md")
        );
    }

    #[test]
    fn html_output() {
        setup();
        let dir = tree();
        let text = lf(index(
            &dir.path().join("docs"),
            &IndexerSettings::default(),
            now(),
        ));
        assert!(text.starts_with("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"UTF-8\">\n<meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\">\n<title>Index for docs</title>\n<style type=\"text/css\">\nbody {"));
        assert!(text.contains("<h1>docs <span class=\"FolderInfo\">3.56 KB (3 files, 2 folders)</span></h1>\n<div class=\"MainFolderBorder\">\n<ul>\n<li>a.txt <span class=\"FileSize\">1.50 KB</span></li>\n<li>B.md <span class=\"FileSize\">10 B</span></li>\n</ul>\n<h2>empty</h2>\n<div class=\"FolderBorder\">\n</div>\n<h2>sub <span class=\"FolderInfo\">2.05 KB (1 file)</span></h2>"), "{text}");
        assert!(text.contains(&here("<li>c &amp; &lt;d&gt;.bin")));
        assert!(text.ends_with("<div>Generated by <a href=\"https://github.com/Nimfek/Modlight-App\">Modlight Directory Indexer</a> on 2026-10-20 at 14:05:09 UTC</div>\n</body>\n</html>"), "{text}");
        let settings = IndexerSettings {
            display_path: true,
            display_path_limited: true,
            add_footer: false,
            ..IndexerSettings::default()
        };
        let text = lf(index(&dir.path().join("docs"), &settings, now()));
        // The path as the system writes it (`docs\\sub` on Windows).
        let sub = format!("<h2>docs{}sub <span", std::path::MAIN_SEPARATOR);
        assert!(text.contains(&sub), "{text}");
    }

    #[test]
    fn xml_output() {
        setup();
        let dir = tree();
        let settings = IndexerSettings {
            output: IndexerOutput::Xml,
            ..IndexerSettings::default()
        };
        let text = lf(index(&dir.path().join("docs"), &settings, now()));
        assert_eq!(
            text,
            here(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Folder Name=\"docs\" Size=\"3.56 KB\">\n  <Files>\n    <File Name=\"a.txt\" Size=\"1.50 KB\" />\n    <File Name=\"B.md\" Size=\"10 B\" />\n  </Files>\n  <Folders>\n    <Folder Name=\"empty\" />\n    <Folder Name=\"sub\" Size=\"2.05 KB\">\n      <Files>\n        <File Name=\"c &amp; &lt;d&gt;.bin\" Size=\"2.05 KB\" />\n      </Files>\n    </Folder>\n  </Folders>\n</Folder>"
            )
        );
        let settings = IndexerSettings {
            output: IndexerOutput::Xml,
            use_attribute: false,
            show_size_info: false,
            ..IndexerSettings::default()
        };
        let text = lf(index(
            &dir.path().join("docs").join("sub"),
            &settings,
            now(),
        ));
        assert_eq!(
            text,
            here(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Folder>\n  <Name>sub</Name>\n  <Files>\n    <File>\n      <Name>c &amp; &lt;d&gt;.bin</Name>\n    </File>\n  </Files>\n</Folder>"
            )
        );
    }

    #[test]
    fn json_output() {
        setup();
        let dir = tree();
        let settings = IndexerSettings {
            output: IndexerOutput::Json,
            ..IndexerSettings::default()
        };
        let text = lf(index(
            &dir.path().join("docs").join("sub"),
            &settings,
            now(),
        ));
        assert_eq!(
            text,
            here(
                "{\n  \"Name\": \"sub\",\n  \"Size\": \"2.05 KB\",\n  \"Files\": [\n    {\n      \"Name\": \"c & <d>.bin\",\n      \"Size\": \"2.05 KB\"\n    }\n  ]\n}"
            )
        );
        let settings = IndexerSettings {
            output: IndexerOutput::Json,
            create_parseable_json: false,
            ..IndexerSettings::default()
        };
        let text = lf(index(&dir.path().join("docs"), &settings, now()));
        assert_eq!(
            text,
            here(
                "{\n  \"docs\": [\n    {\n      \"empty\": []\n    },\n    {\n      \"sub\": [\n        \"c & <d>.bin\"\n      ]\n    },\n    \"a.txt\",\n    \"B.md\"\n  ]\n}"
            )
        );
    }

    #[test]
    fn binary_units_and_polish() {
        set_thread_language(Some(Language::Polish));
        set_thread_culture(Some(Culture::polish()));
        let dir = tree();
        let settings = IndexerSettings {
            output: IndexerOutput::Txt,
            binary_units: true,
            add_footer: false,
            ..IndexerSettings::default()
        };
        let text = lf(index(&dir.path().join("docs"), &settings, now()));
        assert!(text.starts_with("docs [3,47 KiB]"), "{text}");
        set_thread_language(None);
        set_thread_culture(None);
    }
}

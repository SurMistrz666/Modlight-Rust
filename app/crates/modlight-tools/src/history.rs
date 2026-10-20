//! The history windows' logic (ShareX.HistoryLib `HistoryFilter`, `HistoryHelpers`, and
//! the parts of `HistoryWindow` / `ImageHistoryWindow` that are not drawing): filters,
//! statistics, the copy formats, the context menu's states, tags as text, the folder
//! import. The rows themselves are `modlight_core::history`'s, read from History.db.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use modlight_core::history::{HistoryItem, HistoryTags};
use modlight_core::l10n::{Arg, format, history as hl, t};
use modlight_core::names::file_helpers;
use regex::Regex;

/// `HistoryWindows`' wildcard search: `?` one character, `*` any, case ignored.
pub fn wildcard(pattern: &str) -> Option<Regex> {
    let escaped = regex::escape(pattern)
        .replace("\\?", ".")
        .replace("\\*", ".*");
    regex::RegexBuilder::new(&escaped)
        .case_insensitive(true)
        .build()
        .ok()
}

/// The item's time in this computer's zone (UTC in QA renders).
fn local_date(item: &HistoryItem) -> Option<NaiveDate> {
    let at = item.date_time?;
    Some(if modlight_core::culture::thread_utc() {
        at.date_naive()
    } else {
        at.with_timezone(&Local).date_naive()
    })
}

/// `HistoryFilter`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub file_name: String,
    pub url: String,
    pub filter_date: bool,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
    pub filter_type: bool,
    pub kind: Option<String>,
    pub filter_host: bool,
    pub host: String,
    pub favorites: bool,
    pub max_item_count: usize,
}

impl Filter {
    /// `ApplyFilter`: the indices of the items kept, in order.
    pub fn apply(&self, items: &[HistoryItem]) -> Vec<usize> {
        let regex = if self.file_name.is_empty() || self.favorites {
            None
        } else {
            wildcard(&self.file_name)
        };
        let host = self.host.to_lowercase();
        let url = self.url.to_lowercase();
        let mut out = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let keep = if self.favorites {
                item.favorite()
            } else {
                (!self.filter_type
                    || self.kind.as_deref().is_none_or(str::is_empty)
                    || (!item.kind.is_empty()
                        && Some(item.kind.to_lowercase())
                            == self.kind.as_ref().map(|k| k.to_lowercase())))
                    && (!self.filter_host
                        || host.is_empty()
                        || (!item.host.is_empty() && item.host.to_lowercase().contains(&host)))
                    && regex.as_ref().is_none_or(|r| matches_name_or_tags(r, item))
                    && (url.is_empty() || item.url.to_lowercase().contains(&url))
                    && (!self.filter_date || {
                        let date = local_date(item).unwrap_or(NaiveDate::MIN);
                        self.from_date.is_none_or(|from| date >= from)
                            && self.to_date.is_none_or(|to| date <= to)
                    })
            };
            if keep {
                out.push(index);
                if self.max_item_count > 0 && out.len() >= self.max_item_count {
                    break;
                }
            }
        }
        out
    }
}

/// The file name, or a tag's value, matches.
fn matches_name_or_tags(regex: &Regex, item: &HistoryItem) -> bool {
    regex.is_match(&item.file_name)
        || item.tags.as_ref().is_some_and(|tags| {
            tags.iter()
                .any(|(_, v)| v.is_some_and(|v| !v.is_empty() && regex.is_match(v)))
        })
}

/// `ImageHistoryWindow.ApplyFilterAsync`: items with a file, favourites, pictures only,
/// files that exist, a search over names and tags.
pub fn image_filter(
    items: &[HistoryItem],
    search: &str,
    favorites: bool,
    image_only: bool,
    filter_missing: bool,
) -> Vec<usize> {
    let search = search.trim();
    let regex = if search.is_empty() {
        None
    } else {
        wildcard(search)
    };
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            !item.file_path.trim().is_empty()
                && (!favorites || item.favorite())
                && (!image_only || file_helpers::is_image_file(&item.file_path))
                && (!filter_missing || Path::new(&item.file_path).is_file())
                && regex.as_ref().is_none_or(|r| matches_name_or_tags(r, item))
        })
        .map(|(i, _)| i)
        .collect()
}

/// The distinct types and hosts (`TypeFilterComboBox`, `HostFilterComboBox`): case
/// folded, sorted.
pub fn types_and_hosts(items: &[HistoryItem]) -> (Vec<String>, Vec<String>) {
    let distinct = |values: Vec<&str>| {
        let mut seen = HashSet::new();
        let mut out: Vec<String> = values
            .into_iter()
            .filter(|v| !v.trim().is_empty() && seen.insert(v.to_lowercase()))
            .map(str::to_string)
            .collect();
        out.sort_by_key(|v| v.to_lowercase());
        out
    };
    (
        distinct(items.iter().map(|i| i.kind.as_str()).collect()),
        distinct(items.iter().map(|i| i.host.as_str()).collect()),
    )
}

/// `HistoryItemIconConverter`.
pub fn icon(item: &HistoryItem) -> char {
    use modlight_core::icons;
    if item.favorite() {
        return icons::STAR;
    }
    match item.kind.to_lowercase().as_str() {
        "image" => icons::IMAGE,
        "text" => icons::FILE_TEXT,
        "file" => icons::FILE,
        _ => icons::LINK,
    }
}

/// The list's date column (`{0:g}`).
pub fn date_text(item: &HistoryItem) -> String {
    match item.date_time {
        Some(at) => modlight_core::culture::local_short(&at),
        None => String::new(),
    }
}

/// The edit dialog's date (`DateTime.ToString("G")`).
pub fn date_long_text(item: &HistoryItem) -> String {
    let Some(at) = item.date_time else {
        return String::new();
    };
    if modlight_core::culture::thread_utc() {
        crate::dotnet::general(&at)
    } else {
        crate::dotnet::general(&at.with_timezone(&Local))
    }
}

/// Groups in order of first appearance, sorted by size (a stable sort, as LINQ's).
fn grouped<K: PartialEq + Clone>(keys: impl Iterator<Item = K>) -> Vec<(K, usize)> {
    let mut groups: Vec<(K, usize)> = Vec::new();
    for key in keys {
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some(group) => group.1 += 1,
            None => groups.push((key, 1)),
        }
    }
    groups.sort_by_key(|a| std::cmp::Reverse(a.1));
    groups
}

fn local_year(item: &HistoryItem) -> i32 {
    match item.date_time {
        Some(at) if modlight_core::culture::thread_utc() => at.year(),
        Some(at) => at.with_timezone(&Local).year(),
        None => 1,
    }
}

/// `HistoryHelpers.OutputStats`.
pub fn stats(items: &[HistoryItem]) -> String {
    let empty = t(hl::HistoryHelpers_Empty);
    let total = items.len() as f64;
    let percent = |count: usize| count as f64 / total * 100.0;
    let mut lines: Vec<String> = Vec::new();
    lines.push(t(hl::HistoryHelpers_HistoryItemCounts).to_string());
    lines.push(format!("{} {}", t(hl::HistoryHelpers_Total), items.len()));
    lines.push(
        grouped(items.iter().map(|i| i.kind.clone()))
            .into_iter()
            .map(|(kind, count)| {
                format(
                    "{0}: {1} ({2:N0}%)",
                    &[
                        Arg::from(kind),
                        Arg::Int(count as i64),
                        Arg::Float(percent(count)),
                    ],
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    lines.push(String::new());
    lines.push(t(hl::HistoryHelpers_YearlyUsages).to_string());
    let mut years = grouped(items.iter().map(local_year));
    years.sort_by_key(|a| std::cmp::Reverse(a.0));
    lines.push(
        years
            .into_iter()
            .map(|(year, count)| {
                format(
                    "{0}: {1} ({2:N0}%)",
                    &[
                        Arg::Int(year as i64),
                        Arg::Int(count as i64),
                        Arg::Float(percent(count)),
                    ],
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    lines.push(String::new());
    let counted = |title: &str, keys: Vec<String>| -> Vec<String> {
        vec![
            title.to_string(),
            grouped(keys.into_iter())
                .into_iter()
                .map(|(key, count)| format!("[{count}] {key}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ]
    };
    let or_empty = |value: &str| {
        if value.trim().is_empty() {
            empty.to_string()
        } else {
            value.to_string()
        }
    };
    lines.extend(counted(
        t(hl::HistoryHelpers_FileExtensions),
        items
            .iter()
            .filter(|i| !i.file_name.is_empty() && !i.file_name.ends_with(')'))
            .map(|i| or_empty(&file_helpers::get_file_name_extension(&i.file_name, false)))
            .collect(),
    ));
    lines.push(String::new());
    lines.extend(counted(
        t(hl::HistoryHelpers_Hosts),
        items.iter().map(|i| or_empty(&i.host)).collect(),
    ));
    lines.push(String::new());
    lines.extend(counted(
        t(hl::HistoryHelpers_ProcessNames),
        items
            .iter()
            .map(|i| or_empty(i.tags_process_name().unwrap_or("")))
            .collect(),
    ));
    lines.join("\n")
}

/// The edit dialog's tags: `key=value` lines sorted by key.
pub fn tags_text(item: &HistoryItem) -> String {
    let Some(tags) = &item.tags else {
        return String::new();
    };
    let mut pairs: Vec<(&str, Option<&str>)> = tags.iter().collect();
    pairs.sort_by_key(|(k, _)| k.to_lowercase());
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={}", v.unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `ParseTags`: one pair a line; a line without `=` is a key with no value.
pub fn parse_tags(text: &str) -> HistoryTags {
    let mut tags = HistoryTags::new();
    for line in text.split('\n').map(|l| l.trim_end_matches('\r')) {
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.find('=') {
            Some(at) => (line[..at].trim(), Some(line[at + 1..].to_string())),
            None => (line.trim(), None),
        };
        if !key.trim().is_empty() {
            tags.insert(key, value);
        }
    }
    tags
}

/// What "Copy" puts on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyFormat {
    Url,
    ShortUrl,
    ThumbnailUrl,
    DeletionUrl,
    HtmlLink,
    HtmlImage,
    HtmlLinkedImage,
    ForumLink,
    ForumImage,
    ForumLinkedImage,
    MarkdownLink,
    MarkdownImage,
    MarkdownLinkedImage,
    FilePath,
    FileName,
    FileNameWithExtension,
    Folder,
}

fn has_url(item: &HistoryItem) -> bool {
    !item.url.trim().is_empty()
}

fn has_image_url(item: &HistoryItem, image_history: bool) -> bool {
    has_url(item) && (image_history || file_helpers::is_image_file(&item.url))
}

fn has_linked_image(item: &HistoryItem, image_history: bool) -> bool {
    has_image_url(item, image_history) && !item.thumbnail_url.trim().is_empty()
}

fn has_extension(path: &str) -> bool {
    Path::new(path).extension().is_some()
}

/// `CopyValues`: each selected item's text, one a line; `None` when none has one. The
/// image history window checks less (no picture address check, any file path).
pub fn copy_text(
    items: &[&HistoryItem],
    format: CopyFormat,
    image_history: bool,
) -> Option<String> {
    use CopyFormat as F;
    let values: Vec<String> = items
        .iter()
        .filter(|item| match format {
            F::HtmlLink | F::ForumLink | F::MarkdownLink => has_url(item),
            F::HtmlImage | F::ForumImage | F::MarkdownImage => has_image_url(item, image_history),
            F::HtmlLinkedImage | F::ForumLinkedImage | F::MarkdownLinkedImage => {
                has_linked_image(item, image_history)
            }
            F::FilePath => {
                image_history
                    || (!item.file_path.trim().is_empty() && has_extension(&item.file_path))
            }
            F::FileName | F::FileNameWithExtension | F::Folder => {
                image_history || !item.file_path.trim().is_empty()
            }
            _ => true,
        })
        .map(|item| {
            let path = Path::new(&item.file_path);
            match format {
                F::Url => item.url.clone(),
                F::ShortUrl => item.shortened_url.clone(),
                F::ThumbnailUrl => item.thumbnail_url.clone(),
                F::DeletionUrl => item.deletion_url.clone(),
                F::HtmlLink => format!("<a href=\"{0}\">{0}</a>", item.url),
                F::HtmlImage => format!("<img src=\"{}\"/>", item.url),
                F::HtmlLinkedImage => format!(
                    "<a href=\"{}\"><img src=\"{}\"/></a>",
                    item.url, item.thumbnail_url
                ),
                F::ForumLink => format!("[url]{}[/url]", item.url),
                F::ForumImage => format!("[img]{}[/img]", item.url),
                F::ForumLinkedImage => {
                    format!("[url={}][img]{}[/img][/url]", item.url, item.thumbnail_url)
                }
                F::MarkdownLink => format!("[{}]({})", item.file_name, item.url),
                F::MarkdownImage => format!("![{}]({})", item.file_name, item.url),
                F::MarkdownLinkedImage => format!(
                    "[![{}]({})]({})",
                    item.file_name, item.thumbnail_url, item.url
                ),
                F::FilePath => item.file_path.clone(),
                F::FileName => path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                F::FileNameWithExtension => path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                F::Folder => path
                    .parent()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            }
        })
        .filter(|v| !v.trim().is_empty())
        .collect();
    (!values.is_empty()).then(|| values.join("\n"))
}

/// `OnHistoryContextMenuOpening`: which entries of the list window's menu work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    pub single: bool,
    pub url: bool,
    pub short_url: bool,
    pub thumbnail_url: bool,
    pub deletion_url: bool,
    pub valid_path: bool,
    pub file: bool,
    pub image_file: bool,
    pub text_file: bool,
    pub image_url: bool,
    pub favorite: bool,
}

impl MenuState {
    /// For the selection (the first item decides, as upstream).
    pub fn of(selected: &[&HistoryItem]) -> Option<MenuState> {
        let item = selected.first()?;
        let valid_path = !item.file_path.trim().is_empty() && has_extension(&item.file_path);
        let file = valid_path && Path::new(&item.file_path).is_file();
        Some(MenuState {
            single: selected.len() == 1,
            url: has_url(item),
            short_url: !item.shortened_url.trim().is_empty(),
            thumbnail_url: !item.thumbnail_url.trim().is_empty(),
            deletion_url: !item.deletion_url.trim().is_empty(),
            valid_path,
            file,
            image_file: file && file_helpers::is_image_file(&item.file_path),
            text_file: file && file_helpers::is_text_file(&item.file_path),
            image_url: has_url(item) && file_helpers::is_image_file(&item.url),
            favorite: item.favorite(),
        })
    }

    /// Copy entries: on for several items, else when the first item has the value.
    pub fn copy_enabled(&self, format: CopyFormat) -> bool {
        use CopyFormat as F;
        !self.single
            || match format {
                F::Url | F::HtmlLink | F::ForumLink | F::MarkdownLink => self.url,
                F::ShortUrl => self.short_url,
                F::ThumbnailUrl => self.thumbnail_url,
                F::DeletionUrl => self.deletion_url,
                F::HtmlImage | F::ForumImage | F::MarkdownImage => self.url && self.image_url,
                F::HtmlLinkedImage | F::ForumLinkedImage | F::MarkdownLinkedImage => {
                    self.url && self.image_url && self.thumbnail_url
                }
                F::FilePath | F::FileName | F::FileNameWithExtension | F::Folder => self.valid_path,
            }
    }
}

/// `TryOpen`: the shortened address, else the address, else the file.
pub enum OpenTarget {
    Url(String),
    File(String),
}

pub fn open_target(item: &HistoryItem) -> Option<OpenTarget> {
    if !item.shortened_url.trim().is_empty() {
        Some(OpenTarget::Url(item.shortened_url.clone()))
    } else if !item.url.trim().is_empty() {
        Some(OpenTarget::Url(item.url.clone()))
    } else if !item.file_path.trim().is_empty() && Path::new(&item.file_path).is_file() {
        Some(OpenTarget::File(item.file_path.clone()))
    } else {
        None
    }
}

/// `ShowSelectedImage`: up to 100 pictures before and after the item, the ones on disk;
/// the list and where the item is in it.
pub fn viewer_files(items: &[&HistoryItem], current: usize) -> (Vec<String>, usize) {
    let start = current.saturating_sub(100);
    let end = (start + 201).min(items.len());
    let mut files = Vec::new();
    let mut selected = 0;
    for (i, item) in items.iter().enumerate().take(end).skip(start) {
        let path = &item.file_path;
        if !Path::new(path).is_file() || !file_helpers::is_image_file(path) {
            continue;
        }
        if i == current {
            selected = files.len();
        }
        files.push(path.clone());
    }
    (files, selected)
}

/// `ConfirmDelete`'s question.
pub fn delete_question(count: usize, files: bool, image_history: bool) -> (String, String) {
    let (this_file, this_item, these_files, these_items, title_files, title_items, confirm) =
        if image_history {
            (
                hl::ImageHistoryWindow_ThisFile,
                hl::ImageHistoryWindow_ThisItem,
                hl::ImageHistoryWindow_TheseFilesFormat,
                hl::ImageHistoryWindow_TheseItemsFormat,
                hl::ImageHistoryWindow_DeleteFiles,
                hl::ImageHistoryWindow_DeleteHistoryItems,
                hl::ImageHistoryWindow_ConfirmDeleteFormat,
            )
        } else {
            (
                hl::HistoryWindow_ThisFile,
                hl::HistoryWindow_ThisItem,
                hl::HistoryWindow_TheseFilesFormat,
                hl::HistoryWindow_TheseItemsFormat,
                hl::HistoryWindow_DeleteFiles,
                hl::HistoryWindow_DeleteHistoryItems,
                hl::HistoryWindow_ConfirmDeleteFormat,
            )
        };
    let noun = match (count == 1, files) {
        (true, true) => t(this_file).to_string(),
        (true, false) => t(this_item).to_string(),
        (false, true) => format(t(these_files), &[Arg::Int(count as i64)]),
        (false, false) => format(t(these_items), &[Arg::Int(count as i64)]),
    };
    (
        t(if files { title_files } else { title_items }).to_string(),
        format(t(confirm), &[Arg::from(noun)]),
    )
}

/// `ImportFolder`: every file under the folder (pictures only when asked), oldest first,
/// as history items, leaving out paths already in the history.
pub fn import_folder(
    folder: &Path,
    images_only: bool,
    existing: &HashSet<String>,
) -> Vec<HistoryItem> {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let text = path.to_string_lossy();
                if images_only && !file_helpers::is_image_file(&text) {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                files.push((modified, path));
            }
        }
    }
    files.sort_by_key(|a| a.0);
    files
        .into_iter()
        .filter(|(_, path)| !existing.contains(&path.to_string_lossy().to_lowercase()))
        .map(|(modified, path)| {
            let text = path.to_string_lossy().into_owned();
            let kind = if file_helpers::is_image_file(&text) {
                "Image"
            } else if file_helpers::is_text_file(&text) {
                "Text"
            } else {
                "File"
            };
            HistoryItem {
                file_name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                file_path: text,
                date_time: Some(DateTime::<Utc>::from(modified)),
                kind: kind.to_string(),
                ..HistoryItem::default()
            }
        })
        .collect()
}

/// The paths already in the history, folded for the duplicate check.
pub fn existing_paths(items: &[HistoryItem]) -> HashSet<String> {
    items
        .iter()
        .filter(|i| !i.file_path.trim().is_empty())
        .map(|i| i.file_path.to_lowercase())
        .collect()
}

/// `RenameFile`: the new name with the old extension; `None` when it is empty or the
/// same (case ignored). Returns the new file name and path.
pub fn renamed(file_path: &str, new_name: &str) -> Option<(String, String)> {
    let path = Path::new(file_path);
    let old = path.file_stem()?.to_string_lossy().into_owned();
    let new_name = new_name.trim();
    if new_name.is_empty()
        || new_name.eq_ignore_ascii_case(&old)
        || new_name.to_lowercase() == old.to_lowercase()
    {
        return None;
    }
    let mut name = new_name.to_string();
    if let Some(extension) = path.extension() {
        name.push('.');
        name.push_str(&extension.to_string_lossy());
    }
    let new_path = path.with_file_name(&name);
    Some((name, new_path.to_string_lossy().into_owned()))
}

/// `PreviewHttpClient.GetByteArrayAsync`: an uploaded picture for the preview, up to
/// 32 MB, decoded under the app's limits.
pub async fn fetch_preview(url: &str) -> Option<image::RgbaImage> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let client = reqwest::Client::builder()
        .user_agent(modlight_core::components::net::user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;
    let response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if response.content_length().is_some_and(|l| l > LIMIT) {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() as u64 > LIMIT {
        return None;
    }
    modlight_core::image_decode::from_bytes(&bytes)
        .ok()
        .map(|i| i.to_rgba8())
}

/// A time for tests and fixtures.
pub fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::culture::{Culture, set_thread_culture, set_thread_utc};
    use modlight_core::l10n::{Language, set_thread_language};

    fn item(name: &str, kind: &str, host: &str, url: &str) -> HistoryItem {
        HistoryItem {
            id: 1,
            file_name: name.into(),
            file_path: format!("/x/{name}"),
            date_time: Some(at(2025, 3, 4, 10, 30)),
            kind: kind.into(),
            host: host.into(),
            url: url.into(),
            ..HistoryItem::default()
        }
    }

    fn sample() -> Vec<HistoryItem> {
        let mut a = item("beach.png", "Image", "Modlight", "https://f.example/a.png");
        a.date_time = Some(at(2026, 1, 2, 12, 0));
        let mut b = item("notes.txt", "Text", "Modlight", "https://f.example/b");
        b.set_tag("work");
        let mut c = item("clip.mp4", "File", "Other", "");
        c.set_favorite(true);
        c.tags
            .as_mut()
            .unwrap()
            .insert("ProcessName", Some("QuickTime".into()));
        vec![a, b, c]
    }

    #[test]
    fn filters() {
        set_thread_utc(true);
        let items = sample();
        let f = |filter: Filter| filter.apply(&items);
        assert_eq!(f(Filter::default()), [0, 1, 2]);
        assert_eq!(
            f(Filter {
                file_name: "B*.PNG".into(),
                ..Filter::default()
            }),
            [0]
        );
        assert_eq!(
            f(Filter {
                file_name: "wor?".into(),
                ..Filter::default()
            }),
            [1]
        );
        assert_eq!(
            f(Filter {
                favorites: true,
                file_name: "beach".into(),
                ..Filter::default()
            }),
            [2]
        );
        assert_eq!(
            f(Filter {
                filter_type: true,
                kind: Some("image".into()),
                ..Filter::default()
            }),
            [0]
        );
        assert_eq!(
            f(Filter {
                filter_host: true,
                host: "mod".into(),
                ..Filter::default()
            }),
            [0, 1]
        );
        assert_eq!(
            f(Filter {
                url: "/B".into(),
                ..Filter::default()
            }),
            [1]
        );
        assert_eq!(
            f(Filter {
                filter_date: true,
                from_date: NaiveDate::from_ymd_opt(2026, 1, 1),
                to_date: NaiveDate::from_ymd_opt(2026, 1, 2),
                ..Filter::default()
            }),
            [0]
        );
        assert_eq!(
            f(Filter {
                max_item_count: 2,
                ..Filter::default()
            }),
            [0, 1]
        );
        assert_eq!(image_filter(&items, "", false, true, false), [0]);
        assert_eq!(image_filter(&items, "", true, false, false), [2]);
        assert_eq!(image_filter(&items, "WORK", false, false, false), [1]);
        assert!(image_filter(&items, "", false, false, true).is_empty());
        let (types, hosts) = types_and_hosts(&items);
        assert_eq!(types, ["File", "Image", "Text"]);
        assert_eq!(hosts, ["Modlight", "Other"]);
        set_thread_utc(false);
    }

    #[test]
    fn statistics() {
        set_thread_language(Some(Language::English));
        set_thread_culture(Some(Culture::english()));
        set_thread_utc(true);
        let mut items = sample();
        items.push(item("more.png", "Image", "", "x"));
        let text = stats(&items);
        let expected = "History item counts:\nTotal: 4\nImage: 2 (50%)\nText: 1 (25%)\nFile: 1 (25%)\n\nYearly usages:\n2026: 1 (25%)\n2025: 3 (75%)\n\nFile extensions:\n[2] png\n[1] txt\n[1] mp4\n\nHosts:\n[2] Modlight\n[1] Other\n[1] (empty)\n\nProcess names:\n[3] (empty)\n[1] QuickTime";
        assert_eq!(text, expected);
        set_thread_utc(false);
        set_thread_culture(None);
        set_thread_language(None);
    }

    #[test]
    fn tags_copies_and_menus() {
        let items = sample();
        assert_eq!(tags_text(&items[2]), "Favorite=\nProcessName=QuickTime");
        let tags = parse_tags("b=2\r\n a = x\n\n=y\nflag");
        let pairs: Vec<(&str, Option<&str>)> = tags.iter().collect();
        assert_eq!(pairs, [("b", Some("2")), ("a", Some(" x")), ("flag", None)]);
        let refs: Vec<&HistoryItem> = items.iter().collect();
        assert_eq!(
            copy_text(&refs, CopyFormat::Url, false).as_deref(),
            Some("https://f.example/a.png\nhttps://f.example/b")
        );
        assert_eq!(
            copy_text(&refs, CopyFormat::MarkdownImage, false).as_deref(),
            Some("![beach.png](https://f.example/a.png)")
        );
        assert_eq!(
            copy_text(&refs, CopyFormat::MarkdownImage, true).as_deref(),
            Some("![beach.png](https://f.example/a.png)\n![notes.txt](https://f.example/b)")
        );
        assert_eq!(copy_text(&refs[2..], CopyFormat::Url, false), None);
        assert_eq!(
            copy_text(&refs[..1], CopyFormat::Folder, false).as_deref(),
            Some("/x")
        );
        assert_eq!(
            copy_text(&refs[..1], CopyFormat::FileName, false).as_deref(),
            Some("beach")
        );
        let menu = MenuState::of(&refs[1..2]).unwrap();
        assert!(menu.single && menu.url && !menu.image_url && !menu.file);
        assert!(menu.copy_enabled(CopyFormat::HtmlLink));
        assert!(!menu.copy_enabled(CopyFormat::HtmlImage));
        assert!(
            MenuState::of(&refs)
                .unwrap()
                .copy_enabled(CopyFormat::ShortUrl)
        );
        assert!(MenuState::of(&[]).is_none());
        assert!(
            matches!(open_target(&items[0]), Some(OpenTarget::Url(u)) if u == "https://f.example/a.png")
        );
        assert!(open_target(&items[2]).is_none());
        assert_eq!(icon(&items[2]), modlight_core::icons::STAR);
        assert_eq!(icon(&items[1]), modlight_core::icons::FILE_TEXT);
        assert_eq!(
            renamed("/a/old.png", "New"),
            Some(("New.png".into(), "/a/New.png".into()))
        );
        assert_eq!(renamed("/a/old.png", " OLD "), None);
    }

    #[test]
    fn imports_and_viewer_lists() {
        set_thread_language(Some(Language::English));
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        std::fs::write(dir.path().join("sub").join("b.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("c.bin"), b"x").unwrap();
        let all = import_folder(dir.path(), false, &HashSet::new());
        assert_eq!(all.len(), 3);
        let kinds: HashSet<&str> = all.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, HashSet::from(["Image", "Text", "File"]));
        let images = import_folder(dir.path(), true, &HashSet::new());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].file_name, "a.png");
        let existing = existing_paths(&images);
        assert_eq!(import_folder(dir.path(), true, &existing).len(), 0);
        let refs: Vec<&HistoryItem> = images.iter().chain(all.iter()).collect();
        let (files, selected) = viewer_files(&refs, 0);
        assert_eq!(files.len(), 2);
        assert_eq!(selected, 0);
        let (title, question) = delete_question(3, true, false);
        assert_eq!(title, t(hl::HistoryWindow_DeleteFiles));
        assert!(question.contains('3'));
        set_thread_language(None);
    }
}

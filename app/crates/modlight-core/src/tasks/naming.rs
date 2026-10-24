//! The task flow's view of the naming helpers (`TaskHelpers.GetFileName`,
//! `GetScreenshotsFolder`'s pattern, `UploadInfoParser`, the URL and file helpers), so
//! the worker reads like upstream's code and the helpers stay in `names`.

use std::path::{Path, PathBuf};

use crate::names::file_helpers;
use crate::names::name_parser::{NameParser, NameParserType};
use crate::names::upload_info::{UploadInfo, UploadInfoParser, UploadLinks};
use crate::names::url_helpers;
use crate::tasks::context::TaskContext;
use crate::tasks::info::{DataType, TaskInfo, TaskMetadata};
use crate::tasks::settings::TaskSettings;

/// `TaskHelpers.GetFileName`: a name from the naming pattern (the active-window one when
/// a window title is known), with `extension` appended. Returns the name and the counter
/// as the parse left it.
pub fn file_name(
    settings: &TaskSettings,
    extension: Option<&str>,
    metadata: Option<&TaskMetadata>,
    counter: i64,
) -> (String, i64) {
    let mut parser = NameParser::new(NameParserType::FileName);
    parser.auto_increment_number = counter.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    parser.max_name_length = settings.advanced_settings.name_pattern_max_length as i32;
    parser.max_title_length = settings.advanced_settings.name_pattern_max_title_length as i32;
    if settings.upload_settings.use_custom_time_zone {
        // UTC is honoured; another zone falls back to local time (MACOS.md).
        parser.custom_time_zone = Some(
            crate::names::name_parser::CustomTimeZone::from_time_zone_info(
                settings.upload_settings.custom_time_zone.as_ref(),
            ),
        );
    }
    if let Some(metadata) = metadata {
        if let Some(image) = &metadata.image {
            parser.image_width = image.width() as i32;
            parser.image_height = image.height() as i32;
        }
        parser.window_text = metadata.window_title.clone();
        parser.process_name = metadata.process_name.clone();
    }
    let pattern = if !settings
        .upload_settings
        .name_format_pattern_active_window
        .is_empty()
        && parser.window_text.as_deref().is_some_and(|w| !w.is_empty())
    {
        settings
            .upload_settings
            .name_format_pattern_active_window
            .clone()
    } else {
        settings.upload_settings.name_format_pattern.clone()
    };
    let mut name = parser.parse(&pattern).unwrap_or_default();
    let counter = parser.auto_increment_number as i64;
    if let Some(extension) = extension.filter(|e| !e.is_empty()) {
        name.push('.');
        name.push_str(extension.trim_start_matches('.'));
    }
    (name, counter)
}

/// A folder pattern (`SaveImageSubFolderPattern`, an overridden screenshots folder)
/// parsed as a path.
pub fn parse_path_pattern(pattern: &str, metadata: Option<&TaskMetadata>) -> String {
    let mut parser = NameParser::new(NameParserType::FilePath);
    if let Some(metadata) = metadata {
        if let Some(image) = &metadata.image {
            parser.image_width = image.width() as i32;
            parser.image_height = image.height() as i32;
        }
        parser.window_text = metadata.window_title.clone();
        parser.process_name = metadata.process_name.clone();
    }
    parser.parse(pattern).unwrap_or_default()
}

/// `FileHelpers.GetAbsolutePath`.
pub fn absolute_path(path: &Path) -> PathBuf {
    PathBuf::from(file_helpers::get_absolute_path(&path.to_string_lossy()))
}

/// `FileHelpers.ExpandFolderVariables`.
pub fn expand_folder_variables(path: &str) -> String {
    file_helpers::expand_folder_variables(path, true)
}

/// `TaskHelpers.FindDataType`: by the extension lists of the task's settings.
pub fn find_data_type(path: &str, settings: &TaskSettings) -> DataType {
    let images = settings.advanced_settings.image_extensions();
    let images: Vec<&str> = images.iter().map(String::as_str).collect();
    if file_helpers::check_extension(path, &images) {
        return DataType::Image;
    }
    let texts = settings.advanced_settings.text_extensions();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    if file_helpers::check_extension(path, &texts) {
        return DataType::Text;
    }
    DataType::File
}

/// `FileHelpers.GetFileNameExtension` without the dot.
pub fn extension(path: &str) -> String {
    file_helpers::get_file_name_extension(path, false)
}

/// `FileHelpers.AppendExtension`.
pub fn append_extension(name: &str, extension: &str) -> String {
    file_helpers::append_extension(name, extension)
}

/// `Path.ChangeExtension(name, extension)`.
pub fn change_extension(name: &str, extension: &str) -> String {
    file_helpers::change_file_name_extension(name, extension)
}

/// `FileHelpers.SanitizeFileName`.
pub fn sanitize_file_name(name: &str) -> String {
    file_helpers::sanitize_file_name(name)
}

/// `FileHelpers.GetUniqueFilePath`.
pub fn unique_file_path(path: &Path) -> PathBuf {
    PathBuf::from(file_helpers::get_unique_file_path(&path.to_string_lossy()))
}

/// The file name `CreateDownloadTask` makes from an address: decoded (ten passes), the
/// last segment, sanitised.
pub fn file_name_from_url(url: &str) -> String {
    let decoded = url_helpers::url_decode(url, 10);
    let name = url_helpers::get_file_name(&decoded);
    file_helpers::sanitize_file_name(&name)
}

/// `URLHelpers.IsValidURL`.
pub fn is_valid_url(url: &str) -> bool {
    url_helpers::is_valid_url(url)
}

/// `HttpUtility.HtmlEncode`.
pub fn html_encode(text: &str) -> String {
    url_helpers::html_encode(text)
}

/// `Regex.Replace(input, pattern, replacement)`. A pattern the regex engine cannot take
/// leaves the text as it is (and says so in the log).
pub fn regex_replace(input: &str, pattern: &str, replacement: &str) -> String {
    match regex::Regex::new(pattern) {
        Ok(regex) => regex.replace_all(input, replacement).into_owned(),
        Err(error) => {
            log::warn!("Modlight: the URL replacement pattern could not be used: {error}");
            input.to_string()
        }
    }
}

/// `new UploadInfoParser().Parse(info, pattern)`.
pub fn parse_upload_info(info: &TaskInfo, pattern: &str, _ctx: &TaskContext) -> String {
    let values = UploadInfo {
        result: Some(UploadLinks {
            url: info.result.url.clone(),
            thumbnail_url: info.result.thumbnail_url.clone(),
            deletion_url: info.result.deletion_url.clone(),
            shortened_url: info.result.shortened_url.clone(),
        }),
        file_name: info.file_name.clone(),
        file_path: info.file_path().to_string(),
        thumbnail_file_path: info.thumbnail_file_path.clone(),
        upload_duration_ms: info.upload_duration.map(|d| d.as_millis() as u64),
    };
    UploadInfoParser::new()
        .parse(&values, pattern)
        .unwrap_or_else(|error| error.to_string())
}

/// The name an upload goes under, treated as the live path treats it (`NameFor` and
/// `UploadData`): bidirectional controls removed, reserved characters replaced when
/// asked, "file" when nothing is left.
pub fn upload_name(name: &str, settings: &TaskSettings) -> String {
    let mut name = url_helpers::remove_bidi_control_characters(name);
    if settings
        .upload_settings
        .file_upload_replace_problematic_characters
    {
        name = url_helpers::replace_reserved_characters(&name, "_");
    }
    name
}

/// `ModlightUploadQueue.NameFor`.
pub fn name_for_queue(info: &TaskInfo) -> String {
    let mut name = info.file_name.clone();
    if name.is_empty() {
        name = Path::new(info.file_path())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    let name = upload_name(&name, &info.settings);
    if name.is_empty() { "file".into() } else { name }
}

/// `URLHelpers.URLEncode`, for the sharing services' addresses: the UTF-8 bytes, with
/// everything but letters, digits and `-._~` as `%XX`.
pub fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    /// `GetFileName` with "Use custom time zone": the zone the settings name
    /// (`UploadSettings.CustomTimeZone`, a `TimeZoneInfo` as Newtonsoft writes it). UTC is
    /// honoured; any other zone falls back to local time on a Mac (MACOS.md), it is not
    /// read as UTC.
    #[test]
    fn the_custom_time_zone_is_the_one_the_settings_name() {
        let settings = |zone: serde_json::Value| -> TaskSettings {
            serde_json::from_value(serde_json::json!({
                "UploadSettings": {
                    "UseCustomTimeZone": true,
                    "CustomTimeZone": zone,
                    "NameFormatPattern": "%h"
                }
            }))
            .unwrap()
        };
        let hours = |now: chrono::DateTime<chrono::Utc>| {
            (
                format!("{:02}", now.with_timezone(&chrono::Local).hour()),
                format!("{:02}", now.hour()),
            )
        };
        let windows_zone = settings(serde_json::json!({
            "Id": "Central European Standard Time",
            "DisplayName": "(UTC+01:00) Amsterdam, Berlin, Bern, Rome, Stockholm, Vienna",
            "BaseUtcOffset": "01:00:00",
            "SupportsDaylightSavingTime": true
        }));
        let utc_zone = settings(serde_json::json!({"Id": "UTC", "BaseUtcOffset": "00:00:00"}));
        let unnamed = settings(serde_json::Value::Null);
        // The clock may pass a full hour between the reads: accept either side of it.
        let before = hours(chrono::Utc::now());
        let (local, _) = file_name(&windows_zone, None, None, 0);
        let (utc, _) = file_name(&utc_zone, None, None, 0);
        let (default, _) = file_name(&unnamed, None, None, 0);
        let after = hours(chrono::Utc::now());
        assert!(local == before.0 || local == after.0, "{local} {before:?}");
        assert!(utc == before.1 || utc == after.1, "{utc} {before:?}");
        assert!(
            default == before.1 || default == after.1,
            "TimeZoneInfo.Utc"
        );
    }
}

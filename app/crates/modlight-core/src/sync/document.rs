//! What travels to the account's cloud settings, and what does not
//! (`ModlightSettingsDocument.cs`).
//!
//! An ALLOWLIST of `ApplicationConfig` members: anything not named here stays on the
//! machine. No credential is in it (the token is not even in this file), no path, no
//! window position, no proxy password, no AI keys, and not the skin, which has its own
//! endpoint. The document is `{"schema":1,"app":{...},"hotkeys":{...}}`, the same shape
//! the Windows client sends, so both share one row per account.

use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::config::AppConfig;

/// The shape of the document. A client meeting a newer schema does nothing at all.
pub const SCHEMA: i64 = 1;

/// `ApplicationConfig` members that travel with the account, in upstream's order.
pub const APP_MEMBERS: &[&str] = &[
    "DefaultTaskSettings",
    "QuickTaskPresets",
    "Language",
    "ShowTray",
    "SilentRun",
    "TrayIconProgressEnabled",
    "TaskbarProgressEnabled",
    "TrayLeftClickAction",
    "TrayLeftDoubleClickAction",
    "TrayMiddleClickAction",
    "AutoCheckUpdate",
    "SaveImageSubFolderPattern",
    "SaveImageSubFolderPatternWindow",
    "AutoCleanupBackupFiles",
    "AutoCleanupLogFiles",
    "CleanupKeepFileCount",
    "ShowThumbnailTitle",
    "ThumbnailTitleLocation",
    "ThumbnailSize",
    "ThumbnailClickAction",
    "ModlightLibraryPanel",
    "ModlightFoldersSection",
    "ModlightFilesSection",
    "ClipboardContentFormats",
    "UploadLimit",
    "BufferSizePower",
    "MaxUploadFailRetry",
    "HistorySaveTasks",
    "HistoryCheckURL",
    "RecentTasksSave",
    "RecentTasksMaxCount",
    "RecentTasksShowInMainWindow",
    "RecentTasksShowInTrayMenu",
    "RecentTasksTrayMenuMostRecentFirst",
    "HistorySettings",
    "ImageHistorySettings",
    "DontShowPrintSettingsDialog",
    "AutoCaptureRepeatTime",
    "AutoCaptureMinimizeToTray",
    "AutoCaptureWaitUpload",
    "RecentColors",
    "DownloadDestination",
    "DownloadMediaKind",
    "DownloadVideoQuality",
    "DownloadAudioFormat",
    "BinaryUnits",
    "ShowMostRecentTaskFirst",
    "WorkflowsOnlyShowEdited",
    "TrayAutoExpandCaptureMenu",
    "SaveSettingsAfterTaskCompleted",
    "DevMode",
    "DisableHotkeys",
    "DisableHotkeysOnFullscreen",
    "HotkeyRepeatLimit",
    "ShowClipboardContentViewer",
    "DefaultClipboardCopyImageFillBackground",
    "UseAlternativeClipboardCopyImage",
    "UseAlternativeClipboardGetImage",
    "RotateImageByExifOrientationData",
    "PNGStripColorSpaceInformation",
    "URLEncodeIgnoreEmoji",
    "ShowMultiUploadWarning",
    "ShowLargeFileSizeWarning",
    "DropSize",
    "DropOffset",
    "DropAlignment",
    "DropOpacity",
    "DropHoverOpacity",
];

/// Nodes of `TaskSettings.ToolsSettings` that travel (AIOptions carries API keys).
const TOOLS_MEMBERS: &[&str] = &[
    "ScreenColorPickerOptions",
    "PinToScreenOptions",
    "ImageCombinerOptions",
    "BorderlessWindowSettings",
    "BackgroundRemoverOptions",
    "UseLegacyImageEditor",
    "ShowImageEditorSelector",
];

/// Members of a TaskSettings node that name a place on one machine.
const TASK_SETTINGS_PATHS: &[&str] = &["OverrideScreenshotsFolder", "ScreenshotsFolder"];

/// Sound file paths, with the switches that turn them on (they go together).
const GENERAL_SOUND_PATHS: &[&str] = &[
    "UseCustomCaptureSound",
    "CustomCaptureSoundPath",
    "UseCustomTaskCompletedSound",
    "CustomTaskCompletedSoundPath",
    "UseCustomActionCompletedSound",
    "CustomActionCompletedSoundPath",
    "UseCustomErrorSound",
    "CustomErrorSoundPath",
];

/// Everything this machine would send.
pub fn build(config: &AppConfig) -> Map<String, Value> {
    let mut app = Map::new();
    for member in APP_MEMBERS {
        if let Some(value) = config.get(member) {
            app.insert((*member).to_string(), value.clone());
        }
    }
    if let Some(Value::Object(defaults)) = app.get_mut("DefaultTaskSettings") {
        normalize_task_settings(defaults);
    }

    let mut document = Map::new();
    document.insert("schema".into(), json!(SCHEMA));
    document.insert("app".into(), Value::Object(app));

    // Absent rather than empty when there is no hotkeys document: an empty list is a real
    // configuration (somebody deleted every hotkey) and must never be sent by accident.
    if let Some(hotkeys) = config.hotkeys() {
        let mut hotkeys = hotkeys.clone();
        if let Some(Value::Array(list)) = hotkeys.get_mut("Hotkeys") {
            for entry in list.iter_mut() {
                if let Some(Value::Object(task)) = entry.get_mut("TaskSettings") {
                    normalize_task_settings(task);
                }
            }
        }
        document.insert("hotkeys".into(), Value::Object(hotkeys));
    }
    document
}

/// Strip a TaskSettings node to what may travel, and to one canonical form.
pub fn normalize_task_settings(task: &mut Map<String, Value>) {
    for (flag, member) in [
        ("UseDefaultGeneralSettings", "GeneralSettings"),
        ("UseDefaultImageSettings", "ImageSettings"),
        ("UseDefaultCaptureSettings", "CaptureSettings"),
        ("UseDefaultUploadSettings", "UploadSettings"),
        ("UseDefaultActions", "ExternalPrograms"),
        ("UseDefaultToolsSettings", "ToolsSettings"),
        ("UseDefaultAdvancedSettings", "AdvancedSettings"),
    ] {
        if task.get(flag) == Some(&Value::Bool(true)) {
            task.remove(member);
        }
    }

    for member in TASK_SETTINGS_PATHS {
        task.remove(*member);
    }

    if let Some(Value::Object(general)) = task.get_mut("GeneralSettings") {
        for member in GENERAL_SOUND_PATHS {
            general.remove(*member);
        }
    }

    if let Some(Value::Object(capture)) = task.get_mut("CaptureSettings")
        && let Some(Value::Object(ffmpeg)) = capture.get_mut("FFmpegOptions")
    {
        ffmpeg.remove("OverrideCLIPath");
        ffmpeg.remove("CLIPath");
    }

    if let Some(Value::Object(tools)) = task.get_mut("ToolsSettings") {
        tools.retain(|name, _| TOOLS_MEMBERS.contains(&name.as_str()));
    }
}

/// A short, stable name for the contents of a document: lower-case hex SHA-256 of its
/// compact JSON. Members keep their order, so the same values give the same bytes.
pub fn digest(document: Option<&Map<String, Value>>) -> String {
    let Some(document) = document else {
        return String::new();
    };
    let bytes = serde_json::to_vec(&Value::Object(document.clone())).unwrap_or_default();
    hex::encode(Sha256::digest(&bytes))
}

/// How many bytes the document would occupy on the server.
pub fn canonical_length(document: &Map<String, Value>) -> usize {
    serde_json::to_vec(&Value::Object(document.clone()))
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

/// `JObject.Merge` with arrays replaced wholesale and explicit nulls merged as values.
fn merge(target: &mut Map<String, Value>, content: &Map<String, Value>) {
    for (key, value) in content {
        match (target.get_mut(key), value) {
            (Some(Value::Object(existing)), Value::Object(incoming)) => merge(existing, incoming),
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Put an adopted `app` document onto this machine's configuration.
///
/// Merged onto THIS machine's values, member by member: a member the remote document
/// does not mention keeps its local value, and an object member is merged deeply, so a
/// field that never travels (a path, a key) survives. Returns whether anything applied.
pub fn apply_app(config: &mut AppConfig, remote_app: Option<&Map<String, Value>>) -> bool {
    let Some(remote_app) = remote_app else {
        return false;
    };
    for member in APP_MEMBERS {
        let Some(remote) = remote_app.get(*member) else {
            continue;
        };
        let merged = match (config.get(member), remote) {
            (Some(Value::Object(local)), Value::Object(incoming)) => {
                let mut combined = local.clone();
                merge(&mut combined, incoming);
                Value::Object(combined)
            }
            _ => remote.clone(),
        };
        config.set(member, merged);
    }
    true
}

/// The hotkeys document of an adopted settings document, when it carries a list.
pub fn read_hotkeys(remote_hotkeys: Option<&Map<String, Value>>) -> Option<Map<String, Value>> {
    let hotkeys = remote_hotkeys?;
    match hotkeys.get("Hotkeys")? {
        Value::Array(_) => Some(hotkeys.clone()),
        _ => None,
    }
}

/// Put an adopted hotkeys document in place of this machine's.
///
/// The whole object, not only the list: the Mac has no hotkey settings of its own yet,
/// and taking every member keeps the document it builds next time byte-identical to the
/// one it adopted, so it is not mistaken for a machine holding an unsent change.
pub fn apply_hotkeys(config: &mut AppConfig, hotkeys: Map<String, Value>) {
    config.set_hotkeys(hotkeys);
}

/// What the server holds, as far as this build can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaVerdict {
    /// Nothing is stored for this account at all (version 0).
    Missing,
    /// A document this build can read.
    Known,
    /// A row exists and holds no document.
    Empty,
    /// Written by something else, or by a later build: do nothing.
    Unknown,
}

pub fn read_schema(version: i64, settings: &Map<String, Value>) -> SchemaVerdict {
    if version == 0 {
        return SchemaVerdict::Missing;
    }
    if settings.is_empty() {
        return SchemaVerdict::Empty;
    }
    match settings.get("schema") {
        Some(Value::Number(n)) if n.is_i64() => {
            if n.as_i64().unwrap_or(i64::MAX) <= SCHEMA {
                SchemaVerdict::Known
            } else {
                SchemaVerdict::Unknown
            }
        }
        _ => SchemaVerdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(values: Value) -> AppConfig {
        let mut config = AppConfig::in_memory();
        if let Value::Object(map) = values {
            for (k, v) in map {
                config.set(&k, v);
            }
        }
        config
    }

    #[test]
    fn only_the_allowlist_travels() {
        let config = config_with(json!({
            "Language": "Polish",
            "ThemeOptions": {"Skin": "terminal"},
            "ModlightLastFolderId": "abc",
            "ProxySettings": {"Password": "secret"},
            "CustomScreenshotsPath": "/Users/x",
            "ModlightLibraryPanel": true,
            "DefaultTaskSettings": {
                "UseDefaultGeneralSettings": false,
                "ScreenshotsFolder": "/Users/x/Pictures",
                "GeneralSettings": {"PlaySoundAfterCapture": true, "CustomCaptureSoundPath": "/a.wav", "UseCustomCaptureSound": true},
                "CaptureSettings": {"FFmpegOptions": {"CLIPath": "/ffmpeg", "OverrideCLIPath": true, "VideoCodec": "x264"}},
                "UseDefaultToolsSettings": false,
                "ToolsSettings": {"AIOptions": {"OpenAIAPIKey": "sk-x"}, "PinToScreenOptions": {"Opacity": 1}},
                "UseDefaultImageSettings": true,
                "ImageSettings": {"ImageFormat": "PNG"}
            }
        }));
        let document = build(&config);
        let text = serde_json::to_string(&document).unwrap();
        for secret in [
            "secret", "sk-x", "/Users/x", "/a.wav", "/ffmpeg", "terminal", "\"abc\"",
        ] {
            assert!(!text.contains(secret), "{secret} must not travel: {text}");
        }
        assert!(text.starts_with(r#"{"schema":1,"app":{"DefaultTaskSettings":"#));
        let app = document["app"].as_object().unwrap();
        assert_eq!(app["Language"], json!("Polish"));
        assert_eq!(app["ModlightLibraryPanel"], json!(true));
        let task = app["DefaultTaskSettings"].as_object().unwrap();
        assert!(
            !task.contains_key("ImageSettings"),
            "a section that uses the defaults is dropped"
        );
        assert_eq!(
            task["GeneralSettings"],
            json!({"PlaySoundAfterCapture": true})
        );
        assert_eq!(
            task["CaptureSettings"],
            json!({"FFmpegOptions": {"VideoCodec": "x264"}})
        );
        assert_eq!(
            task["ToolsSettings"],
            json!({"PinToScreenOptions": {"Opacity": 1}})
        );
        assert!(
            !document.contains_key("hotkeys"),
            "no hotkeys document, no hotkeys member"
        );
    }

    #[test]
    fn digests_are_stable_and_sensitive() {
        let a = build(&config_with(
            json!({"Language": "English", "ShowTray": true}),
        ));
        let b = build(&config_with(
            json!({"ShowTray": true, "Language": "English"}),
        ));
        assert_eq!(
            digest(Some(&a)),
            digest(Some(&b)),
            "member order is the allowlist's, not the file's"
        );
        let c = build(&config_with(
            json!({"Language": "Polish", "ShowTray": true}),
        ));
        assert_ne!(digest(Some(&a)), digest(Some(&c)));
        assert_eq!(digest(None), "");
        assert_eq!(digest(Some(&a)).len(), 64);
        assert!(canonical_length(&a) > 10);
    }

    #[test]
    fn adopting_merges_onto_what_is_here() {
        let mut config = config_with(json!({
            "Language": "English",
            "DefaultTaskSettings": {"ToolsSettings": {"AIOptions": {"OpenAIAPIKey": "local"}}, "GeneralSettings": {"A": 1, "B": [1, 2]}},
            "SomethingLocal": 5
        }));
        let remote = json!({
            "Language": "Polish",
            "DefaultTaskSettings": {"GeneralSettings": {"B": [3], "C": null}},
            "NotAnAppMember": true
        });
        assert!(apply_app(&mut config, remote.as_object()));
        assert_eq!(config.get("Language"), Some(&json!("Polish")));
        assert_eq!(
            config.get("DefaultTaskSettings"),
            Some(
                &json!({"ToolsSettings": {"AIOptions": {"OpenAIAPIKey": "local"}}, "GeneralSettings": {"A": 1, "B": [3], "C": null}})
            )
        );
        assert_eq!(
            config.get("NotAnAppMember"),
            None,
            "only allowlisted members are applied"
        );
        assert_eq!(config.get("SomethingLocal"), Some(&json!(5)));
        assert!(!apply_app(&mut config, None));
    }

    #[test]
    fn a_document_adopted_whole_reads_back_identically() {
        // What Windows sent, applied to a fresh Mac, must build back to the same bytes:
        // otherwise the next run would mistake the Mac for one holding a change.
        let remote = json!({
            "schema": 1,
            "app": {"DefaultTaskSettings": {"GeneralSettings": {"X": 1.5}}, "Language": "Polish", "ShowTray": false},
            "hotkeys": {"Hotkeys": [{"HotkeyInfo": {"Hotkey": "PrintScreen"}, "TaskSettings": {"Job": "RectangleRegion"}}], "ApplicationVersion": "0.7.7"}
        });
        let remote = remote.as_object().unwrap();
        let mut config = AppConfig::in_memory();
        apply_app(&mut config, remote.get("app").and_then(Value::as_object));
        apply_hotkeys(
            &mut config,
            read_hotkeys(remote.get("hotkeys").and_then(Value::as_object)).unwrap(),
        );
        let rebuilt = build(&config);
        assert_eq!(
            Value::Object(rebuilt.clone()),
            Value::Object(remote.clone())
        );
        assert_eq!(digest(Some(&rebuilt)), digest(Some(remote)));
    }

    #[test]
    fn schemas_are_read_conservatively() {
        let known = json!({"schema": 1, "app": {}});
        let newer = json!({"schema": 2});
        let odd = json!({"schema": "1"});
        assert_eq!(read_schema(0, &Map::new()), SchemaVerdict::Missing);
        assert_eq!(read_schema(3, &Map::new()), SchemaVerdict::Empty);
        assert_eq!(
            read_schema(3, known.as_object().unwrap()),
            SchemaVerdict::Known
        );
        assert_eq!(
            read_schema(3, newer.as_object().unwrap()),
            SchemaVerdict::Unknown
        );
        assert_eq!(
            read_schema(3, odd.as_object().unwrap()),
            SchemaVerdict::Unknown
        );
    }
}

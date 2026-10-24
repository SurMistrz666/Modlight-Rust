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

/// A short, stable name for the CONTENT of a document: lower-case hex SHA-256 of its
/// canonical form, every object's members sorted by name and every number written as
/// JavaScript writes it (see [`canonical`]).
///
/// Upstream hashes the document's bytes as built. The server keeps the document in a
/// Postgres `jsonb` column and hands it back through JavaScript, so its copy comes back
/// with every object's members in jsonb's own order and `3.0` as `3`: the digest of the
/// server's copy never equals the digest of the same settings built here. Every round
/// then read as "the server moved" (adopt), the next as "an unsent local change" (push),
/// for ever, and a machine that had just adopted pushed its settings over a change
/// another machine had made in between (`MACOS.md`, "Behaviour that differs on purpose").
pub fn digest(document: Option<&Map<String, Value>>) -> String {
    let Some(document) = document else {
        return String::new();
    };
    let bytes =
        serde_json::to_vec(&canonical(&Value::Object(document.clone()))).unwrap_or_default();
    hex::encode(Sha256::digest(&bytes))
}

/// The digest an earlier build wrote into the marker: the document's bytes as they are.
fn legacy_digest(document: &Map<String, Value>) -> String {
    let bytes = serde_json::to_vec(&Value::Object(document.clone())).unwrap_or_default();
    hex::encode(Sha256::digest(&bytes))
}

/// The form [`digest`] hashes: members sorted by name at every level (arrays keep their
/// order, which means something), and a number with nothing after the point written
/// without it, as JavaScript prints it and so as the server hands it back.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical(&map[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Number(number) => match number.as_f64() {
            Some(float)
                if number.is_f64()
                    && float.fract() == 0.0
                    && float.abs() < 9_223_372_036_854_775_808.0 =>
            {
                json!(float as i64)
            }
            _ => value.clone(),
        },
        other => other.clone(),
    }
}

/// The document as the server's `jsonb` column prints it: members ordered shorter name
/// first, then by bytes, at every level. Only for recognising a marker an earlier build
/// wrote after adopting (it hashed the server's copy as it came back).
fn jsonb_order(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.len().cmp(&b.len()).then(a.as_bytes().cmp(b.as_bytes())));
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), jsonb_order(&map[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(jsonb_order).collect()),
        other => canonical(other),
    }
}

/// The marker's digest in today's form.
///
/// A marker an earlier build wrote holds the digest of a document's BYTES: of this
/// machine's document (after a push or a confirm) or of the server's copy (after an
/// adopt, when this machine took exactly that content). When it names the bytes of what
/// this machine holds now, in either order, or of the server's copy at the confirmed
/// version, it is the same fact as today's digest of that content, and is read as such:
/// otherwise the first round after an update would take an unchanged machine for one
/// holding a change, and push it over whatever another machine sent meanwhile.
pub fn confirmed_digest(
    marker_digest: &str,
    local: &Map<String, Value>,
    remote: &Map<String, Value>,
    remote_is_the_confirmed_version: bool,
) -> String {
    if marker_digest.is_empty() || marker_digest == digest(Some(local)) {
        return marker_digest.to_string();
    }
    let local_as_stored = match jsonb_order(&Value::Object(local.clone())) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if marker_digest == legacy_digest(local) || marker_digest == legacy_digest(&local_as_stored) {
        return digest(Some(local));
    }
    if remote_is_the_confirmed_version && marker_digest == legacy_digest(remote) {
        return digest(Some(remote));
    }
    marker_digest.to_string()
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
    fn digests_name_the_content_not_the_bytes() {
        let built = json!({
            "schema": 1,
            "app": {"Language": "Polish", "DefaultTaskSettings": {"GeneralSettings": {"ToastWindowDuration": 3.0, "PlaySoundAfterCapture": true}}}
        });
        // The same settings as the server hands them back: jsonb's member order and
        // JavaScript's numbers.
        let stored = json!({
            "app": {"Language": "Polish", "DefaultTaskSettings": {"GeneralSettings": {"ToastWindowDuration": 3, "PlaySoundAfterCapture": true}}},
            "schema": 1
        });
        assert_eq!(digest(built.as_object()), digest(stored.as_object()));

        let changed = json!({
            "app": {"Language": "English", "DefaultTaskSettings": {"GeneralSettings": {"ToastWindowDuration": 3, "PlaySoundAfterCapture": true}}},
            "schema": 1
        });
        assert_ne!(digest(built.as_object()), digest(changed.as_object()));
        // A list's order is part of what it says; a fraction is another number.
        assert_ne!(
            digest(json!({"a": [1, 2]}).as_object()),
            digest(json!({"a": [2, 1]}).as_object())
        );
        assert_ne!(
            digest(json!({"a": 3.5}).as_object()),
            digest(json!({"a": 3}).as_object())
        );
        assert_eq!(
            digest(json!({"a": -0.0}).as_object()),
            digest(json!({"a": 0}).as_object())
        );
    }

    #[test]
    fn a_marker_an_earlier_build_wrote_still_counts() {
        let local = build(&config_with(json!({
            "Language": "Polish",
            "ShowTray": true,
            "DefaultTaskSettings": {"GeneralSettings": {"ToastWindowDuration": 3}}
        })));
        let as_stored = match jsonb_order(&Value::Object(local.clone())) {
            Value::Object(map) => map,
            _ => unreachable!(),
        };
        let elsewhere = json!({"schema": 1, "app": {"Language": "English"}});
        let elsewhere = elsewhere.as_object().unwrap();
        let today = digest(Some(&local));

        // Written after a push: the bytes of this machine's document.
        assert_eq!(
            confirmed_digest(&legacy_digest(&local), &local, elsewhere, false),
            today
        );
        // Written after an adopt: the bytes of the server's copy, the content held here.
        assert_eq!(
            confirmed_digest(&legacy_digest(&as_stored), &local, elsewhere, false),
            today
        );
        // The server's copy at the confirmed version.
        assert_eq!(
            confirmed_digest(&legacy_digest(elsewhere), &local, elsewhere, true),
            digest(Some(elsewhere))
        );
        assert_eq!(
            confirmed_digest(&legacy_digest(elsewhere), &local, elsewhere, false),
            legacy_digest(elsewhere),
            "not when the server has moved since"
        );
        // Today's form, no marker, or a document that is not here: as written.
        assert_eq!(confirmed_digest(&today, &local, elsewhere, true), today);
        assert_eq!(confirmed_digest("", &local, elsewhere, true), "");
        assert_eq!(confirmed_digest("abcd", &local, elsewhere, true), "abcd");
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

//! `ApplicationConfig.json` and `HotkeysConfig.json`: this installation's settings.
//!
//! The file keeps the member names and shapes of the Windows `ApplicationConfig`, and it
//! is held as a JSON object rather than as a struct. That is what lets a Mac adopt the
//! whole document another installation synced (hotkeys, task settings, everything this
//! build has no screen for yet) and push it back later WITHOUT dropping what it does not
//! understand: a member the Mac does not read is kept byte for byte.
//!
//! Members the Mac uses are read through typed accessors with upstream's defaults.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::theme::ThemeOptions;

pub const APPLICATION_CONFIG_FILE: &str = "ApplicationConfig.json";
pub const HOTKEYS_CONFIG_FILE: &str = "HotkeysConfig.json";
pub const BACKUP_FOLDER: &str = "Backup";

/// What the server last CONFIRMED about this installation's settings
/// (`ModlightCloudSettings`). Never part of the synced document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudMarker {
    /// Which deployment and account this marker is about.
    pub account: String,
    /// The version the server answered with. 0: no sync has ever been confirmed.
    pub version: i64,
    /// Digest of the document the server acknowledged at that version.
    pub digest: String,
}

/// The settings of this installation.
#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    path: Option<PathBuf>,
    hotkeys_path: Option<PathBuf>,
    values: Map<String, Value>,
    /// The hotkeys document (`HotkeysConfig.json`), when one exists. `None` means "not
    /// loaded", which the synced document must leave out rather than send as empty.
    hotkeys: Option<Map<String, Value>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            path: None,
            hotkeys_path: None,
            values: Map::new(),
            hotkeys: None,
        }
    }
}

fn read_object(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// Write `value` to `path` atomically: a temporary file next to it, then a rename.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

impl AppConfig {
    /// Load (or start) the settings in `profile`.
    pub fn load(profile: &Path) -> AppConfig {
        let path = profile.join(APPLICATION_CONFIG_FILE);
        let hotkeys_path = profile.join(HOTKEYS_CONFIG_FILE);
        AppConfig {
            values: read_object(&path).unwrap_or_default(),
            hotkeys: read_object(&hotkeys_path),
            path: Some(path),
            hotkeys_path: Some(hotkeys_path),
        }
    }

    /// Settings that live in memory only (previews and tests).
    pub fn in_memory() -> AppConfig {
        AppConfig::default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(path) = &self.path {
            let text = serde_json::to_vec_pretty(&Value::Object(self.values.clone()))?;
            write_atomically(path, &text)?;
        }
        if let (Some(path), Some(hotkeys)) = (&self.hotkeys_path, &self.hotkeys) {
            let text = serde_json::to_vec_pretty(&Value::Object(hotkeys.clone()))?;
            write_atomically(path, &text)?;
        }
        Ok(())
    }

    /// The folder synced documents are backed up to before they are replaced.
    pub fn backup_folder(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.join(BACKUP_FOLDER))
    }

    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }

    pub fn values_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.values
    }

    pub fn hotkeys(&self) -> Option<&Map<String, Value>> {
        self.hotkeys.as_ref()
    }

    pub fn set_hotkeys(&mut self, hotkeys: Map<String, Value>) {
        self.hotkeys = Some(hotkeys);
    }

    pub fn get(&self, member: &str) -> Option<&Value> {
        self.values.get(member)
    }

    pub fn set(&mut self, member: &str, value: Value) {
        self.values.insert(member.to_string(), value);
    }

    fn bool_or(&self, member: &str, default: bool) -> bool {
        self.values
            .get(member)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }

    fn i64_or(&self, member: &str, default: i64) -> i64 {
        self.values
            .get(member)
            .and_then(Value::as_i64)
            .unwrap_or(default)
    }

    fn str_or<'a>(&'a self, member: &str, default: &'a str) -> &'a str {
        self.values
            .get(member)
            .and_then(Value::as_str)
            .unwrap_or(default)
    }

    /// `SupportedLanguage`: "Automatic", "English" or "Polish".
    pub fn language(&self) -> crate::l10n::LanguageSetting {
        crate::l10n::LanguageSetting::parse(self.str_or("Language", "Automatic"))
    }

    pub fn set_language(&mut self, language: crate::l10n::LanguageSetting) {
        self.set("Language", json!(language.as_str()));
    }

    pub fn show_tray(&self) -> bool {
        self.bool_or("ShowTray", true)
    }

    pub fn set_show_tray(&mut self, value: bool) {
        self.set("ShowTray", json!(value));
    }

    pub fn silent_run(&self) -> bool {
        self.bool_or("SilentRun", false)
    }

    pub fn first_time_minimize_to_tray(&self) -> bool {
        self.bool_or("FirstTimeMinimizeToTray", true)
    }

    pub fn set_first_time_minimize_to_tray(&mut self, value: bool) {
        self.set("FirstTimeMinimizeToTray", json!(value));
    }

    pub fn dev_mode(&self) -> bool {
        self.bool_or("DevMode", false)
    }

    pub fn theme_options(&self) -> ThemeOptions {
        self.values
            .get("ThemeOptions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn set_theme_options(&mut self, options: &ThemeOptions) {
        if let Ok(value) = serde_json::to_value(options) {
            self.set("ThemeOptions", value);
        }
    }

    pub fn library_panel(&self) -> bool {
        self.bool_or("ModlightLibraryPanel", false)
    }

    pub fn set_library_panel(&mut self, value: bool) {
        self.set("ModlightLibraryPanel", json!(value));
    }

    pub fn folders_section(&self) -> bool {
        self.bool_or("ModlightFoldersSection", true)
    }

    pub fn set_folders_section(&mut self, value: bool) {
        self.set("ModlightFoldersSection", json!(value));
    }

    pub fn files_section(&self) -> bool {
        self.bool_or("ModlightFilesSection", true)
    }

    pub fn set_files_section(&mut self, value: bool) {
        self.set("ModlightFilesSection", json!(value));
    }

    pub fn last_folder_id(&self) -> String {
        self.str_or("ModlightLastFolderId", "").to_string()
    }

    pub fn set_last_folder_id(&mut self, id: Option<&str>) {
        self.set("ModlightLastFolderId", json!(id.unwrap_or("")));
    }

    pub fn page_size(&self) -> i64 {
        self.i64_or("ModlightPageSize", 120)
    }

    pub fn set_page_size(&mut self, size: i64) {
        self.set("ModlightPageSize", json!(size));
    }

    pub fn cloud_marker(&self) -> CloudMarker {
        let Some(Value::Object(map)) = self.values.get("CloudSettings") else {
            return CloudMarker::default();
        };
        CloudMarker {
            account: map
                .get("Account")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            version: map.get("Version").and_then(Value::as_i64).unwrap_or(0),
            digest: map
                .get("Digest")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }
    }

    pub fn set_cloud_marker(&mut self, marker: &CloudMarker) {
        self.set(
            "CloudSettings",
            json!({ "Account": marker.account, "Version": marker.version, "Digest": marker.digest }),
        );
    }

    /// The main window's remembered size and placement (`MainFormSize`, ...).
    pub fn main_window(&self) -> MainWindowBounds {
        let size = self
            .values
            .get("MainFormSize")
            .and_then(Value::as_str)
            .unwrap_or("");
        let position = self
            .values
            .get("MainFormPosition")
            .and_then(Value::as_str)
            .unwrap_or("");
        MainWindowBounds {
            remember_size: self.bool_or("RememberMainFormSize", true),
            remember_position: self.bool_or("RememberMainFormPosition", true),
            size: parse_pair(size),
            position: parse_pair(position),
            maximized: self.bool_or("MainFormMaximized", false),
        }
    }

    pub fn set_main_window(
        &mut self,
        size: Option<(i64, i64)>,
        position: Option<(i64, i64)>,
        maximized: bool,
    ) {
        if let Some((w, h)) = size {
            self.set("MainFormSize", json!(format!("{w}, {h}")));
        }
        if let Some((x, y)) = position {
            self.set("MainFormPosition", json!(format!("{x}, {y}")));
        }
        self.set("MainFormMaximized", json!(maximized));
    }

    /// The toast settings of the default task (`TaskSettingsGeneral.ToastWindow*`).
    pub fn toast(&self) -> ToastSettings {
        let general = self
            .values
            .get("DefaultTaskSettings")
            .and_then(|v| v.get("GeneralSettings"));
        let number = |key: &str, default: f64| {
            general
                .and_then(|g| g.get(key))
                .and_then(Value::as_f64)
                .unwrap_or(default)
        };
        let size = general
            .and_then(|g| g.get("ToastWindowSize"))
            .and_then(Value::as_str)
            .and_then(parse_pair)
            .unwrap_or((400, 300));
        let placement = general
            .and_then(|g| g.get("ToastWindowPlacement"))
            .and_then(Value::as_str)
            .unwrap_or("BottomRight")
            .to_string();
        let buttons = general
            .and_then(|g| g.get("ToastWindowButtons"))
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|b| b.get("Action").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_else(|| vec!["CopyImageToClipboard".to_string(), "CopyUrl".to_string()]);
        ToastSettings {
            duration_seconds: number("ToastWindowDuration", 3.0),
            fade_seconds: number("ToastWindowFadeDuration", 1.0),
            placement,
            size,
            button_size: number("ToastWindowButtonSize", 40.0) as i64,
            buttons,
        }
    }
}

/// `System.Drawing.Size`/`Point` as Newtonsoft writes them: `"1040, 720"`.
fn parse_pair(text: &str) -> Option<(i64, i64)> {
    let (a, b) = text.split_once(',')?;
    let a = a.trim().parse().ok()?;
    let b = b.trim().parse().ok()?;
    if a == 0 && b == 0 { None } else { Some((a, b)) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MainWindowBounds {
    pub remember_size: bool,
    pub remember_position: bool,
    pub size: Option<(i64, i64)>,
    pub position: Option<(i64, i64)>,
    pub maximized: bool,
}

/// How the notification window behaves (the task's toast settings).
#[derive(Clone, Debug, PartialEq)]
pub struct ToastSettings {
    pub duration_seconds: f64,
    pub fade_seconds: f64,
    /// A `ContentAlignment` name: "BottomRight", "TopLeft", ...
    pub placement: String,
    pub size: (i64, i64),
    pub button_size: i64,
    /// `ToastClickAction` names, in order.
    pub buttons: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_members_survive_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(APPLICATION_CONFIG_FILE),
            r#"{"Language":"Polish","SomethingWindowsOnly":{"a":[1,2]},"ModlightLibraryPanel":true}"#,
        )
        .unwrap();
        let mut config = AppConfig::load(dir.path());
        assert_eq!(config.language(), crate::l10n::LanguageSetting::Polish);
        assert!(config.library_panel());
        assert!(config.folders_section(), "upstream default");
        config.set_page_size(60);
        config.save().unwrap();
        let again = AppConfig::load(dir.path());
        assert_eq!(again.get("SomethingWindowsOnly"), Some(&json!({"a":[1,2]})));
        assert_eq!(again.page_size(), 60);
        assert!(again.hotkeys().is_none());
    }

    #[test]
    fn markers_and_window_bounds() {
        let mut config = AppConfig::in_memory();
        assert_eq!(config.cloud_marker(), CloudMarker::default());
        let marker = CloudMarker {
            account: "h\nT".into(),
            version: 4,
            digest: "ab".into(),
        };
        config.set_cloud_marker(&marker);
        assert_eq!(config.cloud_marker(), marker);
        config.set_main_window(Some((1040, 720)), Some((10, 20)), false);
        let bounds = config.main_window();
        assert_eq!(bounds.size, Some((1040, 720)));
        assert_eq!(bounds.position, Some((10, 20)));
        let toast = config.toast();
        assert_eq!(toast.size, (400, 300));
        assert_eq!(toast.buttons, vec!["CopyImageToClipboard", "CopyUrl"]);
    }
}

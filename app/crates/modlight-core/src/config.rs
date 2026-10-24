//! `ApplicationConfig.json` and `HotkeysConfig.json`: this installation's settings.
//!
//! The file keeps the member names and shapes of the Windows `ApplicationConfig`, and it
//! is held as a JSON object rather than as a struct. That is what lets this port adopt
//! the whole document another installation synced (hotkeys, task settings, everything
//! this build has no screen for yet) and push it back later WITHOUT dropping what it
//! does not understand: a member the port does not read is kept byte for byte.
//!
//! Members the port uses are read through typed accessors with upstream's defaults.
//!
//! Loading and saving follow `SettingsBase` as `SettingManager` sets it up: a save
//! writes `<file>.temp`, keeps the file it replaces as `Backup/<file>` and once a week a
//! dated copy (`Backup/<name>-yyyy-MM-Www.json`); a file that does not load falls back
//! to those, in that order, before the defaults.

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

/// Where a save writes before the rename (`SettingsBase`: `filePath + ".temp"`).
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".temp");
    PathBuf::from(name)
}

/// Write `bytes` to `path` atomically: a temporary file next to it, then a rename.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(path);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// `JsonHelpers.QuickVerifyJsonFile`: the text starts with `{` and ends with `}`.
fn quick_verify_json(bytes: &[u8]) -> bool {
    let text = bytes.trim_ascii();
    text.len() > 1 && text.first() == Some(&b'{') && text.last() == Some(&b'}')
}

/// `SettingsBase.Load` with fallback support: the file itself, then what a save cut
/// short left beside it (`<file>.temp`), then the copy the last save kept
/// (`Backup/<file>`), then the newest weekly copy (`Backup/<name>-*`). A file that is
/// missing, empty or not a JSON object is passed over; `None` when none of them loads.
fn load_settings_file(path: &Path, backup_folder: &Path) -> Option<Map<String, Value>> {
    let mut candidates = vec![path.to_path_buf(), temp_path(path)];
    if backup_folder.is_dir()
        && let Some(file_name) = path.file_name()
    {
        candidates.push(backup_folder.join(file_name));
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let prefix = format!("{stem}-");
        let newest_weekly = std::fs::read_dir(backup_folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&prefix))
            .max();
        if let Some(name) = newest_weekly {
            candidates.push(backup_folder.join(name));
        }
    }
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(map) = read_object(candidate) {
            if index > 0 {
                log::warn!(
                    "{} could not be read; the settings were loaded from {}",
                    path.display(),
                    candidate.display()
                );
            }
            return Some(map);
        }
    }
    None
}

/// `SettingsBase.SaveInternal` with `CreateBackup` and `CreateWeeklyBackup`, as
/// `SettingManager` sets them for every settings file: the text goes to `<file>.temp`
/// and is checked, the file it replaces is kept as `Backup/<file>` (a file that does not
/// read as settings is not kept over a good copy), the new one takes its place, and the
/// first save of a week leaves `Backup/<name>-yyyy-MM-Www.json` too.
fn save_settings_file(path: &Path, bytes: &[u8], backup_folder: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = temp_path(path);
    std::fs::write(&temp, bytes)?;
    if !quick_verify_json(bytes) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("the settings file is corrupt: {}", temp.display()),
        ));
    }
    if let Ok(previous) = std::fs::read(path)
        && quick_verify_json(&previous)
        && let Some(file_name) = path.file_name()
    {
        std::fs::create_dir_all(backup_folder)?;
        std::fs::write(backup_folder.join(file_name), previous)?;
    }
    std::fs::rename(&temp, path)?;
    if let Err(error) = backup_weekly(path, backup_folder, chrono::Local::now()) {
        log::warn!("Weekly settings backup of {}: {error}", path.display());
    }
    Ok(())
}

/// `FileHelpers.BackupFileWeekly`: `<name>-yyyy-MM-Www<ext>` in `folder`, once a week.
fn backup_weekly(
    path: &Path,
    folder: &Path,
    now: chrono::DateTime<chrono::Local>,
) -> std::io::Result<()> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let name = format!(
        "{stem}-{}-W{:02}{extension}",
        now.format("%Y-%m"),
        crate::names::helpers::week_of_year(&now)
    );
    let weekly = folder.join(name);
    if !weekly.exists() {
        std::fs::create_dir_all(folder)?;
        std::fs::copy(path, weekly)?;
    }
    Ok(())
}

impl AppConfig {
    /// Load (or start) the settings in `profile`, falling back to what the last saves
    /// left when a file does not read (`SettingsBase.Load`).
    pub fn load(profile: &Path) -> AppConfig {
        AppConfig::load_with(profile, true)
    }

    /// `Load(filePath, backupFolder, fallbackSupport)`: without fallback support (what
    /// `ResetSettings` asks for) a file that does not read starts from the defaults, and
    /// the copies in `Backup` are left alone.
    pub fn load_with(profile: &Path, fallback: bool) -> AppConfig {
        let path = profile.join(APPLICATION_CONFIG_FILE);
        let hotkeys_path = profile.join(HOTKEYS_CONFIG_FILE);
        let backup = profile.join(BACKUP_FOLDER);
        let read = |file: &Path| {
            if fallback {
                load_settings_file(file, &backup)
            } else {
                read_object(file)
            }
        };
        AppConfig {
            values: read(&path).unwrap_or_default(),
            hotkeys: read(&hotkeys_path),
            path: Some(path),
            hotkeys_path: Some(hotkeys_path),
        }
    }

    /// Settings that live in memory only (previews and tests).
    pub fn in_memory() -> AppConfig {
        AppConfig::default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let backup = self.backup_folder();
        if let (Some(path), Some(backup)) = (&self.path, &backup) {
            let text = serde_json::to_vec_pretty(&Value::Object(self.values.clone()))?;
            save_settings_file(path, &text, backup)?;
        }
        if let (Some(path), Some(hotkeys), Some(backup)) =
            (&self.hotkeys_path, &self.hotkeys, &backup)
        {
            let text = serde_json::to_vec_pretty(&Value::Object(hotkeys.clone()))?;
            save_settings_file(path, &text, backup)?;
        }
        Ok(())
    }

    /// `SettingManager.BackupFolder`: the copies the saves keep, and the synced documents
    /// backed up before they are replaced.
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

    /// `MainWindow.SaveWindowBounds`: minimised is not a place, so nothing is recorded;
    /// maximised is recorded as a state and never as a size (a screen-sized "normal" size
    /// would leave "restore down" nothing to restore to); only a window in its normal
    /// state records where it is (`Position`) and how big its client area is
    /// (`ClientSize`, rounded). Whether to use them is `RememberMainForm*`'s business
    /// at the next start. Returns whether anything changed.
    pub fn record_main_window(&mut self, window: &WindowSnapshot) -> bool {
        if window.minimized {
            return false;
        }
        let members = |config: &AppConfig| {
            ["MainFormSize", "MainFormPosition", "MainFormMaximized"]
                .map(|member| config.get(member).cloned())
        };
        let before = members(self);
        let normal = !window.maximized && !window.fullscreen;
        let round = |(a, b): (f32, f32)| (a.round() as i64, b.round() as i64);
        self.set_main_window(
            window.size.filter(|_| normal).map(round),
            window.position.filter(|_| normal).map(round),
            window.maximized,
        );
        members(self) != before
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

/// The main window as the system reports it in a frame, in points: its state, the top
/// left corner of its frame and the size of its client area.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowSnapshot {
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub position: Option<(f32, f32)>,
    pub size: Option<(f32, f32)>,
}

/// A screen's working area (without the menu bar and the Dock, or the taskbar):
/// x, y, width, height in points, origin at the top left of the primary screen.
pub type ScreenArea = (f32, f32, f32, f32);

/// The size and the position the main window opens with, each when remembered.
pub type RestoredBounds = (Option<(f32, f32)>, Option<(f32, f32)>);

/// `MainWindow.RestoreWindowBounds`: the size (never below `min_size`) and the place the
/// main window opens with, each `None` when it is not remembered. Both are put back on
/// the screen the window would mostly be on (the primary one, `screens[0]`, when it
/// would be on none): a place remembered against a display that has since been
/// unplugged would otherwise open the window nowhere. `screens` empty: no correction.
pub fn restore_main_window(
    bounds: &MainWindowBounds,
    default_size: (f32, f32),
    min_size: (f32, f32),
    screens: &[ScreenArea],
) -> RestoredBounds {
    let want_size = bounds.remember_size && bounds.size.is_some();
    let want_position = bounds.remember_position && bounds.position.is_some();
    if !want_size && !want_position {
        return (None, None);
    }
    let (mut width, mut height) = match bounds.size.filter(|_| want_size) {
        Some((w, h)) => ((w as f32).max(min_size.0), (h as f32).max(min_size.1)),
        None => default_size,
    };
    let (mut x, mut y) = bounds
        .position
        .map(|(x, y)| (x as f32, y as f32))
        .unwrap_or_default();
    // `FindScreenFor`: the most overlap wins; none at all means the primary screen.
    let overlap = |&(sx, sy, sw, sh): &ScreenArea| {
        let w = ((x + width).min(sx + sw) - x.max(sx)).max(0.0);
        let h = ((y + height).min(sy + sh) - y.max(sy)).max(0.0);
        w * h
    };
    let mut screen = screens.first();
    let mut best = 0.0;
    for candidate in screens {
        let area = overlap(candidate);
        if area > best {
            best = area;
            screen = Some(candidate);
        }
    }
    if let Some(&(sx, sy, sw, sh)) = screen {
        width = width.min(min_size.0.max(sw));
        height = height.min(min_size.1.max(sh));
        x = x.clamp(sx, sx.max(sx + sw - width));
        y = y.clamp(sy, sy.max(sy + sh - height));
    }
    (
        want_size.then_some((width, height)),
        want_position.then_some((x, y)),
    )
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

    /// `SettingsBase.Load` with fallback support: a settings file that does not read
    /// (cut short by a crash, emptied, edited wrong) is not the end of the settings; the
    /// temporary file of an unfinished save, the copy the last save kept and the newest
    /// weekly copy are tried in that order.
    #[test]
    fn a_broken_settings_file_falls_back_to_what_the_saves_left() {
        use crate::l10n::LanguageSetting;
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path();
        let backup = profile.join(BACKUP_FOLDER);
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(
            backup.join(APPLICATION_CONFIG_FILE),
            r#"{"Language":"Polish","ModlightPageSize":30}"#,
        )
        .unwrap();
        for broken in ["", "{\"Language\":\"Pol", "null", "[1,2]"] {
            std::fs::write(profile.join(APPLICATION_CONFIG_FILE), broken).unwrap();
            let config = AppConfig::load(profile);
            assert_eq!(config.language(), LanguageSetting::Polish, "{broken:?}");
            assert_eq!(config.page_size(), 30, "{broken:?}");
        }
        // What an unfinished save wrote comes before the backup.
        let temp = profile.join("ApplicationConfig.json.temp");
        std::fs::write(&temp, r#"{"Language":"English"}"#).unwrap();
        assert_eq!(
            AppConfig::load(profile).language(),
            LanguageSetting::English
        );
        std::fs::remove_file(&temp).unwrap();
        // No backup copy: the newest weekly one.
        std::fs::remove_file(backup.join(APPLICATION_CONFIG_FILE)).unwrap();
        std::fs::write(
            backup.join("ApplicationConfig-2026-09-W39.json"),
            r#"{"ModlightPageSize":60}"#,
        )
        .unwrap();
        std::fs::write(
            backup.join("ApplicationConfig-2026-10-W43.json"),
            r#"{"ModlightPageSize":120,"ModlightLibraryPanel":true}"#,
        )
        .unwrap();
        let config = AppConfig::load(profile);
        assert_eq!(config.page_size(), 120);
        assert!(config.library_panel());
        // The hotkeys file the same way.
        std::fs::write(profile.join(HOTKEYS_CONFIG_FILE), "{\"Hotkeys\":[{").unwrap();
        std::fs::write(
            backup.join(HOTKEYS_CONFIG_FILE),
            r#"{"Hotkeys":[{"HotkeyInfo":{"Hotkey":"F13"}}]}"#,
        )
        .unwrap();
        let hotkeys = AppConfig::load(profile).hotkeys().cloned().unwrap();
        assert_eq!(hotkeys["Hotkeys"][0]["HotkeyInfo"]["Hotkey"], json!("F13"));
        // A file that reads is used as it is, whatever the backups hold.
        std::fs::write(
            profile.join(APPLICATION_CONFIG_FILE),
            r#"{"ShowTray":false}"#,
        )
        .unwrap();
        let config = AppConfig::load(profile);
        assert!(!config.show_tray());
        assert_eq!(config.page_size(), 120, "the default, not the backup's");
        assert!(!config.library_panel());
    }

    /// `SettingsBase.SaveInternal` as `SettingManager` sets it up (`CreateBackup`,
    /// `CreateWeeklyBackup`): each save keeps the file it replaces in `Backup`, and the
    /// first save of a week leaves a dated copy there too.
    #[test]
    fn a_save_keeps_the_file_it_replaces_and_a_weekly_copy() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path();
        let backup = profile.join(BACKUP_FOLDER);
        let mut config = AppConfig::load(profile);
        config.set_page_size(30);
        config.set_hotkeys(json!({"Hotkeys": []}).as_object().unwrap().clone());
        config.save().unwrap();
        config.set_page_size(60);
        config.set_hotkeys(json!({"Hotkeys": [{}]}).as_object().unwrap().clone());
        config.save().unwrap();
        assert_eq!(
            read_object(&backup.join(APPLICATION_CONFIG_FILE)).unwrap()["ModlightPageSize"],
            json!(30)
        );
        assert_eq!(
            read_object(&backup.join(HOTKEYS_CONFIG_FILE)).unwrap()["Hotkeys"],
            json!([])
        );
        let names: Vec<String> = std::fs::read_dir(&backup)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let weekly: Vec<&String> = names
            .iter()
            .filter(|n| n.starts_with("ApplicationConfig-"))
            .collect();
        assert_eq!(weekly.len(), 1, "{names:?}");
        assert!(
            names.iter().any(|n| n.starts_with("HotkeysConfig-")),
            "{names:?}"
        );
        assert!(!profile.join("ApplicationConfig.json.temp").exists());
        // A broken file is not kept over the good copy.
        std::fs::write(profile.join(APPLICATION_CONFIG_FILE), "{\"Modl").unwrap();
        config.set_page_size(120);
        config.save().unwrap();
        assert_eq!(
            read_object(&backup.join(APPLICATION_CONFIG_FILE)).unwrap()["ModlightPageSize"],
            json!(30)
        );
        assert_eq!(AppConfig::load(profile).page_size(), 120);
    }

    /// `BackupFileWeekly`: `{name}-{yyyy-MM}-W{week:00}{ext}`, week by `WeekOfYear`
    /// (the first day rule, weeks from Monday), written once.
    #[test]
    fn the_weekly_copy_is_named_like_upstream_names_it() {
        use chrono::TimeZone;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(APPLICATION_CONFIG_FILE);
        let folder = dir.path().join(BACKUP_FOLDER);
        std::fs::write(&file, "{\"a\":1}").unwrap();
        let saturday = chrono::Local
            .with_ymd_and_hms(2026, 10, 24, 12, 0, 0)
            .unwrap();
        backup_weekly(&file, &folder, saturday).unwrap();
        let weekly = folder.join("ApplicationConfig-2026-10-W43.json");
        assert_eq!(std::fs::read(&weekly).unwrap(), b"{\"a\":1}");
        std::fs::write(&file, "{\"a\":2}").unwrap();
        backup_weekly(&file, &folder, saturday).unwrap();
        assert_eq!(std::fs::read(&weekly).unwrap(), b"{\"a\":1}", "once a week");
        let new_year = chrono::Local.with_ymd_and_hms(2027, 1, 1, 9, 0, 0).unwrap();
        backup_weekly(&file, &folder, new_year).unwrap();
        assert!(folder.join("ApplicationConfig-2027-01-W01.json").exists());
    }

    /// `SaveWindowBounds`: what a frame reports goes into `MainFormSize`,
    /// `MainFormPosition` and `MainFormMaximized` the way upstream writes them.
    #[test]
    fn the_main_window_is_recorded_as_upstream_records_it() {
        let mut config = AppConfig::in_memory();
        let normal = WindowSnapshot {
            position: Some((100.4, 50.6)),
            size: Some((1200.2, 800.7)),
            ..Default::default()
        };
        // Minimised is not a place.
        assert!(!config.record_main_window(&WindowSnapshot {
            minimized: true,
            ..normal
        }));
        assert_eq!(config.get("MainFormSize"), None);
        // A window in its normal state: its place and its client size, rounded.
        assert!(config.record_main_window(&normal));
        assert_eq!(config.get("MainFormSize"), Some(&json!("1200, 801")));
        assert_eq!(config.get("MainFormPosition"), Some(&json!("100, 51")));
        assert_eq!(config.get("MainFormMaximized"), Some(&json!(false)));
        assert!(!config.record_main_window(&normal), "nothing new");
        // Maximised is a state, never a size.
        assert!(config.record_main_window(&WindowSnapshot {
            maximized: true,
            position: Some((0.0, 25.0)),
            size: Some((1512.0, 920.0)),
            ..Default::default()
        }));
        let bounds = config.main_window();
        assert!(bounds.maximized);
        assert_eq!(bounds.size, Some((1200, 801)));
        assert_eq!(bounds.position, Some((100, 51)));
        // Full screen is neither a size nor maximised.
        assert!(config.record_main_window(&WindowSnapshot {
            fullscreen: true,
            position: Some((0.0, 0.0)),
            size: Some((1512.0, 982.0)),
            ..Default::default()
        }));
        let bounds = config.main_window();
        assert!(!bounds.maximized);
        assert_eq!(bounds.size, Some((1200, 801)));
        // The switches only decide what the next start uses; the bounds are kept.
        config.set("RememberMainFormSize", json!(false));
        assert!(config.record_main_window(&WindowSnapshot {
            size: Some((900.0, 600.0)),
            ..normal
        }));
        assert_eq!(config.main_window().size, Some((900, 600)));
    }

    /// `RestoreWindowBounds`: remembered bounds come back on a screen that exists.
    #[test]
    fn a_remembered_window_opens_on_a_screen_that_exists() {
        let default = (1040.0, 720.0);
        let min = (880.0, 540.0);
        let laptop: ScreenArea = (0.0, 33.0, 1512.0, 916.0);
        let right: ScreenArea = (1512.0, 0.0, 2560.0, 1415.0);
        let bounds = |size, position| MainWindowBounds {
            remember_size: true,
            remember_position: true,
            size: Some(size),
            position: Some(position),
            maximized: false,
        };
        // On a screen that is there: as it was.
        assert_eq!(
            restore_main_window(&bounds((1200, 800), (100, 60)), default, min, &[laptop]),
            (Some((1200.0, 800.0)), Some((100.0, 60.0)))
        );
        // The display on the right is gone: back onto the one there is.
        assert_eq!(
            restore_main_window(&bounds((1200, 800), (2400, 200)), default, min, &[laptop]),
            (Some((1200.0, 800.0)), Some((312.0, 149.0)))
        );
        // With it plugged in, the window stays there.
        assert_eq!(
            restore_main_window(
                &bounds((1200, 800), (2400, 200)),
                default,
                min,
                &[laptop, right]
            ),
            (Some((1200.0, 800.0)), Some((2400.0, 200.0)))
        );
        // Larger than the screen: the screen's size; smaller than the minimum: the minimum.
        assert_eq!(
            restore_main_window(&bounds((3000, 2000), (0, 33)), default, min, &[laptop]),
            (Some((1512.0, 916.0)), Some((0.0, 33.0)))
        );
        assert_eq!(
            restore_main_window(&bounds((300, 200), (10, 40)), default, min, &[laptop]),
            (Some((880.0, 540.0)), Some((10.0, 40.0)))
        );
        // Above the menu bar or past the left edge: inside the working area.
        assert_eq!(
            restore_main_window(&bounds((1000, 700), (-50, 5)), default, min, &[laptop]).1,
            Some((0.0, 33.0))
        );
        // Not remembered, or never recorded: nothing.
        let mut off = bounds((1200, 800), (100, 60));
        off.remember_size = false;
        off.remember_position = false;
        assert_eq!(
            restore_main_window(&off, default, min, &[laptop]),
            (None, None)
        );
        let never = MainWindowBounds {
            size: None,
            position: None,
            ..bounds((0, 0), (0, 0))
        };
        assert_eq!(
            restore_main_window(&never, default, min, &[laptop]),
            (None, None)
        );
        // No screens known: no correction.
        assert_eq!(
            restore_main_window(&bounds((1200, 800), (5000, 5000)), default, min, &[]),
            (Some((1200.0, 800.0)), Some((5000.0, 5000.0)))
        );
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

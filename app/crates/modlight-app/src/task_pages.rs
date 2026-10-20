//! The task settings pages (`TaskSettingsPageBuilder`): notifications, image, effects,
//! thumbnail, capture, region capture, screen recorder, OCR, file naming, clipboard
//! upload, tools, actions, watch folders and advanced, over a `TaskSettings` object.
//!
//! In the settings window they edit the default task settings; a hotkey's task settings
//! window shows the same pages with the "override" cards that say which groups the
//! hotkey keeps for itself.

use serde_json::{Value, json};

use modlight_core::l10n::{Arg, format, s, t};

use crate::settings_kit::{
    Binder, card, check, check_inverted, compact_button, enum_combo, hint, list_frame, list_row,
    number, override_card, row, text,
};
use crate::theme::{Colors, Weight};

/// What a page asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskPageAsk {
    /// Choose the custom region with the overlay (`Select region...`).
    SelectRegion,
    /// The FFmpeg options window.
    ScreenRecordingOptions,
    /// The notification buttons editor.
    NotificationButtons,
    /// Choose a WAV for a custom sound (the member's path).
    PickSound(Vec<String>),
    /// The action editor (`None`: a new action).
    EditAction(Option<usize>),
    /// The watch folder editor (`None`: a new folder).
    EditWatchFolder(Option<usize>),
    /// Watch folders changed: start or stop them.
    WatchFoldersChanged,
    /// Image effects configuration (phase 5).
    ImageEffects,
    /// Ask the server for the folders (`ModlightFolderPicker.LoadAsync`).
    LoadModlightFolders,
    /// Choose the screenshots folder of a hotkey's settings.
    PickScreenshotsFolder,
}

/// Selection state the pages keep between frames, and the editor open over them.
#[derive(Default, Clone)]
pub struct TaskPagesState {
    pub selected_action: Option<usize>,
    pub selected_watch_folder: Option<usize>,
    pub selected_snap: Option<usize>,
    pub snap_width: f64,
    pub snap_height: f64,
    pub editor: Option<crate::task_editors::TaskEditor>,
    /// The Modlight folder picker of a hotkey's upload page.
    pub folders: ModlightFolders,
    /// The open task or task-flag menu of the Task page.
    pub menu: Option<&'static str>,
}

/// `ModlightFolderPicker`: what the server said the folders are.
#[derive(Clone, Debug, Default)]
pub struct ModlightFolders {
    pub requested: bool,
    pub loading: bool,
    /// The line under the picker (signed out, loading, missing, rejected, failed).
    pub status: String,
    /// `RootValue` once the server named it.
    pub root: Option<String>,
    /// Every folder: id, the path shown ("Parent / Child"), the folder's own name.
    pub folders: Option<Vec<(String, String, String)>>,
}

/// `PathOf`, at most 16 levels up.
pub fn folder_choices(list: &modlight_api::FolderList) -> Vec<(String, String, String)> {
    let folders = list.folders.clone().unwrap_or_default();
    let by_id: std::collections::HashMap<String, &modlight_api::Folder> = folders
        .iter()
        .filter_map(|f| f.id.clone().filter(|id| !id.is_empty()).map(|id| (id, f)))
        .collect();
    let mut choices: Vec<(String, String, String)> = by_id
        .iter()
        .map(|(id, folder)| {
            let mut parts = vec![folder.name.clone().unwrap_or_default()];
            let mut parent = folder.parent_id.clone();
            for _ in 0..16 {
                let Some(parent_id) = parent.filter(|p| !p.is_empty()) else {
                    break;
                };
                let Some(up) = by_id.get(&parent_id) else {
                    break;
                };
                parts.insert(0, up.name.clone().unwrap_or_default());
                parent = up.parent_id.clone();
            }
            (
                id.clone(),
                parts.join(" / "),
                folder.name.clone().unwrap_or_default(),
            )
        })
        .collect();
    choices.sort_by_key(|(_, label, _)| label.to_lowercase());
    choices
}

/// Where the pages are shown.
pub struct TaskPagesCtx<'a> {
    pub colors: &'a Colors,
    /// The default task settings (no override cards).
    pub is_default: bool,
    pub state: &'a mut TaskPagesState,
    pub asks: Vec<TaskPageAsk>,
    /// `Program.Settings.NameParserAutoIncrementNumber`, read and written by the naming page.
    pub auto_increment: &'a mut i64,
    pub auto_increment_changed: bool,
    /// `DevMode`: the recorder's higher frame rate limits.
    pub dev_mode: bool,
    /// Whether file and folder pickers may open (never in automated runs).
    pub dialogs: bool,
}

const TASK_ROW: f32 = 270.0;

fn options(type_name: &str, names: &[&'static str]) -> Vec<(&'static str, String)> {
    names
        .iter()
        .map(|n| (*n, modlight_core::l10n::enum_text(type_name, n, None)))
        .collect()
}

/// `OverrideValue`: true in the defaults; else whether the hotkey overrides the group.
fn group_enabled(b: &Binder, ctx: &TaskPagesCtx, use_default: &str) -> bool {
    ctx.is_default || !b.bool(&[use_default], true)
}

/// `OverrideCard`: only in a hotkey's settings.
fn override_group(
    ui: &mut egui::Ui,
    ctx: &TaskPagesCtx,
    b: &mut Binder,
    use_default: &str,
    label: &str,
) {
    if ctx.is_default {
        return;
    }
    let mut value = !b.bool(&[use_default], true);
    if override_card(ui, ctx.colors, label, &mut value) {
        b.set(&[use_default], json!(!value));
    }
}

/// Open the editors the pages asked for, draw the open one over `area`, and write what
/// it saved. Picking a sound happens here too. The asks left are the app's.
pub fn editors(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder, area: egui::Rect) {
    use crate::task_editors::{
        ActionEditor, ButtonsEditor, EditorResult, TaskEditor, WatchFolderEditor,
    };
    let asks = std::mem::take(&mut ctx.asks);
    for ask in asks {
        match ask {
            TaskPageAsk::EditAction(index) => {
                let current = index
                    .and_then(|i| b.get(&["ExternalPrograms"]).and_then(|l| l.get(i)).cloned());
                ctx.state.editor = Some(TaskEditor::Action(ActionEditor::new(
                    index,
                    current.as_ref(),
                )));
            }
            TaskPageAsk::EditWatchFolder(index) => {
                let current =
                    index.and_then(|i| b.get(&["WatchFolderList"]).and_then(|l| l.get(i)).cloned());
                ctx.state.editor = Some(TaskEditor::WatchFolder(WatchFolderEditor::new(
                    index,
                    current.as_ref(),
                )));
            }
            TaskPageAsk::NotificationButtons => {
                let current = b.get(&["GeneralSettings", "ToastWindowButtons"]).cloned();
                ctx.state.editor = Some(TaskEditor::Buttons(ButtonsEditor::new(current.as_ref())));
            }
            TaskPageAsk::PickScreenshotsFolder => {
                if ctx.dialogs
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_title(t(s::TaskSettingsWindow_ChooseScreenshotsFolder))
                        .pick_folder()
                {
                    b.set(&["ScreenshotsFolder"], json!(folder.display().to_string()));
                }
            }
            TaskPageAsk::PickSound(path) => {
                if ctx.dialogs
                    && let Some(file) = rfd::FileDialog::new()
                        .set_title(t(s::TaskSettingsWindow_ChooseAudioFile))
                        .add_filter(t(s::TaskSettingsWindow_WaveAudio), &["wav"])
                        .pick_file()
                {
                    let path: Vec<&str> = path.iter().map(String::as_str).collect();
                    b.set(&path, json!(file.display().to_string()));
                }
            }
            other => ctx.asks.push(other),
        }
    }
    let Some(editor) = ctx.state.editor.as_mut() else {
        return;
    };
    match crate::task_editors::show(ui, ctx.colors, area, editor, ctx.dialogs) {
        EditorResult::Open => {}
        EditorResult::Cancelled => ctx.state.editor = None,
        EditorResult::Action(index, action) => {
            ctx.state.editor = None;
            let mut list = match b.get(&["ExternalPrograms"]) {
                Some(Value::Array(list)) => list.clone(),
                _ => Vec::new(),
            };
            match index.filter(|i| *i < list.len()) {
                Some(i) => list[i] = action,
                None => {
                    list.push(action);
                    ctx.state.selected_action = Some(list.len() - 1);
                }
            }
            b.set(&["ExternalPrograms"], Value::Array(list));
        }
        EditorResult::WatchFolder(index, folder) => {
            ctx.state.editor = None;
            let mut list = match b.get(&["WatchFolderList"]) {
                Some(Value::Array(list)) => list.clone(),
                _ => Vec::new(),
            };
            match index.filter(|i| *i < list.len()) {
                Some(i) => list[i] = folder,
                None => {
                    list.push(folder);
                    ctx.state.selected_watch_folder = Some(list.len() - 1);
                }
            }
            b.set(&["WatchFolderList"], Value::Array(list));
            ctx.asks.push(TaskPageAsk::WatchFoldersChanged);
        }
        EditorResult::Buttons(buttons) => {
            ctx.state.editor = None;
            b.set(
                &["GeneralSettings", "ToastWindowButtons"],
                Value::Array(buttons),
            );
        }
    }
}

/// Draw one task page. Returns false when `section` is not a task page.
pub fn page(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder, section: &str) -> bool {
    match section {
        "task.task" => task(ui, ctx, b),
        "task.general" => override_group(
            ui,
            ctx,
            b,
            "UseDefaultGeneralSettings",
            t(s::TaskSettingsWindow_OverrideGeneralSettings),
        ),
        "task.upload" => override_group(
            ui,
            ctx,
            b,
            "UseDefaultUploadSettings",
            t(s::TaskSettingsWindow_OverrideUploadSettings),
        ),
        "task.upload-modlight" => modlight_folder(ui, ctx, b),
        "task.general-notifications" => notifications(ui, ctx, b),
        "task.image" => image(ui, ctx, b),
        "task.image-effects" => effects(ui, ctx, b),
        "task.image-thumbnail" => thumbnail(ui, ctx, b),
        "task.capture" => capture(ui, ctx, b),
        "task.capture-region" => region(ui, ctx, b),
        "task.capture-screen-recorder" => recorder(ui, ctx, b),
        "task.capture-ocr" => ocr(ui, ctx, b),
        "task.upload-file-naming" => naming(ui, ctx, b),
        "task.upload-clipboard" => clipboard(ui, ctx, b),
        "task.tools" => tools(ui, ctx, b),
        "task.actions" => actions(ui, ctx, b),
        "task.watch-folders" => watch_folders(ui, ctx, b),
        "task.advanced" => advanced(ui, ctx, b),
        _ => return false,
    }
    true
}

fn notifications(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultGeneralSettings");
    let g = |m: &'static str| ["GeneralSettings", m];
    card(ui, colors, t(s::TaskSettingsWindow_Sounds), on, |ui| {
        check(
            ui,
            colors,
            b,
            &g("PlaySoundAfterCapture"),
            true,
            t(s::TaskSettingsWindow_PlaySoundAfterCaptureIsMade),
        );
        check(
            ui,
            colors,
            b,
            &g("PlaySoundAfterUpload"),
            true,
            t(s::TaskSettingsWindow_PlaySoundAfterTaskIsCompleted),
        );
        check(
            ui,
            colors,
            b,
            &g("PlaySoundAfterAction"),
            true,
            t(s::TaskSettingsWindow_PlaySoundAfterActionIsCompleted),
        );
    });
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_ToastNotification),
        on,
        |ui| {
            let show = check(
                ui,
                colors,
                b,
                &g("ShowToastNotificationAfterTaskCompleted"),
                true,
                t(s::TaskSettingsWindow_ShowToastNotificationAfterTaskIsCompleted),
            );
            ui.add_enabled_ui(show, |ui| {
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_DurationSeconds),
                    |ui| {
                        number(
                            ui,
                            colors,
                            b,
                            &g("ToastWindowDuration"),
                            3.0,
                            0.0,
                            60.0,
                            0.1,
                            1,
                        )
                    },
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_FadeDurationSeconds),
                    |ui| {
                        number(
                            ui,
                            colors,
                            b,
                            &g("ToastWindowFadeDuration"),
                            1.0,
                            0.0,
                            10.0,
                            0.1,
                            1,
                        )
                    },
                );
                let placements = [
                    "TopLeft",
                    "TopCenter",
                    "TopRight",
                    "MiddleLeft",
                    "MiddleCenter",
                    "MiddleRight",
                    "BottomLeft",
                    "BottomCenter",
                    "BottomRight",
                ];
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_Placement),
                    |ui| {
                        enum_combo(
                            ui,
                            colors,
                            b,
                            &g("ToastWindowPlacement"),
                            &options("ContentAlignment", &placements),
                            "BottomRight",
                        )
                    },
                );
                size_rows(
                    ui,
                    colors,
                    b,
                    &["GeneralSettings", "ToastWindowSize"],
                    (400, 300),
                    (100, 2000),
                    (50, 2000),
                );
                let actions = [
                    "CloseNotification",
                    "AnnotateImage",
                    "CopyImageToClipboard",
                    "CopyFile",
                    "CopyFilePath",
                    "CopyUrl",
                    "OpenFile",
                    "OpenFolder",
                    "OpenUrl",
                    "Upload",
                    "PinToScreen",
                    "DeleteFile",
                ];
                let actions = options("ToastClickAction", &actions);
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_LeftClickAction),
                    |ui| {
                        enum_combo(
                            ui,
                            colors,
                            b,
                            &g("ToastWindowLeftClickAction"),
                            &actions,
                            "OpenUrl",
                        )
                    },
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_RightClickAction),
                    |ui| {
                        enum_combo(
                            ui,
                            colors,
                            b,
                            &g("ToastWindowRightClickAction"),
                            &actions,
                            "CloseNotification",
                        )
                    },
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_MiddleClickAction),
                    |ui| {
                        enum_combo(
                            ui,
                            colors,
                            b,
                            &g("ToastWindowMiddleClickAction"),
                            &actions,
                            "AnnotateImage",
                        )
                    },
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_NotificationButtonSize),
                    |ui| {
                        number(
                            ui,
                            colors,
                            b,
                            &g("ToastWindowButtonSize"),
                            40.0,
                            16.0,
                            128.0,
                            1.0,
                            0,
                        )
                    },
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_NotificationButtonsLabel),
                    |ui| {
                        if compact_button(
                            ui,
                            colors,
                            t(s::TaskSettingsWindow_ConfigureWithEllipsis),
                        )
                        .clicked()
                        {
                            ctx.asks.push(TaskPageAsk::NotificationButtons);
                        }
                    },
                );
                check(
                    ui,
                    colors,
                    b,
                    &g("ToastWindowAutoHide"),
                    true,
                    t(s::TaskSettingsWindow_AutomaticallyHideOnScreenCapture),
                );
                check(
                    ui,
                    colors,
                    b,
                    &g("DisableNotificationsOnFullscreen"),
                    false,
                    t(s::TaskSettingsWindow_DisableToastNotificationsOnFullscreen),
                );
            });
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_CustomSounds),
        on,
        |ui| {
            for (use_member, path_member, label) in [
                (
                    "UseCustomCaptureSound",
                    "CustomCaptureSoundPath",
                    t(s::TaskSettingsWindow_UseCustomCaptureSound),
                ),
                (
                    "UseCustomTaskCompletedSound",
                    "CustomTaskCompletedSoundPath",
                    t(s::TaskSettingsWindow_UseCustomTaskCompletedSound),
                ),
                (
                    "UseCustomActionCompletedSound",
                    "CustomActionCompletedSoundPath",
                    t(s::TaskSettingsWindow_UseCustomActionCompletedSound),
                ),
                (
                    "UseCustomErrorSound",
                    "CustomErrorSoundPath",
                    t(s::TaskSettingsWindow_UseCustomErrorSound),
                ),
            ] {
                let enabled = check(ui, colors, b, &g(use_member), false, label);
                ui.add_enabled_ui(enabled, |ui| {
                    ui.horizontal(|ui| {
                        let width = ui.available_width() - 6.0 - 40.0;
                        ui.allocate_ui(egui::vec2(width, 32.0), |ui| {
                            text(ui, colors, b, &g(path_member), "")
                        });
                        ui.add_space(6.0);
                        if compact_button(ui, colors, "...").clicked() {
                            ctx.asks.push(TaskPageAsk::PickSound(vec![
                                "GeneralSettings".into(),
                                path_member.into(),
                            ]));
                        }
                    });
                });
                ui.add_space(3.0);
            }
        },
    );
}

/// Width and height rows of a `Size` member ("400, 300").
fn size_rows(
    ui: &mut egui::Ui,
    colors: &Colors,
    b: &mut Binder,
    path: &[&str],
    default: (i64, i64),
    w: (i64, i64),
    h: (i64, i64),
) {
    let current = b
        .get(path)
        .and_then(|v| v.as_str())
        .and_then(|t| t.split_once(','))
        .and_then(|(a, c)| Some((a.trim().parse::<i64>().ok()?, c.trim().parse::<i64>().ok()?)))
        .unwrap_or(default);
    let mut width = current.0 as f64;
    let mut height = current.1 as f64;
    let id = path.join(".");
    let changed_w = row(ui, colors, TASK_ROW, t(s::TaskSettingsWindow_Width), |ui| {
        crate::settings_kit::number_value(
            ui,
            colors,
            &format!("{id}.w"),
            &mut width,
            w.0 as f64,
            w.1 as f64,
            1.0,
            0,
        )
    });
    let changed_h = row(
        ui,
        colors,
        TASK_ROW,
        t(s::TaskSettingsWindow_Height),
        |ui| {
            crate::settings_kit::number_value(
                ui,
                colors,
                &format!("{id}.h"),
                &mut height,
                h.0 as f64,
                h.1 as f64,
                1.0,
                0,
            )
        },
    );
    if changed_w || changed_h {
        b.set(path, json!(format!("{}, {}", width as i64, height as i64)));
    }
}

fn image(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    override_group(
        ui,
        ctx,
        b,
        "UseDefaultImageSettings",
        t(s::TaskSettingsWindow_OverrideImageSettings),
    );
    let on = group_enabled(b, ctx, "UseDefaultImageSettings");
    let i = |m: &'static str| ["ImageSettings", m];
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_ImageQuality),
        on,
        |ui| {
            let formats: Vec<(&'static str, String)> = ["PNG", "JPEG", "GIF", "BMP", "TIFF"]
                .iter()
                .zip(["png", "jpg", "gif", "bmp", "tif"])
                .map(|(n, d)| {
                    (
                        *n,
                        modlight_core::l10n::enum_text("EImageFormat", n, Some(d)),
                    )
                })
                .collect();
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_ImageFormat),
                |ui| enum_combo(ui, colors, b, &i("ImageFormat"), &formats, "PNG"),
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_PNGBitDepth),
                |ui| {
                    enum_combo(
                        ui,
                        colors,
                        b,
                        &i("ImagePNGBitDepth"),
                        &options("PNGBitDepth", &["Default", "Automatic", "Bit32", "Bit24"]),
                        "Default",
                    )
                },
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_JPEGQuality),
                |ui| {
                    number(
                        ui,
                        colors,
                        b,
                        &i("ImageJPEGQuality"),
                        90.0,
                        0.0,
                        100.0,
                        1.0,
                        0,
                    )
                },
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_GIFQuality),
                |ui| {
                    enum_combo(
                        ui,
                        colors,
                        b,
                        &i("ImageGIFQuality"),
                        &options("GIFQuality", &["Default", "Bit8", "Bit4", "Grayscale"]),
                        "Default",
                    )
                },
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_IfFileExists),
                |ui| {
                    enum_combo(
                        ui,
                        colors,
                        b,
                        &i("FileExistAction"),
                        &options(
                            "FileExistAction",
                            &["Ask", "Overwrite", "UniqueName", "Cancel"],
                        ),
                        "Ask",
                    )
                },
            );
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_AutomaticJPEG),
        on,
        |ui| {
            let auto = check(
                ui,
                colors,
                b,
                &i("ImageAutoUseJPEG"),
                true,
                t(s::TaskSettingsWindow_UseJPEGIfImageSizeIsBiggerThanTheSpecifiedSize),
            );
            ui.add_enabled_ui(auto, |ui| {
            row(ui, colors, TASK_ROW, t(s::TaskSettingsWindow_SizeLimitKB), |ui| {
                number(ui, colors, b, &i("ImageAutoUseJPEGSize"), 2048.0, 0.0, i32::MAX as f64, 1.0, 0)
            });
            check(
                ui,
                colors,
                b,
                &i("ImageAutoJPEGQuality"),
                false,
                t(s::TaskSettingsWindow_AdjustJPEGQualityAutomaticallyToKeepImageSizeNearTheSpecifiedSize),
            );
        });
        },
    );
}

fn effects(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultImageSettings");
    let i = |m: &'static str| ["ImageSettings", m];
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_ImageEffects),
        on,
        |ui| {
            check(
                ui,
                colors,
                b,
                &i("ShowImageEffectsWindowAfterCapture"),
                false,
                t(s::TaskSettingsWindow_ShowImageEffectsWindowAfterCapture),
            );
            check(
                ui,
                colors,
                b,
                &i("ImageEffectOnlyRegionCapture"),
                false,
                t(s::TaskSettingsWindow_OnlyApplyEffectsToRegionCapture),
            );
            check(
                ui,
                colors,
                b,
                &i("UseRandomImageEffect"),
                false,
                t(s::TaskSettingsWindow_UseRandomImageEffect),
            );
            if compact_button(
                ui,
                colors,
                t(s::TaskSettingsWindow_ImageEffectsConfigurationWithEllipsis),
            )
            .clicked()
            {
                ctx.asks.push(TaskPageAsk::ImageEffects);
            }
            hint(ui, colors, t(s::TaskSettingsWindow_YouCanEnableOrDisableImageEffectsFromAfterCaptureTasksAddImageEffects));
        },
    );
}

fn thumbnail(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultImageSettings");
    let i = |m: &'static str| ["ImageSettings", m];
    card(ui, colors, t(s::TaskSettingsWindow_Thumbnail), on, |ui| {
        row(ui, colors, TASK_ROW, t(s::TaskSettingsWindow_Width), |ui| {
            number(
                ui,
                colors,
                b,
                &i("ThumbnailWidth"),
                200.0,
                0.0,
                10000.0,
                1.0,
                0,
            )
        });
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_Height),
            |ui| {
                number(
                    ui,
                    colors,
                    b,
                    &i("ThumbnailHeight"),
                    0.0,
                    0.0,
                    10000.0,
                    1.0,
                    0,
                )
            },
        );
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_ThumbnailName),
            |ui| text(ui, colors, b, &i("ThumbnailName"), "-thumbnail"),
        );
        let name = b.str(&i("ThumbnailName"), "-thumbnail");
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_Preview),
            |ui| {
                hint(
                    ui,
                    colors,
                    &format!("{}{name}.jpg", t(s::TaskSettingsWindow_ImageNameSample)),
                )
            },
        );
        check(
            ui,
            colors,
            b,
            &i("ThumbnailCheckSize"),
            false,
            t(s::TaskSettingsWindow_CreateThumbnailOnlyIfImageIsBiggerThanThumbnailSize),
        );
    });
}

fn capture(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    override_group(
        ui,
        ctx,
        b,
        "UseDefaultCaptureSettings",
        t(s::TaskSettingsWindow_OverrideCaptureSettings),
    );
    let on = group_enabled(b, ctx, "UseDefaultCaptureSettings");
    let c = |m: &'static str| ["CaptureSettings", m];
    card(ui, colors, t(s::TaskSettingsWindow_Screenshots), on, |ui| {
        check(
            ui,
            colors,
            b,
            &c("ShowCursor"),
            true,
            t(s::TaskSettingsWindow_ShowCursorInScreenshots),
        );
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_ScreenshotDelaySeconds),
            |ui| number(ui, colors, b, &c("ScreenshotDelay"), 0.0, 0.0, 60.0, 0.1, 1),
        );
        let transparent = check(
            ui,
            colors,
            b,
            &c("CaptureTransparent"),
            false,
            t(s::TaskSettingsWindow_CaptureWindowWithTransparency),
        );
        ui.add_enabled_ui(transparent, |ui| {
            check(
                ui,
                colors,
                b,
                &c("CaptureShadow"),
                true,
                t(s::TaskSettingsWindow_CaptureWindowWithShadow),
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_ShadowOffset),
                |ui| {
                    number(
                        ui,
                        colors,
                        b,
                        &c("CaptureShadowOffset"),
                        100.0,
                        0.0,
                        1000.0,
                        1.0,
                        0,
                    )
                },
            );
        });
        check(
            ui,
            colors,
            b,
            &c("CaptureClientArea"),
            false,
            t(s::TaskSettingsWindow_CaptureClientAreaForWindowCaptures),
        );
        check(
            ui,
            colors,
            b,
            &c("CaptureAutoHideTaskbar"),
            false,
            t(s::TaskSettingsWindow_HideTaskbarWhenItIntersectsACapturedWindow),
        );
        check(
            ui,
            colors,
            b,
            &c("CaptureAutoHideDesktopIcons"),
            false,
            t(s::TaskSettingsWindow_AutomaticallyHideDesktopIcons),
        );
        check(
            ui,
            colors,
            b,
            &c("HDRScreenshotColorCorrection"),
            false,
            t(s::TaskSettingsWindow_HDRScreenshotColorCorrector),
        );
    });
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_PreconfiguredRegion),
        on,
        |ui| {
            let rect = b
                .get(&c("CaptureCustomRegion"))
                .and_then(modlight_core::capture::geometry::IRect::parse_value)
                .unwrap_or_default();
            let mut values = [
                rect.x as f64,
                rect.y as f64,
                rect.width as f64,
                rect.height as f64,
            ];
            let mut changed = false;
            for (line, pair) in [
                (
                    0usize,
                    [
                        (s::TaskSettingsWindow_X, 0usize),
                        (s::TaskSettingsWindow_Y, 1),
                    ],
                ),
                (
                    1,
                    [
                        (s::TaskSettingsWindow_Width, 2),
                        (s::TaskSettingsWindow_Height, 3),
                    ],
                ),
            ] {
                ui.horizontal(|ui| {
                    for (label, index) in pair {
                        let (min, max) = if index < 2 {
                            (-100000.0, 100000.0)
                        } else {
                            (0.0, 100000.0)
                        };
                        crate::widgets::text(
                            ui,
                            t(label),
                            13.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                        ui.add_space(8.0);
                        if crate::settings_kit::number_value(
                            ui,
                            colors,
                            &format!("custom-region-{line}-{index}"),
                            &mut values[index],
                            min,
                            max,
                            1.0,
                            0,
                        ) {
                            changed = true;
                        }
                        ui.add_space(8.0);
                    }
                });
            }
            if changed {
                let r = modlight_core::capture::geometry::IRect::new(
                    values[0] as i32,
                    values[1] as i32,
                    values[2] as i32,
                    values[3] as i32,
                );
                b.set(&c("CaptureCustomRegion"), json!(r.to_text()));
            }
            if compact_button(
                ui,
                colors,
                t(s::TaskSettingsWindow_SelectRegionWithEllipsis),
            )
            .clicked()
            {
                ctx.asks.push(TaskPageAsk::SelectRegion);
            }
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_PreconfiguredWindow),
        on,
        |ui| {
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_WindowTitle),
                |ui| text(ui, colors, b, &c("CaptureCustomWindow"), ""),
            );
        },
    );
}

fn region(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultCaptureSettings");
    let o = |m: &'static str| ["CaptureSettings", "SurfaceOptions", m];
    card(ui, colors, t(s::TaskSettingsWindow_Selection), on, |ui| {
        check_inverted(
            ui,
            colors,
            b,
            &o("QuickCrop"),
            true,
            t(s::TaskSettingsWindow_UseMultiRegionMode),
        );
        let detect = check(
            ui,
            colors,
            b,
            &o("DetectWindows"),
            true,
            t(s::TaskSettingsWindow_DetectWindowRegions),
        );
        ui.add_enabled_ui(detect, |ui| {
            check(
                ui,
                colors,
                b,
                &o("DetectControls"),
                true,
                t(s::TaskSettingsWindow_AlsoDetectControlsInsideWindows),
            );
        });
        check(
            ui,
            colors,
            b,
            &o("ActiveMonitorMode"),
            false,
            t(s::TaskSettingsWindow_RestrictCaptureAndCursorToTheActiveMonitor),
        );
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_BackgroundDimStrengthPercent),
            |ui| {
                number(
                    ui,
                    colors,
                    b,
                    &o("BackgroundDimStrength"),
                    20.0,
                    0.0,
                    100.0,
                    1.0,
                    0,
                )
            },
        );
    });
    let actions = options(
        "RegionCaptureAction",
        &[
            "None",
            "CancelCapture",
            "RemoveShapeCancelCapture",
            "RemoveShape",
            "SwapToolType",
            "CaptureFullscreen",
            "CaptureActiveMonitor",
            "CaptureLastRegion",
        ],
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_MouseActions),
        on,
        |ui| {
            for (label, member, default) in [
                (
                    s::TaskSettingsWindow_RightClick,
                    "RegionCaptureActionRightClick",
                    "RemoveShapeCancelCapture",
                ),
                (
                    s::TaskSettingsWindow_MiddleClick,
                    "RegionCaptureActionMiddleClick",
                    "SwapToolType",
                ),
                (
                    s::TaskSettingsWindow_Mouse4Click,
                    "RegionCaptureActionX1Click",
                    "CaptureFullscreen",
                ),
                (
                    s::TaskSettingsWindow_Mouse5Click,
                    "RegionCaptureActionX2Click",
                    "CaptureActiveMonitor",
                ),
            ] {
                row(ui, colors, TASK_ROW, t(label), |ui| {
                    enum_combo(ui, colors, b, &o(member), &actions, default)
                });
            }
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_InformationAndMagnifier),
        on,
        |ui| {
            check(
                ui,
                colors,
                b,
                &o("ShowInfo"),
                true,
                t(s::TaskSettingsWindow_ShowPositionAndSizeInfo),
            );
            let custom = check(
                ui,
                colors,
                b,
                &o("UseCustomInfoText"),
                false,
                t(s::TaskSettingsWindow_UseCustomInfoText),
            );
            ui.add_enabled_ui(custom, |ui| {
                text(
                    ui,
                    colors,
                    b,
                    &o("CustomInfoText"),
                    "X: $x, Y: $y$nR: $r, G: $g, B: $b$nHex: $hex",
                );
            });
            let magnifier = check(
                ui,
                colors,
                b,
                &o("ShowMagnifier"),
                true,
                t(s::TaskSettingsWindow_ShowMagnifierNearCursor),
            );
            ui.add_enabled_ui(magnifier, |ui| {
                check(
                    ui,
                    colors,
                    b,
                    &o("UseSquareMagnifier"),
                    false,
                    t(s::TaskSettingsWindow_UseSquareMagnifier),
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_MagnifierPixelCount),
                    |ui| {
                        number(
                            ui,
                            colors,
                            b,
                            &o("MagnifierPixelCount"),
                            15.0,
                            3.0,
                            35.0,
                            1.0,
                            0,
                        )
                    },
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_MagnifierPixelSize),
                    |ui| {
                        number(
                            ui,
                            colors,
                            b,
                            &o("MagnifierPixelSize"),
                            10.0,
                            3.0,
                            30.0,
                            1.0,
                            0,
                        )
                    },
                );
            });
            check(
                ui,
                colors,
                b,
                &o("ShowCenterCrosshair"),
                false,
                t(s::TaskSettingsWindow_ShowCenterCrosshair),
            );
            check(
                ui,
                colors,
                b,
                &o("ShowCrosshair"),
                false,
                t(s::TaskSettingsWindow_ShowScreenWideCrosshair),
            );
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_FixedSizeAndPerformance),
        on,
        |ui| {
            let fixed = check(
                ui,
                colors,
                b,
                &o("IsFixedSize"),
                false,
                t(s::TaskSettingsWindow_FixedSizeRegionMode),
            );
            ui.add_enabled_ui(fixed, |ui| {
                let path = ["CaptureSettings", "SurfaceOptions", "FixedSize"];
                let current = b
                    .get(&path)
                    .and_then(|v| v.as_str())
                    .and_then(|t| t.split_once(','))
                    .and_then(|(a, c)| {
                        Some((a.trim().parse::<f64>().ok()?, c.trim().parse::<f64>().ok()?))
                    })
                    .unwrap_or((250.0, 250.0));
                let (mut w, mut h) = current;
                let cw = row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_FixedWidth),
                    |ui| {
                        crate::settings_kit::number_value(
                            ui, colors, "fixed-w", &mut w, 1.0, 100000.0, 1.0, 0,
                        )
                    },
                );
                let ch = row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_FixedHeight),
                    |ui| {
                        crate::settings_kit::number_value(
                            ui, colors, "fixed-h", &mut h, 1.0, 100000.0, 1.0, 0,
                        )
                    },
                );
                if cw || ch {
                    b.set(&path, json!(format!("{}, {}", w as i64, h as i64)));
                }
            });
            check(
                ui,
                colors,
                b,
                &o("ShowFPS"),
                false,
                t(s::TaskSettingsWindow_ShowFPS),
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_FPSLimit),
                |ui| number(ui, colors, b, &o("FPSLimit"), 100.0, 1.0, 1000.0, 1.0, 0),
            );
        },
    );
    card(ui, colors, t(s::TaskSettingsWindow_SnapSizes), on, |ui| {
        let path = ["CaptureSettings", "SurfaceOptions", "SnapSizes"];
        let mut sizes: Vec<(i64, i64)> = match b.get(&path) {
            Some(Value::Array(list)) => list
                .iter()
                .map(|v| {
                    (
                        v.get("Width").and_then(Value::as_i64).unwrap_or(0),
                        v.get("Height").and_then(Value::as_i64).unwrap_or(0),
                    )
                })
                .collect(),
            _ => vec![
                (426, 240),
                (640, 360),
                (854, 480),
                (1280, 720),
                (1920, 1080),
            ],
        };
        let selected = ctx.state.selected_snap;
        let mut clicked = None;
        list_frame(ui, colors, 125.0, None, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("snap-sizes")
                .max_height(125.0)
                .show(ui, |ui| {
                    for (index, (w, h)) in sizes.iter().enumerate() {
                        if list_row(ui, colors, selected == Some(index), 28.0, |ui, r| {
                            crate::controls::label_at(
                                ui,
                                r.shrink2(egui::vec2(8.0, 0.0)),
                                &format!("{w}x{h}"),
                                Weight::Normal,
                                colors.text(),
                            );
                        })
                        .clicked()
                        {
                            clicked = Some(index);
                        }
                    }
                });
        });
        if clicked.is_some() {
            ctx.state.selected_snap = clicked;
        }
        if ctx.state.snap_width == 0.0 {
            ctx.state.snap_width = 640.0;
            ctx.state.snap_height = 360.0;
        }
        let mut write = false;
        ui.horizontal(|ui| {
            crate::widgets::text(ui, "Width:", 13.0, Weight::Normal, colors.text_secondary());
            crate::settings_kit::number_value(
                ui,
                colors,
                "snap-w",
                &mut ctx.state.snap_width,
                1.0,
                100000.0,
                1.0,
                0,
            );
            crate::widgets::text(ui, "Height:", 13.0, Weight::Normal, colors.text_secondary());
            crate::settings_kit::number_value(
                ui,
                colors,
                "snap-h",
                &mut ctx.state.snap_height,
                1.0,
                100000.0,
                1.0,
                0,
            );
            if compact_button(ui, colors, t(s::TaskSettingsWindow_Add)).clicked() {
                sizes.push((ctx.state.snap_width as i64, ctx.state.snap_height as i64));
                ctx.state.selected_snap = Some(sizes.len() - 1);
                write = true;
            }
            if compact_button(ui, colors, t(s::TaskSettingsWindow_Remove)).clicked()
                && let Some(index) = ctx.state.selected_snap.filter(|i| *i < sizes.len())
            {
                sizes.remove(index);
                ctx.state.selected_snap = None;
                write = true;
            }
        });
        if write {
            let value: Vec<Value> = sizes
                .iter()
                .map(|(w, h)| json!({ "Width": w, "Height": h }))
                .collect();
            b.set(&path, Value::Array(value));
        }
    });
}

fn recorder(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultCaptureSettings");
    let c = |m: &'static str| ["CaptureSettings", m];
    let dev = ctx.dev_mode;
    card(ui, colors, t(s::TaskSettingsWindow_Recording), on, |ui| {
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_ScreenRecordingFPS),
            |ui| {
                number(
                    ui,
                    colors,
                    b,
                    &c("ScreenRecordFPS"),
                    30.0,
                    1.0,
                    if dev { 300.0 } else { 60.0 },
                    1.0,
                    0,
                )
            },
        );
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_GIFFPS),
            |ui| {
                number(
                    ui,
                    colors,
                    b,
                    &c("GIFFPS"),
                    15.0,
                    1.0,
                    if dev { 60.0 } else { 30.0 },
                    1.0,
                    0,
                )
            },
        );
        check(
            ui,
            colors,
            b,
            &c("ScreenRecordShowCursor"),
            true,
            t(s::TaskSettingsWindow_ShowCursorInRecording),
        );
        check(
            ui,
            colors,
            b,
            &c("ScreenRecordShowTimer"),
            true,
            t(s::TaskSettingsWindow_ShowRecordingTimer),
        );
        check(
            ui,
            colors,
            b,
            &c("ScreenRecordShowButtonLabels"),
            true,
            t(s::TaskSettingsWindow_ShowRecordingButtonLabels),
        );
        let auto = check(
            ui,
            colors,
            b,
            &c("ScreenRecordAutoStart"),
            true,
            t(s::TaskSettingsWindow_StartRecordingAfterADelay),
        );
        ui.add_enabled_ui(auto, |ui| {
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_StartDelaySeconds),
                |ui| {
                    number(
                        ui,
                        colors,
                        b,
                        &c("ScreenRecordStartDelay"),
                        0.0,
                        0.0,
                        3600.0,
                        0.1,
                        1,
                    )
                },
            );
        });
        let fixed = check(
            ui,
            colors,
            b,
            &c("ScreenRecordFixedDuration"),
            false,
            t(s::TaskSettingsWindow_UseFixedDuration),
        );
        ui.add_enabled_ui(fixed, |ui| {
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_DurationSeconds),
                |ui| {
                    number(
                        ui,
                        colors,
                        b,
                        &c("ScreenRecordDuration"),
                        3.0,
                        0.0,
                        86400.0,
                        0.1,
                        1,
                    )
                },
            );
        });
    });
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_EncodingAndCapture),
        on,
        |ui| {
            check(
                ui,
                colors,
                b,
                &c("ScreenRecordTwoPassEncoding"),
                false,
                t(s::TaskSettingsWindow_RecordLosslesslyFirstThenApplyEncodingOptions),
            );
            check(
                ui,
                colors,
                b,
                &c("ScreenRecordAskConfirmationOnAbort"),
                false,
                t(s::TaskSettingsWindow_AskForConfirmationWhenAborting),
            );
            check(
                ui,
                colors,
                b,
                &c("ScreenRecordTransparentRegion"),
                false,
                t(s::TaskSettingsWindow_UseTransparentRegionSelection),
            );
            if compact_button(
                ui,
                colors,
                t(s::TaskSettingsWindow_ScreenRecordingOptionsWithEllipsis),
            )
            .clicked()
            {
                ctx.asks.push(TaskPageAsk::ScreenRecordingOptions);
            }
        },
    );
}

fn ocr(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultCaptureSettings");
    let o = |m: &'static str| ["CaptureSettings", "OCROptions", m];
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_OpticalCharacterRecognition),
        on,
        |ui| {
            // The languages come from the OCR engine (Vision, phase 5); until it is there
            // the list is upstream's "unavailable" state.
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_DefaultLanguage),
                |ui| {
                    ui.add_enabled_ui(false, |ui| {
                        let list = vec![(
                            0usize,
                            t(s::TaskSettingsWindow_OCRLanguagesAreUnavailable).to_string(),
                        )];
                        let _ = crate::widgets::combo(
                            ui,
                            colors,
                            "ocr-language",
                            250.0,
                            &0usize,
                            &list,
                        );
                    });
                },
            );
            let silent = check(
                ui,
                colors,
                b,
                &o("Silent"),
                false,
                t(s::TaskSettingsWindow_ProcessOCRSilently),
            );
            ui.add_enabled_ui(!silent, |ui| {
                check(
                    ui,
                    colors,
                    b,
                    &o("AutoCopy"),
                    false,
                    t(s::TaskSettingsWindow_AutomaticallyCopyResultsToClipboard),
                );
            });
            check(
                ui,
                colors,
                b,
                &o("CloseWindowAfterOpeningServiceLink"),
                false,
                t(s::TaskSettingsWindow_CloseOCRWindowAfterOpeningServiceLink),
            );
            // `OCRHelp` opened ShareX's documentation on getsharex.com; not linked here.
        },
    );
}

fn naming(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultUploadSettings");
    let u = |m: &'static str| ["UploadSettings", m];
    // `UpdatePreviews`: a 1920x1080 picture, Modlight's own title for the window sample.
    let preview = |pattern: &str, window: bool, b: &Binder, auto: i64| -> String {
        use modlight_core::names::name_parser::{NameParser, NameParserType};
        let mut parser = NameParser::new(NameParserType::FileName);
        parser.auto_increment_number = auto as i32;
        parser.image_width = 1920;
        parser.image_height = 1080;
        parser.max_name_length = b.f64(&["AdvancedSettings", "NamePatternMaxLength"], 100.0) as i32;
        parser.max_title_length =
            b.f64(&["AdvancedSettings", "NamePatternMaxTitleLength"], 50.0) as i32;
        parser.is_preview_mode = true;
        if b.bool(&["UploadSettings", "UseCustomTimeZone"], false) {
            parser.custom_time_zone = Some(modlight_core::names::name_parser::CustomTimeZone::Utc);
        }
        if window {
            parser.window_text = Some(modlight_core::title());
            parser.process_name = Some("Modlight".into());
        }
        parser.parse(pattern).unwrap_or_default()
    };
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_NamePatterns),
        on,
        |ui| {
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_CaptureOrClipboardUpload),
                |ui| text(ui, colors, b, &u("NameFormatPattern"), "%ra{10}"),
            );
            let pattern = b.str(&u("NameFormatPattern"), "%ra{10}");
            let shown = preview(&pattern, false, b, *ctx.auto_increment);
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_Preview),
                |ui| hint(ui, colors, &shown),
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_WindowCapture),
                |ui| {
                    text(
                        ui,
                        colors,
                        b,
                        &u("NameFormatPatternActiveWindow"),
                        "%pn_%ra{10}",
                    )
                },
            );
            let pattern = b.str(&u("NameFormatPatternActiveWindow"), "%pn_%ra{10}");
            let shown = preview(&pattern, true, b, *ctx.auto_increment);
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_Preview),
                |ui| hint(ui, colors, &shown),
            );
            check(
                ui,
                colors,
                b,
                &u("FileUploadUseNamePattern"),
                false,
                t(s::TaskSettingsWindow_UseNamePatternForFileUploads),
            );
            check(
                ui,
                colors,
                b,
                &u("FileUploadReplaceProblematicCharacters"),
                false,
                t(s::TaskSettingsWindow_ReplaceURLProblematicCharactersWithUnderscores),
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_AutoIncrementNumber),
                |ui| {
                    let mut value = *ctx.auto_increment as f64;
                    if crate::settings_kit::number_value(
                        ui,
                        colors,
                        "auto-increment",
                        &mut value,
                        0.0,
                        i32::MAX as f64,
                        1.0,
                        0,
                    ) {
                        *ctx.auto_increment = value as i64;
                        ctx.auto_increment_changed = true;
                    }
                },
            );
        },
    );
    card(ui, colors, t(s::TaskSettingsWindow_TimeZone), on, |ui| {
        let custom = check(
            ui,
            colors,
            b,
            &u("UseCustomTimeZone"),
            false,
            t(s::TaskSettingsWindow_UseCustomTimeZone),
        );
        ui.add_enabled_ui(custom, |ui| {
            // The system's time zones; the setting stores upstream's TimeZoneInfo shape,
            // and only UTC is honoured by the name patterns (MACOS.md).
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_TimeZoneLabel),
                |ui| {
                    let list = vec![(
                        "UTC".to_string(),
                        "(UTC) Coordinated Universal Time".to_string(),
                    )];
                    let _ = crate::widgets::combo(
                        ui,
                        colors,
                        "time-zone",
                        300.0,
                        &"UTC".to_string(),
                        &list,
                    );
                },
            );
        });
    });
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_URLReplacement),
        on,
        |ui| {
            let regex = check(
                ui,
                colors,
                b,
                &u("URLRegexReplace"),
                false,
                t(s::TaskSettingsWindow_ReplaceResultURLUsingARegularExpression),
            );
            ui.add_enabled_ui(regex, |ui| {
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_Pattern),
                    |ui| {
                        text(
                            ui,
                            colors,
                            b,
                            &u("URLRegexReplacePattern"),
                            "^https?://(.+)$",
                        )
                    },
                );
                row(
                    ui,
                    colors,
                    TASK_ROW,
                    t(s::TaskSettingsWindow_Replacement),
                    |ui| {
                        text(
                            ui,
                            colors,
                            b,
                            &u("URLRegexReplaceReplacement"),
                            "https://$1",
                        )
                    },
                );
            });
        },
    );
}

fn clipboard(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultUploadSettings");
    let u = |m: &'static str| ["UploadSettings", m];
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_ClipboardContent),
        on,
        |ui| {
            check(
                ui,
                colors,
                b,
                &u("ClipboardUploadURLContents"),
                false,
                t(s::TaskSettingsWindow_IfClipboardContainsAFileURLDownloadAndUploadIt),
            );
            check(
                ui,
                colors,
                b,
                &u("ClipboardUploadShortenURL"),
                false,
                t(s::TaskSettingsWindow_IfClipboardContainsAURLUseURLShortener),
            );
            check(
                ui,
                colors,
                b,
                &u("ClipboardUploadShareURL"),
                false,
                t(s::TaskSettingsWindow_IfClipboardContainsAURLShareItUsingURLSharingService),
            );
            check(
                ui,
                colors,
                b,
                &u("ClipboardUploadAutoIndexFolder"),
                false,
                t(s::TaskSettingsWindow_IfClipboardContainsAFolderPathIndexAndUploadIt),
            );
        },
    );
}

fn tools(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    override_group(
        ui,
        ctx,
        b,
        "UseDefaultToolsSettings",
        t(s::TaskSettingsWindow_OverrideToolsSettings),
    );
    let on = group_enabled(b, ctx, "UseDefaultToolsSettings");
    card(ui, colors, t(s::TaskSettingsWindow_ImageEditor), on, |ui| {
        check(
            ui,
            colors,
            b,
            &["ToolsSettings", "UseLegacyImageEditor"],
            false,
            t(s::TaskSettingsWindow_UseLegacyImageEditor),
        );
    });
    let p = |m: &'static str| ["ToolsSettings", "ScreenColorPickerOptions", m];
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_ScreenColorPicker),
        on,
        |ui| {
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_Format),
                |ui| text(ui, colors, b, &p("Format"), "$hex"),
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_FormatCtrlPlusClick),
                |ui| text(ui, colors, b, &p("FormatCtrl"), "$r255, $g255, $b255"),
            );
            row(
                ui,
                colors,
                TASK_ROW,
                t(s::TaskSettingsWindow_InfoText),
                |ui| {
                    text(
                        ui,
                        colors,
                        b,
                        &p("InfoText"),
                        "RGB: $r255, $g255, $b255$nHex: $hex$nX: $x Y: $y",
                    )
                },
            );
            check(
                ui,
                colors,
                b,
                &p("ShowMagnifier"),
                true,
                t(s::TaskSettingsWindow_ShowMagnifier),
            );
        },
    );
}

fn actions(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    override_group(
        ui,
        ctx,
        b,
        "UseDefaultActions",
        t(s::TaskSettingsWindow_OverrideActions),
    );
    let on = group_enabled(b, ctx, "UseDefaultActions");
    let programs: Vec<Value> = match b.get(&["ExternalPrograms"]) {
        Some(Value::Array(list)) => list.clone(),
        _ => Vec::new(),
    };
    card(ui, colors, t(s::TaskSettingsWindow_Actions), on, |ui| {
        let selected = ctx.state.selected_action;
        let mut toggled = None;
        let mut clicked = None;
        list_frame(ui, colors, 230.0, None, |ui| {
            for (index, program) in programs.iter().enumerate() {
                let active = program
                    .get("IsActive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let name = program
                    .get("Name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let path = program
                    .get("Path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let response = list_row(ui, colors, selected == Some(index), 32.0, |ui, r| {
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(r.shrink2(egui::vec2(6.0, 0.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let mut checked = active;
                    if crate::widgets::checkbox(&mut child, colors, &mut checked, None, true)
                        .on_hover_text(t(s::TaskSettingsWindow_EnableAction))
                        .changed()
                    {
                        toggled = Some((index, checked));
                    }
                    crate::widgets::text(&mut child, &name, 13.0, Weight::Normal, colors.text());
                    child.add_space(6.0);
                    crate::widgets::text_trimmed(
                        &mut child,
                        &format!("\u{2014} {path}"),
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                });
                if response.clicked() {
                    clicked = Some(index);
                }
                if response.double_clicked() {
                    ctx.asks.push(TaskPageAsk::EditAction(Some(index)));
                }
            }
        });
        if let Some(index) = clicked {
            ctx.state.selected_action = Some(index);
        }
        if let Some((index, value)) = toggled {
            let mut list = programs.clone();
            list[index]["IsActive"] = json!(value);
            b.set(&["ExternalPrograms"], Value::Array(list));
        }
        ui.horizontal(|ui| {
            // `ButtonRow`: 6 apart.
            ui.spacing_mut().item_spacing.x = 6.0;
            if compact_button(ui, colors, t(s::TaskSettingsWindow_AddWithEllipsis)).clicked() {
                ctx.asks.push(TaskPageAsk::EditAction(None));
            }
            let has = ctx
                .state
                .selected_action
                .is_some_and(|i| i < programs.len());
            ui.add_enabled_ui(has, |ui| {
                if compact_button(ui, colors, t(s::TaskSettingsWindow_EditWithEllipsis)).clicked() {
                    ctx.asks
                        .push(TaskPageAsk::EditAction(ctx.state.selected_action));
                }
                if compact_button(ui, colors, t(s::TaskSettingsWindow_Duplicate)).clicked()
                    && let Some(index) = ctx.state.selected_action
                {
                    let mut list = programs.clone();
                    list.push(programs[index].clone());
                    ctx.state.selected_action = Some(list.len() - 1);
                    b.set(&["ExternalPrograms"], Value::Array(list));
                }
                if compact_button(ui, colors, t(s::TaskSettingsWindow_Remove)).clicked()
                    && let Some(index) = ctx.state.selected_action
                {
                    let mut list = programs.clone();
                    list.remove(index);
                    ctx.state.selected_action = None;
                    b.set(&["ExternalPrograms"], Value::Array(list));
                }
            });
        });
        hint(ui, colors, t(s::TaskSettingsWindow_YouCanEnableOrDisableActionsFromAfterCaptureTasksPerformActions));
    });
}

fn watch_folders(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let folders: Vec<Value> = match b.get(&["WatchFolderList"]) {
        Some(Value::Array(list)) => list.clone(),
        _ => Vec::new(),
    };
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_WatchFolders),
        true,
        |ui| {
            let before = b.bool(&["WatchFolderEnabled"], false);
            let after = check(
                ui,
                colors,
                b,
                &["WatchFolderEnabled"],
                false,
                t(s::TaskSettingsWindow_WatchFoldersAndUploadNewlyCreatedFiles),
            );
            if before != after {
                ctx.asks.push(TaskPageAsk::WatchFoldersChanged);
            }
            let selected = ctx.state.selected_watch_folder;
            let mut clicked = None;
            let empty = folders.is_empty().then(|| {
                (
                    t(s::TaskSettingsWindow_NoWatchFolders),
                    t(s::TaskSettingsWindow_NoWatchFoldersExample),
                )
            });
            list_frame(ui, colors, 230.0, empty, |ui| {
                for (index, folder) in folders.iter().enumerate() {
                    let title = format(
                        t(s::TaskSettingsWindow_WatchFolderSummary),
                        &[
                            Arg::from(
                                folder
                                    .get("FolderPath")
                                    .and_then(Value::as_str)
                                    .unwrap_or(""),
                            ),
                            Arg::from(folder.get("Filter").and_then(Value::as_str).unwrap_or("")),
                            Arg::from(
                                if folder
                                    .get("IncludeSubdirectories")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false)
                                {
                                    t(s::TaskSettingsWindow_Yes)
                                } else {
                                    t(s::TaskSettingsWindow_No)
                                },
                            ),
                        ],
                    );
                    let response = list_row(ui, colors, selected == Some(index), 30.0, |ui, r| {
                        crate::controls::label_at(
                            ui,
                            r.shrink2(egui::vec2(8.0, 0.0)),
                            &title,
                            Weight::Normal,
                            colors.text(),
                        );
                    });
                    if response.clicked() {
                        clicked = Some(index);
                    }
                    if response.double_clicked() {
                        ctx.asks.push(TaskPageAsk::EditWatchFolder(Some(index)));
                    }
                }
            });
            if clicked.is_some() {
                ctx.state.selected_watch_folder = clicked;
            }
            ui.horizontal(|ui| {
                // `ButtonRow`: 6 apart.
                ui.spacing_mut().item_spacing.x = 6.0;
                if compact_button(ui, colors, t(s::TaskSettingsWindow_AddWithEllipsis)).clicked() {
                    ctx.asks.push(TaskPageAsk::EditWatchFolder(None));
                }
                let has = ctx
                    .state
                    .selected_watch_folder
                    .is_some_and(|i| i < folders.len());
                ui.add_enabled_ui(has, |ui| {
                    if compact_button(ui, colors, t(s::TaskSettingsWindow_EditWithEllipsis))
                        .clicked()
                    {
                        ctx.asks.push(TaskPageAsk::EditWatchFolder(
                            ctx.state.selected_watch_folder,
                        ));
                    }
                    if compact_button(ui, colors, t(s::TaskSettingsWindow_Remove)).clicked()
                        && let Some(index) = ctx.state.selected_watch_folder
                    {
                        let mut list = folders.clone();
                        list.remove(index);
                        ctx.state.selected_watch_folder = None;
                        b.set(&["WatchFolderList"], Value::Array(list));
                        ctx.asks.push(TaskPageAsk::WatchFoldersChanged);
                    }
                });
            });
        },
    );
}

/// `BuildAdvancedPage`: `TaskSettingsAdvanced`'s properties by category.
fn advanced(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    override_group(
        ui,
        ctx,
        b,
        "UseDefaultAdvancedSettings",
        t(s::TaskSettingsWindow_OverrideAdvancedSettings),
    );
    let on = group_enabled(b, ctx, "UseDefaultAdvancedSettings");
    let a = |m: &'static str| ["AdvancedSettings", m];
    enum Kind {
        Bool(bool),
        Int(f64),
        Text(&'static str),
        List(&'static [&'static str]),
    }
    type Category = (
        modlight_core::l10n::Key,
        Vec<(&'static str, modlight_core::l10n::Key, Kind)>,
    );
    let image_ext = modlight_core::names::file_helpers::IMAGE_FILE_EXTENSIONS;
    let text_ext = modlight_core::names::file_helpers::TEXT_FILE_EXTENSIONS;
    let categories: [Category; 6] = [
        (
            s::TaskSettingsWindow_General,
            vec![
                (
                    "ProcessImagesDuringFileUpload",
                    s::TaskSettingsWindow_ProcessImagesDuringFileUpload,
                    Kind::Bool(false),
                ),
                (
                    "ProcessImagesDuringClipboardUpload",
                    s::TaskSettingsWindow_ProcessImagesDuringClipboardUpload,
                    Kind::Bool(false),
                ),
                (
                    "UseAfterCaptureTasksDuringFileUpload",
                    s::TaskSettingsWindow_UseAfterCaptureTasksDuringFileUpload,
                    Kind::Bool(true),
                ),
                (
                    "TextTaskSaveAsFile",
                    s::TaskSettingsWindow_TextTaskSaveAsFile,
                    Kind::Bool(true),
                ),
                (
                    "AutoClearClipboard",
                    s::TaskSettingsWindow_AutoClearClipboard,
                    Kind::Bool(false),
                ),
            ],
        ),
        (
            s::TaskSettingsWindow_Capture,
            vec![(
                "RegionCaptureDisableAnnotation",
                s::TaskSettingsWindow_RegionCaptureDisableAnnotation,
                Kind::Bool(false),
            )],
        ),
        (
            s::TaskSettingsWindow_Upload,
            vec![
                (
                    "ImageExtensions",
                    s::TaskSettingsWindow_ImageExtensions,
                    Kind::List(image_ext),
                ),
                (
                    "TextExtensions",
                    s::TaskSettingsWindow_TextExtensions,
                    Kind::List(text_ext),
                ),
                (
                    "EarlyCopyURL",
                    s::TaskSettingsWindow_EarlyCopyURL,
                    Kind::Bool(false),
                ),
            ],
        ),
        (
            s::TaskSettingsWindow_UploadText,
            vec![
                (
                    "TextFileExtension",
                    s::TaskSettingsWindow_TextFileExtension,
                    Kind::Text("txt"),
                ),
                (
                    "TextFormat",
                    s::TaskSettingsWindow_TextFormat,
                    Kind::Text("text"),
                ),
                (
                    "TextCustom",
                    s::TaskSettingsWindow_TextCustom,
                    Kind::Text(""),
                ),
                (
                    "TextCustomEncodeInput",
                    s::TaskSettingsWindow_TextCustomEncodeInput,
                    Kind::Bool(true),
                ),
            ],
        ),
        (
            s::TaskSettingsWindow_AfterUpload,
            vec![
                (
                    "ResultForceHTTPS",
                    s::TaskSettingsWindow_ResultForceHTTPS,
                    Kind::Bool(false),
                ),
                (
                    "ClipboardContentFormat",
                    s::TaskSettingsWindow_ClipboardContentFormat,
                    Kind::Text("$result"),
                ),
                (
                    "BalloonTipContentFormat",
                    s::TaskSettingsWindow_BalloonTipContentFormat,
                    Kind::Text("$result"),
                ),
                (
                    "OpenURLFormat",
                    s::TaskSettingsWindow_OpenURLFormat,
                    Kind::Text("$result"),
                ),
                (
                    "AutoShortenURLLength",
                    s::TaskSettingsWindow_AutoShortenURLLength,
                    Kind::Int(0.0),
                ),
                (
                    "AutoCloseAfterUploadForm",
                    s::TaskSettingsWindow_AutoCloseAfterUploadForm,
                    Kind::Bool(false),
                ),
            ],
        ),
        (
            s::TaskSettingsWindow_NamePattern,
            vec![
                (
                    "NamePatternMaxLength",
                    s::TaskSettingsWindow_NamePatternMaxLength,
                    Kind::Int(100.0),
                ),
                (
                    "NamePatternMaxTitleLength",
                    s::TaskSettingsWindow_NamePatternMaxTitleLength,
                    Kind::Int(50.0),
                ),
            ],
        ),
    ];
    for (title, settings) in categories {
        card(ui, colors, t(title), on, |ui| {
            for (member, label, kind) in settings {
                match kind {
                    Kind::Bool(default) => {
                        check(ui, colors, b, &a(member), default, t(label));
                    }
                    Kind::Int(default) => {
                        row(ui, colors, TASK_ROW, &format!("{}:", t(label)), |ui| {
                            number(
                                ui,
                                colors,
                                b,
                                &a(member),
                                default,
                                i32::MIN as f64,
                                i32::MAX as f64,
                                1.0,
                                0,
                            )
                        });
                    }
                    Kind::Text(default) => {
                        row(ui, colors, TASK_ROW, &format!("{}:", t(label)), |ui| {
                            text(ui, colors, b, &a(member), default)
                        });
                    }
                    Kind::List(defaults) => {
                        let current = match b.get(&a(member)) {
                            Some(Value::Array(list)) => list
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", "),
                            _ => defaults.join(", "),
                        };
                        let mut value = current.clone();
                        row(ui, colors, TASK_ROW, &format!("{}:", t(label)), |ui| {
                            let id = ui.make_persistent_id(("advanced-list", member));
                            let response = crate::widgets::Field {
                                height: 32.0,
                                id: Some(id),
                                enabled: ui.is_enabled(),
                                ..crate::widgets::Field::new(&mut value)
                            }
                            .show(ui, colors);
                            if response.changed() {
                                let list: Vec<Value> = value
                                    .split(',')
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .map(|s| json!(s))
                                    .collect();
                                b.set(&a(member), Value::Array(list));
                            }
                        });
                    }
                }
            }
        });
    }
}

/// Which task settings a page, the region picker or the FFmpeg window edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTarget {
    /// `DefaultTaskSettings`.
    Defaults,
    /// The task settings of the hotkey at this position.
    Hotkey(usize),
}

impl crate::state::AppState {
    /// The settings object `target` names, as stored (a hotkey's keeps `null` groups).
    pub fn task_object(&self, target: TaskTarget) -> serde_json::Map<String, Value> {
        let value = match target {
            TaskTarget::Defaults => self.config.get("DefaultTaskSettings").cloned(),
            TaskTarget::Hotkey(index) => self
                .captures
                .hotkeys
                .hotkeys
                .get(index)
                .map(|h| h.task.clone()),
        };
        match value {
            Some(Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    }

    /// The settings a task run with `target` would use.
    pub fn task_settings_of(&self, target: TaskTarget) -> modlight_core::tasks::TaskSettings {
        match target {
            TaskTarget::Defaults => self.default_task_settings(),
            TaskTarget::Hotkey(index) => match self.captures.hotkeys.hotkeys.get(index) {
                Some(hotkey) => {
                    let defaults = self
                        .config
                        .get("DefaultTaskSettings")
                        .cloned()
                        .unwrap_or(Value::Null);
                    hotkey.task_settings(&defaults)
                }
                None => self.default_task_settings(),
            },
        }
    }

    /// Write one member of `target`'s settings and save.
    pub fn set_task_member(&mut self, target: TaskTarget, path: &[&str], value: Value) {
        let mut object = self.task_object(target);
        let mut binder = Binder::new(&mut object);
        binder.set(path, value);
        if !binder.changed {
            return;
        }
        match target {
            TaskTarget::Defaults => {
                self.config
                    .set("DefaultTaskSettings", Value::Object(object));
                self.save_config();
            }
            TaskTarget::Hotkey(index) => {
                if let Some(hotkey) = self.captures.hotkeys.hotkeys.get_mut(index) {
                    hotkey.task = Value::Object(object);
                    self.save_hotkeys();
                }
            }
        }
    }
}

// ------------------------------------------------------------------ a hotkey's own pages

/// A button that opens a menu under it (`TaskMenu`, `TaskFlagsMenu`, `DestinationMenu`):
/// an accent icon, the text, a chevron.
fn menu_button(
    ui: &mut egui::Ui,
    colors: &Colors,
    icon: char,
    text: &str,
    min_width: f32,
    stretch: bool,
) -> egui::Response {
    let width = if stretch {
        ui.available_width().max(min_width)
    } else {
        min_width.max(250.0)
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::hover());
    let look = crate::widgets::Look::button(colors)
        .height(32.0)
        .padding(8.0);
    let enabled = ui.is_enabled();
    let response = crate::widgets::button_at(
        ui,
        colors,
        &look,
        crate::widgets::Content {
            icon: None,
            label: None,
        },
        enabled,
        rect,
    );
    let color = if enabled {
        colors.text()
    } else {
        colors.text().gamma_multiply(0.5)
    };
    crate::widgets::paint_icon(
        ui,
        ui.painter(),
        egui::pos2(rect.left() + 8.0 + 8.0, rect.center().y),
        icon,
        16.0,
        colors.accent_start(),
    );
    let galley = crate::widgets::layout(
        ui,
        crate::widgets::trimmed(
            text,
            13.0,
            Weight::Normal,
            color,
            rect.width() - 8.0 - 16.0 - 7.0 - 7.0 - 14.0 - 8.0,
            1,
        ),
    );
    ui.painter().galley(
        egui::pos2(
            rect.left() + 8.0 + 16.0 + 7.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
    crate::widgets::paint_icon(
        ui,
        ui.painter(),
        egui::pos2(rect.right() - 8.0 - 7.0, rect.center().y),
        modlight_core::icons::CHEVRON_DOWN,
        14.0,
        colors.text_secondary(),
    );
    response
}

/// Open `menu` under `response` while it is the open one; the position picked.
fn menu_under(
    ctx: &mut TaskPagesCtx,
    response: &egui::Response,
    name: &'static str,
    entries: impl FnOnce() -> Vec<crate::menus::Entry>,
) -> Option<usize> {
    if response.clicked() {
        ctx.state.menu = if ctx.state.menu == Some(name) {
            None
        } else {
            Some(name)
        };
    }
    if ctx.state.menu != Some(name) {
        return None;
    }
    let mut open = true;
    let picked = match crate::menus::popup_open_left(ctx.colors, response, &mut open, entries) {
        Some(crate::commands::Command::Local(index)) => Some(index),
        _ => None,
    };
    if !open {
        ctx.state.menu = None;
    }
    picked
}

/// `BuildTaskPage`: the task, its after capture and after upload tasks, destinations and
/// screenshots folder.
fn task(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    use modlight_core::hotkeys::HotkeyType;
    use modlight_core::tasks::settings::{
        AfterCaptureTasks, AfterUploadTasks, FlagNames, TaskSettings,
    };
    let colors = ctx.colors;
    let defaults = TaskSettings::default();
    card(ui, colors, t(s::TaskSettingsWindow_Task), true, |ui| {
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_TaskLabel),
            |ui| {
                let job = b
                    .get(&["Job"])
                    .and_then(Value::as_str)
                    .and_then(HotkeyType::parse)
                    .unwrap_or(HotkeyType::None);
                let response = menu_button(ui, colors, job.icon(), &job.text(), 250.0, false);
                let jobs: Vec<HotkeyType> = HotkeyType::ALL.to_vec();
                if let Some(index) = menu_under(ctx, &response, "job", || job_menu(job)) {
                    let ordered = job_order();
                    if let Some(picked) = ordered.get(index).or_else(|| jobs.get(index)) {
                        b.set(&["Job"], json!(picked.name()));
                    }
                }
            },
        );
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_Description),
            |ui| text(ui, colors, b, &["Description"], ""),
        );
    });
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_AfterCaptureTasks),
        true,
        |ui| {
            let on = check_inverted_value(
                ui,
                colors,
                b,
                "UseDefaultAfterCaptureJob",
                t(s::TaskSettingsWindow_OverrideAfterCaptureTasks),
            );
            ui.add_enabled_ui(on, |ui| {
                let current: AfterCaptureTasks = b
                    .get(&["AfterCaptureJob"])
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(defaults.after_capture_job);
                let entries = crate::uploads::after_capture_menu(current);
                let shown = flags_text(&entries, || {
                    modlight_core::l10n::enum_text("AfterCaptureTasks", "None", None)
                });
                let response = menu_button(
                    ui,
                    colors,
                    modlight_core::icons::IMAGE_UP,
                    &shown,
                    300.0,
                    true,
                );
                if let Some(index) = menu_under(ctx, &response, "after-capture", || {
                    local_entries(entries.clone())
                }) && let Some(bit) = entry_bit(&entries, index)
                {
                    let flipped = AfterCaptureTasks(current.0 ^ bit);
                    b.set(&["AfterCaptureJob"], json!(flipped.names()));
                }
            });
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_AfterUploadTasks),
        true,
        |ui| {
            let on = check_inverted_value(
                ui,
                colors,
                b,
                "UseDefaultAfterUploadJob",
                t(s::TaskSettingsWindow_OverrideAfterUploadTasks),
            );
            ui.add_enabled_ui(on, |ui| {
                let current: AfterUploadTasks = b
                    .get(&["AfterUploadJob"])
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(defaults.after_upload_job);
                let entries = crate::uploads::after_upload_menu(current);
                let shown = flags_text(&entries, || {
                    modlight_core::l10n::enum_text("AfterUploadTasks", "None", None)
                });
                let response = menu_button(
                    ui,
                    colors,
                    modlight_core::icons::CLOUD_UPLOAD,
                    &shown,
                    300.0,
                    true,
                );
                if let Some(index) = menu_under(ctx, &response, "after-upload", || {
                    local_entries(entries.clone())
                }) && let Some(bit) = entry_bit(&entries, index)
                {
                    let flipped = AfterUploadTasks(current.0 ^ bit);
                    b.set(&["AfterUploadJob"], json!(flipped.names()));
                }
            });
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_Destinations),
        true,
        |ui| {
            let on = check_inverted_value(
                ui,
                colors,
                b,
                "UseDefaultDestinations",
                t(s::TaskSettingsWindow_OverrideDestinations),
            );
            ui.add_enabled_ui(on, |ui| {
                let label = format!("{}...", t(s::TaskSettingsWindow_Destinations));
                let response = menu_button(
                    ui,
                    colors,
                    modlight_core::icons::SERVER,
                    &label,
                    300.0,
                    true,
                );
                let (entries, picks) = destination_menu(b);
                if let Some(index) = menu_under(ctx, &response, "destinations", || entries)
                    && let Some(writes) = picks.get(index)
                {
                    for (member, value) in writes {
                        b.set(&[member], json!(value));
                    }
                }
            });
        },
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_UploaderAccounts),
        true,
        |_ui| {},
    );
    card(
        ui,
        colors,
        t(s::TaskSettingsWindow_ScreenshotsFolder),
        true,
        |ui| {
            let on = check(
                ui,
                colors,
                b,
                &["OverrideScreenshotsFolder"],
                false,
                t(s::TaskSettingsWindow_OverrideScreenshotsFolder),
            );
            ui.add_enabled_ui(on, |ui| {
                ui.horizontal(|ui| {
                    let browse = t(s::TaskSettingsWindow_BrowseWithEllipsis);
                    let w = crate::widgets::button_width(
                        ui,
                        &crate::widgets::Look::button(colors)
                            .height(32.0)
                            .padding(10.0),
                        crate::widgets::Content::label(browse),
                    );
                    ui.allocate_ui(egui::vec2(ui.available_width() - w - 6.0, 32.0), |ui| {
                        text(ui, colors, b, &["ScreenshotsFolder"], "")
                    });
                    ui.add_space(6.0);
                    if compact_button(ui, colors, browse).clicked() {
                        ctx.asks.push(TaskPageAsk::PickScreenshotsFolder);
                    }
                });
            });
        },
    );
}

/// A check box over an inverted flag that defaults to true (`OverrideValue`).
fn check_inverted_value(
    ui: &mut egui::Ui,
    colors: &Colors,
    b: &mut Binder,
    member: &str,
    label: &str,
) -> bool {
    let mut value = !b.bool(&[member], true);
    let enabled = ui.is_enabled();
    if crate::widgets::checkbox_weighted(
        ui,
        colors,
        &mut value,
        Some(label),
        enabled,
        Weight::Normal,
    )
    .changed()
    {
        b.set(&[member], json!(!value));
    }
    value
}

/// The jobs in the order the task menu lists them.
fn job_order() -> Vec<modlight_core::hotkeys::HotkeyType> {
    use modlight_core::hotkeys::{HotkeyCategory, HotkeyType};
    let mut jobs = vec![HotkeyType::None];
    let mut categories: Vec<HotkeyCategory> = Vec::new();
    for job in HotkeyType::ALL.iter().copied() {
        let category = job.category();
        if job != HotkeyType::None
            && category != HotkeyCategory::None
            && !categories.contains(&category)
        {
            categories.push(category);
        }
    }
    for category in categories {
        jobs.extend(
            HotkeyType::ALL
                .iter()
                .copied()
                .filter(|j| *j != HotkeyType::None && j.category() == category),
        );
    }
    jobs
}

/// `TaskMenu`'s menu: None, then a submenu per category.
fn job_menu(current: modlight_core::hotkeys::HotkeyType) -> Vec<crate::menus::Entry> {
    use crate::menus::{Entry, Toggle};
    let jobs = job_order();
    let item = |index: usize| {
        let job = jobs[index];
        Entry::Item {
            label: job.text(),
            icon: Some(job.icon()),
            enabled: true,
            toggle: Toggle::Radio(job == current),
            command: Some(crate::commands::Command::Local(index)),
        }
    };
    let mut entries = vec![item(0)];
    let mut index = 1;
    while index < jobs.len() {
        let category = jobs[index].category();
        let mut children = Vec::new();
        while index < jobs.len() && jobs[index].category() == category {
            children.push(item(index));
            index += 1;
        }
        entries.push(Entry::Submenu {
            label: category.text().to_string(),
            icon: None,
            enabled: true,
            children,
        });
    }
    entries
}

/// The checked labels of a flags menu, or the "None" text (`UpdateSelectedTasks`).
fn flags_text(entries: &[crate::menus::Entry], none: impl FnOnce() -> String) -> String {
    let titles: Vec<&str> = entries
        .iter()
        .filter_map(|e| match e {
            crate::menus::Entry::Item {
                label,
                toggle: crate::menus::Toggle::Check(true),
                ..
            } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    if titles.is_empty() {
        none()
    } else {
        titles.join(", ")
    }
}

/// The flag bit of the entry at `index` (the menus carry `ToggleAfter...Task(bit)`).
fn entry_bit(entries: &[crate::menus::Entry], index: usize) -> Option<u32> {
    match entries.get(index) {
        Some(crate::menus::Entry::Item {
            command:
                Some(
                    crate::commands::Command::ToggleAfterCaptureTask(bit)
                    | crate::commands::Command::ToggleAfterUploadTask(bit),
                ),
            ..
        }) => Some(*bit),
        _ => None,
    }
}

/// The same entries, answering with their position instead of their command.
fn local_entries(entries: Vec<crate::menus::Entry>) -> Vec<crate::menus::Entry> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| match entry {
            crate::menus::Entry::Item {
                label,
                icon,
                enabled,
                toggle,
                ..
            } => crate::menus::Entry::Item {
                label,
                icon,
                enabled,
                toggle,
                command: Some(crate::commands::Command::Local(index)),
            },
            other => other,
        })
        .collect()
}

/// `MainMenuBuilder.BuildDestinationsMenu`: image, text and file uploaders, the URL
/// shortener and the sharing service, each a radio list; the writes each choice makes.
#[allow(clippy::type_complexity)]
fn destination_menu(
    b: &Binder,
) -> (
    Vec<crate::menus::Entry>,
    Vec<Vec<(&'static str, &'static str)>>,
) {
    use crate::menus::{Entry, Toggle};
    use modlight_core::l10n::uploaders as u;
    let get = |member: &str, default: &'static str| b.str(&[member], default);
    let image = get("ImageDestination", "Modlight");
    let image_file = get("ImageFileDestination", "Modlight");
    let text_dest = get("TextDestination", "Modlight");
    let text_file = get("TextFileDestination", "Modlight");
    let file = get("FileDestination", "Modlight");
    let shortener = get("URLShortenerDestination", "Modlight");
    let sharing = get("URLSharingServiceDestination", "GoogleImageSearch");
    let file_uploader = t(u::ImageDestination_FileUploader).to_string();
    let mut picks: Vec<Vec<(&'static str, &'static str)>> = Vec::new();
    let radio = |label: String,
                 checked: bool,
                 writes: Vec<(&'static str, &'static str)>,
                 picks: &mut Vec<_>| {
        let entry = Entry::Item {
            label,
            icon: None,
            enabled: true,
            toggle: Toggle::Radio(checked),
            command: Some(crate::commands::Command::Local(picks.len())),
        };
        picks.push(writes);
        entry
    };
    let image_name = if image == "FileUploader" {
        file_uploader.clone()
    } else {
        "Modlight".to_string()
    };
    let text_name = if text_dest == "FileUploader" {
        file_uploader.clone()
    } else {
        "Modlight".to_string()
    };
    let image_children = vec![
        radio(
            "Modlight".into(),
            image == "Modlight",
            vec![("ImageDestination", "Modlight")],
            &mut picks,
        ),
        Entry::Submenu {
            label: file_uploader.clone(),
            icon: None,
            enabled: true,
            children: vec![radio(
                "Modlight".into(),
                image == "FileUploader" && image_file == "Modlight",
                vec![
                    ("ImageDestination", "FileUploader"),
                    ("ImageFileDestination", "Modlight"),
                ],
                &mut picks,
            )],
        },
    ];
    let text_children = vec![
        radio(
            "Modlight".into(),
            text_dest == "Modlight",
            vec![("TextDestination", "Modlight")],
            &mut picks,
        ),
        Entry::Submenu {
            label: file_uploader.clone(),
            icon: None,
            enabled: true,
            children: vec![radio(
                "Modlight".into(),
                text_dest == "FileUploader" && text_file == "Modlight",
                vec![
                    ("TextDestination", "FileUploader"),
                    ("TextFileDestination", "Modlight"),
                ],
                &mut picks,
            )],
        },
    ];
    let file_children = vec![radio(
        "Modlight".into(),
        file == "Modlight",
        vec![("FileDestination", "Modlight")],
        &mut picks,
    )];
    let shortener_children = vec![radio(
        "Modlight".into(),
        shortener == "Modlight",
        vec![("URLShortenerDestination", "Modlight")],
        &mut picks,
    )];
    let sharing_children = vec![
        radio(
            t(u::URLSharingServices_GoogleImageSearch).to_string(),
            sharing == "GoogleImageSearch",
            vec![("URLSharingServiceDestination", "GoogleImageSearch")],
            &mut picks,
        ),
        radio(
            t(u::URLSharingServices_BingVisualSearch).to_string(),
            sharing == "BingVisualSearch",
            vec![("URLSharingServiceDestination", "BingVisualSearch")],
            &mut picks,
        ),
    ];
    let sharing_name = if sharing == "BingVisualSearch" {
        t(u::URLSharingServices_BingVisualSearch)
    } else {
        t(u::URLSharingServices_GoogleImageSearch)
    };
    let parent = |key: modlight_core::l10n::Key, name: &str, icon: char, children: Vec<Entry>| {
        Entry::Submenu {
            label: format(t(key), &[Arg::from(name)]),
            icon: Some(icon),
            enabled: true,
            children,
        }
    };
    let entries = vec![
        parent(
            s::TaskSettingsForm_UpdateUploaderMenuNames_Image_uploader___0_,
            &image_name,
            modlight_core::icons::IMAGE,
            image_children,
        ),
        parent(
            s::TaskSettingsForm_UpdateUploaderMenuNames_Text_uploader___0_,
            &text_name,
            modlight_core::icons::FILE_TEXT,
            text_children,
        ),
        parent(
            s::TaskSettingsForm_UpdateUploaderMenuNames_File_uploader___0_,
            "Modlight",
            modlight_core::icons::FILE_UP,
            file_children,
        ),
        parent(
            s::TaskSettingsForm_UpdateUploaderMenuNames_URL_shortener___0_,
            "Modlight",
            modlight_core::icons::LINK_2,
            shortener_children,
        ),
        parent(
            s::TaskSettingsForm_UpdateUploaderMenuNames_URL_sharing_service___0_,
            sharing_name,
            modlight_core::icons::SHARE_2,
            sharing_children,
        ),
    ];
    (entries, picks)
}

/// `BuildModlightPage`: the folder a hotkey's uploads go to.
fn modlight_folder(ui: &mut egui::Ui, ctx: &mut TaskPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let on = group_enabled(b, ctx, "UseDefaultUploadSettings");
    if !ctx.state.folders.requested {
        ctx.state.folders.requested = true;
        ctx.asks.push(TaskPageAsk::LoadModlightFolders);
    }
    card(ui, colors, "", on, |ui| {
        row(
            ui,
            colors,
            TASK_ROW,
            t(s::TaskSettingsWindow_ModlightFolder),
            |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let folders = &ctx.state.folders;
                    let root = folders
                        .root
                        .clone()
                        .unwrap_or_else(|| modlight_api::ROOT_FOLDER.to_string());
                    let selected = b.str(&["UploadSettings", "ModlightFolderId"], "");
                    let selected_name = b.str(&["UploadSettings", "ModlightFolderName"], "");
                    let is_folder = |id: &str| !id.is_empty() && id != root;
                    // `Fixed`: the window's folder, then no folder.
                    let mut choices: Vec<(String, String, String)> = vec![
                        (
                            String::new(),
                            t(s::TaskSettingsWindow_ModlightFolderAmbient).to_string(),
                            String::new(),
                        ),
                        (
                            root.clone(),
                            t(s::TaskSettingsWindow_ModlightFolderNone).to_string(),
                            String::new(),
                        ),
                    ];
                    match &folders.folders {
                        Some(list) => {
                            choices.extend(list.iter().cloned());
                            if is_folder(&selected)
                                && !list.iter().any(|(id, _, _)| *id == selected)
                            {
                                let label = if selected_name.is_empty() {
                                    t(s::TaskSettingsWindow_ModlightFolderUnknown).to_string()
                                } else {
                                    format(
                                        t(s::TaskSettingsWindow_ModlightFolderGone),
                                        &[Arg::from(selected_name.as_str())],
                                    )
                                };
                                choices.push((selected.clone(), label, String::new()));
                            }
                        }
                        None => {
                            if is_folder(&selected) {
                                let label = if selected_name.is_empty() {
                                    t(s::TaskSettingsWindow_ModlightFolderUnknown).to_string()
                                } else {
                                    selected_name.clone()
                                };
                                choices.push((selected.clone(), label, selected_name.clone()));
                            }
                        }
                    }
                    let current = if choices.iter().any(|(id, _, _)| *id == selected) {
                        selected.clone()
                    } else {
                        String::new()
                    };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let refresh = t(s::TaskSettingsWindow_ModlightFolderRefresh);
                        let refresh_w = crate::widgets::button_width(
                            ui,
                            &crate::widgets::Look::button(colors)
                                .height(32.0)
                                .padding(10.0),
                            crate::widgets::Content::label(refresh),
                        );
                        let list: Vec<(String, String)> = choices
                            .iter()
                            .map(|(id, label, _)| (id.clone(), label.clone()))
                            .collect();
                        let width = (ui.available_width() - refresh_w - 6.0).max(250.0);
                        if ui.is_enabled() {
                            if let Some(picked) = crate::widgets::combo(
                                ui,
                                colors,
                                "modlight-folder",
                                width,
                                &current,
                                &list,
                            ) && let Some((id, _, name)) =
                                choices.iter().find(|(id, _, _)| *id == picked)
                            {
                                b.set(&["UploadSettings", "ModlightFolderId"], json!(id));
                                b.set(&["UploadSettings", "ModlightFolderName"], json!(name));
                            }
                        } else {
                            let shown: Vec<(String, String)> = list
                                .iter()
                                .filter(|(id, _)| *id == current)
                                .cloned()
                                .collect();
                            let _ = crate::widgets::combo(
                                ui,
                                colors,
                                "modlight-folder",
                                width,
                                &current,
                                &shown,
                            );
                        }
                        let busy = ctx.state.folders.loading;
                        if crate::widgets::button(
                            ui,
                            colors,
                            &crate::widgets::Look::button(colors)
                                .height(32.0)
                                .padding(10.0),
                            crate::widgets::Content::label(refresh),
                            ui.is_enabled() && !busy,
                        )
                        .clicked()
                        {
                            ctx.asks.push(TaskPageAsk::LoadModlightFolders);
                        }
                    });
                    if !ctx.state.folders.status.is_empty() {
                        crate::widgets::text_wrapped(
                            ui,
                            &ctx.state.folders.status,
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                    }
                });
            },
        );
        hint(ui, colors, t(s::TaskSettingsWindow_ModlightFolderHint));
    });
}

impl crate::state::AppState {
    fn folder_pickers(&mut self) -> impl Iterator<Item = &mut ModlightFolders> {
        [
            self.windows.settings.as_mut(),
            self.windows.hotkey_task.as_mut(),
        ]
        .into_iter()
        .flatten()
        .map(|w| &mut w.task_pages.folders)
    }

    /// `ModlightFolderPicker.LoadAsync`: the account's folders, 20 s at most.
    pub fn load_modlight_folders(&mut self) {
        let Some(credential) = self.credential() else {
            for picker in self.folder_pickers() {
                picker.loading = false;
                picker.status = t(s::TaskSettingsWindow_ModlightFolderSignedOut).to_string();
            }
            return;
        };
        for picker in self.folder_pickers() {
            picker.loading = true;
            picker.status = t(s::TaskSettingsWindow_ModlightFolderLoading).to_string();
        }
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                api.get_folders(&credential),
            )
            .await;
            Box::new(
                move |state: &mut crate::state::AppState, _ctx: &egui::Context| {
                    let selected: Vec<(String, String)> = [
                        state.windows.settings.as_ref(),
                        state.windows.hotkey_task.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    .map(|w| {
                        let task = state.task_object(w.target);
                        let upload = task.get("UploadSettings");
                        (
                            upload
                                .and_then(|u| u.get("ModlightFolderId"))
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            upload
                                .and_then(|u| u.get("ModlightFolderName"))
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        )
                    })
                    .collect();
                    for (index, picker) in state.folder_pickers().enumerate() {
                        picker.loading = false;
                        match &answer {
                            Ok(Ok(list)) => {
                                let root = list
                                    .root_value
                                    .clone()
                                    .filter(|r| !r.is_empty())
                                    .unwrap_or_else(|| modlight_api::ROOT_FOLDER.to_string());
                                let choices = folder_choices(list);
                                let (id, _) = selected.get(index).cloned().unwrap_or_default();
                                let is_folder = !id.is_empty() && id != root;
                                let found = choices
                                    .iter()
                                    .any(|(c, _, name)| *c == id && !name.is_empty());
                                picker.status = if is_folder && !found {
                                    t(s::TaskSettingsWindow_ModlightFolderMissing).to_string()
                                } else {
                                    String::new()
                                };
                                picker.root = Some(root);
                                picker.folders = Some(choices);
                            }
                            Ok(Err(modlight_api::ApiError::Unauthorized)) => {
                                picker.status =
                                    t(s::TaskSettingsWindow_ModlightFolderRejected).to_string()
                            }
                            _ => {
                                picker.status =
                                    t(s::TaskSettingsWindow_ModlightFolderFailed).to_string()
                            }
                        }
                    }
                },
            ) as Box<dyn FnOnce(&mut crate::state::AppState, &egui::Context) + Send>
        });
    }
}

//! The application settings pages (`ApplicationSettingsView`): general (with the tray
//! actions and updates), integration, local folders, backup and cleanup, main window
//! tiles, copy formats (with their editor), upload limits, history, print, proxy and
//! advanced, over `ApplicationConfig` by JSON path.
//!
//! The theme page and the components page are drawn elsewhere (`settings.rs`,
//! `components_window.rs`).

use egui::{Align, Layout, Order, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde_json::{Value, json};

use crate::platform::integration::StartupState;
use modlight_core::hotkeys::HotkeyType;
use modlight_core::icons;
#[cfg(not(windows))]
use modlight_core::l10n::mac;
use modlight_core::l10n::{Arg, Key, LanguageSetting, format, helpers, s, t};

use crate::commands::Command;
use crate::menus::Entry;
use crate::settings_kit::{
    Binder, card, check, compact_button, hint, list_frame, list_row, number, row, text,
};
use crate::theme::{Colors, Weight};
use crate::widgets::{Field, Look, button, combo, layout, paint_icon, trimmed, wrapped};

/// "Don't show the print dialog": upstream's on Windows, the Mac's own text elsewhere.
#[cfg(windows)]
const DONT_SHOW_PRINT_DIALOG: Key = s::ApplicationSettingsWindow_DontShowWindowsPrintDialog;
#[cfg(not(windows))]
const DONT_SHOW_PRINT_DIALOG: Key = mac::DontShowPrintDialog;

/// What a page asks of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum AppPageAsk {
    Language(LanguageSetting),
    ShowTray(bool),
    EditQuickTaskMenu,
    OpenPersonalFolder,
    OpenScreenshotsFolder,
    BrowseScreenshotsFolder,
    Export,
    Import,
    ResetSettings,
    ImagePrintSettings,
    StartAtLogin(bool),
    OpenLoginItems,
    OpenServicesSettings,
    /// Windows: "Upload with Modlight" in Explorer's context menu.
    #[cfg(windows)]
    ContextMenu(bool),
    /// Windows: Modlight in Explorer's "Send to" menu.
    #[cfg(windows)]
    SendTo(bool),
    /// Windows: pick another personal folder (`OnBrowsePersonalFolderClick`).
    #[cfg(windows)]
    BrowsePersonalFolder,
    DisableHotkeys(bool),
    /// "Automatically check for updates" was turned on: arm the hourly check.
    UpdatesOn,
}

/// Between frames: the copy format selection and editor, the export choices, the
/// status line of a backup.
#[derive(Clone, Debug)]
pub struct AppPagesState {
    pub selected_format: Option<usize>,
    pub editor: Option<FormatEditor>,
    pub export_settings: bool,
    pub export_history: bool,
    pub status: String,
    pub busy: bool,
    /// Windows: the personal folder box (`PersonalFolderPath`), read from
    /// `PersonalPath.cfg` when the page first shows, and whether it was edited.
    pub personal_path: Option<String>,
    pub personal_path_dirty: bool,
}

impl Default for AppPagesState {
    fn default() -> Self {
        AppPagesState {
            selected_format: None,
            editor: None,
            export_settings: true,
            export_history: true,
            status: String::new(),
            busy: false,
            personal_path: None,
            personal_path_dirty: false,
        }
    }
}

/// Windows: Modlight's entries in Explorer's menus (`CheckShellContextMenuButton`,
/// `CheckSendToMenuButton`), read when the integration page opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExplorerMenus {
    pub context_menu: bool,
    pub send_to: bool,
}

/// Where the pages are drawn and what they show that is not a setting.
pub struct AppPagesCtx<'a> {
    pub colors: &'a Colors,
    pub state: &'a mut AppPagesState,
    pub asks: Vec<AppPageAsk>,
    pub language: LanguageSetting,
    pub personal_folder: String,
    /// Windows: the folder an empty personal folder box means (`DefaultPersonalFolder`).
    pub personal_default: String,
    pub screenshots_preview: String,
    pub startup: StartupState,
    /// Windows: the folder of another copy Windows starts (`OtherCopyStartupTargetPath`).
    pub startup_other_copy: Option<String>,
    /// Windows: the Explorer menu check boxes.
    pub explorer: ExplorerMenus,
    pub updates_visible: bool,
    pub dialogs: bool,
}

fn options(type_name: &str, names: &[&'static str]) -> Vec<(&'static str, String)> {
    names
        .iter()
        .map(|n| (*n, modlight_core::l10n::enum_text(type_name, n, None)))
        .collect()
}

/// Draw one application page; false when `section` is not one of these.
pub fn page(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder, section: &str) -> bool {
    if !ctx.state.status.is_empty() {
        status_banner(ui, ctx);
    }
    match section {
        "application.general" => general(ui, ctx, b),
        "application.integration" => integration(ui, ctx),
        "application.paths" => paths(ui, ctx, b),
        "application.settings" => backup(ui, ctx, b),
        "application.main-window" => main_window(ui, ctx, b),
        "application.clipboard-formats" => clipboard_formats(ui, ctx, b),
        "application.upload" => upload(ui, ctx, b),
        "application.history" => history(ui, ctx, b),
        "application.print" => print(ui, ctx, b),
        "application.proxy" => proxy(ui, ctx, b),
        "application.advanced" => advanced(ui, ctx, b),
        _ => return false,
    }
    true
}

/// The status line of a backup (`HasStatusMessage`): a spinner while busy.
fn status_banner(ui: &mut Ui, ctx: &mut AppPagesCtx) {
    let colors = ctx.colors;
    egui::Frame::new()
        .fill(colors.control_background())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ctx.state.busy {
                    ui.add(egui::Spinner::new().size(18.0).color(colors.accent_start()));
                }
                crate::widgets::text_wrapped(
                    ui,
                    &ctx.state.status,
                    13.0,
                    Weight::Normal,
                    colors.text(),
                );
            });
        });
    ui.add_space(12.0);
}

fn general(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_Application),
        true,
        |ui| {
            row(
                ui,
                colors,
                190.0,
                t(s::ApplicationSettingsWindow_LanguageLabel),
                |ui| {
                    let list = vec![
                        (
                            LanguageSetting::Automatic,
                            t(helpers::SupportedLanguage_Automatic).to_string(),
                        ),
                        (LanguageSetting::English, "English".to_string()),
                        (LanguageSetting::Polish, "Polski (Polish)".to_string()),
                    ];
                    if let Some(language) =
                        combo(ui, colors, "language", 250.0, &ctx.language, &list)
                    {
                        ctx.asks.push(AppPageAsk::Language(language));
                    }
                },
            );
            let before = b.bool(&["ShowTray"], true);
            let tray = check(
                ui,
                colors,
                b,
                &["ShowTray"],
                true,
                t(s::ApplicationSettingsWindow_ShowTrayIcon),
            );
            if tray != before {
                ctx.asks.push(AppPageAsk::ShowTray(tray));
            }
            ui.add_enabled_ui(tray, |ui| {
                check(
                    ui,
                    colors,
                    b,
                    &["SilentRun"],
                    false,
                    t(s::ApplicationSettingsWindow_MinimizeToTrayOnStart),
                );
            });
            check(
                ui,
                colors,
                b,
                &["TrayIconProgressEnabled"],
                true,
                t(s::ApplicationSettingsWindow_ShowProgressInTrayIcon),
            );
            // `TaskbarManager.IsPlatformSupported`: Windows only (MACOS.md).
            ui.add_enabled_ui(cfg!(windows), |ui| {
                check(
                    ui,
                    colors,
                    b,
                    &["TaskbarProgressEnabled"],
                    true,
                    t(s::ApplicationSettingsWindow_ShowProgressInTaskbarButton),
                );
            });
            check(
                ui,
                colors,
                b,
                &["UseWhiteShareXIcon"],
                false,
                t(s::ApplicationSettingsWindow_UseWhiteShareXIcon),
            );
            check(
                ui,
                colors,
                b,
                &["RememberMainFormPosition"],
                true,
                t(s::ApplicationSettingsWindow_RememberMainWindowPosition),
            );
            check(
                ui,
                colors,
                b,
                &["RememberMainFormSize"],
                true,
                t(s::ApplicationSettingsWindow_RememberMainWindowSize),
            );
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_TrayActions),
        true,
        |ui| {
            let jobs: Vec<(String, String)> = HotkeyType::ALL
                .iter()
                .map(|job| (job.name().to_string(), job.text()))
                .collect();
            for (label, member, default) in [
                (
                    s::ApplicationSettingsWindow_OnTrayIconDoubleLeftClickLabel,
                    "TrayLeftDoubleClickAction",
                    HotkeyType::OpenMainWindow,
                ),
                (
                    s::ApplicationSettingsWindow_OnTrayIconLeftClickLabel,
                    "TrayLeftClickAction",
                    HotkeyType::RectangleRegion,
                ),
                (
                    s::ApplicationSettingsWindow_OnTrayIconMiddleClickLabel,
                    "TrayMiddleClickAction",
                    HotkeyType::ClipboardUploadWithContentViewer,
                ),
            ] {
                row(ui, colors, 220.0, t(label), |ui| {
                    let current = b
                        .get(&[member])
                        .and_then(Value::as_str)
                        .and_then(HotkeyType::parse)
                        .unwrap_or(default)
                        .name()
                        .to_string();
                    let width = crate::settings_kit::combo_width(ui, &jobs);
                    if let Some(picked) = combo(ui, colors, member, width, &current, &jobs) {
                        b.set(&[member], json!(picked));
                    }
                });
            }
            if compact_button(
                ui,
                colors,
                t(s::ApplicationSettingsWindow_EditQuickTaskMenu),
            )
            .clicked()
            {
                ctx.asks.push(AppPageAsk::EditQuickTaskMenu);
            }
        },
    );
    // `UpdatesVisible`: only a build with an update host has a release channel.
    if ctx.updates_visible {
        card(
            ui,
            colors,
            t(s::ApplicationSettingsWindow_Updates),
            true,
            |ui| {
                let before = b.bool(&["AutoCheckUpdate"], true);
                let after = check(
                    ui,
                    colors,
                    b,
                    &["AutoCheckUpdate"],
                    true,
                    t(s::ApplicationSettingsWindow_AutomaticallyCheckForUpdates),
                );
                if after && !before {
                    ctx.asks.push(AppPageAsk::UpdatesOn);
                }
            },
        );
    }
}

/// `RefreshStartWithWindows`: the check box's text, whether it can change, whether it is
/// on. `other_copy` is the folder of another Modlight.exe that Windows starts instead of
/// this one (the settings page names it; the first-run window does not).
#[cfg(windows)]
pub fn start_with_windows(
    state: StartupState,
    first_run: bool,
    other_copy: Option<&str>,
) -> (String, bool, bool) {
    let (text, disabled_by_user) = if first_run {
        (
            s::FirstTimeConfigWindow_StartWithWindows,
            s::FirstTimeConfigWindow_StartWithWindowsDisabledByUser,
        )
    } else {
        (
            s::ApplicationSettingsForm_cbStartWithWindows_Text,
            s::ApplicationSettingsForm_cbStartWithWindows_DisabledByUser_Text,
        )
    };
    // "Off" must not read as "nothing starts": say which copy Windows starts.
    let text = match other_copy {
        Some(folder) if !first_run => format(
            t(s::ApplicationSettingsForm_cbStartWithWindows_OtherCopy_Text),
            &[Arg::from(folder)],
        ),
        _ => t(text).to_string(),
    };
    match state {
        // Turned off in Task Manager's Startup apps.
        StartupState::DisabledByUser => (t(disabled_by_user).to_string(), false, false),
        // A development build: registration is refused there.
        StartupState::NotAvailable => (text, false, false),
        StartupState::Enabled | StartupState::RequiresApproval => (text, true, true),
        StartupState::Disabled => (text, true, false),
    }
}

/// The Integration page (`IsIntegrationPage`): the "Windows" card with Start with Windows
/// and Modlight's entries in Explorer's context menu and "Send to" menu. Upstream's
/// "Edit with Modlight" entry is not offered: the platform layer does not register it.
#[cfg(windows)]
fn integration(ui: &mut Ui, ctx: &mut AppPagesCtx) {
    let colors = ctx.colors;
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_Windows),
        true,
        |ui| {
            let (label, enabled, mut on) =
                start_with_windows(ctx.startup, false, ctx.startup_other_copy.as_deref());
            if crate::widgets::checkbox_weighted(
                ui,
                colors,
                &mut on,
                Some(&label),
                enabled,
                Weight::Normal,
            )
            .changed()
            {
                ctx.asks.push(AppPageAsk::StartAtLogin(on));
            }
            let mut context_menu = ctx.explorer.context_menu;
            if crate::widgets::checkbox_weighted(
            ui,
            colors,
            &mut context_menu,
            Some(t(
                s::ApplicationSettingsWindow_ShowUploadWithShareXButtonInWindowsExplorerContextMenu,
            )),
            true,
            Weight::Normal,
        )
        .changed()
        {
            ctx.asks.push(AppPageAsk::ContextMenu(context_menu));
        }
            let mut send_to = ctx.explorer.send_to;
            if crate::widgets::checkbox_weighted(
                ui,
                colors,
                &mut send_to,
                Some(t(s::ApplicationSettingsWindow_ShowShareXInSendToMenu)),
                true,
                Weight::Normal,
            )
            .changed()
            {
                ctx.asks.push(AppPageAsk::SendTo(send_to));
            }
        },
    );
}

#[cfg(not(windows))]
fn integration(ui: &mut Ui, ctx: &mut AppPagesCtx) {
    let colors = ctx.colors;
    card(ui, colors, t(mac::MacTitle), true, |ui| {
        // `RefreshStartWithWindows`: the label says why the box cannot change.
        let (label, enabled) = match ctx.startup {
            StartupState::DisabledByUser => (t(mac::StartAtLoginDisabledByUser), false),
            StartupState::RequiresApproval => (t(mac::StartAtLoginRequiresApproval), true),
            StartupState::NotAvailable => (t(mac::StartAtLoginNotAvailable), false),
            StartupState::Enabled | StartupState::Disabled => (t(mac::StartAtLogin), true),
        };
        let mut on = matches!(
            ctx.startup,
            StartupState::Enabled | StartupState::RequiresApproval
        );
        if crate::widgets::checkbox_weighted(
            ui,
            colors,
            &mut on,
            Some(label),
            enabled,
            Weight::Normal,
        )
        .changed()
        {
            ctx.asks.push(AppPageAsk::StartAtLogin(on));
        }
        if matches!(
            ctx.startup,
            StartupState::DisabledByUser | StartupState::RequiresApproval
        ) && compact_button(ui, colors, t(mac::OpenLoginItems)).clicked()
        {
            ctx.asks.push(AppPageAsk::OpenLoginItems);
        }
        ui.add_space(4.0);
        hint(ui, colors, t(mac::FinderServiceHint));
        if compact_button(ui, colors, t(mac::FinderServiceButton)).clicked() {
            ctx.asks.push(AppPageAsk::OpenServicesSettings);
        }
    });
}

/// Windows: the personal folder card as upstream draws it: the box (`PersonalFolderPath`,
/// stored in `PersonalPath.cfg` when the window closes), Browse, Open, the folder it means
/// and the note that a change waits for a restart.
#[cfg(windows)]
fn personal_folder_card(ui: &mut Ui, ctx: &mut AppPagesCtx) {
    let colors = ctx.colors;
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_ShareXPersonalFolder),
        true,
        |ui| {
            let mut text = ctx.state.personal_path.clone().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let open_w = crate::widgets::button_width(
                    ui,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Open)),
                );
                let browse_w = crate::widgets::button_width(
                    ui,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Browse)),
                );
                let response = Field {
                    height: 32.0,
                    width: Some(ui.available_width() - open_w - browse_w - 16.0),
                    id: Some(egui::Id::new("personal-folder")),
                    ..Field::new(&mut text)
                }
                .show(ui, colors);
                if response.changed() {
                    ctx.state.personal_path = Some(text.clone());
                    ctx.state.personal_path_dirty = true;
                }
                if compact_button(ui, colors, t(s::ApplicationSettingsWindow_Browse)).clicked() {
                    ctx.asks.push(AppPageAsk::BrowsePersonalFolder);
                }
                if compact_button(ui, colors, t(s::ApplicationSettingsWindow_Open)).clicked() {
                    ctx.asks.push(AppPageAsk::OpenPersonalFolder);
                }
            });
            let preview = if text.trim().is_empty() {
                ctx.personal_default.clone()
            } else {
                crate::personal_folder::absolute(&text)
                    .display()
                    .to_string()
            };
            hint(ui, colors, &preview);
            hint(
                ui,
                colors,
                t(s::ApplicationSettingsWindow_ChangesToThePersonalFolderAreStoredImmediatelyAndTakeEffectAfterRestart),
            );
        },
    );
}

fn paths(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    #[cfg(windows)]
    personal_folder_card(ui, ctx);
    #[cfg(not(windows))]
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_ShareXPersonalFolder),
        true,
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let open_w = crate::widgets::button_width(
                    ui,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Open)),
                );
                let browse_w = crate::widgets::button_width(
                    ui,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Browse)),
                );
                let mut shown = ctx.personal_folder.clone();
                Field {
                    height: 32.0,
                    width: Some(ui.available_width() - open_w - browse_w - 16.0),
                    enabled: false,
                    id: Some(egui::Id::new("personal-folder")),
                    ..Field::new(&mut shown)
                }
                .show(ui, colors);
                // On a Mac the folder is fixed (MACOS.md): Browse stays off.
                let _ = button(
                    ui,
                    colors,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Browse)),
                    false,
                );
                if button(
                    ui,
                    colors,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Open)),
                    true,
                )
                .clicked()
                {
                    ctx.asks.push(AppPageAsk::OpenPersonalFolder);
                }
            });
            hint(ui, colors, &ctx.personal_folder);
            hint(ui, colors, t(mac::PersonalFolderFixed));
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_ScreenshotsFolder),
        true,
        |ui| {
            check(
                ui,
                colors,
                b,
                &["UseCustomScreenshotsPath"],
                false,
                t(s::ApplicationSettingsWindow_UseCustomScreenshotsFolder),
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let browse_w = crate::widgets::button_width(
                    ui,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Browse)),
                );
                ui.allocate_ui(vec2(ui.available_width() - browse_w - 8.0, 32.0), |ui| {
                    sanitized_text(ui, colors, b, "CustomScreenshotsPath", "");
                });
                if compact_button(ui, colors, t(s::ApplicationSettingsWindow_Browse)).clicked() {
                    ctx.asks.push(AppPageAsk::BrowseScreenshotsFolder);
                }
            });
            row(
                ui,
                colors,
                210.0,
                t(s::ApplicationSettingsWindow_SubFolderPatternLabel),
                |ui| sanitized_text(ui, colors, b, "SaveImageSubFolderPattern", "%y-%mo"),
            );
            row(
                ui,
                colors,
                210.0,
                t(s::ApplicationSettingsWindow_SubFolderPatternForWindowLabel),
                |ui| sanitized_text(ui, colors, b, "SaveImageSubFolderPatternWindow", ""),
            );
            ui.horizontal(|ui| {
                let open_w = crate::widgets::button_width(
                    ui,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Open)),
                );
                let galley = layout(
                    ui,
                    trimmed(
                        &ctx.screenshots_preview,
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        ui.available_width() - open_w - 8.0,
                        1,
                    ),
                );
                let (r, _) = ui.allocate_exact_size(
                    vec2(ui.available_width() - open_w - 8.0, 32.0),
                    Sense::hover(),
                );
                ui.painter().galley(
                    pos2(r.left(), r.center().y - galley.size().y / 2.0),
                    galley,
                    colors.text_secondary(),
                );
                ui.add_space(8.0);
                if compact_button(ui, colors, t(s::ApplicationSettingsWindow_Open)).clicked() {
                    ctx.asks.push(AppPageAsk::OpenScreenshotsFolder);
                }
            });
        },
    );
}

fn compact_look(colors: &Colors) -> Look {
    Look {
        string_text: Some(colors.text()),
        ..Look::button(colors).height(32.0).padding(10.0)
    }
}

/// A path box whose value goes through `FileHelpers.SanitizePath`.
fn sanitized_text(
    ui: &mut Ui,
    colors: &Colors,
    b: &mut Binder,
    member: &str,
    default: &str,
) -> bool {
    let mut value = b.str(&[member], default);
    let id = ui.make_persistent_id(("app-path", member));
    let response = Field {
        height: 32.0,
        enabled: ui.is_enabled(),
        id: Some(id),
        ..Field::new(&mut value)
    }
    .show(ui, colors);
    if response.changed() {
        b.set(
            &[member],
            json!(modlight_core::names::file_helpers::sanitize_path(&value)),
        );
        return true;
    }
    false
}

fn backup(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_ExportAndImport),
        true,
        |ui| {
            hint(
            ui,
            colors,
            t(s::ApplicationSettingsWindow_DoNotShareAnExportedBackupItCanContainPrivateInformationSuchAsAccountDetailsAndUploadHistory),
        );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                crate::widgets::checkbox_weighted(
                    ui,
                    colors,
                    &mut ctx.state.export_settings,
                    Some(t(s::ApplicationSettingsWindow_Settings)),
                    true,
                    Weight::Normal,
                );
                crate::widgets::checkbox_weighted(
                    ui,
                    colors,
                    &mut ctx.state.export_history,
                    Some(t(s::ApplicationSettingsWindow_History)),
                    true,
                    Weight::Normal,
                );
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let busy = ctx.state.busy;
                let can_export = !busy && (ctx.state.export_settings || ctx.state.export_history);
                if button(
                    ui,
                    colors,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Export)),
                    can_export,
                )
                .clicked()
                {
                    ctx.asks.push(AppPageAsk::Export);
                }
                if button(
                    ui,
                    colors,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Import)),
                    !busy,
                )
                .clicked()
                {
                    ctx.asks.push(AppPageAsk::Import);
                }
                if button(
                    ui,
                    colors,
                    &compact_look(colors),
                    crate::widgets::Content::label(t(s::ApplicationSettingsWindow_ResetSettings)),
                    !busy,
                )
                .clicked()
                {
                    ctx.asks.push(AppPageAsk::ResetSettings);
                }
            });
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AutomaticCleanup),
        true,
        |ui| {
            check(
                ui,
                colors,
                b,
                &["AutoCleanupBackupFiles"],
                false,
                t(s::ApplicationSettingsWindow_AutomaticallyCleanupOldBackupFiles),
            );
            check(
                ui,
                colors,
                b,
                &["AutoCleanupLogFiles"],
                false,
                t(s::ApplicationSettingsWindow_AutomaticallyCleanupOldLogFiles),
            );
            row(
                ui,
                colors,
                190.0,
                t(s::ApplicationSettingsWindow_NumberOfFilesToKeepLabel),
                |ui| {
                    number(
                        ui,
                        colors,
                        b,
                        &["CleanupKeepFileCount"],
                        10.0,
                        1.0,
                        10000.0,
                        1.0,
                        0,
                    )
                },
            );
        },
    );
}

/// `System.Drawing.Size` ("200, 150").
fn size_of(b: &Binder, member: &str, default: (i64, i64)) -> (i64, i64) {
    b.get(&[member])
        .and_then(|v| match v {
            Value::String(text) => text
                .split_once(',')
                .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?))),
            Value::Object(map) => Some((
                map.get("Width").and_then(Value::as_i64)?,
                map.get("Height").and_then(Value::as_i64)?,
            )),
            _ => None,
        })
        .unwrap_or(default)
}

fn main_window(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_ThumbnailView),
        true,
        |ui| {
            check(
                ui,
                colors,
                b,
                &["ShowThumbnailTitle"],
                true,
                t(s::ApplicationSettingsWindow_ShowTitle),
            );
            row(
                ui,
                colors,
                180.0,
                t(s::ApplicationSettingsWindow_TitleLocationLabel),
                |ui| {
                    crate::settings_kit::enum_combo(
                        ui,
                        colors,
                        b,
                        &["ThumbnailTitleLocation"],
                        &options("ThumbnailTitleLocation", &["Top", "Bottom"]),
                        "Top",
                    )
                },
            );
            let (mut w, mut h) = size_of(b, "ThumbnailSize", (200, 150));
            row(
                ui,
                colors,
                180.0,
                t(s::ApplicationSettingsWindow_ThumbnailSizeLabel),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let mut wf = w as f64;
                    let mut hf = h as f64;
                    let cw = crate::settings_kit::number_value(
                        ui, colors, "thumb-w", &mut wf, 50.0, 500.0, 1.0, 0,
                    );
                    crate::widgets::text(
                        ui,
                        "\u{00D7}",
                        13.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                    let ch = crate::settings_kit::number_value(
                        ui, colors, "thumb-h", &mut hf, 50.0, 500.0, 1.0, 0,
                    );
                    if cw || ch {
                        w = wf as i64;
                        h = hf as i64;
                        b.set(&["ThumbnailSize"], json!(format!("{w}, {h}")));
                    }
                    if compact_button(ui, colors, t(s::ApplicationSettingsWindow_Reset)).clicked() {
                        b.set(&["ThumbnailSize"], json!("200, 150"));
                    }
                },
            );
            row(
                ui,
                colors,
                180.0,
                t(s::ApplicationSettingsWindow_ThumbnailClickActionLabel),
                |ui| {
                    crate::settings_kit::enum_combo(
                        ui,
                        colors,
                        b,
                        &["ThumbnailClickAction"],
                        &options(
                            "ThumbnailViewClickAction",
                            &[
                                "Default",
                                "Select",
                                "OpenImageViewer",
                                "OpenFile",
                                "OpenFolder",
                                "OpenURL",
                                "EditImage",
                            ],
                        ),
                        "Default",
                    )
                },
            );
        },
    );
}

fn formats(b: &Binder) -> Vec<Value> {
    match b.get(&["ClipboardContentFormats"]) {
        Some(Value::Array(list)) => list.clone(),
        _ => Vec::new(),
    }
}

fn clipboard_formats(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let list = formats(b);
    card(ui, colors, "", true, |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        hint(
            ui,
            colors,
            t(s::ApplicationSettingsWindow_TheseFormatsAppearUnderTheCopySubmenuInTheMainWindowContextMenu),
        );
        ui.add_space(6.0);
        let empty = list.is_empty().then(|| {
            (
                t(s::ApplicationSettingsWindow_NoClipboardFormats),
                t(s::ApplicationSettingsWindow_NoClipboardFormatsExample),
            )
        });
        let selected = ctx.state.selected_format.filter(|i| *i < list.len());
        let mut clicked = None;
        let mut edit = None;
        list_frame(ui, colors, 260.0, empty, |ui| {
            for (index, format) in list.iter().enumerate() {
                let description = format
                    .get("Description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let value = format.get("Format").and_then(Value::as_str).unwrap_or("");
                let response = list_row(ui, colors, selected == Some(index), 40.0, |ui, r| {
                    let inner = r.shrink2(vec2(8.0, 3.0));
                    let top = layout(
                        ui,
                        trimmed(
                            description,
                            13.0,
                            Weight::Normal,
                            colors.text(),
                            inner.width(),
                            1,
                        ),
                    );
                    let bottom = layout(
                        ui,
                        trimmed(
                            value,
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                            inner.width(),
                            1,
                        ),
                    );
                    let y = inner.center().y - (top.size().y + bottom.size().y) / 2.0;
                    let top_h = top.size().y;
                    ui.painter()
                        .galley(pos2(inner.left(), y), top, colors.text());
                    ui.painter().galley(
                        pos2(inner.left(), y + top_h),
                        bottom,
                        colors.text_secondary(),
                    );
                });
                if response.clicked() {
                    clicked = Some(index);
                }
                if response.double_clicked() {
                    edit = Some(index);
                }
            }
        });
        if clicked.is_some() {
            ctx.state.selected_format = clicked;
        }
        let selected = ctx.state.selected_format.filter(|i| *i < list.len());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            if compact_button(ui, colors, t(s::ApplicationSettingsWindow_Add)).clicked() {
                ctx.state.editor = Some(FormatEditor::new(None, None));
            }
            if button(
                ui,
                colors,
                &compact_look(colors),
                crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Edit)),
                selected.is_some(),
            )
            .clicked()
            {
                edit = selected;
            }
            if button(
                ui,
                colors,
                &compact_look(colors),
                crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Remove)),
                selected.is_some(),
            )
            .clicked()
                && let Some(index) = selected
            {
                let mut list = list.clone();
                list.remove(index);
                ctx.state.selected_format = if list.is_empty() {
                    None
                } else {
                    Some(index.min(list.len() - 1))
                };
                b.set(&["ClipboardContentFormats"], Value::Array(list));
            }
        });
        if let Some(index) = edit {
            ctx.state.editor = Some(FormatEditor::new(Some(index), list.get(index)));
        }
    });
}

fn upload(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    card(ui, colors, "", true, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        row(
            ui,
            colors,
            240.0,
            t(s::ApplicationSettingsWindow_SimultaneousUploadLimitLabel),
            |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                number(ui, colors, b, &["UploadLimit"], 0.0, 0.0, 25.0, 1.0, 0);
                crate::widgets::text(
                    ui,
                    t(s::ApplicationSettingsWindow_ZeroDisablesTheLimit),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
            },
        );
        row(
            ui,
            colors,
            240.0,
            t(s::ApplicationSettingsWindow_BufferSizeLabel),
            |ui| {
                // `RefreshBufferSizeOptions`: 2^n KB for n in 0..14, in the chosen units.
                let binary = b.bool(&["BinaryUnits"], false);
                let list: Vec<(i64, String)> = (0..14)
                    .map(|power| {
                        (
                            power,
                            modlight_core::format::local_size((1i64 << power) * 1024, binary, 0),
                        )
                    })
                    .collect();
                let current = b
                    .get(&["BufferSizePower"])
                    .and_then(Value::as_i64)
                    .unwrap_or(5);
                let width = (list
                    .iter()
                    .map(|(_, l)| {
                        layout(
                            ui,
                            crate::widgets::job(l, 13.0, Weight::Normal, colors.text()),
                        )
                        .size()
                        .x
                    })
                    .fold(0.0_f32, f32::max)
                    + 48.0)
                    .max(250.0);
                if let Some(picked) = combo(ui, colors, "buffer-size", width, &current, &list) {
                    b.set(&["BufferSizePower"], json!(picked));
                }
            },
        );
        row(
            ui,
            colors,
            240.0,
            t(s::ApplicationSettingsWindow_NumberOfTimesToRetryIfUploadFailsLabel),
            |ui| {
                number(
                    ui,
                    colors,
                    b,
                    &["MaxUploadFailRetry"],
                    1.0,
                    0.0,
                    5.0,
                    1.0,
                    0,
                )
            },
        );
    });
}

fn history(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_History),
        true,
        |ui| {
            check(
                ui,
                colors,
                b,
                &["HistorySaveTasks"],
                true,
                t(s::ApplicationSettingsWindow_SaveTasksToHistory),
            );
            check(
                ui,
                colors,
                b,
                &["HistoryCheckURL"],
                false,
                t(s::ApplicationSettingsWindow_OnlySaveIfUrlIsNotEmpty),
            );
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_RecentTasks),
        true,
        |ui| {
            check(
                ui,
                colors,
                b,
                &["RecentTasksSave"],
                false,
                t(s::ApplicationSettingsWindow_SaveRecentTasks),
            );
            row(
                ui,
                colors,
                230.0,
                t(s::ApplicationSettingsWindow_MaximumNumberOfTasksToSaveLabel),
                |ui| {
                    number(
                        ui,
                        colors,
                        b,
                        &["RecentTasksMaxCount"],
                        10.0,
                        1.0,
                        10000.0,
                        1.0,
                        0,
                    )
                },
            );
            check(
                ui,
                colors,
                b,
                &["RecentTasksShowInMainWindow"],
                true,
                t(s::ApplicationSettingsWindow_ShowRecentTasksInMainWindowOnStartup),
            );
            check(
                ui,
                colors,
                b,
                &["RecentTasksShowInTrayMenu"],
                true,
                t(s::ApplicationSettingsWindow_ShowRecentTasksInTrayMenu),
            );
            check(
                ui,
                colors,
                b,
                &["RecentTasksTrayMenuMostRecentFirst"],
                false,
                t(s::ApplicationSettingsWindow_InTrayMenuShowMostRecentTasksFirst),
            );
        },
    );
}

fn print(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    card(ui, colors, "", true, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        check(
            ui,
            colors,
            b,
            &["DontShowPrintSettingsDialog"],
            false,
            t(s::ApplicationSettingsWindow_DontShowImagePrintSettingsDialog),
        );
        if compact_button(
            ui,
            colors,
            t(s::ApplicationSettingsWindow_ImagePrintSettings),
        )
        .clicked()
        {
            ctx.asks.push(AppPageAsk::ImagePrintSettings);
        }
        // `DontShowWindowsPrintDialog` is `!PrintSettings.ShowPrintDialog`.
        crate::settings_kit::check_inverted(
            ui,
            colors,
            b,
            &["PrintSettings", "ShowPrintDialog"],
            true,
            t(DONT_SHOW_PRINT_DIALOG),
        );
        if !b.bool(&["PrintSettings", "ShowPrintDialog"], true) {
            row(
                ui,
                colors,
                190.0,
                t(s::ApplicationSettingsWindow_DefaultPrinterOverrideLabel),
                |ui| {
                    text(
                        ui,
                        colors,
                        b,
                        &["PrintSettings", "DefaultPrinterOverride"],
                        "",
                    )
                },
            );
        }
    });
}

fn proxy(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    card(ui, colors, "", true, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        row(
            ui,
            colors,
            170.0,
            t(s::ApplicationSettingsWindow_ProxyConfigurationLabel),
            |ui| {
                crate::settings_kit::enum_combo(
                    ui,
                    colors,
                    b,
                    &["ProxySettings", "ProxyMethod"],
                    &options("ProxyMethod", &["None", "Manual", "Automatic"]),
                    "None",
                )
            },
        );
        let method = b.name(
            &["ProxySettings", "ProxyMethod"],
            &["None", "Manual", "Automatic"],
            "None",
        );
        let credentials = method != "None";
        let manual = method == "Manual";
        ui.add_enabled_ui(credentials, |ui| {
            row(
                ui,
                colors,
                170.0,
                t(s::ApplicationSettingsWindow_UsernameLabel),
                |ui| text(ui, colors, b, &["ProxySettings", "Username"], ""),
            );
            row(
                ui,
                colors,
                170.0,
                t(s::ApplicationSettingsWindow_PasswordLabel),
                |ui| {
                    let mut value = b.str(&["ProxySettings", "Password"], "");
                    let response = Field {
                        height: 32.0,
                        password: true,
                        enabled: ui.is_enabled(),
                        id: Some(ui.make_persistent_id("proxy-password")),
                        ..Field::new(&mut value)
                    }
                    .show(ui, colors);
                    if response.changed() {
                        b.set(&["ProxySettings", "Password"], json!(value));
                    }
                },
            );
        });
        ui.add_enabled_ui(manual, |ui| {
            row(
                ui,
                colors,
                170.0,
                t(s::ApplicationSettingsWindow_HostLabel),
                |ui| text(ui, colors, b, &["ProxySettings", "Host"], ""),
            );
            row(
                ui,
                colors,
                170.0,
                t(s::ApplicationSettingsWindow_PortLabel),
                |ui| {
                    number(
                        ui,
                        colors,
                        b,
                        &["ProxySettings", "Port"],
                        0.0,
                        0.0,
                        65535.0,
                        1.0,
                        0,
                    )
                },
            );
        });
    });
}

fn advanced(ui: &mut Ui, ctx: &mut AppPagesCtx, b: &mut Binder) {
    let colors = ctx.colors;
    let int_row = |ui: &mut Ui,
                   b: &mut Binder,
                   label: &str,
                   member: &str,
                   default: f64,
                   min: f64,
                   max: f64| {
        row(ui, colors, 240.0, label, |ui| {
            number(ui, colors, b, &[member], default, min, max, 1.0, 0)
        });
    };
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AdvancedApplication),
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            check(
                ui,
                colors,
                b,
                &["BinaryUnits"],
                false,
                t(s::ApplicationSettingsWindow_BinaryUnits),
            );
            check(
                ui,
                colors,
                b,
                &["ShowMostRecentTaskFirst"],
                false,
                t(s::ApplicationSettingsWindow_ShowMostRecentTaskFirst),
            );
            check(
                ui,
                colors,
                b,
                &["WorkflowsOnlyShowEdited"],
                false,
                t(s::ApplicationSettingsWindow_WorkflowsOnlyShowEdited),
            );
            check(
                ui,
                colors,
                b,
                &["TrayAutoExpandCaptureMenu"],
                false,
                t(s::ApplicationSettingsWindow_TrayAutoExpandCaptureMenu),
            );
            row(
                ui,
                colors,
                240.0,
                t(s::ApplicationSettingsWindow_BrowserPath),
                |ui| text(ui, colors, b, &["BrowserPath"], ""),
            );
            check(
                ui,
                colors,
                b,
                &["SaveSettingsAfterTaskCompleted"],
                false,
                t(s::ApplicationSettingsWindow_SaveSettingsAfterTaskCompleted),
            );
            check(
                ui,
                colors,
                b,
                &["DevMode"],
                false,
                t(s::ApplicationSettingsWindow_DevMode),
            );
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AdvancedHotkeys),
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let before = b.bool(&["DisableHotkeys"], false);
            let after = check(
                ui,
                colors,
                b,
                &["DisableHotkeys"],
                false,
                t(s::ApplicationSettingsWindow_DisableHotkeys),
            );
            if before != after {
                ctx.asks.push(AppPageAsk::DisableHotkeys(after));
            }
            check(
                ui,
                colors,
                b,
                &["DisableHotkeysOnFullscreen"],
                false,
                t(s::ApplicationSettingsWindow_DisableHotkeysOnFullscreen),
            );
            int_row(
                ui,
                b,
                t(s::ApplicationSettingsWindow_HotkeyRepeatLimit),
                "HotkeyRepeatLimit",
                500.0,
                200.0,
                i32::MAX as f64,
            );
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AdvancedClipboard),
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            check(
                ui,
                colors,
                b,
                &["ShowClipboardContentViewer"],
                true,
                t(s::ApplicationSettingsWindow_ShowClipboardContentViewer),
            );
            check(
                ui,
                colors,
                b,
                &["DefaultClipboardCopyImageFillBackground"],
                true,
                t(s::ApplicationSettingsWindow_DefaultClipboardCopyImageFillBackground),
            );
            check(
                ui,
                colors,
                b,
                &["UseAlternativeClipboardCopyImage"],
                false,
                t(s::ApplicationSettingsWindow_UseAlternativeClipboardCopyImage),
            );
            check(
                ui,
                colors,
                b,
                &["UseAlternativeClipboardGetImage"],
                false,
                t(s::ApplicationSettingsWindow_UseAlternativeClipboardGetImage),
            );
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AdvancedImage),
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            check(
                ui,
                colors,
                b,
                &["RotateImageByExifOrientationData"],
                true,
                t(s::ApplicationSettingsWindow_RotateImageByExifOrientationData),
            );
            check(
                ui,
                colors,
                b,
                &["PNGStripColorSpaceInformation"],
                false,
                t(s::ApplicationSettingsWindow_PNGStripColorSpaceInformation),
            );
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AdvancedUpload),
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            check(
                ui,
                colors,
                b,
                &["DisableUpload"],
                false,
                t(s::ApplicationSettingsWindow_DisableUpload),
            );
            check(
                ui,
                colors,
                b,
                &["URLEncodeIgnoreEmoji"],
                true,
                t(s::ApplicationSettingsWindow_URLEncodeIgnoreEmoji),
            );
            check(
                ui,
                colors,
                b,
                &["ShowMultiUploadWarning"],
                true,
                t(s::ApplicationSettingsWindow_ShowMultiUploadWarning),
            );
            int_row(
                ui,
                b,
                t(s::ApplicationSettingsWindow_ShowLargeFileSizeWarning),
                "ShowLargeFileSizeWarning",
                100.0,
                0.0,
                i32::MAX as f64,
            );
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AdvancedPaths),
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            check(
                ui,
                colors,
                b,
                &["UseMachineSpecificUploadersConfig"],
                false,
                t(s::ApplicationSettingsWindow_UseMachineSpecificUploadersConfig),
            );
            for (label, member) in [
                (
                    s::ApplicationSettingsWindow_CustomUploadersConfigPath,
                    "CustomUploadersConfigPath",
                ),
                (
                    s::ApplicationSettingsWindow_CustomHotkeysConfigPath,
                    "CustomHotkeysConfigPath",
                ),
                (
                    s::ApplicationSettingsWindow_CustomScreenshotsPath2,
                    "CustomScreenshotsPath2",
                ),
            ] {
                row(ui, colors, 240.0, t(label), |ui| {
                    text(ui, colors, b, &[member], "")
                });
            }
        },
    );
    card(
        ui,
        colors,
        t(s::ApplicationSettingsWindow_AdvancedDragAndDropWindow),
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            int_row(
                ui,
                b,
                t(s::ApplicationSettingsWindow_DropSize),
                "DropSize",
                150.0,
                1.0,
                i32::MAX as f64,
            );
            int_row(
                ui,
                b,
                t(s::ApplicationSettingsWindow_DropOffset),
                "DropOffset",
                5.0,
                i32::MIN as f64,
                i32::MAX as f64,
            );
            row(
                ui,
                colors,
                240.0,
                t(s::ApplicationSettingsWindow_DropAlignment),
                |ui| {
                    let list: Vec<(&'static str, String)> = [
                        ("TopLeft", s::ApplicationSettingsWindow_TopLeft),
                        ("TopCenter", s::ApplicationSettingsWindow_TopCenter),
                        ("TopRight", s::ApplicationSettingsWindow_TopRight),
                        ("MiddleLeft", s::ApplicationSettingsWindow_MiddleLeft),
                        ("MiddleCenter", s::ApplicationSettingsWindow_MiddleCenter),
                        ("MiddleRight", s::ApplicationSettingsWindow_MiddleRight),
                        ("BottomLeft", s::ApplicationSettingsWindow_BottomLeft),
                        ("BottomCenter", s::ApplicationSettingsWindow_BottomCenter),
                        ("BottomRight", s::ApplicationSettingsWindow_BottomRight),
                    ]
                    .into_iter()
                    .map(|(name, key)| (name, t(key).to_string()))
                    .collect();
                    crate::settings_kit::enum_combo(
                        ui,
                        colors,
                        b,
                        &["DropAlignment"],
                        &list,
                        "BottomRight",
                    )
                },
            );
            int_row(
                ui,
                b,
                t(s::ApplicationSettingsWindow_DropOpacity),
                "DropOpacity",
                100.0,
                0.0,
                255.0,
            );
            int_row(
                ui,
                b,
                t(s::ApplicationSettingsWindow_DropHoverOpacity),
                "DropHoverOpacity",
                255.0,
                0.0,
                255.0,
            );
        },
    );
}

// ------------------------------------------------------------------ copy format editor

/// The tokens of an upload's result (`ClipboardResultTokens`).
const RESULT_TOKENS: [&str; 13] = [
    "$result",
    "$url",
    "$shorturl",
    "$thumbnailurl",
    "$deletionurl",
    "$filepath",
    "$filename",
    "$filenamenoext",
    "$thumbnailfilename",
    "$thumbnailfilenamenoext",
    "$folderpath",
    "$foldername",
    "$uploadtime",
];

/// `ClipboardFormatEditorOverlay`.
#[derive(Clone, Debug)]
pub struct FormatEditor {
    pub index: Option<usize>,
    original: Value,
    pub description: String,
    pub format: String,
    focus: bool,
}

impl FormatEditor {
    pub fn new(index: Option<usize>, format: Option<&Value>) -> FormatEditor {
        let text = |key: &str| {
            format
                .and_then(|f| f.get(key))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        FormatEditor {
            index: if format.is_some() { index } else { None },
            original: format.cloned().unwrap_or(Value::Null),
            description: text("Description"),
            format: text("Format"),
            focus: true,
        }
    }

    fn save(&self) -> Value {
        let mut value = match &self.original {
            Value::Object(_) => self.original.clone(),
            _ => json!({}),
        };
        value["Description"] = json!(self.description);
        value["Format"] = json!(self.format);
        value
    }
}

/// Draw the copy format editor over `area` when it is open, and write what it saved.
/// Escape closes it.
pub fn format_editor(
    ui: &mut Ui,
    colors: &Colors,
    state: &mut AppPagesState,
    b: &mut Binder,
    area: Rect,
) {
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    let ctx = ui.ctx().clone();
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut done: Option<bool> = None;
    egui::Area::new(egui::Id::new("format-editor-veil"))
        .order(Order::Foreground)
        .fixed_pos(area.min)
        .show(&ctx, |ui| {
            let (veil, _) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
            ui.painter().rect_filled(veil, 0.0, colors.overlay_modal());
            let size = vec2(640.0_f32.min(veil.width() - 24.0), 560.0_f32.min(veil.height() - 24.0));
            let panel = Rect::from_center_size(veil.center(), size);
            ui.painter().rect_filled(panel, 8.0, colors.background_panel());
            ui.painter()
                .rect_stroke(panel, 8.0, Stroke::new(1.0, colors.border()), StrokeKind::Inside);
            let inner = panel.shrink(16.0);
            let title = if editor.index.is_none() {
                t(s::ApplicationSettingsWindow_AddClipboardFormat)
            } else {
                t(s::ApplicationSettingsWindow_EditClipboardFormat)
            };
            let width = inner.width() - 30.0;
            let heading = layout(ui, trimmed(title, 19.0, Weight::SemiBold, colors.text_on_panel(), width, 1));
            let sub = layout(
                ui,
                wrapped(
                    t(s::ApplicationSettingsWindow_CustomizeTheTextCopiedFromCompletedUploads),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary_on_panel(),
                    width,
                ),
            );
            let head_h = heading.size().y + 1.0 + sub.size().y;
            paint_icon(
                ui,
                ui.painter(),
                pos2(inner.left() + 10.5, inner.top() + head_h / 2.0),
                icons::CLIPBOARD_LIST,
                21.0,
                colors.accent_start(),
            );
            let heading_h = heading.size().y;
            ui.painter()
                .galley(pos2(inner.left() + 30.0, inner.top()), heading, colors.text_on_panel());
            ui.painter().galley(
                pos2(inner.left() + 30.0, inner.top() + heading_h + 1.0),
                sub,
                colors.text_secondary_on_panel(),
            );
            let body = Rect::from_min_max(
                pos2(inner.left(), inner.top() + head_h + 12.0),
                pos2(inner.right(), inner.bottom() - 32.0 - 12.0),
            );
            let mut child = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
            let focus = std::mem::take(&mut editor.focus);
            egui::ScrollArea::vertical()
                .id_salt("format-editor")
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let w = ui.available_width() - 4.0;
                    crate::widgets::text(ui, t(s::ApplicationSettingsWindow_Description), 13.0, Weight::Normal, colors.text_secondary_on_panel());
                    ui.add_space(3.0);
                    let id = egui::Id::new("format-description");
                    let response = Field {
                        height: 32.0,
                        width: Some(w),
                        id: Some(id),
                        ..Field::new(&mut editor.description)
                    }
                    .show(ui, colors);
                    if focus {
                        response.request_focus();
                    }
                    ui.add_space(8.0);
                    crate::widgets::text(ui, t(s::ApplicationSettingsWindow_Format), 13.0, Weight::Normal, colors.text_secondary_on_panel());
                    ui.add_space(3.0);
                    let format_id = egui::Id::new("format-text");
                    let output = egui::Frame::new()
                        .fill(colors.control_background())
                        .stroke(Stroke::new(1.0, colors.control_border()))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.set_width(w - 16.0);
                            egui::TextEdit::multiline(&mut editor.format)
                                .id(format_id)
                                .frame(egui::Frame::NONE)
                                .font(egui::FontId::proportional(13.0))
                                .text_color(colors.text())
                                .desired_width(f32::INFINITY)
                                .min_size(vec2(w - 16.0, 150.0 - 8.0))
                                .show(ui)
                        });
                    // The variables menu (`AttachClipboardFormatMenu`).
                    let tokens = token_menu();
                    if let Some(Command::Local(index)) =
                        crate::menus::context(colors, &output.inner.response, || tokens.0.clone())
                        && let Some(token) = tokens.1.get(index)
                    {
                        crate::task_editors::insert_at_caret(ui.ctx(), format_id, &mut editor.format, token);
                    }
                    ui.add_space(4.0);
                    crate::widgets::text_wrapped(
                        ui,
                        t(s::ApplicationSettingsWindow_RightClickToInsertAVariable),
                        12.0,
                        Weight::Normal,
                        colors.text_secondary_on_panel(),
                    );
                    ui.add_space(8.0);
                    crate::widgets::text_wrapped(
                        ui,
                        &format(
                            t(s::ClipboardFormatForm_ClipboardFormatForm_Supported_variables___0__and_other_variables_such_as__1__etc_),
                            &[Arg::from(RESULT_TOKENS.join(", ")), Arg::from("%y, %mo, %d")],
                        ),
                        12.0,
                        Weight::Normal,
                        colors.text_secondary_on_panel(),
                    );
                });
            let footer = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 32.0), inner.max);
            let mut row = ui.new_child(UiBuilder::new().max_rect(footer).layout(Layout::right_to_left(Align::Center)));
            row.spacing_mut().item_spacing.x = 8.0;
            let look = Look {
                min_width: 84.0,
                ..compact_look(colors)
            };
            if button(&mut row, colors, &look, crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Cancel)), true).clicked() {
                done = Some(false);
            }
            if button(&mut row, colors, &look, crate::widgets::Content::label(t(s::ApplicationSettingsWindow_Save)), true).clicked() {
                done = Some(true);
            }
        });
    if escape && done.is_none() {
        done = Some(false);
    }
    match done {
        Some(true) => {
            let value = editor.save();
            let index = editor.index;
            let mut list = formats(b);
            match index.filter(|i| *i < list.len()) {
                Some(i) => list[i] = value,
                None => {
                    list.push(value);
                    state.selected_format = Some(list.len() - 1);
                }
            }
            b.set(&["ClipboardContentFormats"], Value::Array(list));
            state.editor = None;
        }
        Some(false) => state.editor = None,
        None => {}
    }
}

/// The variables menu: "Upload result" with the result tokens, then the name codes by
/// category; and the token each position inserts.
fn token_menu() -> (Vec<Entry>, Vec<String>) {
    let mut tokens: Vec<String> = Vec::new();
    let mut result = Vec::new();
    for token in RESULT_TOKENS {
        result.push(Entry::plain(token, Command::Local(tokens.len())));
        tokens.push(token.to_string());
    }
    let mut entries = vec![Entry::submenu(
        t(s::ApplicationSettingsWindow_UploadResult),
        icons::LINK,
        true,
        result,
    )];
    let mut groups: Vec<(Option<modlight_core::l10n::Key>, Vec<Entry>)> = Vec::new();
    for code in modlight_core::names::CODE_MENU_ENTRIES.iter() {
        let token = code.to_prefix_string();
        let entry = Entry::plain(token.clone(), Command::Local(tokens.len()));
        tokens.push(token);
        match groups.iter_mut().find(|(c, _)| *c == code.category) {
            Some((_, list)) => list.push(entry),
            None => groups.push((code.category, vec![entry])),
        }
    }
    for (category, list) in groups {
        match category {
            Some(key) => entries.push(Entry::submenu(t(key), icons::HASH, true, list)),
            None => entries.extend(list),
        }
    }
    (entries, tokens)
}

// ------------------------------------------------------------------ what the asks do

impl crate::state::AppState {
    /// `TaskHelpers.GetScreenshotsFolder()` for the paths page.
    pub fn screenshots_folder_preview(&self) -> String {
        let settings = self.default_task_settings();
        self.uploads
            .ctx
            .screenshots_folder(&settings, None)
            .display()
            .to_string()
    }

    /// Read every part of the app that follows the settings document again: after an
    /// import, a reset, or a synced document from another installation.
    pub fn apply_settings_document(&mut self, ctx: Option<&egui::Context>) {
        self.options = self.config.theme_options();
        self.library_panel = self.config.library_panel();
        self.folders_section = self.config.folders_section();
        self.files_section = self.config.files_section();
        let language = self.config.language();
        self.set_language_setting(language);
        self.refresh_theme(ctx);
        self.sync_upload_settings();
        self.update_watch_folders();
        self.captures.hotkeys.unregister_all(false);
        self.start_hotkeys();
    }

    /// `SettingManager.LoadAllSettings` after an import, or after a reset, which loads
    /// without fallback support (the backups the saves kept hold what was just reset)
    /// and writes every file at once (`SaveAllSettings`), so the next start finds the
    /// defaults rather than falling back to those backups.
    fn reload_settings_files(&mut self, reset: bool) {
        self.config = modlight_core::config::AppConfig::load_with(&self.env.profile, !reset);
        // My space reads its page size only when it starts: the restored one (the default
        // after a reset) is taken before anything saves the one on screen over it.
        let page_size = self.config.page_size();
        self.apply_settings_document(None);
        self.space_set_page_size(page_size);
        if reset {
            self.save_hotkeys();
        } else {
            self.save_config();
        }
    }

    fn app_status(&mut self, text: String, busy: bool) {
        if let Some(window) = self.windows.settings.as_mut() {
            window.app_pages.status = text;
            window.app_pages.busy = busy;
        }
    }

    fn dialogs_allowed(&self) -> bool {
        !self.env.qa && self.mode == crate::app::Mode::Normal
    }

    pub fn app_page_ask(&mut self, ask: AppPageAsk) {
        match ask {
            AppPageAsk::Language(language) => {
                self.set_language_setting(language);
                self.save_config();
            }
            // The tray icon follows `ShowTray` on its own (on Windows from the next frame).
            AppPageAsk::ShowTray(_) => {}
            AppPageAsk::EditQuickTaskMenu => self.open_quick_task_menu_editor(),
            AppPageAsk::OpenPersonalFolder => {
                let folder = self.personal_folder_preview();
                if folder.is_dir() {
                    self.effects.push(crate::state::Effect::OpenFile(folder));
                }
            }
            #[cfg(windows)]
            AppPageAsk::BrowsePersonalFolder => {
                if !self.dialogs_allowed() {
                    return;
                }
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title(t(
                        s::ApplicationSettingsWindow_ChooseShareXPersonalFolderPath,
                    ))
                    .pick_folder()
                    && let Some(window) = self.windows.settings.as_mut()
                {
                    window.app_pages.personal_path = Some(folder.display().to_string());
                    window.app_pages.personal_path_dirty = true;
                }
            }
            #[cfg(windows)]
            AppPageAsk::ContextMenu(on) => self.set_explorer_menu(on, false),
            #[cfg(windows)]
            AppPageAsk::SendTo(on) => self.set_explorer_menu(on, true),
            AppPageAsk::OpenScreenshotsFolder => {
                let folder = std::path::PathBuf::from(self.screenshots_folder_preview());
                if folder.is_dir() {
                    self.effects.push(crate::state::Effect::OpenFile(folder));
                }
            }
            AppPageAsk::BrowseScreenshotsFolder => {
                if !self.dialogs_allowed() {
                    return;
                }
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title(t(s::ApplicationSettingsWindow_ChooseScreenshotsFolderPath))
                    .pick_folder()
                {
                    let value = modlight_core::names::file_helpers::sanitize_path(
                        &folder.display().to_string(),
                    );
                    self.config.set("CustomScreenshotsPath", json!(value));
                    self.sync_upload_settings();
                    self.save_config();
                }
            }
            AppPageAsk::Export => self.export_backup(),
            AppPageAsk::Import => self.import_backup(),
            AppPageAsk::ResetSettings => {
                let mut confirm = crate::dialogs::confirm::Confirm::new(
                    t(s::Confirmation),
                    t(s::ApplicationSettingsForm_btnResetSettings_Click_WouldYouLikeToResetShareXSettings),
                    crate::dialogs::confirm::ConfirmAction::ResetSettings,
                );
                confirm.title = format!("Modlight - {}", t(s::Confirmation));
                self.windows.confirm = Some(confirm);
            }
            // Printing is not part of this phase.
            AppPageAsk::ImagePrintSettings => self.open_print_settings(),
            AppPageAsk::StartAtLogin(on) => {
                if !self.dialogs_allowed() {
                    return;
                }
                match crate::platform::integration::set_startup(on) {
                    Ok(state) => {
                        if let Some(window) = self.windows.settings.as_mut() {
                            window.startup = Some(state);
                        }
                    }
                    Err(error) => {
                        log::warn!("Start at login: {error}");
                        self.app_status(error, false);
                        if let Some(window) = self.windows.settings.as_mut() {
                            window.startup = Some(crate::platform::integration::startup_state());
                        }
                    }
                }
                #[cfg(windows)]
                if let Some(window) = self.windows.settings.as_mut() {
                    window.startup_other_copy = crate::platform::integration::startup_other_copy()
                        .and_then(|exe| exe.parent().map(|f| f.display().to_string()));
                }
            }
            AppPageAsk::OpenLoginItems => {
                if self.dialogs_allowed() {
                    crate::platform::integration::open_login_items_settings();
                }
            }
            AppPageAsk::OpenServicesSettings => {
                if self.dialogs_allowed() {
                    let _ = crate::platform::integration::open_services_settings();
                }
            }
            AppPageAsk::DisableHotkeys(disabled) => self.captures.hotkeys.toggle(disabled),
            AppPageAsk::UpdatesOn => crate::update_window::on_startup(self),
        }
    }

    /// The folder Open shows on the Paths page (`PersonalFolderPreview`): on Windows the
    /// one the box names, on a Mac the fixed one.
    fn personal_folder_preview(&self) -> std::path::PathBuf {
        #[cfg(windows)]
        if let Some(text) = self
            .windows
            .settings
            .as_ref()
            .and_then(|w| w.app_pages.personal_path.as_deref())
            .filter(|text| !text.trim().is_empty())
        {
            return crate::personal_folder::absolute(text);
        }
        #[cfg(windows)]
        if !self.env.qa {
            return crate::personal_folder::default_folder();
        }
        self.env.profile.clone()
    }

    /// Windows: `CreateShellContextMenuButton` / `CreateSendToMenuButton`, then the box
    /// shows what Explorer has now.
    #[cfg(windows)]
    fn set_explorer_menu(&mut self, on: bool, send_to: bool) {
        if !self.dialogs_allowed() {
            return;
        }
        use crate::platform::integration as i;
        let result = if send_to {
            i::set_send_to(on)
        } else {
            i::set_context_menu(on)
        };
        if let Err(error) = result {
            log::warn!("Explorer integration: {error}");
            self.app_status(error, false);
        }
        if let Some(window) = self.windows.settings.as_mut() {
            window.explorer = Some(ExplorerMenus {
                context_menu: i::context_menu_enabled(),
                send_to: i::send_to_enabled(),
            });
        }
    }

    /// Windows, when the settings window closes (`FlushPersonalPath`): write an edited
    /// personal folder to `PersonalPath.cfg`. A file that cannot be written says so as
    /// upstream does.
    #[cfg(windows)]
    pub(crate) fn flush_personal_path(&mut self, pages: &AppPagesState) {
        if !pages.personal_path_dirty || !self.dialogs_allowed() {
            return;
        }
        let text = pages.personal_path.clone().unwrap_or_default();
        if let Err(error) = crate::personal_folder::write_config(&text) {
            log::warn!("PersonalPath.cfg: {error}");
            let file = crate::personal_folder::config_file();
            self.uploads
                .windows
                .messages
                .push_back(crate::dialogs::message::MessageBox::new(
                    "Modlight",
                    &format(
                        t(s::Program_WritePersonalPathConfig_Cant_access_to_file),
                        &[Arg::from(file.display().to_string())],
                    ),
                    modlight_core::tasks::host::MessageIcon::Warning,
                ));
        }
    }

    /// What `ExportAsync` runs first: `SaveAllSettings`, the hotkeys file with the rest
    /// (it is written only once the hotkeys change, so a profile may have none yet).
    fn save_for_export(&mut self) {
        self.save_hotkeys();
    }

    /// `OnExportClick` + `ExportAsync`.
    fn export_backup(&mut self) {
        if !self.dialogs_allowed() {
            return;
        }
        let (settings, history) = self
            .windows
            .settings
            .as_ref()
            .map(|w| (w.app_pages.export_settings, w.app_pages.export_history))
            .unwrap_or((true, true));
        let machine = modlight_core::names::file_helpers::sanitize_file_name(
            &self.env.machine.to_lowercase(),
        );
        let name = format!(
            "Modlight-{}-{machine}-backup.sxb",
            env!("CARGO_PKG_VERSION")
        );
        let Some(path) = rfd::FileDialog::new()
            .set_title(t(s::ApplicationSettingsWindow_ExportShareXBackup))
            .set_file_name(&name)
            .add_filter(t(s::ApplicationSettingsWindow_ShareXBackup), &["sxb"])
            .save_file()
        else {
            return;
        };
        self.save_for_export();
        self.app_status(
            t(s::ApplicationSettingsWindow_ExportingBackup).to_string(),
            true,
        );
        let profile = self.env.profile.clone();
        self.rt.spawn_blocking(move || {
            let result = modlight_core::backup::export(&profile, &path, settings, history);
            move |state: &mut crate::state::AppState, _ctx: &egui::Context| {
                let text = match result {
                    Ok(()) => format(
                        t(s::ApplicationSettingsWindow_BackupExportedTo),
                        &[Arg::from(path.display().to_string())],
                    ),
                    Err(error) => {
                        log::warn!("Backup export: {error}");
                        t(s::ApplicationSettingsWindow_BackupExportFailed).to_string()
                    }
                };
                state.app_status(text, false);
            }
        });
    }

    /// `OnImportClick` + `ImportAsync`.
    fn import_backup(&mut self) {
        if !self.dialogs_allowed() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title(t(s::ApplicationSettingsWindow_ImportShareXBackup))
            .add_filter(t(s::ApplicationSettingsWindow_ShareXBackup), &["sxb"])
            .pick_file()
        else {
            return;
        };
        if !path.is_file() {
            return;
        }
        // The import compares what it restores with the settings as they are now.
        self.save_config();
        self.app_status(
            t(s::ApplicationSettingsWindow_ImportingBackup).to_string(),
            true,
        );
        let profile = self.env.profile.clone();
        self.rt.spawn_blocking(move || {
            let result = modlight_core::backup::import(&path, &profile);
            move |state: &mut crate::state::AppState, _ctx: &egui::Context| match result {
                Ok(report) => {
                    state.reload_settings_files(false);
                    let mut text = format(
                        t(s::ApplicationSettingsWindow_BackupImportedFrom),
                        &[Arg::from(path.display().to_string())],
                    );
                    // What the backup held but did not get in is said out loud, in the
                    // status line and in a message the person has to close.
                    let notes = report.notes();
                    if !notes.is_empty() {
                        let body = notes.join("\n\n");
                        log::warn!("Backup import: {}", body.replace('\n', " "));
                        text.push_str("\n\n");
                        text.push_str(&body);
                        state.uploads.windows.messages.push_back(
                            crate::dialogs::message::MessageBox::new(
                                "Modlight",
                                &body,
                                modlight_core::tasks::host::MessageIcon::Warning,
                            ),
                        );
                    }
                    state.app_status(text, false);
                }
                Err(error) => {
                    log::warn!("Backup import: {error}");
                    state.app_status(
                        t(s::ApplicationSettingsWindow_BackupImportFailed).to_string(),
                        false,
                    );
                }
            }
        });
    }

    /// The reset question was answered Yes (`ResetAsync`).
    pub fn reset_settings_confirmed(&mut self) {
        match modlight_core::backup::reset(&self.env.profile) {
            Ok(()) => {
                self.reload_settings_files(true);
                self.app_status(
                    t(s::ApplicationSettingsWindow_SettingsReset).to_string(),
                    false,
                );
            }
            Err(error) => self.app_status(error.to_string(), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variables_menu_names_every_token_once() {
        let (entries, tokens) = token_menu();
        assert_eq!(
            tokens.len(),
            RESULT_TOKENS.len() + modlight_core::names::CODE_MENU_ENTRIES.len()
        );
        assert!(entries.len() > 2);
        assert_eq!(tokens[0], "$result");
        assert!(tokens.contains(&"%y".to_string()));
    }

    /// A state over a personal folder of its own, as the QA app runs (no Keychain, no
    /// system hotkeys).
    fn profile_state(profile: &std::path::Path) -> crate::state::AppState {
        use modlight_core::account::{AccountStore, FileCredentialStore};
        let env = crate::state::Env {
            profile: profile.to_path_buf(),
            qa: true,
            frozen_now: None,
            machine: "QA Mac".into(),
            system_language: "en".into(),
            system_locale: "en_US".into(),
            reduce_motion: true,
            system_dark: Some(true),
        };
        let account = AccountStore::load(
            profile,
            Box::new(FileCredentialStore::new(profile.join("credentials"))),
        );
        let config = modlight_core::config::AppConfig::load(profile);
        crate::state::AppState::new(env, config, account)
    }

    /// `SettingManager.ResetSettings` loads without fallback support: the settings that
    /// were just deleted must not come back from the copy the last save kept.
    #[test]
    fn a_reset_does_not_come_back_from_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path();
        std::fs::write(
            profile.join(modlight_core::config::APPLICATION_CONFIG_FILE),
            r#"{"ShowTray":false,"UploadLimit":3}"#,
        )
        .unwrap();
        // No hotkeys at all (every default one removed).
        std::fs::write(
            profile.join(modlight_core::config::HOTKEYS_CONFIG_FILE),
            r#"{"Hotkeys":[]}"#,
        )
        .unwrap();
        let mut state = profile_state(profile);
        state.start_hotkeys();
        assert!(!state.config.show_tray());
        assert!(state.captures.hotkeys.hotkeys.is_empty());
        // Two saves: the second keeps the first in Backup.
        state.save_hotkeys();
        state.save_hotkeys();
        state.reset_settings_confirmed();
        let defaults = modlight_core::hotkeys::default_hotkeys().len();
        assert!(state.config.show_tray(), "the default after a reset");
        assert_eq!(state.config.get("UploadLimit"), None);
        assert_eq!(state.captures.hotkeys.hotkeys.len(), defaults);
        // What the next start reads: the defaults, as upstream's SaveAllSettings left
        // them, not the backups of what was reset.
        let again = modlight_core::config::AppConfig::load(profile);
        assert!(again.show_tray());
        assert_eq!(again.get("UploadLimit"), None);
        assert_eq!(modlight_core::hotkeys::load(&again).len(), defaults);
    }

    /// After an import (`LoadAllSettings`) the restored file is what the app runs with
    /// and what stays on disk: the page size My space was showing must not be written
    /// over the one the backup brought, nor survive a reset.
    #[test]
    fn an_import_keeps_the_page_size_the_backup_brought() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path();
        let file = profile.join(modlight_core::config::APPLICATION_CONFIG_FILE);
        std::fs::write(&file, r#"{"ModlightPageSize":120}"#).unwrap();
        let mut state = profile_state(profile);
        state.space.page_size = 120;
        // What backup::import wrote.
        std::fs::write(&file, r#"{"ModlightPageSize":30}"#).unwrap();
        state.reload_settings_files(false);
        assert_eq!(state.space.page_size, 30);
        assert_eq!(
            modlight_core::config::AppConfig::load(profile).page_size(),
            30
        );
        // A reset brings back the default.
        modlight_core::backup::reset(profile).unwrap();
        state.reload_settings_files(true);
        assert_eq!(state.space.page_size, 120);
        assert_eq!(
            modlight_core::config::AppConfig::load(profile).page_size(),
            120
        );
    }

    /// `ExportAsync` saves every settings file first (`SaveAllSettings`), so a backup
    /// carries the hotkeys this installation runs with even when they were never changed
    /// (no `HotkeysConfig.json` yet): restored elsewhere, they replace that one's.
    #[test]
    fn an_export_carries_the_hotkeys_even_when_never_changed() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let mut state = profile_state(&profile);
        state.start_hotkeys();
        assert!(
            !profile
                .join(modlight_core::config::HOTKEYS_CONFIG_FILE)
                .exists()
        );
        state.save_for_export();
        let archive = dir.path().join("backup.sxb");
        modlight_core::backup::export(&profile, &archive, true, false).unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(
            elsewhere.join(modlight_core::config::HOTKEYS_CONFIG_FILE),
            r#"{"Hotkeys":[]}"#,
        )
        .unwrap();
        let report = modlight_core::backup::import(&archive, &elsewhere).unwrap();
        assert!(
            report
                .written
                .contains(&modlight_core::config::HOTKEYS_CONFIG_FILE.to_string()),
            "{report:?}"
        );
        let restored = modlight_core::config::AppConfig::load(&elsewhere);
        assert_eq!(
            modlight_core::hotkeys::load(&restored).len(),
            modlight_core::hotkeys::default_hotkeys().len()
        );
    }

    #[test]
    fn an_edited_format_keeps_unknown_members() {
        let original = json!({"Description": "A", "Format": "$url", "Extra": 1});
        let mut editor = FormatEditor::new(Some(0), Some(&original));
        editor.format = "$result".into();
        let saved = editor.save();
        assert_eq!(saved["Extra"], json!(1));
        assert_eq!(saved["Format"], json!("$result"));
    }
}

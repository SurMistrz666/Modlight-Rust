//! The settings window (`SettingsWindow`, `SettingsNavigation`, `AccountSettingsView`
//! and the theme and general pages of `ApplicationSettingsView`).
//!
//! Every page upstream has is in the navigation with its header; phase 1 fills the
//! account pages, "Theme & colours" and "Language & tray". The other pages open with
//! their header and description, and are filled in the phases that bring their
//! features (see `docs/STATUS.md`).

use egui::{Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_api::AccountInfo;
use modlight_core::format::cloud_size;
use modlight_core::icons;
use modlight_core::l10n::{Arg, Key, avalonia, format, s, t};
use modlight_core::theme::{SKINS, resolve_skin};

use crate::commands::Command;
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button, button_width, checkbox, combo, icon_galley, job, layout,
    paint_icon, trimmed, wrapped,
};

pub const ACCOUNT_SECTION: &str = "account.profile";
pub const DEFAULT_SECTION: &str = "account.profile";
pub const WIDTH: f32 = 1120.0;
pub const HEIGHT: f32 = 780.0;
pub const MIN_WIDTH: f32 = 950.0;
pub const MIN_HEIGHT: f32 = 700.0;

/// One page (`SettingsSection`).
pub struct Section {
    pub id: &'static str,
    pub title: Key,
    pub icon: char,
    pub hint: Key,
    pub aliases: &'static str,
}

/// A group of pages (`SettingsGroup`).
pub struct Group {
    pub title: Key,
    pub icon: char,
    pub sections: &'static [Section],
}

macro_rules! section {
    ($id:literal, $title:expr, $icon:expr, $hint:expr, $aliases:literal) => {
        Section {
            id: $id,
            title: $title,
            icon: $icon,
            hint: $hint,
            aliases: $aliases,
        }
    };
}

/// Texts upstream words for a PC that the Mac port words for a Mac
/// (`modlight_core::l10n::mac`, `MACOS.md` "Texts"): upstream's own on Windows.
#[cfg(windows)]
mod texts {
    use modlight_core::l10n::{Key, s};
    pub const HINT_FILE_NAMES: Key = s::SettingsWindow_HintFileNames;
    pub const HINT_WATCH_FOLDERS: Key = s::SettingsWindow_HintWatchFolders;
    pub const PAGE_INTEGRATION: Key = s::SettingsWindow_PageWindowsIntegration;
    pub const HINT_INTEGRATION: Key = s::SettingsWindow_HintWindowsIntegration;
    pub const PAGE_LOCAL_FOLDERS: Key = s::SettingsWindow_PageLocalFolders;
    pub const HINT_LOCAL_FOLDERS: Key = s::SettingsWindow_HintLocalFolders;
    pub const ACCOUNT_SIGNED_OUT_SYNC: Key = s::SettingsWindow_AccountSignedOutSync;
}
#[cfg(not(windows))]
mod texts {
    use modlight_core::l10n::{Key, mac};
    pub const HINT_FILE_NAMES: Key = mac::HintFileNames;
    pub const HINT_WATCH_FOLDERS: Key = mac::HintWatchFolders;
    pub const PAGE_INTEGRATION: Key = mac::PageMacIntegration;
    pub const HINT_INTEGRATION: Key = mac::HintMacIntegration;
    pub const PAGE_LOCAL_FOLDERS: Key = mac::PageLocalFolders;
    pub const HINT_LOCAL_FOLDERS: Key = mac::HintLocalFolders;
    pub const ACCOUNT_SIGNED_OUT_SYNC: Key = mac::AccountSignedOutSync;
}

/// The integration page's search words (`SearchAliases`): upstream's on Windows, the
/// Mac's elsewhere.
#[cfg(windows)]
const INTEGRATION_ALIASES: &str =
    "integration windows startup context menu send to steam chrome extension firefox addon";
#[cfg(not(windows))]
const INTEGRATION_ALIASES: &str = "integration login startup open at login finder services";

/// `SettingsSections.All()`, with upstream's search aliases.
pub static GROUPS: [Group; 11] = [
    Group {
        title: s::SettingsWindow_GroupAccount,
        icon: icons::CIRCLE_USER,
        sections: &[
            section!(
                "account.profile",
                s::SettingsWindow_AccountProfile,
                icons::CIRCLE_USER_ROUND,
                s::SettingsWindow_HintProfile,
                "account profile token api key sign in"
            ),
            section!(
                "account.links",
                s::SettingsWindow_AccountLinks,
                icons::LINK,
                s::SettingsWindow_HintLinks,
                "url shortener short links copy url"
            ),
            section!(
                "account.storage",
                s::SettingsWindow_AccountStorage,
                icons::HARD_DRIVE,
                s::SettingsWindow_HintStorage,
                "storage quota disk space"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupCapture,
        icon: icons::CAMERA,
        sections: &[
            section!(
                "task.capture-region",
                s::SettingsWindow_PageRegion,
                icons::CROP,
                s::SettingsWindow_HintRegion,
                "region capture rectangle capture crop"
            ),
            section!(
                "task.capture",
                s::SettingsWindow_PageScreenshots,
                icons::CAMERA,
                s::SettingsWindow_HintScreenshots,
                "capture screenshot screen capture window capture cursor"
            ),
            section!(
                "task.image",
                s::SettingsWindow_PageImageFormat,
                icons::IMAGE,
                s::SettingsWindow_HintImageFormat,
                "image format quality png jpeg jpg gif"
            ),
            section!(
                "task.image-effects",
                s::SettingsWindow_PageEffects,
                icons::WAND_SPARKLES,
                s::SettingsWindow_HintEffects,
                "effects image effects watermark"
            ),
            section!(
                "task.image-thumbnail",
                s::SettingsWindow_PageFileThumbnails,
                icons::IMAGES,
                s::SettingsWindow_HintFileThumbnails,
                "thumbnail thumbnails"
            ),
            section!(
                "task.upload-file-naming",
                s::SettingsWindow_PageFileNames,
                icons::FILE_PEN,
                texts::HINT_FILE_NAMES,
                "file naming name pattern name format filename"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupRecording,
        icon: icons::VIDEO,
        sections: &[section!(
            "task.capture-screen-recorder",
            s::SettingsWindow_GroupRecording,
            icons::VIDEO,
            s::SettingsWindow_HintRecording,
            "screen recorder recording ffmpeg gif"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupTextFromImage,
        icon: icons::SCAN_TEXT,
        sections: &[section!(
            "task.capture-ocr",
            s::SettingsWindow_GroupTextFromImage,
            icons::SCAN_TEXT,
            s::SettingsWindow_HintOcr,
            "ocr text capture"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupSharing,
        icon: icons::UPLOAD,
        sections: &[
            section!(
                "application.clipboard-formats",
                s::SettingsWindow_PageCopyFormats,
                icons::CLIPBOARD_LIST,
                s::SettingsWindow_HintCopyFormats,
                "clipboard formats copy formats"
            ),
            section!(
                "task.upload-clipboard",
                s::SettingsWindow_PageFromClipboard,
                icons::CLIPBOARD,
                s::SettingsWindow_HintFromClipboard,
                "clipboard upload clipboard"
            ),
            section!(
                "application.upload",
                s::SettingsWindow_PageUploadLimits,
                icons::UPLOAD,
                s::SettingsWindow_HintUploadLimits,
                "upload uploader retry buffer"
            ),
            section!(
                "application.history",
                s::SettingsWindow_PageHistory,
                icons::HISTORY,
                s::SettingsWindow_HintHistory,
                "history"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupShortcuts,
        icon: icons::KEYBOARD,
        sections: &[section!(
            "hotkeys.list",
            s::SettingsWindow_GroupShortcuts,
            icons::KEYBOARD,
            s::SettingsWindow_HintShortcuts,
            "hotkey hotkeys shortcut keys keyboard"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupSounds,
        icon: icons::BELL,
        sections: &[section!(
            "task.general-notifications",
            s::SettingsWindow_GroupSounds,
            icons::BELL,
            s::SettingsWindow_HintSounds,
            "sound sounds notification toast"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupTools,
        icon: icons::WRENCH,
        sections: &[
            section!(
                "task.tools",
                s::SettingsWindow_PageEditor,
                icons::WRENCH,
                s::SettingsWindow_HintEditor,
                "image editor tools"
            ),
            section!(
                "task.actions",
                s::SettingsWindow_PageExternalPrograms,
                icons::ZAP,
                s::SettingsWindow_HintExternalPrograms,
                "actions external programs"
            ),
            section!(
                "task.watch-folders",
                s::TaskSettingsWindow_WatchFolders,
                icons::FOLDER_SEARCH,
                texts::HINT_WATCH_FOLDERS,
                "watch folders watch folder"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupAppearance,
        icon: icons::PALETTE,
        sections: &[
            section!(
                "application.theme",
                s::SettingsWindow_PageThemeColours,
                icons::PALETTE,
                s::SettingsWindow_HintThemeColours,
                "theme skin accent color dark light"
            ),
            section!(
                "application.main-window",
                s::SettingsWindow_PageMainWindowTiles,
                icons::MONITOR,
                s::SettingsWindow_HintMainWindowTiles,
                "main window thumbnail view tiles"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupApplication,
        icon: icons::SETTINGS,
        sections: &[
            section!(
                "application.general",
                s::SettingsWindow_PageLanguageTray,
                icons::SETTINGS,
                s::SettingsWindow_HintLanguageTray,
                "general language tray tray icon"
            ),
            Section {
                id: "application.integration",
                title: texts::PAGE_INTEGRATION,
                icon: icons::PLUG,
                hint: texts::HINT_INTEGRATION,
                aliases: INTEGRATION_ALIASES,
            },
            section!(
                "application.components",
                s::SettingsWindow_PageComponents,
                icons::PACKAGE,
                s::SettingsWindow_HintComponents,
                "components ffmpeg yt-dlp ytdlp exiftool"
            ),
            section!(
                "application.paths",
                texts::PAGE_LOCAL_FOLDERS,
                icons::FOLDER,
                texts::HINT_LOCAL_FOLDERS,
                "paths personal folder screenshots folder sub folder pattern"
            ),
            section!(
                "application.settings",
                s::SettingsWindow_PageBackup,
                icons::DATABASE_BACKUP,
                s::SettingsWindow_HintBackup,
                "settings backup export import reset"
            ),
            section!(
                "application.print",
                s::ApplicationSettingsWindow_Print,
                icons::PRINTER,
                s::SettingsWindow_HintPrint,
                "print printer"
            ),
            section!(
                "application.proxy",
                s::SettingsWindow_PageProxy,
                icons::NETWORK,
                s::SettingsWindow_HintProxy,
                "proxy"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupAdvanced,
        icon: icons::SLIDERS_HORIZONTAL,
        sections: &[
            section!(
                "application.advanced",
                s::SettingsWindow_PageAllApplication,
                icons::SETTINGS,
                s::SettingsWindow_HintAllApplication,
                "advanced"
            ),
            section!(
                "task.advanced",
                s::SettingsWindow_AdvancedTasks,
                icons::ZAP,
                s::SettingsWindow_HintAllCapture,
                "advanced"
            ),
        ],
    },
];

/// `SettingsSections.ForTaskOverrides()`: one hotkey's settings window. The Task group
/// first (task, general, upload, Modlight folder), then the task sections of every group
/// of the full window, under the same group names.
pub static OVERRIDE_GROUPS: [Group; 8] = [
    Group {
        title: s::TaskSettingsWindow_Task,
        icon: icons::KEYBOARD,
        sections: &[
            section!(
                "task.task",
                s::TaskSettingsWindow_Task,
                icons::KEYBOARD,
                s::SettingsWindow_HintTask,
                "task job description after capture after upload destinations"
            ),
            section!(
                "task.general",
                s::TaskSettingsWindow_General,
                icons::SETTINGS,
                s::SettingsWindow_HintTaskGeneral,
                "general"
            ),
            section!(
                "task.upload",
                s::TaskSettingsWindow_Upload,
                icons::UPLOAD,
                s::SettingsWindow_HintTaskUpload,
                "upload"
            ),
            section!(
                "task.upload-modlight",
                s::TaskSettingsWindow_ModlightDestination,
                icons::FOLDER_UP,
                s::SettingsWindow_HintTaskFolder,
                "folder modlight folder destination"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupCapture,
        icon: icons::CAMERA,
        sections: &[
            section!(
                "task.capture-region",
                s::SettingsWindow_PageRegion,
                icons::CROP,
                s::SettingsWindow_HintRegion,
                "region capture rectangle capture crop"
            ),
            section!(
                "task.capture",
                s::SettingsWindow_PageScreenshots,
                icons::CAMERA,
                s::SettingsWindow_HintScreenshots,
                "capture screenshot screen capture window capture cursor"
            ),
            section!(
                "task.image",
                s::SettingsWindow_PageImageFormat,
                icons::IMAGE,
                s::SettingsWindow_HintImageFormat,
                "image format quality png jpeg jpg gif"
            ),
            section!(
                "task.image-effects",
                s::SettingsWindow_PageEffects,
                icons::WAND_SPARKLES,
                s::SettingsWindow_HintEffects,
                "effects image effects watermark"
            ),
            section!(
                "task.image-thumbnail",
                s::SettingsWindow_PageFileThumbnails,
                icons::IMAGES,
                s::SettingsWindow_HintFileThumbnails,
                "thumbnail thumbnails"
            ),
            section!(
                "task.upload-file-naming",
                s::SettingsWindow_PageFileNames,
                icons::FILE_PEN,
                texts::HINT_FILE_NAMES,
                "file naming name pattern name format filename"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupRecording,
        icon: icons::VIDEO,
        sections: &[section!(
            "task.capture-screen-recorder",
            s::SettingsWindow_GroupRecording,
            icons::VIDEO,
            s::SettingsWindow_HintRecording,
            "screen recorder recording ffmpeg gif"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupTextFromImage,
        icon: icons::SCAN_TEXT,
        sections: &[section!(
            "task.capture-ocr",
            s::SettingsWindow_GroupTextFromImage,
            icons::SCAN_TEXT,
            s::SettingsWindow_HintOcr,
            "ocr text capture"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupSharing,
        icon: icons::UPLOAD,
        sections: &[section!(
            "task.upload-clipboard",
            s::SettingsWindow_PageFromClipboard,
            icons::CLIPBOARD,
            s::SettingsWindow_HintFromClipboard,
            "clipboard upload clipboard"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupSounds,
        icon: icons::BELL,
        sections: &[section!(
            "task.general-notifications",
            s::SettingsWindow_GroupSounds,
            icons::BELL,
            s::SettingsWindow_HintSounds,
            "sound sounds notification toast"
        )],
    },
    Group {
        title: s::SettingsWindow_GroupTools,
        icon: icons::WRENCH,
        sections: &[
            section!(
                "task.tools",
                s::SettingsWindow_PageEditor,
                icons::WRENCH,
                s::SettingsWindow_HintEditor,
                "image editor tools"
            ),
            section!(
                "task.actions",
                s::SettingsWindow_PageExternalPrograms,
                icons::ZAP,
                s::SettingsWindow_HintExternalPrograms,
                "actions external programs"
            ),
            section!(
                "task.watch-folders",
                s::TaskSettingsWindow_WatchFolders,
                icons::FOLDER_SEARCH,
                texts::HINT_WATCH_FOLDERS,
                "watch folders watch folder"
            ),
        ],
    },
    Group {
        title: s::SettingsWindow_GroupAdvanced,
        icon: icons::SLIDERS_HORIZONTAL,
        sections: &[section!(
            "task.advanced",
            s::SettingsWindow_AdvancedTasks,
            icons::ZAP,
            s::SettingsWindow_HintAllCapture,
            "advanced"
        )],
    },
];

pub fn find_section(id: &str) -> Option<(&'static Group, &'static Section)> {
    GROUPS
        .iter()
        .chain(OVERRIDE_GROUPS.iter())
        .find_map(|g| g.sections.iter().find(|s| s.id == id).map(|s| (g, s)))
}

/// The account page's own reading of `/api/me` (`AccountSettingsViewModel`).
#[derive(Clone, Debug, Default)]
pub struct AccountPage {
    pub info: Option<AccountInfo>,
    pub host: String,
    pub status: String,
    pub busy: bool,
    pub generation: u64,
}

pub struct SettingsWindow {
    pub section: String,
    pub search: String,
    pub expanded: Vec<bool>,
    pub account: AccountPage,
    /// The skin when the window opened: pushed to the server on close if it changed.
    pub opened_skin: String,
    /// The task pages' selections and open editor.
    pub task_pages: crate::task_pages::TaskPagesState,
    /// The application pages' selections, editor and backup status.
    pub app_pages: crate::app_pages::AppPagesState,
    /// `StartupManager.State`, read when the integration page opens.
    pub startup: Option<crate::platform::integration::StartupState>,
    /// Windows: Modlight's entries in Explorer's menus, read when the page opens.
    pub explorer: Option<crate::app_pages::ExplorerMenus>,
    /// Windows: the folder of another copy Windows starts, read with `startup`.
    pub startup_other_copy: Option<String>,
    /// Where the page is drawn (the editors' veil covers it).
    pub page_area: Rect,
    /// The keyboard shortcuts page.
    pub hotkeys: crate::hotkey_page::HotkeyPageState,
    /// The sections this window lists: the full window or one hotkey's.
    pub groups: &'static [Group],
    /// The task settings the task pages edit.
    pub target: crate::task_pages::TaskTarget,
    /// The window title of a hotkey's settings (`ShareXTaskSettingsFor0`).
    pub title: String,
}

impl SettingsWindow {
    pub fn new(section: &str, skin: &str) -> SettingsWindow {
        let mut window = SettingsWindow {
            section: section.to_string(),
            search: String::new(),
            expanded: vec![false; GROUPS.len().max(OVERRIDE_GROUPS.len())],
            account: AccountPage::default(),
            opened_skin: skin.to_string(),
            task_pages: Default::default(),
            app_pages: Default::default(),
            startup: None,
            explorer: None,
            startup_other_copy: None,
            page_area: Rect::NOTHING,
            hotkeys: Default::default(),
            groups: &GROUPS,
            target: crate::task_pages::TaskTarget::Defaults,
            title: t(s::SettingsWindow_WindowTitle).to_string(),
        };
        window.reveal(section);
        window
    }

    /// The same window holding one hotkey's overrides (`SettingsWindow(TaskSettings)`).
    pub fn for_hotkey(index: usize, title: String, skin: &str) -> SettingsWindow {
        let mut window = SettingsWindow::new("task.task", skin);
        window.groups = &OVERRIDE_GROUPS;
        window.target = crate::task_pages::TaskTarget::Hotkey(index);
        window.title = title;
        window.expanded = vec![false; OVERRIDE_GROUPS.len()];
        window.reveal("task.task");
        window
    }

    /// Expand the group holding `section` so the selection is on screen.
    pub fn reveal(&mut self, section: &str) {
        for (index, group) in self.groups.iter().enumerate() {
            if group.sections.len() > 1 && group.sections.iter().any(|s| s.id == section) {
                self.expanded[index] = true;
            }
        }
    }

    fn matches(&self, group: &Group, section: &Section) -> bool {
        let query = self.search.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {} {}",
            t(section.title),
            t(section.hint),
            section.aliases,
            t(group.title)
        )
        .to_lowercase();
        query.split_whitespace().all(|word| haystack.contains(word))
    }
}

impl AppState {
    /// Open the settings window, or bring it to `section` (`SettingsWindowIntegration`).
    pub fn show_settings(&mut self, section: Option<&str>) {
        let section = section.unwrap_or(DEFAULT_SECTION);
        match self.windows.settings.as_mut() {
            Some(window) => {
                window.section = section.to_string();
                window.reveal(section);
            }
            None => {
                self.windows.settings = Some(SettingsWindow::new(section, &self.options.skin));
                self.refresh_account_page();
                // `HotkeySettingsAvaloniaService`: the keys that failed are tried again.
                self.captures.hotkeys.register_failed();
            }
        }
    }

    /// Re-read the account for the settings pages (`AccountSettingsViewModel.Refresh`).
    pub fn refresh_account_page(&mut self) {
        let credential = self.credential();
        let Some(window) = self.windows.settings.as_mut() else {
            return;
        };
        let Some(credential) = credential else {
            window.account = AccountPage::default();
            return;
        };
        window.account.generation += 1;
        window.account.busy = true;
        window.account.status = t(s::SettingsWindow_AccountLoading).to_string();
        window.account.host = credential.host.clone();
        let generation = window.account.generation;
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_account(&credential).await;
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                let Some(window) = state.windows.settings.as_mut() else {
                    return;
                };
                if window.account.generation != generation {
                    return;
                }
                window.account.busy = false;
                match answer {
                    Ok(Some(info)) => {
                        window.account.status.clear();
                        window.account.info = Some(info.clone());
                        state.note_account(&credential, &info);
                        state.request_avatar(ctx);
                    }
                    Ok(None) => {
                        window.account.status = t(s::ModlightSignIn_TokenRejected).to_string();
                        state.note_token_refused(&credential);
                    }
                    Err(_) => {
                        window.account.status = t(s::SettingsWindow_AccountUnreachable).to_string()
                    }
                }
            })
        });
    }

    /// The window closed: settings are saved and a changed skin goes to the server once
    /// (`ApplicationSettingsViewModel.Dispose`).
    pub fn close_settings(&mut self) {
        let Some(window) = self.windows.settings.take() else {
            return;
        };
        #[cfg(windows)]
        self.flush_personal_path(&window.app_pages);
        self.save_config();
        if !window.opened_skin.eq_ignore_ascii_case(&self.options.skin) {
            self.push_skin();
        }
    }
}

/// What the window asked for this frame.
pub enum SettingsAsk {
    Command(Command),
    Close,
    Refresh,
    OpenOnServer(&'static str),
    SignOut,
    /// What a task page asked of the app.
    TaskPage(crate::task_pages::TaskPageAsk),
    /// What an application page asked of the app.
    AppPage(crate::app_pages::AppPageAsk),
}

pub fn ui(ui: &mut Ui, state: &mut AppState) -> Vec<SettingsAsk> {
    let colors = state.colors.clone();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let nav_width = (full.width() * 0.26).clamp(220.0, 276.0);
    let nav = Rect::from_min_size(full.min, vec2(nav_width, full.height()));
    let mut asks = Vec::new();
    navigation(ui, state, &colors, nav);
    let right = Rect::from_min_max(pos2(nav.right(), full.top()), full.max);
    let Some(window) = state.windows.settings.as_ref() else {
        return asks;
    };
    let section_id = window.section.clone();
    let any_match = window
        .groups
        .iter()
        .any(|g| g.sections.iter().any(|sec| window.matches(g, sec)));
    if !any_match {
        no_results(ui, state, &colors, right);
        return asks;
    }
    let editor_open = window.task_pages.editor.is_some() || window.app_pages.editor.is_some();
    if let Some(window) = state.windows.settings.as_mut() {
        window.page_area = right;
    }
    let header_h = header(ui, &colors, right, &section_id);
    let body = Rect::from_min_max(pos2(right.left(), right.top() + header_h), right.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(body);
    egui::ScrollArea::vertical()
        .id_salt(("settings-page", section_id.as_str()))
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let width = body.width().min(820.0 + 56.0);
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.vertical(|ui| {
                    ui.set_width((width - 56.0).max(200.0));
                    page(ui, state, &colors, &section_id, &mut asks);
                });
            });
            ui.add_space(36.0);
        });
    if !editor_open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        asks.push(SettingsAsk::Close);
    }
    asks
}

fn navigation(ui: &mut Ui, state: &mut AppState, colors: &Colors, rect: Rect) {
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_toolbar());
    painter.vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, colors.border()),
    );
    let inner = rect.shrink(8.0);
    let Some(window) = state.windows.settings.as_mut() else {
        return;
    };

    // The search box: icon at 8, text padded 30 left, clear button when there is a query.
    let search = Rect::from_min_size(inner.min, vec2(inner.width(), 36.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(search));
    Field {
        height: 36.0,
        width: Some(search.width()),
        hint: t(avalonia::SettingsNavigation_Search_settings),
        padding_x: 30.0,
        id: Some(Id::new("settings-search")),
        ..Field::new(&mut window.search)
    }
    .show(&mut child, colors);
    paint_icon(
        ui,
        &painter,
        pos2(search.left() + 8.0 + 8.0, search.center().y),
        icons::SEARCH,
        16.0,
        colors.text_secondary(),
    );
    if !window.search.is_empty() {
        let clear = Rect::from_center_size(
            pos2(search.right() - 7.0 - 13.0, search.center().y),
            vec2(26.0, 26.0),
        );
        let response = ui.interact(clear, Id::new("settings-search-clear"), Sense::click());
        if response.hovered() {
            painter.circle_filled(clear.center(), 13.0, colors.control_background_hover());
        }
        paint_icon(
            ui,
            &painter,
            clear.center(),
            icons::X,
            15.0,
            colors.text_secondary(),
        );
        if response
            .on_hover_text(t(avalonia::SettingsNavigation_ClearSearch))
            .clicked()
        {
            window.search.clear();
        }
    }

    let list = Rect::from_min_max(pos2(inner.left(), search.bottom() + 10.0), inner.max);
    let mut list_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(list)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    list_ui.set_clip_rect(list);
    let searching = !window.search.trim().is_empty();
    let mut select = None;
    egui::ScrollArea::vertical()
        .id_salt("settings-nav")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            let width = list.width();
            for (index, group) in window.groups.iter().enumerate() {
                let visible: Vec<&Section> = group
                    .sections
                    .iter()
                    .filter(|sec| window.matches(group, sec))
                    .collect();
                if visible.is_empty() {
                    continue;
                }
                if group.sections.len() == 1 {
                    let section = group.sections[0].id;
                    let selected = window.section == section;
                    if nav_item(
                        ui,
                        colors,
                        width,
                        0.0,
                        Some(group.icon),
                        t(group.title),
                        selected,
                        None,
                    )
                    .clicked()
                    {
                        select = Some(section);
                    }
                    continue;
                }
                let expanded = window.expanded[index] || searching;
                let response = nav_item(
                    ui,
                    colors,
                    width,
                    0.0,
                    Some(group.icon),
                    t(group.title),
                    false,
                    Some(expanded),
                );
                if response.clicked() {
                    window.expanded[index] = !window.expanded[index];
                    if !window.expanded[index] {
                        continue;
                    }
                    select = Some(visible[0].id);
                }
                if expanded {
                    for section in visible {
                        let selected = window.section == section.id;
                        if nav_item(
                            ui,
                            colors,
                            width,
                            16.0,
                            Some(section.icon),
                            t(section.title),
                            selected,
                            None,
                        )
                        .clicked()
                        {
                            select = Some(section.id);
                        }
                    }
                }
            }
        });
    if let Some(section) = select {
        window.section = section.to_string();
        window.reveal(section);
    }
}

/// One line of the tree: 32 tall, 1 apart, the selected one on the accent.
#[allow(clippy::too_many_arguments)]
fn nav_item(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    indent: f32,
    glyph: Option<char>,
    title: &str,
    selected: bool,
    expander: Option<bool>,
) -> egui::Response {
    ui.add_space(1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    ui.add_space(1.0);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 4.0, colors.accent_start());
    } else if response.hovered() {
        painter.rect_filled(rect, 4.0, colors.control_background_hover());
    }
    let color = if selected {
        colors.accent_foreground()
    } else {
        colors.text_on_panel()
    };
    let mut x = rect.left() + 4.0 + indent;
    if let Some(open) = expander {
        let chevron = if open {
            icons::CHEVRON_DOWN
        } else {
            icons::CHEVRON_RIGHT
        };
        paint_icon(
            ui,
            painter,
            pos2(x + 8.0, rect.center().y),
            chevron,
            12.0,
            color,
        );
    }
    x += 16.0;
    if let Some(glyph) = glyph {
        paint_icon(
            ui,
            painter,
            pos2(x + 2.0 + 10.0, rect.center().y),
            glyph,
            16.0,
            color,
        );
        x += 2.0 + 20.0 + 6.0;
    }
    let text = layout(
        ui,
        trimmed(
            title,
            13.0,
            Weight::Normal,
            color,
            (rect.right() - 6.0 - x).max(10.0),
            1,
        ),
    );
    painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text, color);
    response
}

fn header(ui: &mut Ui, colors: &Colors, rect: Rect, section_id: &str) -> f32 {
    let Some((group, section)) = find_section(section_id) else {
        return 0.0;
    };
    let width = (rect.width() - 56.0).min(820.0);
    let left = rect.left() + 28.0;
    let group_title = t(group.title);
    let show_group = !group_title.is_empty() && group_title != t(section.title);
    let group_g = show_group.then(|| {
        layout(
            ui,
            job(group_title, 12.0, Weight::Normal, colors.text_secondary()),
        )
    });
    let icon = icon_galley(ui, section.icon, 22.0, colors.accent_start());
    let title = layout(
        ui,
        trimmed(
            t(section.title),
            24.0,
            Weight::SemiBold,
            colors.accent_start(),
            width - icon.size().x - 10.0,
            1,
        ),
    );
    let description = layout(
        ui,
        wrapped(
            t(section.hint),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            width.min(640.0),
        ),
    );
    let mut h = 18.0;
    if let Some(g) = &group_g {
        h += g.size().y + 5.0;
    }
    let title_row = title.size().y.max(icon.size().y);
    h += title_row + 5.0 + description.size().y + 16.0 + 1.0;
    let band = Rect::from_min_size(rect.min, vec2(rect.width(), h));
    let painter = ui.painter();
    painter.hline(
        band.x_range(),
        band.bottom() - 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let mut y = band.top() + 18.0;
    if let Some(g) = group_g {
        let gh = g.size().y;
        painter.galley(pos2(left, y), g, colors.text_secondary());
        y += gh + 5.0;
    }
    let icon_w = icon.size().x;
    painter.galley(
        pos2(left, y + (title_row - icon.size().y) / 2.0),
        icon,
        colors.accent_start(),
    );
    painter.galley(
        pos2(left + icon_w + 10.0, y + (title_row - title.size().y) / 2.0),
        title,
        colors.accent_start(),
    );
    y += title_row + 5.0;
    painter.galley(pos2(left, y), description, colors.text_secondary());
    h
}

fn no_results(ui: &mut Ui, state: &mut AppState, colors: &Colors, rect: Rect) {
    let Some(window) = state.windows.settings.as_mut() else {
        return;
    };
    let query = window.search.clone();
    let painter = ui.painter().clone();
    let title = layout(
        ui,
        wrapped(
            &format(
                t(s::SettingsWindow_NoResultsTitle),
                &[Arg::from(query.as_str())],
            ),
            17.0,
            Weight::SemiBold,
            colors.text_secondary(),
            440.0,
        ),
    );
    let hint = layout(
        ui,
        wrapped(
            t(s::SettingsWindow_NoResultsHint),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            440.0,
        ),
    );
    let icon = icon_galley(ui, icons::SEARCH_X, 38.0, colors.text_secondary());
    let h = icon.size().y + 12.0 + title.size().y + 12.0 + hint.size().y + 12.0 + 40.0;
    let mut y = rect.center().y - h / 2.0;
    let cx = rect.center().x;
    let icon_h = icon.size().y;
    painter.galley(
        pos2(cx - icon.size().x / 2.0, y),
        icon,
        colors.text_secondary(),
    );
    y += icon_h + 12.0;
    let title_h = title.size().y;
    painter.galley(
        pos2(cx - title.size().x / 2.0, y),
        title,
        colors.text_secondary(),
    );
    y += title_h + 12.0;
    let hint_h = hint.size().y;
    painter.galley(
        pos2(cx - hint.size().x / 2.0, y),
        hint,
        colors.text_secondary(),
    );
    y += hint_h + 12.0;
    let look = Look::button(colors);
    let label = t(s::SettingsWindow_ClearSearch);
    let w = button_width(ui, &look, Content::label(label));
    if crate::widgets::button_at(
        ui,
        colors,
        &look,
        Content::label(label),
        true,
        Rect::from_min_size(pos2(cx - w / 2.0, y), vec2(w, 40.0)),
    )
    .clicked()
    {
        window.search.clear();
    }
}

/// `Border.section-card`: popup fill, 8 px corners, 16,14 padding, 12 below.
fn section_card(ui: &mut Ui, colors: &Colors, add: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    egui::Frame::new()
        .fill(colors.background_popup())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(width - 34.0);
            add(ui);
        });
    ui.add_space(12.0);
}

fn section_title(ui: &mut Ui, colors: &Colors, text: &str) {
    ui.add_space(1.0);
    crate::widgets::text(ui, text, 15.0, Weight::SemiBold, colors.text());
    ui.add_space(3.0);
}

fn hint(ui: &mut Ui, colors: &Colors, text: &str) {
    crate::widgets::text_wrapped(ui, text, 12.0, Weight::Normal, colors.text_secondary());
}

/// `Button.compact` of the settings pages: 32 tall, 10 px padding.
fn compact(colors: &Colors) -> Look {
    Look {
        spacing: 6.0,
        ..Look::button(colors).height(32.0).padding(10.0)
    }
}

/// A label column of `label_w` and a value.
fn field_row(ui: &mut Ui, colors: &Colors, label_w: f32, label: &str, value: &str) {
    let width = ui.available_width();
    let label_g = layout(
        ui,
        wrapped(
            label,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            label_w,
        ),
    );
    let value_g = layout(
        ui,
        wrapped(
            value,
            13.0,
            Weight::Normal,
            colors.text(),
            (width - label_w - 8.0).max(40.0),
        ),
    );
    let h = label_g.size().y.max(value_g.size().y);
    let (r, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    ui.painter().galley(
        pos2(r.left(), r.center().y - label_g.size().y / 2.0),
        label_g,
        colors.text_secondary(),
    );
    ui.painter().galley(
        pos2(
            r.left() + label_w + 8.0,
            r.center().y - value_g.size().y / 2.0,
        ),
        value_g,
        colors.text(),
    );
}

fn page(
    ui: &mut Ui,
    state: &mut AppState,
    colors: &Colors,
    section: &str,
    asks: &mut Vec<SettingsAsk>,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    match section {
        "account.profile" => profile_page(ui, state, colors, asks),
        "account.links" => links_page(ui, state, colors, asks),
        "account.storage" => storage_page(ui, colors),
        "application.theme" => theme_page(ui, state, colors),
        "application.general"
        | "application.integration"
        | "application.paths"
        | "application.settings"
        | "application.main-window"
        | "application.clipboard-formats"
        | "application.upload"
        | "application.history"
        | "application.print"
        | "application.proxy"
        | "application.advanced" => app_page(ui, state, colors, section, asks),
        "application.components" => crate::components_window::settings_page(ui, state, colors),
        "hotkeys.list" => {
            let area = state
                .windows
                .settings
                .as_ref()
                .map(|w| w.page_area)
                .unwrap_or(Rect::NOTHING);
            crate::hotkey_page::page(ui, state, colors, area)
        }
        task if task.starts_with("task.") => task_page(ui, state, colors, task, asks),
        _ => {}
    }
}

/// An application page over the settings document.
fn app_page(
    ui: &mut Ui,
    state: &mut AppState,
    colors: &Colors,
    section: &str,
    asks: &mut Vec<SettingsAsk>,
) {
    use crate::app_pages::{AppPagesCtx, format_editor, page};
    use crate::settings_kit::Binder;
    let qa = state.env.qa || state.mode != crate::app::Mode::Normal;
    let screenshots_preview = state.screenshots_folder_preview();
    let personal_folder = state.env.profile.display().to_string();
    // Windows: an empty box means `Documents\Modlight`; QA shows its own folder there.
    #[cfg(windows)]
    let personal_default = if qa {
        personal_folder.clone()
    } else {
        crate::personal_folder::default_folder()
            .display()
            .to_string()
    };
    #[cfg(not(windows))]
    let personal_default = personal_folder.clone();
    let language = state.config.language();
    let Some(window) = state.windows.settings.as_mut() else {
        return;
    };
    if section == "application.integration" && window.startup.is_none() {
        window.startup = Some(if qa {
            crate::platform::integration::StartupState::NotAvailable
        } else {
            crate::platform::integration::startup_state()
        });
        // `StartupManager.OtherCopyStartupTargetPath`: the folder, which tells the copies
        // apart and fits on the check box.
        #[cfg(windows)]
        if !qa {
            window.startup_other_copy = crate::platform::integration::startup_other_copy()
                .and_then(|exe| exe.parent().map(|f| f.display().to_string()));
        }
    }
    #[cfg(windows)]
    if section == "application.integration" && window.explorer.is_none() {
        window.explorer = Some(if qa {
            crate::app_pages::ExplorerMenus::default()
        } else {
            crate::app_pages::ExplorerMenus {
                context_menu: crate::platform::integration::context_menu_enabled(),
                send_to: crate::platform::integration::send_to_enabled(),
            }
        });
    }
    // Windows: `ReadPersonalPathConfig` when the page first shows (QA reads no file).
    #[cfg(windows)]
    if section == "application.paths" && window.app_pages.personal_path.is_none() {
        window.app_pages.personal_path = Some(if qa {
            String::new()
        } else {
            crate::personal_folder::read_config()
        });
    }
    let explorer = window.explorer.unwrap_or_default();
    let startup = window
        .startup
        .unwrap_or(crate::platform::integration::StartupState::NotAvailable);
    let mut pages = std::mem::take(&mut window.app_pages);
    let area = window.page_area;
    let mut ctx = AppPagesCtx {
        colors,
        state: &mut pages,
        asks: Vec::new(),
        language,
        personal_folder,
        personal_default,
        screenshots_preview,
        startup,
        startup_other_copy: window.startup_other_copy.clone(),
        explorer,
        updates_visible: modlight_core::update::has_release_channel(),
        dialogs: !qa,
    };
    let mut root = std::mem::take(state.config.values_mut());
    let mut binder = Binder::new(&mut root);
    page(ui, &mut ctx, &mut binder, section);
    format_editor(ui, colors, ctx.state, &mut binder, area);
    let changed = binder.changed;
    let page_asks = std::mem::take(&mut ctx.asks);
    *state.config.values_mut() = root;
    if let Some(window) = state.windows.settings.as_mut() {
        window.app_pages = pages;
    }
    if changed {
        state.sync_upload_settings();
        state.save_config();
    }
    asks.extend(page_asks.into_iter().map(SettingsAsk::AppPage));
}

/// A task page over `DefaultTaskSettings` (every card enabled, no override cards).
fn task_page(
    ui: &mut Ui,
    state: &mut AppState,
    colors: &Colors,
    section: &str,
    asks: &mut Vec<SettingsAsk>,
) {
    use crate::settings_kit::Binder;
    use crate::task_pages::{TaskPagesCtx, TaskTarget, editors, page};
    let Some(window) = state.windows.settings.as_mut() else {
        return;
    };
    let mut pages = std::mem::take(&mut window.task_pages);
    let area = window.page_area;
    let target = window.target;
    let mut object = state.task_object(target);
    let mut auto_increment = state
        .config
        .get("NameParserAutoIncrementNumber")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let mut ctx = TaskPagesCtx {
        colors,
        is_default: target == TaskTarget::Defaults,
        state: &mut pages,
        asks: Vec::new(),
        auto_increment: &mut auto_increment,
        auto_increment_changed: false,
        dev_mode: state.config.dev_mode(),
        dialogs: !state.env.qa && state.mode == crate::app::Mode::Normal,
    };
    let mut binder = Binder::new(&mut object);
    page(ui, &mut ctx, &mut binder, section);
    editors(ui, &mut ctx, &mut binder, area);
    let changed = binder.changed;
    let page_asks = std::mem::take(&mut ctx.asks);
    let auto_changed = ctx.auto_increment_changed;
    if let Some(window) = state.windows.settings.as_mut() {
        window.task_pages = pages;
    }
    if changed {
        match target {
            TaskTarget::Defaults => {
                state
                    .config
                    .set("DefaultTaskSettings", serde_json::Value::Object(object));
            }
            TaskTarget::Hotkey(index) => {
                if let Some(hotkey) = state.captures.hotkeys.hotkeys.get_mut(index) {
                    hotkey.task = serde_json::Value::Object(object);
                }
                state.save_hotkeys();
            }
        }
    }
    if auto_changed {
        state.config.set(
            "NameParserAutoIncrementNumber",
            serde_json::json!(auto_increment),
        );
    }
    if (changed && target == TaskTarget::Defaults) || auto_changed {
        state.save_config();
    }
    asks.extend(page_asks.into_iter().map(SettingsAsk::TaskPage));
}

fn signed_out_card(ui: &mut Ui, colors: &Colors, asks: &mut Vec<SettingsAsk>, tall: bool) {
    section_card(ui, colors, |ui| {
        if tall {
            ui.set_min_height(380.0 - 28.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(440.0);
                ui.add_space(((380.0 - 28.0 - 300.0) / 2.0_f32).max(0.0));
                crate::widgets::icon(ui, icons::CIRCLE_USER_ROUND, 44.0, colors.accent_start());
                ui.add_space(10.0);
                section_title(ui, colors, t(s::SettingsWindow_AccountSignedOut));
                ui.add_space(10.0);
                let g = layout(ui, {
                    let mut j = wrapped(
                        t(s::SettingsWindow_AccountSignedOutHint),
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        440.0,
                    );
                    j.halign = egui::Align::Center;
                    j
                });
                let (r, _) = ui.allocate_exact_size(vec2(440.0, g.size().y), Sense::hover());
                ui.painter()
                    .galley(pos2(r.center().x, r.top()), g, colors.text_secondary());
                ui.add_space(16.0);
                for (glyph, key) in [
                    (icons::UPLOAD, s::SettingsWindow_AccountSignedOutUploads),
                    (icons::IMAGES, s::SettingsWindow_AccountSignedOutLibrary),
                    (icons::PALETTE, texts::ACCOUNT_SIGNED_OUT_SYNC),
                ] {
                    let text = layout(
                        ui,
                        wrapped(
                            t(key),
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                            440.0 - 24.0,
                        ),
                    );
                    let (r, _) = ui
                        .allocate_exact_size(vec2(440.0, text.size().y.max(16.0)), Sense::hover());
                    paint_icon(
                        ui,
                        ui.painter(),
                        pos2(r.left() + 7.5, r.top() + 8.0),
                        glyph,
                        15.0,
                        colors.text_secondary(),
                    );
                    ui.painter().galley(
                        pos2(r.left() + 24.0, r.top()),
                        text,
                        colors.text_secondary(),
                    );
                    ui.add_space(7.0);
                }
                ui.add_space(4.0);
                if button(
                    ui,
                    colors,
                    &compact(colors),
                    Content::label(t(s::MainMenuBuilder_SignIn)),
                    true,
                )
                .clicked()
                {
                    asks.push(SettingsAsk::Command(Command::ShowSignIn));
                }
            });
        } else {
            section_title(ui, colors, t(s::SettingsWindow_AccountSignedOut));
            ui.add_space(8.0);
            hint(ui, colors, t(s::SettingsWindow_AccountSignedOutHint));
            ui.add_space(8.0);
            if button(
                ui,
                colors,
                &compact(colors),
                Content::label(t(s::MainMenuBuilder_SignIn)),
                true,
            )
            .clicked()
            {
                asks.push(SettingsAsk::Command(Command::ShowSignIn));
            }
        }
    });
}

fn profile_page(ui: &mut Ui, state: &mut AppState, colors: &Colors, asks: &mut Vec<SettingsAsk>) {
    if !state.account.is_signed_in() {
        signed_out_card(ui, colors, asks, true);
        return;
    }
    let Some(window) = state.windows.settings.as_ref() else {
        return;
    };
    let page = window.account.clone();
    let stored = state.account.account().clone();
    let name = page
        .info
        .as_ref()
        .map(modlight_api::display_name)
        .unwrap_or_else(|| state.account.display_name());
    let email = page
        .info
        .as_ref()
        .and_then(|i| i.email.clone())
        .unwrap_or(stored.email.clone());
    let host = page
        .info
        .as_ref()
        .and_then(|i| i.host.clone())
        .unwrap_or(stored.host.clone());
    let texture = state.media.avatar.texture().cloned();
    section_card(ui, colors, |ui| {
        ui.horizontal(|ui| {
            let (circle, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
            crate::signin::paint_avatar(ui, colors, circle, texture.as_ref(), &name, 34.0, 24.0);
            ui.painter()
                .circle_stroke(circle.center(), 32.0, Stroke::new(1.0, colors.border()));
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                crate::widgets::text(ui, &name, 18.0, Weight::SemiBold, colors.text_secondary());
                ui.add_space(3.0);
                crate::widgets::text(ui, &email, 13.0, Weight::Normal, colors.text());
                ui.add_space(3.0);
                crate::widgets::text(ui, &host, 13.0, Weight::Normal, colors.text());
            });
        });
        ui.add_space(10.0);
        hint(ui, colors, t(s::SettingsWindow_AccountAvatarHint));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if button(
                ui,
                colors,
                &compact(colors),
                Content::both(
                    icons::EXTERNAL_LINK,
                    t(s::SettingsWindow_AccountOpenProfile),
                ),
                true,
            )
            .clicked()
            {
                asks.push(SettingsAsk::OpenOnServer("/dashboard/profile"));
            }
            ui.add_space(8.0);
            if button(
                ui,
                colors,
                &compact(colors),
                Content::label(t(s::SettingsWindow_AccountRefresh)),
                true,
            )
            .clicked()
            {
                asks.push(SettingsAsk::Refresh);
            }
        });
        if !page.status.is_empty() {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if page.busy {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    crate::widgets::paint_progress(
                        ui,
                        colors,
                        Rect::from_center_size(r.center(), vec2(16.0, 3.0)),
                        None,
                    );
                    ui.add_space(8.0);
                }
                hint(ui, colors, &page.status);
            });
        }
    });
    // Storage.
    let (quota_text, percent) = quota(page.info.as_ref());
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::SettingsWindow_AccountStorageUsed));
        ui.add_space(6.0);
        crate::widgets::text_wrapped(ui, &quota_text, 13.0, Weight::Normal, colors.text());
        if let Some(percent) = percent {
            ui.add_space(6.0);
            let w = ui.available_width();
            crate::widgets::progress(ui, colors, w, 8.0, Some(percent / 100.0));
        }
    });
    // Security.
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::SettingsWindow_AccountSecurity));
        ui.add_space(6.0);
        hint(ui, colors, t(s::SettingsWindow_AccountPasswordHint));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if button(
                ui,
                colors,
                &compact(colors),
                Content::both(icons::KEY_ROUND, t(s::SettingsWindow_AccountChangePassword)),
                true,
            )
            .clicked()
            {
                asks.push(SettingsAsk::OpenOnServer("/dashboard/profile"));
            }
            ui.add_space(8.0);
            if button(
                ui,
                colors,
                &compact(colors),
                Content::both(icons::LOG_OUT, t(s::SettingsWindow_AccountSignOut)),
                true,
            )
            .clicked()
            {
                asks.push(SettingsAsk::SignOut);
            }
        });
    });
}

/// `ShowQuota`: the sentence, and the percentage when there is a limit.
fn quota(info: Option<&AccountInfo>) -> (String, Option<f32>) {
    let Some(quota) = info.and_then(|i| i.quota.as_ref()) else {
        return (String::new(), None);
    };
    match quota.max.filter(|max| !quota.unlimited && *max > 0) {
        None => (
            format(
                t(s::SettingsWindow_AccountQuotaUnlimited),
                &[Arg::from(cloud_size(quota.used))],
            ),
            None,
        ),
        Some(max) => (
            format(
                t(s::SettingsWindow_AccountQuotaUsed),
                &[
                    Arg::from(cloud_size(quota.used)),
                    Arg::from(cloud_size(max)),
                ],
            ),
            Some((quota.used as f32 * 100.0 / max as f32).clamp(0.0, 100.0)),
        ),
    }
}

fn links_page(ui: &mut Ui, state: &mut AppState, colors: &Colors, asks: &mut Vec<SettingsAsk>) {
    if !state.account.is_signed_in() {
        signed_out_card(ui, colors, asks, false);
    } else {
        let Some(window) = state.windows.settings.as_ref() else {
            return;
        };
        let info = window.account.info.clone();
        let stored = state.account.account().clone();
        let host = info
            .as_ref()
            .and_then(|i| i.host.clone())
            .unwrap_or(stored.host.clone());
        let tag = info
            .as_ref()
            .and_then(|i| i.tag.clone())
            .unwrap_or(stored.tag.clone());
        let preview = if host.is_empty() {
            String::new()
        } else if tag.is_empty() {
            format!("{}/…", host.trim_end_matches('/'))
        } else {
            format!("{}/{}/…", host.trim_end_matches('/'), tag)
        };
        section_card(ui, colors, |ui| {
            section_title(ui, colors, t(s::SettingsWindow_AccountLinkShape));
            ui.add_space(8.0);
            field_row(ui, colors, 190.0, t(s::SettingsWindow_AccountDomain), &host);
            ui.add_space(6.0);
            field_row(ui, colors, 190.0, t(s::SettingsWindow_AccountTag), &tag);
            ui.add_space(6.0);
            field_row(
                ui,
                colors,
                190.0,
                t(s::SettingsWindow_AccountLinkPreview),
                &preview,
            );
            ui.add_space(8.0);
            hint(ui, colors, t(s::SettingsWindow_AccountDomainHint));
        });
        let privacy = info
            .as_ref()
            .map(|i| {
                if i.hide_uploads {
                    t(s::SettingsWindow_AccountPrivateYes)
                } else {
                    t(s::SettingsWindow_AccountPrivateNo)
                }
            })
            .unwrap_or("");
        let copied = info
            .as_ref()
            .map(|i| {
                if i.copy_raw {
                    t(s::SettingsWindow_AccountCopyRaw)
                } else {
                    t(s::SettingsWindow_AccountCopyPage)
                }
            })
            .unwrap_or("");
        section_card(ui, colors, |ui| {
            section_title(ui, colors, t(s::SettingsWindow_AccountPrivacy));
            ui.add_space(8.0);
            field_row(
                ui,
                colors,
                190.0,
                t(s::SettingsWindow_AccountPrivateByDefault),
                privacy,
            );
            ui.add_space(6.0);
            field_row(
                ui,
                colors,
                190.0,
                t(s::SettingsWindow_AccountCopiedLink),
                copied,
            );
            ui.add_space(8.0);
            hint(ui, colors, t(s::SettingsWindow_AccountPrivacyHint));
        });
    }
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::SettingsWindow_AccountNaming));
        ui.add_space(8.0);
        hint(ui, colors, t(s::SettingsWindow_AccountNamingHint));
    });
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::SettingsWindow_AccountFilePassword));
        ui.add_space(8.0);
        hint(ui, colors, t(s::SettingsWindow_AccountFilePasswordHint));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            crate::widgets::icon(ui, icons::TRIANGLE_ALERT, 16.0, colors.accent_start());
            ui.add_space(8.0);
            ui.vertical(|ui| hint(ui, colors, t(s::SettingsWindow_AccountFilePasswordWarning)));
        });
    });
    if button(
        ui,
        colors,
        &compact(colors),
        Content::both(
            icons::EXTERNAL_LINK,
            t(s::SettingsWindow_AccountOpenProfile),
        ),
        true,
    )
    .clicked()
    {
        asks.push(SettingsAsk::OpenOnServer("/dashboard/profile"));
    }
}

fn storage_page(ui: &mut Ui, colors: &Colors) {
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::SettingsWindow_AccountDefaultFolder));
        ui.add_space(8.0);
        hint(ui, colors, t(s::TaskSettingsWindow_ModlightFolderHint));
    });
}

fn label_row(ui: &mut Ui, colors: &Colors, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let g = layout(
            ui,
            wrapped(label, 13.0, Weight::Normal, colors.text_secondary(), 190.0),
        );
        let (r, _) = ui.allocate_exact_size(vec2(190.0, 32.0), Sense::hover());
        ui.painter().galley(
            pos2(r.left(), r.center().y - g.size().y / 2.0),
            g,
            colors.text_secondary(),
        );
        ui.add_space(8.0);
        add(ui);
    });
}

fn theme_page(ui: &mut Ui, state: &mut AppState, colors: &Colors) {
    let mut changed = false;
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::ApplicationSettingsWindow_ApplicationTheme));
        ui.add_space(6.0);
        let mut follow = state.options.use_system_theme;
        if checkbox(
            ui,
            colors,
            &mut follow,
            Some(t(s::ApplicationSettingsWindow_FollowSystemTheme)),
            true,
        )
        .changed()
        {
            state.options.use_system_theme = follow;
            changed = true;
        }
        if !state.options.use_system_theme {
            ui.add_space(6.0);
            let current = if state.options.theme.eq_ignore_ascii_case("light") {
                "Light"
            } else {
                "Dark"
            }
            .to_string();
            let options = vec![
                (
                    "Dark".to_string(),
                    t(s::ApplicationSettingsWindow_Dark).to_string(),
                ),
                (
                    "Light".to_string(),
                    t(s::ApplicationSettingsWindow_Light).to_string(),
                ),
            ];
            label_row(
                ui,
                colors,
                t(s::ApplicationSettingsWindow_ThemeLabel),
                |ui| {
                    if let Some(theme) = combo(ui, colors, "theme", 130.0, &current, &options) {
                        state.options.theme = theme;
                        changed = true;
                    }
                },
            );
        }
    });
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::ApplicationSettingsWindow_Skin));
        ui.add_space(6.0);
        let current = resolve_skin(Some(&state.options.skin)).id.to_string();
        let options: Vec<(String, String)> = SKINS
            .iter()
            .map(|skin| (skin.id.to_string(), skin.name(state.language).to_string()))
            .collect();
        label_row(ui, colors, t(s::ApplicationSettingsWindow_Skin), |ui| {
            if let Some(skin) = combo(ui, colors, "skin", 200.0, &current, &options) {
                state.options.set_skin(&skin);
                changed = true;
            }
        });
        ui.add_space(6.0);
        hint(ui, colors, t(s::ApplicationSettingsWindow_SkinNote));
    });
    section_card(ui, colors, |ui| {
        section_title(ui, colors, t(s::ApplicationSettingsWindow_AccentColor));
        ui.add_space(6.0);
        let mut follow = state.options.use_system_accent_color;
        if checkbox(
            ui,
            colors,
            &mut follow,
            Some(t(s::ApplicationSettingsWindow_FollowSystemAccentColor)),
            true,
        )
        .changed()
        {
            state.options.use_system_accent_color = follow;
            changed = true;
        }
        if !state.options.use_system_accent_color {
            ui.add_space(6.0);
            label_row(
                ui,
                colors,
                t(s::ApplicationSettingsWindow_AccentColorLabel),
                |ui| {
                    let accent = crate::theme::rgba(state.options.accent_color());
                    let (r, _) = ui.allocate_exact_size(vec2(130.0, 32.0), Sense::hover());
                    ui.painter()
                        .rect_filled(r, 6.0, colors.control_background());
                    ui.painter().rect_stroke(
                        r,
                        6.0,
                        Stroke::new(1.0, colors.border()),
                        StrokeKind::Inside,
                    );
                    let swatch = Rect::from_min_size(r.min + vec2(6.0, 6.0), vec2(20.0, 20.0));
                    ui.painter().rect_filled(swatch, 4.0, accent);
                    let hex = state.options.accent_color_hex.clone();
                    let g = layout(ui, job(&hex, 13.0, Weight::Normal, colors.text()));
                    ui.painter().galley(
                        pos2(swatch.right() + 8.0, r.center().y - g.size().y / 2.0),
                        g,
                        colors.text(),
                    );
                },
            );
        }
    });
    if changed {
        state.refresh_theme(Some(ui.ctx()));
        state.save_config();
    }
}

/// Carry out what the window asked for.
pub fn handle(state: &mut AppState, asks: Vec<SettingsAsk>) {
    for ask in asks {
        match ask {
            SettingsAsk::Command(command) => state.push(command),
            SettingsAsk::Close => state.close_settings(),
            SettingsAsk::Refresh => state.refresh_account_page(),
            SettingsAsk::OpenOnServer(path) => {
                let host = state.account.host().to_string();
                if state.credential().is_none() || host.is_empty() {
                    state.show_sign_in();
                } else {
                    let url = format!("{}{}", host.trim_end_matches('/'), path);
                    state.open_url(&url);
                }
            }
            SettingsAsk::SignOut => {
                state.push(Command::SignOut);
                if let Some(window) = state.windows.settings.as_mut() {
                    window.account = AccountPage::default();
                }
            }
            SettingsAsk::AppPage(ask) => state.app_page_ask(ask),
            SettingsAsk::TaskPage(ask) => {
                let target = state
                    .windows
                    .settings
                    .as_ref()
                    .map(|w| w.target)
                    .unwrap_or(crate::task_pages::TaskTarget::Defaults);
                task_page_ask(state, target, ask)
            }
        }
    }
}

/// What a task page asked of the app, for the settings `target` it edits.
pub fn task_page_ask(
    state: &mut AppState,
    target: crate::task_pages::TaskTarget,
    ask: crate::task_pages::TaskPageAsk,
) {
    use crate::task_pages::TaskPageAsk as A;
    match ask {
        A::SelectRegion => state.select_custom_region(target),
        A::ScreenRecordingOptions => state.open_ffmpeg_options(target),
        A::WatchFoldersChanged => state.update_watch_folders(),
        A::ImageEffects => state.open_image_effects_presets(target, None),
        A::LoadModlightFolders => state.load_modlight_folders(),
        // Handled by the pages themselves.
        A::NotificationButtons
        | A::PickSound(_)
        | A::EditAction(_)
        | A::EditWatchFolder(_)
        | A::PickScreenshotsFolder => {}
    }
}

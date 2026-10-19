//! The menu bar icon and its menu (`TrayIconService`, `MainMenuBuilder.BuildTrayMenu`).
//!
//! Only in the real app: QA runs never create it. The menu is built from the same
//! [`Entry`] lists the window's menus use, in upstream's order, with what exists so far:
//! the account, Upload, the after-capture and after-upload tasks, Settings, the
//! screenshots folder, recent items, Show Modlight, Exit. Capture, Tools and the hotkey
//! switch join it with phase 3.

use std::sync::mpsc::{Receiver, channel};

use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use tray_icon::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::commands::Command;
use crate::menus::{Entry, Toggle};
use crate::state::AppState;

const EXIT: &str = "exit";
/// Ids of entries that bring the app forward (they open a window) start with this;
/// the rest ("q") act quietly, without showing hidden windows.
const SHOWS: &str = "w";
const QUIET: &str = "q";

pub struct Tray {
    icon: TrayIcon,
    /// What the menu was built from; a change rebuilds it.
    built_for: Vec<Entry>,
    /// The command behind each id, by index.
    commands: Vec<Command>,
    events: Receiver<String>,
}

/// `BuildTrayMenu`, as entries.
pub fn tray_entries(state: &AppState) -> Vec<Entry> {
    let uploads = !state.app_upload_settings().disable_upload;
    let defaults = state.default_task_settings();
    let mut entries = Vec::new();
    if state.account.is_signed_in() {
        entries.push(Entry::submenu(
            format(
                t(s::MainMenuBuilder_SignedInAs),
                &[Arg::from(state.account_label())],
            ),
            icons::CIRCLE_USER,
            true,
            vec![
                Entry::item(
                    t(s::MainMenuBuilder_AccountSettings),
                    icons::CIRCLE_USER,
                    Command::ShowSettings(Some(crate::settings::ACCOUNT_SECTION)),
                ),
                Entry::item(
                    t(s::MainMenuBuilder_ModlightAccount),
                    icons::CIRCLE_USER,
                    Command::ShowSignIn,
                ),
                Entry::Separator,
                Entry::item(
                    t(s::MainMenuBuilder_SignOut),
                    icons::LOG_OUT,
                    Command::SignOut,
                ),
            ],
        ));
    } else {
        entries.push(Entry::item(
            t(s::MainMenuBuilder_SignIn),
            icons::LOG_IN,
            Command::ShowSignIn,
        ));
    }
    entries.push(Entry::Separator);
    entries.push(Entry::submenu(
        t(s::MainMenuBuilder_Upload),
        icons::UPLOAD,
        uploads,
        crate::uploads::upload_menu(),
    ));
    entries.push(Entry::Separator);
    entries.push(Entry::submenu(
        t(s::MainMenuBuilder_AfterCaptureTasks),
        icons::IMAGE_UP,
        true,
        crate::uploads::after_capture_menu(defaults.after_capture_job),
    ));
    entries.push(Entry::submenu(
        t(s::MainMenuBuilder_AfterUploadTasks),
        icons::CLOUD_UPLOAD,
        uploads,
        crate::uploads::after_upload_menu(defaults.after_upload_job),
    ));
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        t(s::SettingsWindow_WindowTitle),
        icons::SETTINGS,
        Command::ShowSettings(None),
    ));
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        t(s::MainMenuBuilder_ScreenshotsFolder),
        icons::FOLDER_OPEN,
        Command::OpenScreenshotsFolder,
    ));
    entries.push(Entry::Separator);
    let settings = state.app_upload_settings();
    let recent = &state.uploads.tasks.recent.tasks;
    if settings.recent_tasks_save && settings.recent_tasks_show_in_tray_menu && !recent.is_empty() {
        let mut tasks: Vec<_> = recent.iter().collect();
        if settings.recent_tasks_tray_menu_most_recent_first {
            tasks.reverse();
        }
        let children = tasks
            .into_iter()
            .map(|task| {
                let open = if !task.shortened_url.is_empty() {
                    Command::OpenUrl(task.shortened_url.clone())
                } else if !task.url.is_empty() {
                    Command::OpenUrl(task.url.clone())
                } else {
                    Command::OpenFile(task.file_path.clone().into())
                };
                Entry::submenu(
                    task.tray_menu_text(),
                    icons::LINK,
                    true,
                    vec![
                        Entry::item(
                            t(s::MainMenuBuilder_Copy),
                            icons::COPY,
                            Command::CopyText(task.text().to_string()),
                        ),
                        Entry::item(t(s::MainMenuBuilder_Open), icons::EXTERNAL_LINK, open),
                    ],
                )
            })
            .collect();
        entries.push(Entry::submenu(
            t(s::MainMenuBuilder_RecentItems),
            icons::CLIPBOARD_LIST,
            true,
            children,
        ));
    }
    entries.push(Entry::item(
        t(s::MainMenuBuilder_ShowShareX),
        icons::MAXIMIZE,
        Command::ShowMainWindow,
    ));
    entries.push(Entry::item(
        t(s::MainMenuBuilder_Exit),
        icons::LOG_OUT,
        Command::Quit,
    ));
    entries
}

/// Whether choosing this brings the app's windows forward.
fn shows_windows(command: &Command) -> bool {
    !matches!(
        command,
        Command::SignOut
            | Command::ToggleAfterUploadTask(_)
            | Command::ToggleAfterCaptureTask(_)
            | Command::CopyText(_)
            | Command::OpenUrl(_)
            | Command::OpenFile(_)
            | Command::OpenScreenshotsFolder
    )
}

fn append(
    target: &dyn Fn(&dyn IsMenuItem) -> Result<(), tray_icon::menu::Error>,
    entries: &[Entry],
    commands: &mut Vec<Command>,
) -> Result<(), tray_icon::menu::Error> {
    for entry in entries {
        match entry {
            Entry::Separator => target(&PredefinedMenuItem::separator())?,
            Entry::Item {
                label,
                enabled,
                toggle,
                command,
                ..
            } => {
                let id = match command {
                    Some(Command::Quit) => EXIT.to_string(),
                    Some(command) => {
                        commands.push(command.clone());
                        let kind = if shows_windows(command) { SHOWS } else { QUIET };
                        format!("{kind}{}", commands.len() - 1)
                    }
                    None => String::new(),
                };
                match toggle {
                    Toggle::Check(checked) | Toggle::Radio(checked) => {
                        target(&CheckMenuItem::with_id(id, label, *enabled, *checked, None))?
                    }
                    Toggle::None => target(&MenuItem::with_id(id, label, *enabled, None))?,
                }
            }
            Entry::Submenu {
                label,
                enabled,
                children,
                ..
            } => {
                let submenu = Submenu::new(label, *enabled);
                append(&|item| submenu.append(item), children, commands)?;
                target(&submenu)?;
            }
        }
    }
    Ok(())
}

fn build_menu(entries: &[Entry]) -> (Menu, Vec<Command>) {
    let menu = Menu::new();
    let mut commands = Vec::new();
    if let Err(error) = append(&|item| menu.append(item), entries, &mut commands) {
        log::warn!("Modlight: the tray menu could not be built: {error}");
    }
    (menu, commands)
}

impl Tray {
    pub fn new(ctx: &egui::Context, state: &AppState) -> Option<Tray> {
        let icon = crate::app::app_icon().and_then(|image| {
            let small =
                image::imageops::resize(&image, 36, 36, image::imageops::FilterType::Lanczos3);
            Icon::from_rgba(small.into_raw(), 36, 36).ok()
        })?;
        let (sender, events) = channel();
        let wake = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id().0.clone();
            if id == EXIT {
                // The window may be hidden and not drawing: leave from here. The settings
                // are saved as they change, so there is nothing left to write.
                std::process::exit(0);
            }
            if id.starts_with(SHOWS) {
                modlight_macos::system::show_app_windows();
            }
            let _ = sender.send(id);
            wake.request_repaint();
        }));
        let entries = tray_entries(state);
        let (menu, commands) = build_menu(&entries);
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(modlight_core::APP_NAME)
            .with_menu(Box::new(menu))
            .build()
            .map_err(|error| log::warn!("Modlight: the tray icon could not be created: {error}"))
            .ok()?;
        Some(Tray {
            icon: tray,
            built_for: entries,
            commands,
            events,
        })
    }

    /// Rebuild the menu when anything it shows changed.
    pub fn update(&mut self, state: &AppState) {
        let entries = tray_entries(state);
        if entries != self.built_for {
            let (menu, commands) = build_menu(&entries);
            self.icon.set_menu(Some(Box::new(menu)));
            self.commands = commands;
            self.built_for = entries;
        }
    }

    /// What the menu asked for since the last frame.
    pub fn commands(&self) -> Vec<Command> {
        self.events
            .try_iter()
            .filter_map(|id| {
                let index: usize = id.get(1..)?.parse().ok()?;
                self.commands.get(index).cloned()
            })
            .collect()
    }
}

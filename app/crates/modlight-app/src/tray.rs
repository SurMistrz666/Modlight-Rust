//! The tray icon and its menu (`TrayIconService`, `MainMenuBuilder.BuildTrayMenu`): the
//! notification area icon on Windows, the menu bar extra on a Mac.
//!
//! Only in the real app: QA runs never create it. The menu is built from the same
//! [`Entry`] lists the window's menus use, in upstream's order, with what exists so far:
//! the account, Upload, the after-capture and after-upload tasks, Settings, the
//! screenshots folder, recent items, Show Modlight, Exit. Capture, Tools and the hotkey
//! switch join it with phase 3.
//!
//! On Windows the icon answers the mouse as upstream's `WinFormsTrayIconService`: the
//! right button opens the menu, the left button runs "On tray icon left click", a double
//! click "On tray icon double left click" (the single click waits the system's
//! double-click time when a double-click action is set), the middle button "On tray icon
//! middle click". A Mac's menu bar extra opens its menu on any click.

use std::sync::mpsc::{Receiver, channel};
#[cfg(windows)]
use std::time::{Duration, Instant};

use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use tray_icon::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::commands::Command;
use crate::menus::{Entry, Toggle};
use crate::state::AppState;
#[cfg(windows)]
use modlight_core::hotkeys::HotkeyType;

const EXIT: &str = "exit";
/// Ids of entries that bring the app forward (they open a window) start with this;
/// the rest ("q") act quietly, without showing hidden windows.
const SHOWS: &str = "w";
const QUIET: &str = "q";

pub struct Tray {
    icon: TrayIcon,
    /// The app's own icon, for when no upload is running.
    app_icon: Option<Icon>,
    /// The percentage the icon shows, -1 for the app icon (`lastIconStatus`).
    shown_progress: i32,
    /// What the menu was built from; a change rebuilds it.
    built_for: Vec<Entry>,
    /// The command behind each id, by index.
    commands: Vec<Command>,
    events: Receiver<String>,
    /// Windows: the icon's own clicks.
    #[cfg(windows)]
    clicks: Clicks,
    /// Windows: Exit was chosen; the window closes the app on its next pass.
    #[cfg(windows)]
    exit: bool,
}

/// Windows: a button of the mouse let go over the icon, and when.
#[cfg(windows)]
enum Click {
    Left(Instant),
    Middle,
}

/// Windows: the icon's clicks, and the single click that may still become a double one
/// (`_leftClickCount` and `_singleClickTimer`).
#[cfg(windows)]
struct Clicks {
    events: Receiver<Click>,
    first_left: Option<Instant>,
    double_click: Duration,
    wake: egui::Context,
}

/// Windows: the job a tray action setting names (`ApplicationConfig.Tray...Action`).
#[cfg(windows)]
fn tray_action(state: &AppState, member: &str, default: HotkeyType) -> HotkeyType {
    state
        .config
        .get(member)
        .and_then(|v| v.as_str())
        .and_then(HotkeyType::parse)
        .unwrap_or(default)
}

#[cfg(windows)]
impl Clicks {
    fn new(wake: &egui::Context) -> Clicks {
        let (sender, events) = channel();
        let repaint = wake.clone();
        tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
            use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
            let click = match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => Click::Left(Instant::now()),
                TrayIconEvent::Click {
                    button: MouseButton::Middle,
                    button_state: MouseButtonState::Up,
                    ..
                } => Click::Middle,
                _ => return,
            };
            let _ = sender.send(click);
            repaint.request_repaint();
        }));
        Clicks {
            events,
            first_left: None,
            double_click: crate::win32::double_click_time(),
            wake: wake.clone(),
        }
    }

    /// `OnMouseUp` and `OnSingleClickTimerTick`: the jobs the clicks since the last call
    /// asked for.
    fn jobs(&mut self, state: &AppState) -> Vec<Command> {
        let left = tray_action(state, "TrayLeftClickAction", HotkeyType::RectangleRegion);
        let double = tray_action(
            state,
            "TrayLeftDoubleClickAction",
            HotkeyType::OpenMainWindow,
        );
        let middle = tray_action(
            state,
            "TrayMiddleClickAction",
            HotkeyType::ClipboardUploadWithContentViewer,
        );
        let mut jobs = Vec::new();
        for click in self.events.try_iter() {
            match click {
                Click::Middle => jobs.push(middle),
                Click::Left(_) if double == HotkeyType::None => jobs.push(left),
                Click::Left(at) => match self.first_left.take() {
                    Some(first) if at.duration_since(first) < self.double_click => {
                        jobs.push(double)
                    }
                    Some(_) => {
                        // The first click's wait ran out unseen: it was a single click.
                        jobs.push(left);
                        self.first_left = Some(at);
                    }
                    None => self.first_left = Some(at),
                },
            }
        }
        if let Some(first) = self.first_left {
            let waited = first.elapsed();
            if waited >= self.double_click {
                self.first_left = None;
                jobs.push(left);
            } else {
                self.wake.request_repaint_after(self.double_click - waited);
            }
        }
        jobs.into_iter()
            .filter(|job| *job != HotkeyType::None)
            .map(Command::RunJob)
            .collect()
    }
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
        t(s::MainMenuBuilder_Capture),
        icons::CAMERA,
        true,
        crate::capture_flow::capture_menu(state, true),
    ));
    entries.push(Entry::submenu(
        t(s::MainMenuBuilder_Upload),
        icons::UPLOAD,
        uploads,
        crate::uploads::upload_menu(),
    ));
    entries.push(Entry::submenu(
        t(s::MainMenuBuilder_Tools),
        icons::WRENCH,
        true,
        crate::capture_flow::tools_menu(),
    ));
    entries.push(Entry::Separator);
    entries.push(Entry::submenu(
        t(s::MainMenuBuilder_AfterCaptureTasks),
        icons::IMAGE_UP,
        true,
        crate::tools::image_effects::after_capture_menu_with_presets(
            state,
            defaults.after_capture_job,
        ),
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
    let disabled = state.captures.hotkeys.disabled;
    entries.push(Entry::item(
        if disabled {
            t(s::MainMenuBuilder_EnableHotkeys)
        } else {
            t(s::MainMenuBuilder_DisableHotkeys)
        },
        if disabled {
            icons::KEYBOARD
        } else {
            icons::KEYBOARD_OFF
        },
        Command::ToggleHotkeys,
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
            | Command::SelectImageEffectPreset(_)
            | Command::CopyText(_)
            | Command::OpenUrl(_)
            | Command::OpenFile(_)
            | Command::OpenScreenshotsFolder
            | Command::Capture(..)
            | Command::StartRecording(..)
            | Command::ToggleHotkeys
            | Command::ToggleShowCursor
            | Command::SetScreenshotDelay(_)
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
        let icon = app_tray_icon()?;
        let (sender, events) = channel();
        let wake = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id().0.clone();
            // A Mac's hidden window may never be redrawn to receive a close: leave from
            // here; the settings are saved as they change. Windows draws a hidden window
            // on request, so there the window closes the app properly and the icon
            // leaves the notification area with it.
            #[cfg(not(windows))]
            if id == EXIT {
                std::process::exit(0);
            }
            if id.starts_with(SHOWS) {
                crate::platform::system::show_app_windows();
            }
            let _ = sender.send(id);
            wake.request_repaint();
        }));
        let entries = tray_entries(state);
        let (menu, commands) = build_menu(&entries);
        let builder = TrayIconBuilder::new()
            .with_icon(icon.clone())
            .with_tooltip(modlight_core::APP_NAME)
            .with_menu(Box::new(menu));
        // Windows: the left button runs its action instead of opening the menu.
        #[cfg(windows)]
        let builder = builder.with_menu_on_left_click(false);
        let tray = builder
            .build()
            .map_err(|error| log::warn!("Modlight: the tray icon could not be created: {error}"))
            .ok()?;
        Some(Tray {
            icon: tray,
            app_icon: Some(icon),
            shown_progress: -1,
            built_for: entries,
            commands,
            events,
            #[cfg(windows)]
            clicks: Clicks::new(ctx),
            #[cfg(windows)]
            exit: false,
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

    /// `UpdateTrayIcon`: the average progress drawn as the icon while uploads run, the
    /// app icon otherwise (-1).
    pub fn show_progress(&mut self, progress: i32) {
        if progress == self.shown_progress {
            return;
        }
        let icon = if progress >= 0 {
            progress_icon(progress)
        } else {
            self.app_icon.clone()
        };
        if let Some(icon) = icon
            && let Err(error) = self.icon.set_icon(Some(icon))
        {
            log::warn!("Modlight: the tray icon could not be changed: {error}");
        }
        self.shown_progress = progress;
    }

    /// What the menu (and on Windows the icon's clicks) asked for since the last frame.
    pub fn commands(&mut self, state: &AppState) -> Vec<Command> {
        let ids: Vec<String> = self.events.try_iter().collect();
        #[cfg(windows)]
        if ids.iter().any(|id| id == EXIT) {
            self.exit = true;
        }
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut commands: Vec<Command> = ids
            .iter()
            .filter_map(|id| {
                let index: usize = id.get(1..)?.parse().ok()?;
                self.commands.get(index).cloned()
            })
            .collect();
        #[cfg(windows)]
        commands.extend(self.clicks.jobs(state));
        #[cfg(not(windows))]
        let _ = state;
        commands
    }

    /// Windows: whether Exit was chosen since the last call.
    #[cfg(windows)]
    pub fn take_exit(&mut self) -> bool {
        std::mem::take(&mut self.exit)
    }
}

/// The icon's side in pixels: the menu bar's at 2x on a Mac; on Windows twice the
/// notification area's 16, which the system scales to its own size.
#[cfg(not(windows))]
const ICON_SIZE: u32 = 36;
#[cfg(windows)]
const ICON_SIZE: u32 = 32;

fn app_tray_icon() -> Option<Icon> {
    crate::app::app_icon().and_then(|image| {
        let small = image::imageops::resize(
            &image,
            ICON_SIZE,
            ICON_SIZE,
            image::imageops::FilterType::Lanczos3,
        );
        Icon::from_rgba(small.into_raw(), ICON_SIZE, ICON_SIZE).ok()
    })
}

/// The digits of the progress icon, 5 by 7 (Arial 10 has no counterpart to draw with
/// here; the shapes are plain enough to read at menu bar size).
const DIGITS: [[u8; 7]; 10] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
];

/// `Helpers.GetProgressIcon`: a dark square filled from the bottom in (16, 116, 193)
/// up to the percentage, a lighter line on the fill's edge, and the number (0 to 99)
/// in white in the middle. Drawn at the icon's size, digits at twice upstream's pixels.
pub fn progress_image(percentage: i32) -> image::RgbaImage {
    const SIZE: u32 = ICON_SIZE;
    const SCALE: u32 = 2;
    let percentage = percentage.clamp(0, 100);
    let fill = image::Rgba([16, 116, 193, 255]);
    // `ColorHelpers.LighterColor`: a float lerp towards white, truncated.
    let lerp = |c: u8| (c as f32 + (255.0 - c as f32) * 0.3) as u8;
    let line = image::Rgba([lerp(16), lerp(116), lerp(193), 255]);
    let mut image = image::RgbaImage::from_pixel(SIZE, SIZE, image::Rgba([39, 39, 39, 255]));
    let y = (SIZE as f32 * (percentage as f32 / 100.0)) as u32;
    if y > 0 {
        for py in SIZE - y..SIZE {
            for px in 0..SIZE {
                image.put_pixel(px, py, fill);
            }
        }
        if y < SIZE {
            for px in 0..SIZE {
                image.put_pixel(px, SIZE - y, line);
            }
        }
    }
    let text = percentage.clamp(0, 99).to_string();
    let glyph_w = 5 * SCALE;
    let gap = SCALE;
    let width = text.len() as u32 * glyph_w + (text.len() as u32 - 1) * gap;
    let left = (SIZE - width) / 2;
    let top = (SIZE - 7 * SCALE) / 2;
    for (i, digit) in text.bytes().enumerate() {
        let rows = DIGITS[(digit - b'0') as usize];
        let x0 = left + i as u32 * (glyph_w + gap);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (0x10 >> col) != 0 {
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            image.put_pixel(
                                x0 + col * SCALE + dx,
                                top + row as u32 * SCALE + dy,
                                image::Rgba([255, 255, 255, 255]),
                            );
                        }
                    }
                }
            }
        }
    }
    // `bmp.SetPixel(0, 0, Color.Transparent)`.
    image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
    image
}

fn progress_icon(percentage: i32) -> Option<Icon> {
    let image = progress_image(percentage);
    let (w, h) = image.dimensions();
    Icon::from_rgba(image.into_raw(), w, h).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_progress_icon_fills_from_the_bottom() {
        let size = super::ICON_SIZE;
        let image = super::progress_image(50);
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "the corner pixel is cleared");
        assert_eq!(image.get_pixel(1, size - 1).0, [16, 116, 193, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [39, 39, 39, 255]);
        assert_eq!(
            image.get_pixel(1, size / 2).0,
            [87, 157, 211, 255],
            "the edge line"
        );
    }
}

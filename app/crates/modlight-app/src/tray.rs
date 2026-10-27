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
/// How many rebuilt menus are still answered: the one open when a click comes may have
/// been replaced a few times since it opened.
const KEPT_MENUS: usize = 32;

/// The commands behind the menus' ids. An id names the menu it belongs to and its
/// place there ("w12.40"), so a click on a menu replaced since it opened (the window
/// list or the recent items changed) still runs the entry the person saw.
#[derive(Default)]
struct MenuCommands {
    generation: u64,
    menus: std::collections::VecDeque<(u64, Vec<Command>)>,
}

impl MenuCommands {
    /// A new menu is being built.
    fn begin(&mut self) {
        self.generation += 1;
        self.menus.push_back((self.generation, Vec::new()));
        while self.menus.len() > KEPT_MENUS {
            self.menus.pop_front();
        }
    }

    /// The id of an entry of the menu being built.
    fn id_for(&mut self, command: &Command) -> String {
        if self.menus.is_empty() {
            self.begin();
        }
        let (generation, list) = self.menus.back_mut().expect("a menu being built");
        list.push(command.clone());
        let kind = if shows_windows(command) { SHOWS } else { QUIET };
        format!("{kind}{generation}.{}", list.len() - 1)
    }

    /// The command an id stands for, if its menu is still known.
    fn command(&self, id: &str) -> Option<Command> {
        let (generation, index) = id.get(1..)?.split_once('.')?;
        let generation: u64 = generation.parse().ok()?;
        let index: usize = index.parse().ok()?;
        self.menus
            .iter()
            .find(|(g, _)| *g == generation)
            .and_then(|(_, list)| list.get(index))
            .cloned()
    }
}

pub struct Tray {
    icon: TrayIcon,
    /// The app's own icon, for when no upload is running.
    app_icon: Option<Icon>,
    /// The percentage the icon shows, -1 for the app icon (`lastIconStatus`).
    shown_progress: i32,
    /// What the menu was built from; a change rebuilds it.
    built_for: Vec<Entry>,
    /// The command behind each id of this menu and the ones it replaced.
    commands: MenuCommands,
    events: Receiver<String>,
    /// Windows: the icon's own clicks.
    #[cfg(windows)]
    clicks: Clicks,
    /// Windows: Exit was chosen; the window closes the app on its next pass.
    #[cfg(windows)]
    exit: bool,
    /// Windows: an entry that shows the app's windows was chosen.
    #[cfg(windows)]
    showed: bool,
    /// Windows: whether the icon is in the notification area (`ShowTray`).
    #[cfg(windows)]
    visible: bool,
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
    commands: &mut MenuCommands,
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
                    Some(command) => commands.id_for(command),
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

fn build_menu(entries: &[Entry], commands: &mut MenuCommands) -> Menu {
    let menu = Menu::new();
    commands.begin();
    if let Err(error) = append(&|item| menu.append(item), entries, commands) {
        log::warn!("Modlight: the tray menu could not be built: {error}");
    }
    menu
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
        let mut commands = MenuCommands::default();
        let menu = build_menu(&entries, &mut commands);
        let builder = TrayIconBuilder::new()
            .with_icon(icon.clone())
            .with_icon_as_template(TEMPLATE)
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
            #[cfg(windows)]
            showed: false,
            #[cfg(windows)]
            visible: true,
        })
    }

    /// Rebuild the menu when anything it shows changed.
    pub fn update(&mut self, state: &AppState) {
        let entries = tray_entries(state);
        if entries != self.built_for {
            let menu = build_menu(&entries, &mut self.commands);
            self.icon.set_menu(Some(Box::new(menu)));
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
            && let Err(error) = self.icon.set_icon_with_as_template(Some(icon), TEMPLATE)
        {
            log::warn!("Modlight: the tray icon could not be changed: {error}");
        }
        self.shown_progress = progress;
    }

    /// What the menu (and on Windows the icon's clicks) asked for since the last frame.
    pub fn commands(&mut self, state: &AppState) -> Vec<Command> {
        let ids: Vec<String> = self.events.try_iter().collect();
        #[cfg(windows)]
        {
            self.exit |= ids.iter().any(|id| id == EXIT);
            self.showed |= ids.iter().any(|id| id.starts_with(SHOWS));
        }
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut commands: Vec<Command> = ids
            .iter()
            .filter_map(|id| self.commands.command(id))
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

    /// Windows: `ShowTray` switched: the icon leaves or comes back to the notification
    /// area (`TrayIconService.Visible`).
    #[cfg(windows)]
    pub fn set_visible(&mut self, visible: bool) {
        if visible != self.visible {
            if let Err(error) = self.icon.set_visible(visible) {
                log::warn!("Modlight: the tray icon could not be shown or hidden: {error}");
            }
            self.visible = visible;
        }
    }

    /// Windows: whether an entry that shows the app's windows was chosen since the last
    /// call. The menu's handler showed them through Win32, behind winit's back.
    #[cfg(windows)]
    pub fn take_showed(&mut self) -> bool {
        std::mem::take(&mut self.showed)
    }
}

/// The icon's side in pixels: the menu bar's at 2x on a Mac; on Windows twice the
/// notification area's 16, which the system scales to its own size.
#[cfg(not(windows))]
const ICON_SIZE: u32 = 36;
#[cfg(windows)]
const ICON_SIZE: u32 = 32;

/// A Mac draws the menu bar extra as a template image: only its alpha counts, and the
/// system colours it for a light or dark menu bar, highlighted or not. Windows shows
/// the colours as they are.
const TEMPLATE: bool = cfg!(not(windows));

fn app_tray_icon() -> Option<Icon> {
    let image = if TEMPLATE {
        tray_glyph(ICON_SIZE)
    } else {
        // The hand-drawn 32 px frame of the app icon: the big artwork scaled down
        // smudges the corners and the arrow.
        image::load_from_memory_with_format(
            include_bytes!("../../../assets/icons/Modlight-32.png"),
            image::ImageFormat::Png,
        )
        .ok()?
        .to_rgba8()
    };
    let (w, h) = image.dimensions();
    Icon::from_rgba(image.into_raw(), w, h).ok()
}

/// Where the glyph's strokes run, in pixels of a `size` square: the four capture
/// corners and the upload arrow of the app icon, with stroke widths rounded to whole
/// pixels and straight strokes on the pixel grid, so the menu bar shows them sharp.
struct GlyphGeometry {
    /// The outer edges of the corners' box.
    lo: f32,
    hi: f32,
    /// The corners' stroke and the arrow's.
    stroke: f32,
    arrow: f32,
    /// How far each corner's arms reach along the box, from its outer edge.
    arm: f32,
}

impl GlyphGeometry {
    fn new(size: u32) -> GlyphGeometry {
        let s = size as f32;
        let stroke = (s * 0.085).round().max(1.0);
        let inset = (s * 0.06).round().max(1.0);
        // The shaft is centred on the square: an even width keeps it on whole pixels
        // when the side is even, an odd one when it is odd.
        let mut arrow = stroke;
        if (size as f32 - arrow) % 2.0 != 0.0 {
            arrow += 1.0;
        }
        let (lo, hi) = (inset, s - inset);
        GlyphGeometry {
            lo,
            hi,
            stroke,
            arrow,
            arm: ((hi - lo) * 0.32).round(),
        }
    }

    /// The corners as segments (round-capped, `stroke` wide).
    fn corners(&self) -> Vec<[f32; 4]> {
        let half = self.stroke / 2.0;
        let (a, b) = (self.lo + half, self.hi - half);
        let reach = self.arm - self.stroke;
        let mut out = Vec::new();
        for (x, dx) in [(a, 1.0), (b, -1.0)] {
            for (y, dy) in [(a, 1.0), (b, -1.0)] {
                out.push([x, y, x + dx * reach, y]);
                out.push([x, y, x, y + dy * reach]);
            }
        }
        out
    }

    /// The arrow as segments (round-capped, `arrow` wide): the shaft and the chevron.
    fn arrow(&self) -> Vec<[f32; 4]> {
        let box_side = self.hi - self.lo;
        let half = self.arrow / 2.0;
        let x = (self.lo + self.hi) / 2.0;
        let tip = self.lo + box_side * 0.2 + half;
        let bottom = self.hi - box_side * 0.17 - half;
        let spread = box_side * 0.25 - half;
        vec![
            [x, tip, x, bottom],
            [x, tip, x - spread, tip + spread],
            [x, tip, x + spread, tip + spread],
        ]
    }
}

/// How much of the pixel (`px`, `py`) the round-capped segments `width` wide cover,
/// sampled 4 by 4.
fn coverage(segments: &[[f32; 4]], width: f32, px: u32, py: u32) -> f32 {
    let r2 = (width / 2.0) * (width / 2.0);
    let mut hits = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let x = px as f32 + (sx as f32 + 0.5) / 4.0;
            let y = py as f32 + (sy as f32 + 0.5) / 4.0;
            let inside = segments.iter().any(|&[x0, y0, x1, y1]| {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len2 = dx * dx + dy * dy;
                let t = if len2 > 0.0 {
                    (((x - x0) * dx + (y - y0) * dy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (cx, cy) = (x0 + t * dx - x, y0 + t * dy - y);
                cx * cx + cy * cy <= r2 + 1e-4
            });
            hits += inside as u32;
        }
    }
    hits as f32 / 16.0
}

fn stroke_into(image: &mut image::RgbaImage, segments: &[[f32; 4]], width: f32) {
    let (w, h) = image.dimensions();
    for py in 0..h {
        for px in 0..w {
            let c = coverage(segments, width, px, py);
            if c > 0.0 {
                let p = image.get_pixel_mut(px, py);
                let a = (p.0[3] as f32 / 255.0).max(c);
                *p = image::Rgba([0, 0, 0, (a * 255.0).round() as u8]);
            }
        }
    }
}

/// The menu bar glyph: the app icon's capture corners and upload arrow, black on
/// transparent (a template image), `size` pixels square.
pub fn tray_glyph(size: u32) -> image::RgbaImage {
    let g = GlyphGeometry::new(size);
    let mut image = image::RgbaImage::new(size, size);
    stroke_into(&mut image, &g.corners(), g.stroke);
    stroke_into(&mut image, &g.arrow(), g.arrow);
    image
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

/// The number (0 to 99) centred in the box `left..right`, `top..bottom`, digits at
/// `scale` pixels per dot: calls `dot` for every pixel of it.
fn each_digit_pixel(
    percentage: i32,
    (left, top, right, bottom): (u32, u32, u32, u32),
    scale: u32,
    mut dot: impl FnMut(u32, u32),
) {
    let text = percentage.clamp(0, 99).to_string();
    let glyph_w = 5 * scale;
    let gap = scale;
    let width = text.len() as u32 * glyph_w + (text.len() as u32 - 1) * gap;
    let x_start = left + (right - left).saturating_sub(width) / 2;
    let y_start = top + (bottom - top).saturating_sub(7 * scale) / 2;
    for (i, digit) in text.bytes().enumerate() {
        let rows = DIGITS[(digit - b'0') as usize];
        let x0 = x_start + i as u32 * (glyph_w + gap);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (0x10 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            dot(x0 + col * scale + dx, y_start + row as u32 * scale + dy);
                        }
                    }
                }
            }
        }
    }
}

/// The menu bar's progress glyph (a template image): the capture corners stay, the
/// number (0 to 99) stands between them, and a bar along their bottom fills from the
/// left up to the percentage over a faint track.
pub fn template_progress_image(size: u32, percentage: i32) -> image::RgbaImage {
    let g = GlyphGeometry::new(size);
    let mut image = image::RgbaImage::new(size, size);
    stroke_into(&mut image, &g.corners(), g.stroke);
    let gap = (g.stroke / 2.0).ceil().max(1.0);
    let inner_lo = (g.lo + g.stroke + gap) as u32;
    let inner_hi = (g.hi - g.stroke - gap) as u32;
    let side = inner_hi - inner_lo;
    let bar = (side / 6).max(2);
    let bar_top = inner_hi - bar;
    let filled_to =
        inner_lo + (side as f32 * percentage.clamp(0, 100) as f32 / 100.0).round() as u32;
    for py in bar_top..inner_hi {
        for px in inner_lo..inner_hi {
            let alpha = if px < filled_to {
                255
            } else {
                PROGRESS_TRACK_ALPHA
            };
            image.put_pixel(px, py, image::Rgba([0, 0, 0, alpha]));
        }
    }
    // Two digits and their gap are 11 dots wide, 7 tall.
    let scale = (side / 11).clamp(1, 2);
    each_digit_pixel(
        percentage,
        (
            inner_lo,
            inner_lo,
            inner_hi,
            bar_top.saturating_sub(bar / 2),
        ),
        scale,
        |x, y| image.put_pixel(x, y, image::Rgba([0, 0, 0, 255])),
    );
    image
}

/// The bar's track: present, but well behind the fill and the number.
const PROGRESS_TRACK_ALPHA: u8 = 80;

/// `Helpers.GetProgressIcon` in the app's colours (Windows): a dark square filled from
/// the bottom in the accent up to the percentage, a lighter line on the fill's edge,
/// and the number (0 to 99) in white in the middle. Drawn at the icon's size, digits at
/// twice upstream's pixels.
pub fn progress_image(percentage: i32) -> image::RgbaImage {
    const SIZE: u32 = ICON_SIZE;
    let percentage = percentage.clamp(0, 100);
    let fill = image::Rgba(PROGRESS_FILL);
    // `ColorHelpers.LighterColor`: a float lerp towards white, truncated.
    let lerp = |c: u8| (c as f32 + (255.0 - c as f32) * 0.3) as u8;
    let line = image::Rgba([
        lerp(PROGRESS_FILL[0]),
        lerp(PROGRESS_FILL[1]),
        lerp(PROGRESS_FILL[2]),
        255,
    ]);
    let mut image = image::RgbaImage::from_pixel(SIZE, SIZE, image::Rgba(PROGRESS_BACK));
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
    each_digit_pixel(percentage, (0, 0, SIZE, SIZE), 2, |x, y| {
        image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]))
    });
    // `bmp.SetPixel(0, 0, Color.Transparent)`.
    image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
    image
}

/// The progress square's colours: the app's surface and its accent (#171717, #3b82f6).
const PROGRESS_BACK: [u8; 4] = [23, 23, 23, 255];
const PROGRESS_FILL: [u8; 4] = [59, 130, 246, 255];

fn progress_icon(percentage: i32) -> Option<Icon> {
    let image = if TEMPLATE {
        template_progress_image(ICON_SIZE, percentage)
    } else {
        progress_image(percentage)
    };
    let (w, h) = image.dimensions();
    Icon::from_rgba(image.into_raw(), w, h).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu is rebuilt whenever what it shows changes (a window opened, a title
    /// changed, an upload finished); a click on the menu the person had open must run
    /// the entry they saw, not whatever stands at that position in the new menu.
    #[test]
    fn a_click_on_a_replaced_menu_runs_what_it_showed() {
        let mut table = MenuCommands::default();
        table.begin();
        let first = table.id_for(&Command::Capture(
            modlight_core::capture::CaptureKind::Fullscreen,
            false,
        ));
        let settings = table.id_for(&Command::ShowSettings(None));
        let folder = table.id_for(&Command::OpenScreenshotsFolder);
        assert!(settings.starts_with(SHOWS) && folder.starts_with(QUIET));
        // Rebuilt with one more window listed before them.
        table.begin();
        table.id_for(&Command::Capture(
            modlight_core::capture::CaptureKind::Fullscreen,
            false,
        ));
        table.id_for(&Command::ToggleHotkeys);
        let new_settings = table.id_for(&Command::ShowSettings(None));
        table.id_for(&Command::OpenScreenshotsFolder);
        assert_eq!(table.command(&settings), Some(Command::ShowSettings(None)));
        assert_eq!(table.command(&folder), Some(Command::OpenScreenshotsFolder));
        assert_eq!(
            table.command(&new_settings),
            Some(Command::ShowSettings(None))
        );
        assert!(table.command(&first).is_some());
        assert_eq!(table.command("q999.0"), None);
        assert_eq!(table.command(""), None);
        // Old menus are forgotten after a while.
        for _ in 0..KEPT_MENUS {
            table.begin();
        }
        assert_eq!(table.command(&settings), None);
    }

    #[test]
    fn the_progress_icon_fills_from_the_bottom() {
        let size = super::ICON_SIZE;
        let image = super::progress_image(50);
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "the corner pixel is cleared");
        assert_eq!(image.get_pixel(1, size - 1).0, super::PROGRESS_FILL);
        assert_eq!(image.get_pixel(1, 1).0, super::PROGRESS_BACK);
        assert_eq!(
            image.get_pixel(1, size / 2).0,
            [117, 167, 248, 255],
            "the edge line"
        );
    }

    /// The menu bar glyph is a template: black with alpha, the corners solid on whole
    /// pixels, nothing in the square's own corner.
    #[test]
    fn the_menu_bar_glyph_is_a_sharp_template() {
        for size in [16, 18, 32, 36] {
            let image = super::tray_glyph(size);
            assert!(image.pixels().all(|p| p.0[..3] == [0, 0, 0]));
            assert_eq!(image.get_pixel(0, 0).0[3], 0);
            let g = super::GlyphGeometry::new(size);
            // The middle of the top-left corner's horizontal arm.
            let (x, y) = ((g.lo + g.arm / 2.0) as u32, (g.lo + g.stroke / 2.0) as u32);
            assert_eq!(image.get_pixel(x, y).0[3], 255, "size {size}");
            // The middle of the shaft.
            assert_eq!(
                image.get_pixel(size / 2, size * 3 / 5).0[3],
                255,
                "size {size}"
            );
        }
    }

    #[test]
    fn the_menu_bar_progress_fills_its_bar_from_the_left() {
        let alpha = |i: &image::RgbaImage, x, y| i.get_pixel(x, y).0[3];
        let empty = super::template_progress_image(36, 0);
        let half = super::template_progress_image(36, 50);
        let full = super::template_progress_image(36, 100);
        // At 36 px the box between the corners is 7..29, the bar its last 3 rows.
        assert_eq!(alpha(&empty, 7, 28), super::PROGRESS_TRACK_ALPHA);
        assert_eq!(alpha(&half, 7, 28), 255);
        assert_eq!(alpha(&half, 28, 28), super::PROGRESS_TRACK_ALPHA);
        assert_eq!(alpha(&full, 28, 28), 255);
        assert_eq!(
            alpha(&full, 28, 24),
            0,
            "nothing between the bar and the number"
        );
        // The number is drawn above the bar.
        let ink = (7..24)
            .flat_map(|y| (7..29).map(move |x| (x, y)))
            .filter(|&(x, y)| alpha(&half, x, y) == 255)
            .count();
        assert!(ink > 20);
    }
}

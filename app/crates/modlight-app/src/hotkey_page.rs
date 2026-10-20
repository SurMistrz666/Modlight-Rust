//! Settings -> Keyboard shortcuts (`HotkeySettingsView` with its view model and the
//! Avalonia service): the toolbar, the "hotkeys are disabled" notice, one row per hotkey
//! with its task menu, its key capture button and its registration state, and the reset
//! question.
//!
//! On a Mac a key is captured from the window's own key events. While one is being
//! captured every hotkey is unregistered (upstream only ignores them): a combination the
//! system holds for this app is otherwise delivered as a hotkey, never as a key press, so
//! a shortcut could not be typed again.

use egui::{
    Align, Color32, Layout, Order, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use modlight_core::hotkeys::{
    HotkeyCategory, HotkeyInfo, HotkeySettings, HotkeyStatus, HotkeyType, gesture,
};
use modlight_core::icons;
use modlight_core::l10n::{s, t};

use crate::commands::Command;
use crate::menus::{Entry, Toggle};
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Look, button, button_at, layout, paint_icon, trimmed, wrapped};

/// The page's own state: the selected row, the row whose key is being captured, the
/// reset question, the open task menu.
#[derive(Clone, Debug, Default)]
pub struct HotkeyPageState {
    pub selected: Option<usize>,
    pub capturing: Option<usize>,
    pub confirm_reset: bool,
    pub task_menu: Option<usize>,
    /// Where the capturing row's button was last drawn (a click elsewhere ends it).
    pub capture_rect: Option<Rect>,
}

/// What a click asked for.
#[derive(Clone, Debug, PartialEq)]
enum Ask {
    Add,
    Remove,
    EditTask(usize),
    Duplicate,
    Move(i64),
    Reset,
    ConfirmReset,
    CancelReset,
    EnableHotkeys,
    Select(usize),
    SetTask(usize, HotkeyType),
    Capture(usize),
}

/// The WinForms name of a key egui reports (`ConvertGesture`'s `KeyName`); `None` for
/// keys a hotkey cannot use.
pub fn key_name(key: egui::Key) -> Option<&'static str> {
    use egui::Key as K;
    Some(match key {
        K::A => "A",
        K::B => "B",
        K::C => "C",
        K::D => "D",
        K::E => "E",
        K::F => "F",
        K::G => "G",
        K::H => "H",
        K::I => "I",
        K::J => "J",
        K::K => "K",
        K::L => "L",
        K::M => "M",
        K::N => "N",
        K::O => "O",
        K::P => "P",
        K::Q => "Q",
        K::R => "R",
        K::S => "S",
        K::T => "T",
        K::U => "U",
        K::V => "V",
        K::W => "W",
        K::X => "X",
        K::Y => "Y",
        K::Z => "Z",
        K::Num0 => "D0",
        K::Num1 => "D1",
        K::Num2 => "D2",
        K::Num3 => "D3",
        K::Num4 => "D4",
        K::Num5 => "D5",
        K::Num6 => "D6",
        K::Num7 => "D7",
        K::Num8 => "D8",
        K::Num9 => "D9",
        K::F1 => "F1",
        K::F2 => "F2",
        K::F3 => "F3",
        K::F4 => "F4",
        K::F5 => "F5",
        K::F6 => "F6",
        K::F7 => "F7",
        K::F8 => "F8",
        K::F9 => "F9",
        K::F10 => "F10",
        K::F11 => "F11",
        K::F12 => "F12",
        K::F13 => "F13",
        K::F14 => "F14",
        K::F15 => "F15",
        K::F16 => "F16",
        K::F17 => "F17",
        K::F18 => "F18",
        K::F19 => "F19",
        K::F20 => "F20",
        K::ArrowLeft => "Left",
        K::ArrowUp => "Up",
        K::ArrowRight => "Right",
        K::ArrowDown => "Down",
        K::Tab => "Tab",
        K::Space => "Space",
        K::Enter => "Return",
        K::Backspace => "Back",
        K::Delete => "Delete",
        K::Insert => "Insert",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::Minus => "OemMinus",
        K::Plus | K::Equals => "Oemplus",
        K::Comma => "Oemcomma",
        K::Period => "OemPeriod",
        K::Semicolon => "OemSemicolon",
        K::Slash => "OemQuestion",
        K::Backslash => "OemPipe",
        K::OpenBracket => "OemOpenBrackets",
        K::CloseBracket => "OemCloseBrackets",
        K::Quote => "OemQuotes",
        K::Backtick => "Oemtilde",
        _ => return None,
    })
}

/// The binding while only modifiers are down (`LeftCtrl` and friends): "Cmd + ...".
fn modifiers_only(m: egui::Modifiers) -> HotkeyInfo {
    let key = if m.mac_cmd || m.command && !m.ctrl {
        Some("ControlKey")
    } else if m.shift {
        Some("ShiftKey")
    } else if m.alt {
        Some("Menu")
    } else {
        None
    };
    gesture(key, m.mac_cmd, m.shift, m.alt, m.ctrl)
}

/// The task menu (`OnTaskClick`): None, then a submenu per category, radio marks.
fn task_menu(current: HotkeyType) -> (Vec<Entry>, Vec<HotkeyType>) {
    let mut jobs: Vec<HotkeyType> = Vec::new();
    let item = |job: HotkeyType, jobs: &mut Vec<HotkeyType>| {
        let entry = Entry::Item {
            label: job.text(),
            icon: Some(job.icon()),
            enabled: true,
            toggle: Toggle::Radio(job == current),
            command: Some(Command::Local(jobs.len())),
        };
        jobs.push(job);
        entry
    };
    let mut entries = vec![item(HotkeyType::None, &mut jobs)];
    let mut groups: Vec<(HotkeyCategory, Vec<Entry>)> = Vec::new();
    for job in HotkeyType::ALL
        .iter()
        .copied()
        .filter(|j| *j != HotkeyType::None)
    {
        let category = job.category();
        if category == HotkeyCategory::None {
            continue;
        }
        let entry = item(job, &mut jobs);
        match groups.iter_mut().find(|(c, _)| *c == category) {
            Some((_, list)) => list.push(entry),
            None => groups.push((category, vec![entry])),
        }
    }
    for (category, list) in groups {
        entries.push(Entry::Submenu {
            label: category.text().to_string(),
            icon: None,
            enabled: true,
            children: list,
        });
    }
    (entries, jobs)
}

fn compact(colors: &Colors) -> Look {
    Look::button(colors)
        .height(34.0)
        .padding(9.0)
        .icon(14.0, None)
        .spacing(7.0)
}

/// Draw the page; `rows` are the hotkeys as the manager has them.
pub fn page(ui: &mut Ui, state: &mut AppState, colors: &Colors, area: Rect) {
    let Some(window) = state.windows.settings.as_mut() else {
        return;
    };
    let mut page = std::mem::take(&mut window.hotkeys);
    let disabled = state.captures.hotkeys.disabled;
    let rows: Vec<HotkeySettings> = state.captures.hotkeys.hotkeys.clone();
    let mut asks = Vec::new();
    if page.selected.is_some_and(|i| i >= rows.len()) {
        page.selected = None;
    }
    if page.capturing.is_some() {
        capture_keys(ui, state, &mut page);
    }
    let rows: Vec<HotkeySettings> = if page.capturing.is_some() {
        state.captures.hotkeys.hotkeys.clone()
    } else {
        rows
    };

    toolbar(ui, colors, &page, rows.len(), &mut asks);
    ui.add_space(8.0);
    if disabled {
        disabled_notice(ui, colors, &mut asks);
        ui.add_space(8.0);
    }
    list(ui, colors, &mut page, &rows, &mut asks);
    if page.confirm_reset {
        reset_question(ui, colors, area, &mut asks);
    }

    if let Some(window) = state.windows.settings.as_mut() {
        window.hotkeys = page;
    }
    for ask in asks {
        state.hotkey_page_ask(ask);
    }
}

fn toolbar(
    ui: &mut Ui,
    colors: &Colors,
    page: &HotkeyPageState,
    count: usize,
    asks: &mut Vec<Ask>,
) {
    let has = page.selected.is_some();
    let can_move = has && count > 1;
    let width = ui.available_width();
    egui::Frame::new()
        .fill(colors.background_popup())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 4,
            se: 4,
        })
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(width - 18.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let look = compact(colors);
                if button(
                    ui,
                    colors,
                    &look,
                    Content::both(icons::PLUS, t(s::HotkeySettingsWindow_Add)),
                    true,
                )
                .clicked()
                {
                    asks.push(Ask::Add);
                }
                if button(
                    ui,
                    colors,
                    &look,
                    Content::both(icons::TRASH_2, t(s::HotkeySettingsWindow_Remove)),
                    has,
                )
                .clicked()
                {
                    asks.push(Ask::Remove);
                }
                if button(
                    ui,
                    colors,
                    &look,
                    Content::both(icons::SETTINGS_2, t(s::HotkeySettingsWindow_EditTask)),
                    has,
                )
                .clicked()
                    && let Some(index) = page.selected
                {
                    asks.push(Ask::EditTask(index));
                }
                if button(
                    ui,
                    colors,
                    &look,
                    Content::both(icons::COPY_PLUS, t(s::HotkeySettingsWindow_Duplicate)),
                    has,
                )
                .clicked()
                {
                    asks.push(Ask::Duplicate);
                }
                let square = Look::button(colors)
                    .height(34.0)
                    .width(34.0)
                    .padding(0.0)
                    .icon(16.0, None);
                if button(
                    ui,
                    colors,
                    &square,
                    Content::icon(icons::CHEVRON_UP),
                    can_move,
                )
                .on_hover_text(t(s::HotkeySettingsWindow_MoveUp))
                .clicked()
                {
                    asks.push(Ask::Move(-1));
                }
                if button(
                    ui,
                    colors,
                    &square,
                    Content::icon(icons::CHEVRON_DOWN),
                    can_move,
                )
                .on_hover_text(t(s::HotkeySettingsWindow_MoveDown))
                .clicked()
                {
                    asks.push(Ask::Move(1));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(
                        ui,
                        colors,
                        &look,
                        Content::both(
                            icons::ROTATE_CCW,
                            t(s::HotkeySettingsWindow_ResetWithEllipsis),
                        ),
                        true,
                    )
                    .clicked()
                    {
                        asks.push(Ask::Reset);
                    }
                });
            });
        });
}

fn disabled_notice(ui: &mut Ui, colors: &Colors, asks: &mut Vec<Ask>) {
    let width = ui.available_width();
    egui::Frame::new()
        .fill(colors.control_background())
        .stroke(Stroke::new(1.0, colors.cancel_border()))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(width - 18.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                crate::widgets::icon(ui, icons::TRIANGLE_ALERT, 18.0, colors.cancel_border());
                let look = Look {
                    string_text: Some(colors.text()),
                    ..compact(colors)
                };
                let enable = t(s::HotkeySettingsWindow_EnableHotkeys);
                let button_w = crate::widgets::button_width(ui, &look, Content::label(enable));
                let text_w = ui.available_width() - button_w - 9.0;
                let galley = layout(
                    ui,
                    wrapped(
                        t(s::HotkeySettingsWindow_GlobalHotkeysAreCurrentlyDisabled),
                        13.0,
                        Weight::Normal,
                        colors.text(),
                        text_w,
                    ),
                );
                let (r, _) =
                    ui.allocate_exact_size(vec2(text_w, galley.size().y.max(34.0)), Sense::hover());
                ui.painter().galley(
                    pos2(r.left(), r.center().y - galley.size().y / 2.0),
                    galley,
                    colors.text(),
                );
                if button(ui, colors, &look, Content::label(enable), true).clicked() {
                    asks.push(Ask::EnableHotkeys);
                }
            });
        });
}

/// The width of the hotkey column (`SharedSizeGroup="HotkeyGroupWidth"`): the widest
/// capture button's text, 16 padding each side, then 4 and the 32 wide state icon.
fn hotkey_column(ui: &Ui, rows: &[HotkeySettings], page: &HotkeyPageState) -> f32 {
    let mut widest = layout(
        ui,
        crate::widgets::job(
            t(s::HotkeySettingsWindow_Hotkey),
            12.0,
            Weight::Normal,
            Color32::WHITE,
        ),
    )
    .size()
    .x;
    for (index, row) in rows.iter().enumerate() {
        let text = display_text(row, page.capturing == Some(index));
        widest = widest.max(
            layout(
                ui,
                crate::widgets::job(&text, 13.0, Weight::Normal, Color32::WHITE),
            )
            .size()
            .x + 32.0
                + 4.0
                + 32.0,
        );
    }
    widest.min(320.0)
}

fn display_text(row: &HotkeySettings, capturing: bool) -> String {
    if capturing {
        t(s::HotkeySettingsWindow_PressAHotkey).to_string()
    } else {
        row.info.text()
    }
}

fn list(
    ui: &mut Ui,
    colors: &Colors,
    page: &mut HotkeyPageState,
    rows: &[HotkeySettings],
    asks: &mut Vec<Ask>,
) {
    let width = ui.available_width();
    let hotkey_w = hotkey_column(ui, rows, page);
    egui::Frame::new()
        .fill(colors.background_popup())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(4)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(width - 18.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            // The column headings, 4 in and 2 above the rows.
            let (heading, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
            let task_heading = layout(
                ui,
                crate::widgets::job(
                    t(s::HotkeySettingsWindow_Task),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                ),
            );
            ui.painter().galley(
                pos2(heading.left() + 4.0, heading.top()),
                task_heading,
                colors.text_secondary(),
            );
            let key_heading = layout(
                ui,
                crate::widgets::job(
                    t(s::HotkeySettingsWindow_Hotkey),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                ),
            );
            ui.painter().galley(
                pos2(heading.right() - 4.0 - hotkey_w, heading.top()),
                key_heading,
                colors.text_secondary(),
            );
            ui.add_space(6.0);
            if rows.is_empty() {
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::hover());
                paint_icon(
                    ui,
                    ui.painter(),
                    pos2(r.center().x, r.center().y - 18.0),
                    icons::KEYBOARD,
                    42.0,
                    colors.text_secondary(),
                );
                let text = layout(
                    ui,
                    crate::widgets::job(
                        t(s::HotkeySettingsWindow_AddAHotkeyToCreateAWorkflow),
                        13.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    ),
                );
                ui.painter().galley(
                    pos2(r.center().x - text.size().x / 2.0, r.center().y + 12.0),
                    text,
                    colors.text_secondary(),
                );
                return;
            }
            for (index, hotkey) in rows.iter().enumerate() {
                row(ui, colors, page, index, hotkey, hotkey_w, asks);
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    colors: &Colors,
    page: &mut HotkeyPageState,
    index: usize,
    hotkey: &HotkeySettings,
    hotkey_w: f32,
    asks: &mut Vec<Ask>,
) {
    // `Border.hotkey-item`: 4,3 padding, the hover colour when selected or hovered.
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 34.0 + 6.0), Sense::click());
    let selected = page.selected == Some(index);
    if selected || response.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, colors.control_background_hover());
    }
    if response.clicked() {
        asks.push(Ask::Select(index));
    }
    let inner = rect.shrink2(vec2(4.0, 3.0));
    let key_rect = Rect::from_min_max(pos2(inner.right() - hotkey_w, inner.top()), inner.max);
    let task_rect = Rect::from_min_max(inner.min, pos2(key_rect.left() - 4.0, inner.bottom()));
    let edit_rect = Rect::from_min_max(
        pos2(task_rect.right() - 34.0, task_rect.top()),
        task_rect.max,
    );
    let selector = Rect::from_min_max(
        task_rect.min,
        pos2(edit_rect.left() - 4.0, task_rect.bottom()),
    );

    // The task selector: the task's icon, its name, a pencil when it has its own
    // settings, a chevron.
    let job = hotkey.job();
    let look = Look::button(colors).height(34.0).padding(8.0);
    let selector_response = button_at(
        ui,
        colors,
        &look,
        Content {
            icon: None,
            label: None,
        },
        true,
        selector,
    );
    let painter = ui.painter();
    paint_icon(
        ui,
        painter,
        pos2(selector.left() + 8.0 + 9.0, selector.center().y),
        job.icon(),
        16.0,
        colors.accent_start(),
    );
    let customized = !hotkey.is_using_default_settings();
    let right_icons = if customized { 14.0 + 7.0 + 14.0 } else { 14.0 };
    let name_w = selector.width() - 8.0 - 18.0 - 7.0 - 7.0 - right_icons - 8.0;
    let name = layout(
        ui,
        trimmed(
            &job.text(),
            13.0,
            Weight::Normal,
            colors.text(),
            name_w.max(10.0),
            1,
        ),
    );
    painter.galley(
        pos2(
            selector.left() + 8.0 + 18.0 + 7.0,
            selector.center().y - name.size().y / 2.0,
        ),
        name,
        colors.text(),
    );
    paint_icon(
        ui,
        painter,
        pos2(selector.right() - 8.0 - 7.0, selector.center().y),
        icons::CHEVRON_DOWN,
        14.0,
        colors.text_secondary(),
    );
    if customized {
        let pencil = pos2(
            selector.right() - 8.0 - 14.0 - 7.0 - 7.0,
            selector.center().y,
        );
        paint_icon(
            ui,
            painter,
            pencil,
            icons::PENCIL,
            14.0,
            colors.accent_start(),
        );
        let tip = ui.interact(
            Rect::from_center_size(pencil, vec2(16.0, 16.0)),
            ui.id().with(("customized", index)),
            Sense::hover(),
        );
        tip.on_hover_text(t(s::HotkeySettingsWindow_ThisTaskHasCustomSettings));
    }
    if selector_response.clicked() {
        asks.push(Ask::Select(index));
        page.task_menu = Some(index);
    }
    if page.task_menu == Some(index) {
        let mut open = true;
        let (entries, jobs) = task_menu(job);
        if let Some(Command::Local(pick)) =
            crate::menus::popup_open_left(colors, &selector_response, &mut open, || entries)
            && let Some(job) = jobs.get(pick)
        {
            asks.push(Ask::SetTask(index, *job));
            open = false;
        }
        if !open {
            page.task_menu = None;
        }
    }

    // Edit task settings.
    let edit_look = Look::button(colors)
        .height(34.0)
        .width(34.0)
        .padding(0.0)
        .icon(17.0, None);
    if button_at(
        ui,
        colors,
        &edit_look,
        Content::icon(icons::PENCIL),
        true,
        edit_rect,
    )
    .on_hover_text(t(s::HotkeySettingsWindow_EditTaskSettings))
    .clicked()
    {
        asks.push(Ask::EditTask(index));
    }

    // The capture button and the registration state.
    let capture_rect = Rect::from_min_max(
        key_rect.min,
        pos2(key_rect.right() - 32.0 - 4.0, key_rect.bottom()),
    );
    let capturing = page.capturing == Some(index);
    let capture_look = Look {
        string_text: Some(colors.text()),
        ..Look::button(colors).height(34.0).padding(16.0)
    };
    let text = display_text(hotkey, capturing);
    let capture = button_at(
        ui,
        colors,
        &capture_look,
        Content::label(&text),
        true,
        capture_rect,
    );
    if capturing {
        page.capture_rect = Some(capture_rect);
        ui.painter().rect_stroke(
            capture_rect,
            10.0,
            Stroke::new(1.5, colors.focus()),
            StrokeKind::Inside,
        );
    }
    if capture.clicked() {
        asks.push(Ask::Capture(index));
    }
    let state_rect =
        Rect::from_min_max(pos2(key_rect.right() - 32.0, key_rect.top()), key_rect.max);
    let (glyph, color, tip) = match hotkey.status {
        HotkeyStatus::Registered => (
            icons::CIRCLE_CHECK,
            colors.continue_border(),
            t(s::HotkeySettingsWindow_Registered),
        ),
        HotkeyStatus::Failed => (
            icons::CIRCLE_X,
            colors.cancel_border(),
            t(s::HotkeySettingsWindow_RegistrationFailed),
        ),
        HotkeyStatus::NotConfigured => (
            icons::CIRCLE_DASHED,
            colors.text_secondary(),
            t(s::HotkeySettingsWindow_NotConfigured),
        ),
    };
    paint_icon(ui, ui.painter(), state_rect.center(), glyph, 19.0, color);
    ui.interact(state_rect, ui.id().with(("state", index)), Sense::hover())
        .on_hover_text(tip);
}

fn reset_question(ui: &mut Ui, colors: &Colors, area: Rect, asks: &mut Vec<Ask>) {
    let ctx = ui.ctx().clone();
    egui::Area::new(egui::Id::new("hotkeys-reset-veil"))
        .order(Order::Foreground)
        .fixed_pos(area.min)
        .show(&ctx, |ui| {
            let (veil, _) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
            ui.painter().rect_filled(veil, 0.0, colors.overlay_modal());
            let inner_w = 390.0 - 36.0;
            let heading = layout(
                ui,
                wrapped(
                    t(s::HotkeySettingsWindow_ResetHotkeysQuestion),
                    15.0,
                    Weight::SemiBold,
                    colors.text(),
                    inner_w - 30.0,
                ),
            );
            let text = layout(
                ui,
                wrapped(
                    t(s::HotkeySettingsWindow_ResetHotkeysDescription),
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    inner_w,
                ),
            );
            let height =
                18.0 + heading.size().y.max(20.0) + 12.0 + text.size().y + 12.0 + 40.0 + 18.0;
            let panel = Rect::from_center_size(veil.center(), vec2(390.0, height));
            ui.painter()
                .rect_filled(panel, 4.0, colors.background_popup());
            ui.painter().rect_stroke(
                panel,
                4.0,
                Stroke::new(1.0, colors.border()),
                StrokeKind::Inside,
            );
            let inner = panel.shrink(18.0);
            let head_h = heading.size().y.max(20.0);
            paint_icon(
                ui,
                ui.painter(),
                pos2(inner.left() + 10.0, inner.top() + head_h / 2.0),
                icons::ROTATE_CCW,
                20.0,
                colors.accent_start(),
            );
            ui.painter().galley(
                pos2(
                    inner.left() + 30.0,
                    inner.top() + (head_h - heading.size().y) / 2.0,
                ),
                heading,
                colors.text(),
            );
            let text_top = inner.top() + head_h + 12.0;
            let text_h = text.size().y;
            ui.painter()
                .galley(pos2(inner.left(), text_top), text, colors.text_secondary());
            let row = Rect::from_min_max(
                pos2(inner.left(), text_top + text_h + 12.0),
                pos2(inner.right(), inner.bottom()),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(row)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.spacing_mut().item_spacing.x = 8.0;
            let danger = Look::button(colors).danger(colors);
            if button(
                &mut child,
                colors,
                &danger,
                Content::label(t(s::HotkeySettingsWindow_Reset)),
                true,
            )
            .clicked()
            {
                asks.push(Ask::ConfirmReset);
            }
            let cancel = Look {
                string_text: Some(colors.text()),
                ..Look::button(colors)
            };
            if button(
                &mut child,
                colors,
                &cancel,
                Content::label(t(s::HotkeySettingsWindow_Cancel)),
                true,
            )
            .clicked()
            {
                asks.push(Ask::CancelReset);
            }
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        asks.push(Ask::CancelReset);
    }
}

/// While a row captures: key presses become its binding (`OnHotkeyKeyDown`), Escape
/// clears it, a click elsewhere ends the capture (`OnHotkeyLostFocus`).
fn capture_keys(ui: &mut Ui, state: &mut AppState, page: &mut HotkeyPageState) {
    let Some(index) = page.capturing else {
        return;
    };
    let events = ui.input(|i| i.events.clone());
    let modifiers = ui.input(|i| i.modifiers);
    let mut done = false;
    let mut seen_key = false;
    for event in events {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        {
            seen_key = true;
            if key == egui::Key::Escape {
                state.set_hotkey_binding(index, HotkeyInfo::default());
                done = true;
                break;
            }
            let Some(name) = key_name(key) else {
                continue;
            };
            let info = gesture(
                Some(name),
                modifiers.mac_cmd,
                modifiers.shift,
                modifiers.alt,
                modifiers.ctrl,
            );
            state.set_hotkey_binding(index, info);
            if info.is_valid() {
                done = true;
                break;
            }
        }
    }
    if !done && !seen_key {
        let current = state
            .captures
            .hotkeys
            .hotkeys
            .get(index)
            .map(|h| h.info)
            .unwrap_or_default();
        let wanted = if modifiers.is_none() {
            HotkeyInfo::default()
        } else {
            modifiers_only(modifiers)
        };
        if current != wanted && (current.key_code() == 0 || current.is_only_modifiers()) {
            state.set_hotkey_binding(index, wanted);
        }
    }
    // Keys typed here go nowhere else (focus stays on the capture).
    ui.ctx().input_mut(|i| {
        i.events
            .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
    });
    let pressed_at = ui.input(|i| {
        if i.pointer.any_pressed() {
            i.pointer.interact_pos()
        } else {
            None
        }
    });
    let clicked_away =
        pressed_at.is_some_and(|p| !page.capture_rect.is_some_and(|r| r.contains(p)));
    if done || clicked_away {
        state.stop_hotkey_capture(page);
    }
}

impl AppState {
    /// `TaskSettingsIntegration.Show(settings, false, closed)`: one hotkey's settings in
    /// the settings shell. One such window at a time; asking again while it is open
    /// leaves it as it is (upstream activates it).
    pub fn open_hotkey_task_settings(&mut self, index: usize) {
        if self.windows.hotkey_task.is_some() {
            return;
        }
        let defaults = self
            .config
            .get("DefaultTaskSettings")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let Some(hotkey) = self.captures.hotkeys.hotkeys.get_mut(index) else {
            return;
        };
        modlight_core::hotkeys::set_default_settings(&mut hotkey.task, &defaults);
        let title = modlight_core::l10n::format(
            t(s::TaskSettingsWindow_ShareXTaskSettingsFor0),
            &[modlight_core::l10n::Arg::from(hotkey.description())],
        );
        self.windows.hotkey_task = Some(crate::settings::SettingsWindow::for_hotkey(
            index,
            title,
            &self.options.skin,
        ));
    }

    /// The hotkey window closed: the list is written (`SaveHotkeysConfigAsync`).
    pub fn close_hotkey_task_settings(&mut self) {
        if self.windows.hotkey_task.take().is_some() {
            self.save_hotkeys();
        }
    }

    /// Keep an open hotkey window on its hotkey when the list changes: `from` moved to
    /// `to`, or was removed (`to` None); both None means the list was replaced.
    fn hotkey_window_follows(&mut self, from: Option<usize>, to: Option<usize>) {
        let Some(window) = self.windows.hotkey_task.as_mut() else {
            return;
        };
        let crate::task_pages::TaskTarget::Hotkey(open) = window.target else {
            return;
        };
        let next = match (from, to) {
            (None, _) => None,
            (Some(from), None) if from == open => None,
            (Some(from), None) => Some(if open > from { open - 1 } else { open }),
            (Some(from), Some(to)) => {
                // `move_by` takes the hotkey out at `from` and puts it in at `to`.
                if open == from {
                    Some(to)
                } else {
                    let after_remove = if open > from { open - 1 } else { open };
                    Some(if after_remove >= to {
                        after_remove + 1
                    } else {
                        after_remove
                    })
                }
            }
        };
        match next {
            Some(index) => window.target = crate::task_pages::TaskTarget::Hotkey(index),
            None => self.close_hotkey_task_settings(),
        }
    }

    fn set_hotkey_binding(&mut self, index: usize, info: HotkeyInfo) {
        if let Some(hotkey) = self.captures.hotkeys.hotkeys.get_mut(index) {
            hotkey.info = info;
        }
    }

    /// `StopCapture` + `FinalizeHotkey`: modifiers alone mean no hotkey; everything is
    /// registered again and the document written.
    fn stop_hotkey_capture(&mut self, page: &mut HotkeyPageState) {
        let Some(index) = page.capturing.take() else {
            return;
        };
        if let Some(hotkey) = self.captures.hotkeys.hotkeys.get_mut(index)
            && hotkey.info.is_only_modifiers()
        {
            hotkey.info = HotkeyInfo::default();
        }
        self.captures.hotkeys.ignore = false;
        if !self.captures.hotkeys.disabled {
            self.captures.hotkeys.register_all();
        }
        self.save_hotkeys();
    }

    fn hotkey_page_ask(&mut self, ask: Ask) {
        let mut page = self
            .windows
            .settings
            .as_mut()
            .map(|w| std::mem::take(&mut w.hotkeys))
            .unwrap_or_default();
        match ask {
            Ask::Select(index) => page.selected = Some(index),
            Ask::Add => {
                self.stop_hotkey_capture(&mut page);
                let index = self
                    .captures
                    .hotkeys
                    .add(HotkeySettings::new(HotkeyType::None, 0));
                page.selected = Some(index);
                self.save_hotkeys();
            }
            Ask::Remove => {
                if let Some(index) = page.selected {
                    self.stop_hotkey_capture(&mut page);
                    self.hotkey_window_follows(Some(index), None);
                    self.captures.hotkeys.remove(index);
                    let count = self.captures.hotkeys.hotkeys.len();
                    page.selected = if count == 0 {
                        None
                    } else {
                        Some(index.min(count - 1))
                    };
                    self.save_hotkeys();
                }
            }
            Ask::Duplicate => {
                if let Some(source) = page
                    .selected
                    .and_then(|i| self.captures.hotkeys.hotkeys.get(i))
                    .cloned()
                {
                    let mut task = source.task.clone();
                    if task.is_object() {
                        task["WatchFolderEnabled"] = serde_json::json!(false);
                        task["WatchFolderList"] = serde_json::json!([]);
                    }
                    let duplicate = HotkeySettings {
                        task,
                        ..HotkeySettings::new(HotkeyType::None, 0)
                    };
                    let index = self.captures.hotkeys.add(duplicate);
                    page.selected = Some(index);
                    self.save_hotkeys();
                }
            }
            Ask::Move(offset) => {
                if let Some(index) = page.selected {
                    let to = self.captures.hotkeys.move_by(index, offset);
                    self.hotkey_window_follows(Some(index), Some(to));
                    page.selected = Some(to);
                    self.save_hotkeys();
                }
            }
            Ask::EditTask(index) => {
                page.selected = Some(index);
                self.open_hotkey_task_settings(index);
            }
            Ask::SetTask(index, job) => {
                page.selected = Some(index);
                if let Some(hotkey) = self.captures.hotkeys.hotkeys.get_mut(index)
                    && hotkey.job() != job
                {
                    hotkey.set_job(job);
                    self.save_hotkeys();
                }
            }
            Ask::Capture(index) => {
                page.selected = Some(index);
                if page.capturing == Some(index) {
                    self.stop_hotkey_capture(&mut page);
                } else {
                    self.stop_hotkey_capture(&mut page);
                    page.capturing = Some(index);
                    // `SetCaptureMode(true)`; on a Mac also nothing registered (above).
                    self.captures.hotkeys.ignore = true;
                    self.captures.hotkeys.unregister_all(false);
                    self.set_hotkey_binding(index, HotkeyInfo::default());
                }
            }
            Ask::Reset => page.confirm_reset = true,
            Ask::CancelReset => page.confirm_reset = false,
            Ask::ConfirmReset => {
                self.stop_hotkey_capture(&mut page);
                self.hotkey_window_follows(None, None);
                self.captures.hotkeys.reset();
                page.confirm_reset = false;
                page.selected = None;
                self.save_hotkeys();
            }
            Ask::EnableHotkeys => {
                if self.captures.hotkeys.disabled {
                    self.toggle_hotkeys();
                }
            }
        }
        if let Some(window) = self.windows.settings.as_mut() {
            window.hotkeys = page;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_winforms_names() {
        assert_eq!(key_name(egui::Key::Num2), Some("D2"));
        assert_eq!(key_name(egui::Key::Enter), Some("Return"));
        assert_eq!(key_name(egui::Key::F13), Some("F13"));
        assert_eq!(key_name(egui::Key::Escape), None);
        for key in [
            egui::Key::A,
            egui::Key::Minus,
            egui::Key::Backtick,
            egui::Key::PageDown,
        ] {
            let name = key_name(key).unwrap();
            assert!(modlight_core::hotkeys::key_code(name).is_some(), "{name}");
        }
    }

    #[test]
    fn modifiers_alone_show_as_an_unfinished_binding() {
        let m = egui::Modifiers {
            mac_cmd: true,
            command: true,
            shift: true,
            ..Default::default()
        };
        let info = modifiers_only(m);
        assert!(info.is_only_modifiers());
        assert!(!info.is_valid());
        assert!(info.text().ends_with("..."));
    }

    #[test]
    fn the_task_menu_has_every_job_once() {
        let (entries, jobs) = task_menu(HotkeyType::RectangleRegion);
        assert_eq!(jobs[0], HotkeyType::None);
        assert!(entries.len() > 3);
        let mut unique = jobs.clone();
        unique.dedup();
        assert_eq!(unique.len(), jobs.len());
    }
}

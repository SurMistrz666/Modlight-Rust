//! The Graphite skin's side panel (`shell::uses_side_panel`): every place of the app as
//! a row of the main window. The account and its storage on a card at the top; under
//! it, scrolling, the tabs, then under small headings the settings sections (the
//! settings window's tree, whose pages open in the window's content area) and the rest
//! of the settings menu (the task menus, the screenshots folder, the history windows),
//! and last About, the Information page; the app's name at the bottom. What is a window
//! of its own (the history, the tools) still opens as one; only its entry is here.

use egui::{Align, Id, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use modlight_core::hotkeys::HotkeyType;
use modlight_core::icons;
use modlight_core::l10n::{mac, s, t};

use crate::commands::Command;
use crate::settings::GROUPS;
use crate::state::{AppState, Tab};
use crate::theme::{Colors, Weight};
use crate::widgets::{job, layout, paint_icon, paint_progress, trimmed};

/// A tab's row.
const TAB_ROW: f32 = 36.0;
/// A settings group's row, a section's of its own, the other entries'.
const ROW: f32 = 34.0;
/// A section's row inside an unfolded group.
const SUB_ROW: f32 = 30.0;
/// Between two rows.
const GAP: f32 = 2.0;
/// Where an unfolded group's sections start: their labels line up with the group's.
const SUB_INDENT: f32 = 28.0;

/// The panel's own state.
pub struct SidePanel {
    /// Which groups of the settings tree are unfolded, one per [`GROUPS`] entry.
    pub expanded: Vec<bool>,
    /// The Information page, while it is the one shown.
    pub information: Option<crate::information::InformationPage>,
    /// Bring the chosen row into view on the next frame: its page was opened from
    /// somewhere else (a menu, the tray, a button on another page).
    pub reveal: bool,
}

impl Default for SidePanel {
    fn default() -> Self {
        SidePanel {
            expanded: vec![false; GROUPS.len()],
            information: None,
            reveal: false,
        }
    }
}

impl SidePanel {
    /// Unfold the group holding `section` and bring its row into view.
    pub fn reveal_section(&mut self, section: &str) {
        for (index, group) in GROUPS.iter().enumerate() {
            if group.sections.len() > 1 && group.sections.iter().any(|s| s.id == section) {
                self.expanded[index] = true;
            }
        }
        self.reveal = true;
    }
}

/// What the main window's content area shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Tab(Tab),
    /// The section of the settings window, drawn in the window (`SettingsWindow.embedded`).
    Settings,
    /// About, with the app's update.
    Information,
}

impl AppState {
    /// What the content area shows: a page of the side panel, or the tab under it.
    pub fn page(&self) -> Page {
        if self.side.information.is_some() {
            Page::Information
        } else if self.windows.settings.as_ref().is_some_and(|w| w.embedded) {
            Page::Settings
        } else {
            Page::Tab(self.tab)
        }
    }

    /// Whether `tab` is the one on screen (not covered by a page of the side panel).
    pub fn shows_tab(&self, tab: Tab) -> bool {
        self.page() == Page::Tab(tab)
    }

    /// Back to the tab: the settings pages close as their window does (saved, a changed
    /// skin sent once), the Information page goes.
    pub fn leave_side_pages(&mut self) {
        self.side.information = None;
        if self.windows.settings.as_ref().is_some_and(|w| w.embedded) {
            self.close_settings();
        }
    }

    /// A skin without the side panel: the settings shown in the window move to a window
    /// of their own, on the same page; the Information page goes.
    pub fn undock_side_pages(&mut self) {
        self.side.information = None;
        if let Some(window) = self.windows.settings.as_mut() {
            window.embedded = false;
        }
    }
}

/// The panel, in `rect`.
pub fn ui(ui: &mut Ui, state: &mut AppState, rect: Rect, logo: Option<&egui::TextureHandle>) {
    let colors = state.colors.clone();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_toolbar());
    painter.vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, colors.border()),
    );
    let inner = rect.shrink2(vec2(10.0, 10.0));
    let card = account_card(ui, state, &colors, inner);

    // The app's name at the bottom, under a rule.
    let footer = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 30.0), inner.max);
    painter.hline(
        rect.left() + 10.0..=rect.right() - 10.0,
        footer.top() - 8.0,
        Stroke::new(1.0, colors.border()),
    );
    app_name(ui, &colors, footer, logo);

    // The rows between them, scrolling. The floating scroll bar keeps to the margin, and
    // at rest only its handle shows: a track the panel's height would read as a second
    // border.
    let list = Rect::from_min_max(
        pos2(inner.left(), card.bottom() + 6.0),
        pos2(rect.right() - 4.0, footer.top() - 9.0),
    );
    let mut list_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(list)
            .layout(Layout::top_down(Align::Min)),
    );
    list_ui.set_clip_rect(list);
    list_ui.spacing_mut().scroll.dormant_background_opacity = 0.0;
    let mut commands = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("side-panel-rows")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            rows(ui, state, &colors, inner.width(), &mut commands)
        });
    state.side.reveal = false;
    for command in commands {
        state.push(command);
    }
}

/// The account card, with the storage bar along its bottom when the server reports a
/// quota. Returns where it is.
fn account_card(ui: &mut Ui, state: &mut AppState, colors: &Colors, inner: Rect) -> Rect {
    let painter = ui.painter().clone();
    let signed_in = state.account.is_signed_in();
    let with_bar = signed_in && state.strip.has_storage_bar;
    let card = Rect::from_min_size(
        inner.min,
        vec2(inner.width(), if with_bar { 66.0 } else { 56.0 }),
    );
    let response = ui.interact(card, Id::new("side-account"), Sense::click());
    let fill = if response.hovered() || response.is_pointer_button_down_on() {
        colors.control_background_hover()
    } else {
        colors.control_background()
    };
    painter.rect_filled(card, 10.0, fill);
    painter.rect_stroke(
        card,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let circle = Rect::from_min_size(
        pos2(card.left() + 10.0, card.top() + 12.0),
        vec2(32.0, 32.0),
    );
    if signed_in {
        let texture = state.media.avatar.texture().cloned();
        crate::signin::paint_avatar(
            ui,
            colors,
            circle,
            texture.as_ref(),
            &state.account.display_name(),
            20.0,
            12.0,
        );
    } else {
        painter.circle_filled(circle.center(), 16.0, colors.control_background_hover());
        paint_icon(
            ui,
            &painter,
            circle.center(),
            icons::LOG_IN,
            15.0,
            colors.text_secondary(),
        );
    }
    let text_left = circle.right() + 10.0;
    // Not upstream: a lock at the card's right edge when the account's files are
    // encrypted, open while this app holds a grant for them.
    let badge = state.vault_badge().map(|badge| match badge {
        crate::vault::Badge::Unlocked => icons::LOCK_KEYHOLE_OPEN,
        crate::vault::Badge::Locked => icons::LOCK_KEYHOLE,
    });
    let badge_room = if badge.is_some() { 20.0 } else { 0.0 };
    let text_width = card.right() - 10.0 - badge_room - text_left;
    if let Some(glyph) = badge {
        paint_icon(
            ui,
            &painter,
            pos2(card.right() - 18.0, circle.center().y),
            glyph,
            14.0,
            colors.text_secondary(),
        );
    }
    let label = if signed_in {
        state.account_label()
    } else {
        t(s::SettingsWindow_AccountSignedOut).to_string()
    };
    let name = layout(
        ui,
        trimmed(&label, 13.0, Weight::SemiBold, colors.text(), text_width, 1),
    );
    let sub = if !signed_in {
        t(s::MainWindow_SignIn).to_string()
    } else if state.strip.has_storage {
        state.strip.storage_text.clone()
    } else {
        String::new()
    };
    let sub = layout(
        ui,
        trimmed(
            &sub,
            11.0,
            Weight::Normal,
            colors.text_secondary(),
            text_width,
            1,
        ),
    );
    let block = name.size().y + 2.0 + sub.size().y;
    let top = circle.center().y - block / 2.0;
    painter.galley(pos2(text_left, top), name.clone(), colors.text());
    painter.galley(
        pos2(text_left, top + name.size().y + 2.0),
        sub,
        colors.text_secondary(),
    );
    let response = response.on_hover_text(state.account_tooltip());
    if !signed_in {
        if response.clicked() {
            state.push(Command::ShowSignIn);
        }
    } else if let Some(command) =
        crate::menus::popup_beside(colors, &response, || crate::shell::account_menu(state))
    {
        state.push(command);
    }
    if with_bar {
        let bar = Rect::from_min_max(
            pos2(card.left() + 12.0, card.bottom() - 12.0),
            pos2(card.right() - 12.0, card.bottom() - 9.0),
        );
        paint_progress(ui, colors, bar, Some(state.strip.storage_percent / 100.0));
        let _ = ui
            .interact(
                bar.expand2(vec2(0.0, 4.0)),
                Id::new("side-storage"),
                Sense::hover(),
            )
            .on_hover_text(state.strip.storage_tooltip.clone());
    }
    card
}

/// The logo and the app's name, in `footer`.
fn app_name(ui: &Ui, colors: &Colors, footer: Rect, logo: Option<&egui::TextureHandle>) {
    let painter = ui.painter();
    let mut x = footer.left() + 6.0;
    let cy = footer.center().y;
    if let Some(logo) = logo {
        let r = Rect::from_min_size(pos2(x, cy - 10.0), vec2(20.0, 20.0));
        painter.image(
            logo.id(),
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        x = r.right() + 8.0;
    }
    let name = layout(
        ui,
        job(
            modlight_core::APP_NAME,
            12.0,
            Weight::Normal,
            colors.text_secondary(),
        ),
    );
    painter.galley(
        pos2(x, cy - name.size().y / 2.0),
        name,
        colors.text_secondary(),
    );
}

/// Every row, top to bottom, `width` wide; what a click asks for goes to `commands`.
fn rows(
    ui: &mut Ui,
    state: &mut AppState,
    colors: &Colors,
    width: f32,
    commands: &mut Vec<Command>,
) {
    let page = state.page();
    let reveal = state.side.reveal;
    // The row of what is shown comes into view when it was opened from elsewhere, if it
    // is not in view already.
    let reveal_if = |ui: &Ui, response: &egui::Response, selected: bool| {
        if reveal && selected && !ui.clip_rect().contains_rect(response.rect) {
            response
                .scroll_to_me_animation(Some(Align::Center), egui::style::ScrollAnimation::none());
        }
    };
    ui.add_space(6.0);

    // The tabs.
    let admin = state.is_admin();
    for spec in &crate::shell::TABS {
        if spec.tab == Tab::Admin && !admin {
            continue;
        }
        let selected = page == Page::Tab(spec.tab);
        let response = row(
            ui,
            colors,
            width,
            Row {
                selected,
                ..Row::new(
                    Id::new(("side-tab", spec.tab)),
                    TAB_ROW,
                    Some(spec.icon),
                    t(spec.label),
                )
            },
        );
        reveal_if(ui, &response, selected);
        if response.clicked() {
            commands.push(Command::SelectTab(spec.tab));
        }
    }

    // The settings sections: the settings window's tree, its pages shown in the window.
    heading(ui, colors, width, t(s::MainWindow_Settings));
    let open = match page {
        Page::Settings => state.windows.settings.as_ref().map(|w| w.section.clone()),
        _ => None,
    };
    let is_open = |id: &str| open.as_deref() == Some(id);
    for (index, group) in GROUPS.iter().enumerate() {
        if let [section] = group.sections {
            let selected = is_open(section.id);
            let response = row(
                ui,
                colors,
                width,
                Row {
                    selected,
                    ..Row::new(
                        Id::new(("side-section", section.id)),
                        ROW,
                        Some(group.icon),
                        t(group.title),
                    )
                },
            );
            reveal_if(ui, &response, selected);
            if response.clicked() {
                commands.push(Command::ShowSettings(Some(section.id)));
            }
            continue;
        }
        let expanded = state.side.expanded.get(index).copied().unwrap_or(false);
        let holds_open = group.sections.iter().any(|sec| is_open(sec.id));
        // Folded over the open page, the group stands for it.
        let selected = holds_open && !expanded;
        let response = row(
            ui,
            colors,
            width,
            Row {
                selected,
                trailing: Some(if expanded {
                    icons::CHEVRON_DOWN
                } else {
                    icons::CHEVRON_RIGHT
                }),
                ..Row::new(
                    Id::new(("side-group", index)),
                    ROW,
                    Some(group.icon),
                    t(group.title),
                )
            },
        );
        reveal_if(ui, &response, selected);
        if response.clicked() {
            // As the window's tree: unfolding a group opens its first page (unless the
            // open page is one of its own); folding it opens nothing.
            if let Some(fold) = state.side.expanded.get_mut(index) {
                *fold = !expanded;
            }
            if !expanded && !holds_open {
                commands.push(Command::ShowSettings(Some(group.sections[0].id)));
            }
        }
        if !expanded {
            continue;
        }
        for section in group.sections {
            let selected = is_open(section.id);
            let response = row(
                ui,
                colors,
                width,
                Row {
                    selected,
                    indent: SUB_INDENT,
                    guide: true,
                    ..Row::new(
                        Id::new(("side-section", section.id)),
                        SUB_ROW,
                        None,
                        t(section.title),
                    )
                },
            );
            reveal_if(ui, &response, selected);
            if response.clicked() {
                commands.push(Command::ShowSettings(Some(section.id)));
            }
        }
    }

    // The rest of the settings menu (`BuildSettingsMenu`): the task menus open beside
    // their rows (their names take the whole row), the folder in Finder, the history in
    // its windows.
    heading(ui, colors, width, t(mac::SidePanelMore));
    let response = row(
        ui,
        colors,
        width,
        Row::new(
            Id::new("side-after-capture"),
            ROW,
            Some(icons::IMAGE_UP),
            t(s::MainMenuBuilder_AfterCaptureTasks),
        ),
    );
    if let Some(command) = crate::menus::popup_right(colors, &response, || {
        let defaults = state.default_task_settings();
        crate::tools::image_effects::after_capture_menu_with_presets(
            state,
            defaults.after_capture_job,
        )
    }) {
        commands.push(command);
    }
    let uploads = !state.app_upload_settings().disable_upload;
    let response = row(
        ui,
        colors,
        width,
        Row {
            enabled: uploads,
            ..Row::new(
                Id::new("side-after-upload"),
                ROW,
                Some(icons::CLOUD_UPLOAD),
                t(s::MainMenuBuilder_AfterUploadTasks),
            )
        },
    );
    if uploads
        && let Some(command) = crate::menus::popup_right(colors, &response, || {
            crate::uploads::after_upload_menu(state.default_task_settings().after_upload_job)
        })
    {
        commands.push(command);
    }
    let elsewhere = [
        (
            "side-screenshots-folder",
            icons::FOLDER_OPEN,
            s::MainMenuBuilder_ScreenshotsFolder,
            Command::OpenScreenshotsFolder,
        ),
        (
            "side-history",
            icons::HISTORY,
            s::MainMenuBuilder_History,
            Command::RunJob(HotkeyType::OpenHistory),
        ),
        (
            "side-image-history",
            icons::IMAGES,
            s::MainMenuBuilder_ImageHistory,
            Command::RunJob(HotkeyType::OpenImageHistory),
        ),
    ];
    for (id, icon, label, command) in elsewhere {
        // A row names a place, not a menu command: no "...".
        let label = t(label).trim_end_matches("...").trim_end_matches('…');
        let response = row(
            ui,
            colors,
            width,
            Row {
                trailing: Some(icons::ARROW_UP_RIGHT),
                ..Row::new(Id::new(id), ROW, Some(icon), label)
            },
        );
        if response.clicked() {
            commands.push(command);
        }
    }

    // About, last.
    ui.add_space(10.0);
    let selected = page == Page::Information;
    let response = row(
        ui,
        colors,
        width,
        Row {
            selected,
            ..Row::new(
                Id::new("side-about"),
                ROW,
                Some(icons::INFO),
                t(mac::SidePanelAbout),
            )
        },
    );
    reveal_if(ui, &response, selected);
    if response.clicked() {
        commands.push(Command::ShowAbout);
    }
    ui.add_space(8.0);
}

/// A small heading over a group of rows.
fn heading(ui: &mut Ui, colors: &Colors, width: f32, text: &str) {
    ui.add_space(14.0);
    let galley = layout(
        ui,
        trimmed(
            text,
            11.0,
            Weight::SemiBold,
            colors.text_secondary(),
            width - 24.0,
            1,
        ),
    );
    let (r, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    let mut painter = ui.painter().clone();
    painter.set_opacity(0.75);
    painter.galley(
        pos2(r.left() + 12.0, r.top()),
        galley,
        colors.text_secondary(),
    );
    ui.add_space(6.0);
}

/// What one row shows.
struct Row<'a> {
    id: Id,
    height: f32,
    icon: Option<char>,
    label: &'a str,
    selected: bool,
    enabled: bool,
    /// How far in the row starts (a section under its group).
    indent: f32,
    /// The thin line down the left of an unfolded group's sections.
    guide: bool,
    /// A mark at the right end: a group's fold, another window.
    trailing: Option<char>,
}

impl<'a> Row<'a> {
    fn new(id: Id, height: f32, icon: Option<char>, label: &'a str) -> Row<'a> {
        Row {
            id,
            height,
            icon,
            label,
            selected: false,
            enabled: true,
            indent: 0.0,
            guide: false,
            trailing: None,
        }
    }
}

/// One row: an icon and a label, the chosen one filled, with the accent on its icon.
/// The full label shows under the pointer when it does not fit.
fn row(ui: &mut Ui, colors: &Colors, width: f32, spec: Row) -> egui::Response {
    let (outer, _) = ui.allocate_exact_size(vec2(width, spec.height), Sense::hover());
    ui.add_space(GAP);
    let r = Rect::from_min_max(pos2(outer.left() + spec.indent, outer.top()), outer.max);
    let sense = if spec.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(r, spec.id, sense);
    let mut painter = ui.painter().clone();
    if !spec.enabled {
        painter.set_opacity(0.45);
    }
    if spec.guide {
        painter.vline(
            outer.left() + 20.0,
            outer.top() - GAP / 2.0..=outer.bottom() + GAP / 2.0,
            Stroke::new(1.0, colors.border()),
        );
        // The chosen section marks its stretch of the line with the accent, as the
        // chosen rows with an icon have it on their icon.
        if spec.selected {
            painter.vline(
                outer.left() + 20.0,
                outer.top() + 6.0..=outer.bottom() - 6.0,
                Stroke::new(2.0, colors.accent_start()),
            );
        }
    }
    let hovered = spec.enabled && (response.hovered() || response.is_pointer_button_down_on());
    // On the light panel (the hover colour itself) the chosen row takes the border's
    // grey, so it still stands out.
    let (chosen, hover) = if colors.dark {
        (
            colors.control_background_hover(),
            colors.control_background(),
        )
    } else {
        (
            colors.border(),
            crate::widgets::lerp_color(colors.background_toolbar(), colors.border(), 0.5),
        )
    };
    if spec.selected {
        painter.rect_filled(r, 8.0, chosen);
    } else if hovered {
        painter.rect_filled(r, 8.0, hover);
    }
    let cy = r.center().y;
    let text_left = match spec.icon {
        Some(icon) => {
            let icon_color = if spec.selected {
                colors.accent_start()
            } else {
                colors.text_secondary()
            };
            paint_icon(
                ui,
                &painter,
                pos2(r.left() + 12.0 + 8.0, cy),
                icon,
                16.0,
                icon_color,
            );
            r.left() + 40.0
        }
        None => r.left() + 12.0,
    };
    let trailing_w = if spec.trailing.is_some() { 22.0 } else { 0.0 };
    let weight = if spec.selected {
        Weight::SemiBold
    } else {
        Weight::Normal
    };
    let text_color = if spec.selected || hovered {
        colors.text()
    } else {
        colors.text_secondary()
    };
    let text = layout(
        ui,
        trimmed(
            spec.label,
            13.0,
            weight,
            text_color,
            (r.right() - 8.0 - trailing_w - text_left).max(10.0),
            1,
        ),
    );
    let elided = text.elided;
    painter.galley(pos2(text_left, cy - text.size().y / 2.0), text, text_color);
    if let Some(glyph) = spec.trailing {
        let mut faint = painter.clone();
        faint.set_opacity(if hovered { 0.9 } else { 0.6 } * painter.opacity());
        paint_icon(
            ui,
            &faint,
            pos2(r.right() - 10.0 - 7.0, cy),
            glyph,
            14.0,
            colors.text_secondary(),
        );
    }
    if elided {
        response.on_hover_text(spec.label)
    } else {
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::about_window::AboutUpdate;
    use crate::qa::{Look, state};

    fn graphite() -> AppState {
        let mut state = state(
            &Look {
                skin: crate::theme::GRAPHITE.into(),
                ..Look::default()
            },
            false,
        );
        // Never the real app: nothing asks the release host, whatever the build.
        state.mode = crate::app::Mode::Shot;
        state
    }

    fn group_of(section: &str) -> usize {
        GROUPS
            .iter()
            .position(|g| g.sections.iter().any(|s| s.id == section))
            .expect("a section of the tree")
    }

    /// Under the Graphite skin a settings section opens in the main window, over the
    /// tab, with its group unfolded in the panel; a tab, the same one too, closes it as
    /// its window closes.
    #[test]
    fn graphite_opens_a_settings_section_in_the_main_window() {
        let mut state = graphite();
        state.run(
            Command::ShowSettings(Some("task.capture")),
            &egui::Context::default(),
        );
        assert!(state.windows.settings.as_ref().is_some_and(|w| w.embedded));
        assert_eq!(state.page(), Page::Settings);
        assert!(!state.shows_tab(Tab::MySpace));
        assert!(state.side.expanded[group_of("task.capture")]);
        assert!(state.side.reveal);
        state.run(Command::SelectTab(Tab::MySpace), &egui::Context::default());
        assert!(state.windows.settings.is_none());
        assert!(state.shows_tab(Tab::MySpace));
    }

    /// The other skins keep the settings window, which a tab does not close.
    #[test]
    fn the_other_skins_keep_the_settings_window() {
        let mut state = state(&Look::default(), false);
        state.show_settings(Some("task.capture"));
        assert!(state.windows.settings.as_ref().is_some_and(|w| !w.embedded));
        assert_eq!(state.page(), Page::Tab(Tab::MySpace));
        state.select_tab(Tab::Upload);
        assert!(state.windows.settings.is_some());
        assert!(state.shows_tab(Tab::Upload));
    }

    /// About is the Information page under the Graphite skin: the settings page gives
    /// way to it, and it to a settings section.
    #[test]
    fn about_is_the_information_page_under_graphite() {
        let mut state = graphite();
        let ctx = egui::Context::default();
        state.run(Command::ShowSettings(None), &ctx);
        state.run(Command::ShowAbout, &ctx);
        assert_eq!(state.page(), Page::Information);
        assert!(state.windows.settings.is_none());
        assert!(state.windows.about.is_none());
        assert_eq!(
            state.side.information.as_ref().map(|p| p.update.clone()),
            Some(AboutUpdate::Hidden)
        );
        state.run(Command::ShowSettings(Some("hotkeys.list")), &ctx);
        assert_eq!(state.page(), Page::Settings);
        assert!(state.side.information.is_none());
    }

    /// A settings window opened before the skin turned to Graphite moves into the main
    /// window when the panel opens a section, without the search it had.
    #[test]
    fn an_open_settings_window_moves_into_the_main_window() {
        let mut state = state(&Look::default(), false);
        state.show_settings(Some("application.theme"));
        if let Some(window) = state.windows.settings.as_mut() {
            window.search = "proxy".into();
        }
        state.options.set_skin(crate::theme::GRAPHITE);
        state.refresh_theme(None);
        assert!(state.windows.settings.as_ref().is_some_and(|w| !w.embedded));
        state.show_settings(Some("application.proxy"));
        let window = state.windows.settings.as_ref().unwrap();
        assert!(window.embedded);
        assert!(window.search.is_empty());
        assert_eq!(window.section, "application.proxy");
    }

    /// Leaving the Graphite skin moves the page shown in the window to a window of its
    /// own, on the same section; the Information page goes.
    #[test]
    fn another_skin_moves_the_page_into_its_own_window() {
        let mut state = graphite();
        state.show_settings(Some("application.theme"));
        state.options.set_skin("default");
        state.refresh_theme(None);
        let window = state.windows.settings.as_ref().unwrap();
        assert!(!window.embedded);
        assert_eq!(window.section, "application.theme");
        assert_eq!(state.page(), Page::Tab(Tab::MySpace));

        let mut state = graphite();
        state.show_information();
        state.options.set_skin("default");
        state.refresh_theme(None);
        assert!(state.side.information.is_none());
        assert_eq!(state.page(), Page::Tab(Tab::MySpace));
    }
}

//! `HistoryWindow`: every upload in History.db as a list (icon, date, file name, address),
//! newest first; search by name and tags (or the advanced filters: name, address, type,
//! host, date range), favourites, statistics, folder import, the preview of the selected
//! picture (its file, else its address), and the context menu and keys of
//! `history.rs`. The date fields of the advanced filters are text in the culture's short
//! date (Avalonia's date pickers have no egui counterpart).

use std::sync::Arc;

use chrono::NaiveDate;
use egui::{Align2, Id, Rect, Sense, TextureHandle, Ui, pos2, vec2};
use modlight_core::history::{HistoryItem, HistoryStore};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, history as hl, t};
use modlight_core::names::file_helpers;
use modlight_tools::history::{self as logic, Filter};
use modlight_tools::settings::HistorySettings;

use super::history::{self as shared, Act, HistoryData, HistoryHost, Modal};
use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::AppState;
use crate::theme::Weight;

/// What the preview shows.
#[derive(Clone)]
pub enum Preview {
    NoSelection,
    Loading,
    /// A picture file read while the next frame is drawn (QA renders, which draw once).
    Pending(String),
    Image(TextureHandle),
    Unavailable,
}

/// The advanced filters' fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Advanced {
    pub file_name: String,
    pub url: String,
    pub type_on: bool,
    pub kind: Option<usize>,
    pub host_on: bool,
    pub host: String,
    pub date_on: bool,
    pub from: String,
    pub to: String,
}

/// A date in the culture's short pattern ("M/d/yyyy" or "dd.MM.yyyy").
pub fn short_date(date: NaiveDate) -> String {
    use chrono::Datelike;
    if modlight_core::culture::Culture::current().decimal == ',' {
        format!("{:02}.{:02}.{}", date.day(), date.month(), date.year())
    } else {
        format!("{}/{}/{}", date.month(), date.day(), date.year())
    }
}

/// Read a short date back (either pattern, or ISO).
pub fn parse_date(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    for pattern in ["%m/%d/%Y", "%d.%m.%Y", "%Y-%m-%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(text, pattern) {
            return Some(date);
        }
    }
    None
}

fn today() -> NaiveDate {
    if modlight_core::culture::thread_utc() {
        chrono::Utc::now().date_naive()
    } else {
        chrono::Local::now().date_naive()
    }
}

impl Advanced {
    fn new() -> Advanced {
        let today = short_date(today());
        Advanced {
            file_name: String::new(),
            url: String::new(),
            type_on: false,
            kind: None,
            host_on: false,
            host: String::new(),
            date_on: false,
            from: today.clone(),
            to: today,
        }
    }
}

pub struct HistoryWindow {
    pub key: u64,
    pub data: HistoryData,
    pub settings: HistorySettings,
    pub search: String,
    pub advanced: bool,
    pub adv: Advanced,
    pub types: Vec<String>,
    pub hosts: Vec<String>,
    /// Indices into `data.items`.
    pub filtered: Vec<usize>,
    pub selected: Vec<i64>,
    anchor: Option<usize>,
    pub list_width: f32,
    pending_filter: Option<f64>,
    pub preview: Preview,
    preview_for: Option<i64>,
    preview_version: u64,
    started: bool,
    /// The settings dialog's toggles while it is open.
    draft: (bool, bool),
    scroll_to: Option<usize>,
}

impl HistoryWindow {
    pub fn new(
        key: u64,
        store: Option<Arc<HistoryStore>>,
        settings: HistorySettings,
    ) -> HistoryWindow {
        let search = if settings.remember_search_text {
            settings.search_text.clone()
        } else {
            String::new()
        };
        let list_width = if settings.remember_window_state {
            settings.history_list_width.max(500.0) as f32
        } else {
            650.0
        };
        HistoryWindow {
            key,
            data: HistoryData::new(store, false),
            settings,
            search,
            advanced: false,
            adv: Advanced::new(),
            types: Vec::new(),
            hosts: Vec::new(),
            filtered: Vec::new(),
            selected: Vec::new(),
            anchor: None,
            list_width,
            pending_filter: None,
            preview: Preview::NoSelection,
            preview_for: None,
            preview_version: 0,
            started: false,
            draft: (false, false),
            scroll_to: None,
        }
    }

    /// QA renders: rows given, the first selected.
    pub fn qa_load(&mut self, state: &mut AppState, items: Vec<HistoryItem>) {
        self.started = true;
        self.data.set_items(items);
        self.refilter(state);
    }

    /// `CreateCurrentFilter`.
    fn filter(&self) -> Filter {
        if !self.advanced {
            return Filter {
                file_name: self.search.clone(),
                favorites: self.settings.favorites,
                ..Filter::default()
            };
        }
        Filter {
            file_name: self.adv.file_name.clone(),
            url: self.adv.url.clone(),
            filter_type: self.adv.type_on,
            kind: self.adv.kind.and_then(|i| self.types.get(i).cloned()),
            filter_host: self.adv.host_on,
            host: self.adv.host.clone(),
            filter_date: self.adv.date_on,
            from_date: Some(parse_date(&self.adv.from).unwrap_or_else(today)),
            to_date: Some(parse_date(&self.adv.to).unwrap_or_else(today)),
            ..Filter::default()
        }
    }

    fn item_at(&self, row: usize) -> Option<&HistoryItem> {
        self.filtered.get(row).and_then(|i| self.data.items.get(*i))
    }

    fn row_of(&self, id: i64) -> Option<usize> {
        self.filtered
            .iter()
            .position(|i| self.data.items.get(*i).is_some_and(|item| item.id == id))
    }

    fn select_only(&mut self, state: &mut AppState, row: usize) {
        if let Some(item) = self.item_at(row) {
            self.selected = vec![item.id];
            self.anchor = Some(row);
            self.update_preview(state);
        }
    }

    /// `UpdatePreviewAsync` for the first selected row.
    fn update_preview(&mut self, state: &mut AppState) {
        let first = self.selected.first().copied();
        if first == self.preview_for && !matches!(self.preview, Preview::Loading) {
            return;
        }
        self.preview_for = first;
        self.preview_version += 1;
        let Some(item) = first.and_then(|id| self.data.find(id)).cloned() else {
            self.preview = Preview::NoSelection;
            return;
        };
        let local = !item.file_path.trim().is_empty()
            && std::path::Path::new(&item.file_path).is_file()
            && file_helpers::is_image_file(&item.file_path);
        let remote =
            !local && !item.url.trim().is_empty() && file_helpers::is_image_file(&item.url);
        if !local && !remote {
            self.preview = Preview::Unavailable;
            return;
        }
        let headless = state.env.qa || state.mode != crate::app::Mode::Normal;
        if local && headless {
            self.preview = Preview::Pending(item.file_path.clone());
            return;
        }
        if remote && headless {
            self.preview = Preview::Unavailable;
            return;
        }
        self.preview = Preview::Loading;
        let (key, version) = (self.key, self.preview_version);
        let done = move |image: Option<image::RgbaImage>| {
            move |state: &mut AppState, ctx: &egui::Context| {
                super::with_window::<HistoryWindow, _>(state, key, |window, _| {
                    if window.preview_version != version {
                        return;
                    }
                    window.preview = match image {
                        Some(image) => Preview::Image(kit::texture(
                            ctx,
                            &format!("history-preview-{key}"),
                            &image,
                            false,
                        )),
                        None => Preview::Unavailable,
                    };
                });
            }
        };
        if local {
            let path = item.file_path.clone();
            state.rt.spawn_blocking(move || {
                let image = super::files::load_image(std::path::Path::new(&path)).map(fit_texture);
                done(image)
            });
        } else {
            let url = item.url.clone();
            state.rt.spawn(async move {
                let image = logic::fetch_preview(&url).await.map(fit_texture);
                done(image)
            });
        }
    }

    /// Shift and Cmd clicks (`SelectionMode="Multiple"`).
    fn click_row(&mut self, state: &mut AppState, row: usize, modifiers: egui::Modifiers) {
        let Some(id) = self.item_at(row).map(|i| i.id) else {
            return;
        };
        if modifiers.shift
            && let Some(anchor) = self.anchor
        {
            let (a, b) = (anchor.min(row), anchor.max(row));
            self.selected = (a..=b)
                .filter_map(|r| self.item_at(r).map(|i| i.id))
                .collect();
        } else if modifiers.command {
            if let Some(at) = self.selected.iter().position(|s| *s == id) {
                self.selected.remove(at);
            } else {
                self.selected.push(id);
            }
            self.anchor = Some(row);
        } else {
            self.select_only(state, row);
            return;
        }
        // The window's order, first row first.
        let order: Vec<i64> = self
            .filtered
            .iter()
            .filter_map(|i| self.data.items.get(*i).map(|item| item.id))
            .filter(|id| self.selected.contains(id))
            .collect();
        self.selected = order;
        self.update_preview(state);
    }

    fn save_settings(&self, state: &mut AppState) {
        state.save_app_options("HistorySettings", &self.settings);
    }

    fn settings_dialog(&mut self, ui: &mut Ui, state: &mut AppState, full: Rect) {
        let colors = state.colors.clone();
        let card = Rect::from_center_size(
            full.center(),
            vec2(440.0, 18.0 * 2.0 + 20.0 + 30.0 * 2.0 + 40.0 + 14.0 * 3.0),
        );
        kit::panel(ui, &colors, card);
        let inner = card.shrink(18.0);
        let rows = kit::rows(
            inner,
            &[
                Track::Px(20.0),
                Track::Px(30.0),
                Track::Px(30.0),
                Track::Px(40.0),
            ],
            14.0,
        );
        kit::heading(ui, &colors, rows[0], t(hl::HistoryWindows_History_settings));
        for (row, label, value) in [
            (
                rows[1],
                t(hl::HistoryWindows_Remember_search_text),
                &mut self.draft.0,
            ),
            (
                rows[2],
                t(hl::HistoryWindows_Remember_window_size_and_position),
                &mut self.draft.1,
            ),
        ] {
            kit::text_at(
                ui,
                row,
                Align2::LEFT_CENTER,
                label,
                13.0,
                Weight::Normal,
                colors.text(),
            );
            kit::toggle_at(
                ui,
                &colors,
                row,
                Id::new(("history-setting", self.key, label)),
                value,
                true,
            );
        }
        let done = t(hl::HistoryWindows_Done);
        let r = kit::right_aligned(ui, &colors, rows[3], &[(None, Some(done))], 8.0)[0];
        if kit::button_at(ui, &colors, r, None, Some(done), true).clicked() {
            // `OnSaveSettingsClick`.
            self.settings.remember_search_text = self.draft.0;
            self.settings.remember_window_state = self.draft.1;
            self.settings.search_text = if self.settings.remember_search_text {
                self.search.clone()
            } else {
                String::new()
            };
            self.save_settings(state);
            self.data.modal = None;
        }
    }

    fn advanced_panel(&mut self, ui: &mut Ui, state: &mut AppState, rect: Rect) -> bool {
        let colors = state.colors.clone();
        let mut changed = false;
        let inner = rect.shrink(12.0);
        let mut y = inner.top();
        let line =
            |y: f32, h: f32| Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), h));
        let head = line(y, 32.0);
        kit::heading(ui, &colors, head, t(hl::HistoryWindows_Advanced_filters));
        let reset = Rect::from_min_size(pos2(head.right() - 32.0, head.top()), vec2(32.0, 32.0));
        if kit::button_at(ui, &colors, reset, Some(icons::ROTATE_CCW), None, true)
            .on_hover_text(t(hl::HistoryWindows_Reset_filters))
            .clicked()
        {
            self.adv = Advanced::new();
            changed = true;
        }
        y += 32.0 + 8.0;
        let key = self.key;
        changed |= kit::text_box_at(
            ui,
            &colors,
            line(y, 40.0),
            Id::new(("history-adv-name", key)),
            &mut self.adv.file_name,
            t(hl::HistoryWindows_File_name),
            true,
            false,
        )
        .changed();
        y += 48.0;
        changed |= kit::text_box_at(
            ui,
            &colors,
            line(y, 40.0),
            Id::new(("history-adv-url", key)),
            &mut self.adv.url,
            t(hl::HistoryWindows_URL),
            true,
            false,
        )
        .changed();
        y += 48.0;
        let type_label = t(hl::HistoryWindows_Type);
        let c = kit::columns(
            line(y, 40.0),
            &[
                Track::Px(kit::radio_width(ui, type_label) + 8.0),
                Track::Star(1.0),
            ],
            8.0,
        );
        changed |= kit::check_normal_at(ui, &colors, c[0], &mut self.adv.type_on, type_label, true);
        let types: Vec<(usize, String)> = self.types.iter().cloned().enumerate().collect();
        let current = self.adv.kind.unwrap_or(usize::MAX);
        if let Some(i) = kit::combo_at(
            ui,
            &colors,
            c[1],
            &format!("history-adv-type-{key}"),
            &current,
            &types,
            true,
        ) {
            self.adv.kind = Some(i);
            changed = true;
        }
        if self.adv.kind.is_none() {
            kit::text_at(
                ui,
                c[1].shrink2(vec2(12.0, 0.0)),
                Align2::LEFT_CENTER,
                t(hl::HistoryWindows_All_types),
                13.0,
                Weight::Normal,
                colors.text_secondary().gamma_multiply(0.75),
            );
        }
        y += 48.0;
        let host_label = t(hl::HistoryWindows_Host);
        let c = kit::columns(
            line(y, 40.0),
            &[
                Track::Px(kit::radio_width(ui, host_label) + 8.0),
                Track::Star(1.0),
            ],
            8.0,
        );
        changed |= kit::check_normal_at(ui, &colors, c[0], &mut self.adv.host_on, host_label, true);
        changed |= kit::editable_combo_at(
            ui,
            &colors,
            c[1],
            Id::new(("history-adv-host", key)),
            &mut self.adv.host,
            &self.hosts,
            true,
        );
        if self.adv.host.is_empty() {
            kit::text_at(
                ui,
                c[1].shrink2(vec2(8.0, 0.0)),
                Align2::LEFT_CENTER,
                t(hl::HistoryWindows_All_hosts),
                13.0,
                Weight::Normal,
                colors.text_secondary().gamma_multiply(0.75),
            );
        }
        y += 48.0;
        changed |= kit::check_normal_at(
            ui,
            &colors,
            line(y, 30.0),
            &mut self.adv.date_on,
            t(hl::HistoryWindows_Date_range),
            true,
        );
        y += 38.0;
        for (index, value) in [&mut self.adv.from, &mut self.adv.to]
            .into_iter()
            .enumerate()
        {
            let r = line(y, 40.0);
            if kit::text_box_at(
                ui,
                &colors,
                r,
                Id::new(("history-adv-date", key, index)),
                value,
                "",
                true,
                false,
            )
            .changed()
                && parse_date(value).is_some()
            {
                changed = true;
            }
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(r.right() - 18.0, r.center().y),
                icons::CALENDAR,
                14.0,
                colors.text_secondary(),
            );
            y += 48.0;
        }
        changed
    }

    /// The advanced panel's height.
    fn advanced_height() -> f32 {
        12.0 * 2.0 + 32.0 + 8.0 + 48.0 * 4.0 + 38.0 + 48.0 * 2.0 - 8.0
    }
}

/// A preview the GPU can take (at most 2048 on a side).
fn fit_texture(image: image::RgbaImage) -> image::RgbaImage {
    let max = 2048u32;
    if image.width() <= max && image.height() <= max {
        return image;
    }
    let scale = max as f32 / image.width().max(image.height()) as f32;
    image::imageops::thumbnail(
        &image,
        ((image.width() as f32 * scale) as u32).max(1),
        ((image.height() as f32 * scale) as u32).max(1),
    )
}

impl HistoryHost for HistoryWindow {
    fn data(&mut self) -> &mut HistoryData {
        &mut self.data
    }

    /// `ApplyFilterAsync` (plus the type and host lists after a load).
    fn refilter(&mut self, state: &mut AppState) {
        let (types, hosts) = logic::types_and_hosts(&self.data.items);
        if types != self.types {
            self.adv.kind = self
                .adv
                .kind
                .and_then(|i| self.types.get(i))
                .and_then(|k| types.iter().position(|t| t == k));
            self.types = types;
        }
        self.hosts = hosts;
        self.filtered = self.filter().apply(&self.data.items);
        // Kept rows stay selected; else the first row (`SelectedIndex = 0`).
        let listed: Vec<i64> = self
            .filtered
            .iter()
            .filter_map(|i| self.data.items.get(*i).map(|item| item.id))
            .collect();
        self.selected.retain(|id| listed.contains(id));
        if self.selected.is_empty()
            && let Some(first) = listed.first()
        {
            self.selected = vec![*first];
            self.anchor = Some(0);
        }
        self.preview_for = None;
        self.update_preview(state);
    }

    fn selected_ids(&self) -> Vec<i64> {
        self.selected.clone()
    }

    fn listed_ids(&self) -> Vec<i64> {
        self.filtered
            .iter()
            .filter_map(|i| self.data.items.get(*i).map(|item| item.id))
            .collect()
    }

    fn key(&self) -> u64 {
        self.key
    }
}

impl ToolWindow for HistoryWindow {
    fn spec(&self) -> WindowSpec {
        let title = if self.data.loaded {
            format!(
                "{} ({})",
                t(hl::HistoryWindows_ShareX_History),
                format(
                    t(hl::HistoryWindow_ItemsFormat),
                    &[Arg::Int(self.filtered.len() as i64)]
                )
            )
        } else {
            t(hl::HistoryWindows_ShareX_History).to_string()
        };
        shared::remembered_spec(
            title,
            vec2(1300.0, 700.0),
            vec2(850.0, 560.0),
            self.settings.remember_window_state,
            &self.settings.window_state,
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        let now = state.time;
        if !self.started {
            self.started = true;
            shared::refresh(self, state);
        }
        shared::track_window(
            ui,
            &mut self.settings.window_state,
            self.settings.remember_window_state,
        );
        if let Some(at) = self.pending_filter {
            if now >= at {
                self.pending_filter = None;
                self.refilter(state);
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        kit::fill(ui, &colors);
        let modal_open = self.data.modal.is_some();
        let mut close = false;
        if !modal_open {
            let (escape, f5, select_all) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::F5),
                    i.key_pressed(egui::Key::A) && i.modifiers.command,
                )
            });
            if f5 {
                shared::refresh(self, state);
            }
            if escape {
                close = true;
            }
            if !shared::typing(ui) {
                if select_all {
                    self.selected = self.listed_ids();
                }
                if let Some(act) = shared::key_act(ui) {
                    shared::run(self, state, act);
                } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some(item) = self.selected.first().and_then(|id| self.data.find(*id)) {
                        match logic::open_target(item) {
                            Some(logic::OpenTarget::Url(url)) => state.open_url(&url),
                            Some(logic::OpenTarget::File(path)) => state
                                .effects
                                .push(crate::state::Effect::OpenFile(path.into())),
                            None => {}
                        }
                    }
                } else {
                    let (up, down) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::ArrowDown),
                        )
                    });
                    if (up || down) && !self.filtered.is_empty() {
                        let current = self
                            .selected
                            .first()
                            .and_then(|id| self.row_of(*id))
                            .unwrap_or(0);
                        let next = if up {
                            current.saturating_sub(1)
                        } else {
                            (current + 1).min(self.filtered.len() - 1)
                        };
                        self.select_only(state, next);
                        self.scroll_to = Some(next);
                    }
                }
            }
        }

        let full = ui.max_rect();
        let body = Rect::from_min_max(
            pos2(full.left() + 10.0, full.top()),
            pos2(full.right() - 10.0, full.bottom() - 10.0),
        );
        // The toolbar.
        let bar = Rect::from_min_size(body.min, vec2(body.width(), 12.0 * 2.0 + 40.0));
        kit::panel(ui, &colors, bar);
        let inner = bar.shrink(12.0);
        let search = Rect::from_min_size(inner.min, vec2(250.0, 40.0));
        if shared::search_box(
            ui,
            &colors,
            search,
            Id::new(("history-search", self.key)),
            &mut self.search,
        ) {
            if self.settings.remember_search_text {
                self.settings.search_text = self.search.clone();
            }
            self.pending_filter = Some(now + 0.22);
        }
        let items = [
            (Some(icons::FILTER), Some(t(hl::HistoryWindows_Advanced))),
            (Some(icons::STAR), Some(t(hl::HistoryWindows_Favorites))),
            (Some(icons::CHART_BAR), Some(t(hl::HistoryWindows_Stats))),
            (
                Some(icons::FOLDER_SEARCH),
                Some(t(hl::HistoryWindows_Import)),
            ),
            (Some(icons::SETTINGS), Some(t(hl::HistoryWindows_Settings))),
        ];
        let row = Rect::from_min_max(pos2(search.right() + 8.0, inner.top()), inner.max);
        let rects = kit::left_aligned(ui, &colors, row, &items, 8.0);
        if shared::toggle_button(
            ui,
            &colors,
            rects[0],
            icons::FILTER,
            items[0].1.unwrap_or(""),
            self.advanced,
        ) {
            self.advanced = !self.advanced;
            self.refilter(state);
        }
        if shared::toggle_button(
            ui,
            &colors,
            rects[1],
            icons::STAR,
            items[1].1.unwrap_or(""),
            self.settings.favorites,
        ) {
            self.settings.favorites = !self.settings.favorites;
            self.save_settings(state);
            self.refilter(state);
        }
        if kit::button_at(ui, &colors, rects[2], items[2].0, items[2].1, true).clicked() {
            shared::open_stats(self, state);
        }
        if kit::button_at(ui, &colors, rects[3], items[3].0, items[3].1, true).clicked() {
            shared::open_import(self, false);
        }
        if kit::button_at(ui, &colors, rects[4], items[4].0, items[4].1, true).clicked() {
            self.draft = (
                self.settings.remember_search_text,
                self.settings.remember_window_state,
            );
            self.data.modal = Some(Modal::Settings);
        }

        // The list, the splitter, the preview.
        let main = Rect::from_min_max(pos2(body.left(), bar.bottom() + 10.0), body.max);
        let list_w = self
            .list_width
            .clamp(300.0, (main.width() - 220.0).max(300.0));
        let list = Rect::from_min_size(main.min, vec2(list_w, main.height()));
        let splitter =
            Rect::from_min_size(pos2(list.right(), main.top()), vec2(10.0, main.height()));
        let right = Rect::from_min_max(pos2(splitter.right(), main.top()), main.max);
        let drag = ui.interact(
            splitter,
            Id::new(("history-splitter", self.key)),
            Sense::drag(),
        );
        if drag.hovered() || drag.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if drag.dragged() {
            self.list_width = (list_w + drag.drag_delta().x).max(300.0);
        }
        if drag.drag_stopped() {
            self.settings.history_list_width = self.list_width.max(500.0) as f64;
        }
        self.list(ui, state, list);

        // Right: the advanced filters over the preview.
        let mut preview_area = right;
        if self.advanced {
            let h = Self::advanced_height().min(right.height() - 60.0);
            let panel = Rect::from_min_size(right.min, vec2(right.width(), h));
            if self.advanced_panel(ui, state, panel) {
                self.pending_filter = Some(now + 0.22);
            }
            ui.painter().hline(
                panel.x_range(),
                panel.bottom(),
                egui::Stroke::new(1.0, colors.border()),
            );
            preview_area = Rect::from_min_max(pos2(right.left(), panel.bottom() + 1.0), right.max);
        }
        if let Preview::Pending(path) = &self.preview {
            self.preview =
                match super::files::load_image(std::path::Path::new(path)).map(fit_texture) {
                    Some(image) => Preview::Image(kit::texture(
                        ui.ctx(),
                        &format!("history-preview-{}", self.key),
                        &image,
                        false,
                    )),
                    None => Preview::Unavailable,
                };
        }
        match &self.preview {
            Preview::NoSelection => kit::placeholder(
                ui,
                &colors,
                preview_area,
                icons::MOUSE_POINTER_CLICK,
                t(hl::HistoryWindows_Select_an_item_to_preview),
                40.0,
            ),
            other => {
                kit::paint_checkered_big(ui, &colors, preview_area);
                ui.painter().rect_stroke(
                    preview_area,
                    4.0,
                    egui::Stroke::new(1.0, colors.border()),
                    egui::StrokeKind::Inside,
                );
                match other {
                    Preview::Image(texture) => {
                        kit::image_uniform(ui, texture, preview_area.shrink(1.0), false);
                        let click = ui.interact(
                            preview_area,
                            Id::new(("history-preview", self.key)),
                            Sense::click(),
                        );
                        if click.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if click.clicked() {
                            shared::run(self, state, Act::ShowImage);
                        }
                    }
                    _ => kit::placeholder(
                        ui,
                        &colors,
                        preview_area,
                        icons::FILE,
                        t(hl::HistoryWindows_No_image_preview),
                        38.0,
                    ),
                }
            }
        }

        if modal_open {
            let mut child = kit::child(ui, full);
            shared::modals(self, &mut child, state, |window, ui, state, full| {
                window.settings_dialog(ui, state, full)
            });
        }
        close
    }

    fn closed(&mut self, state: &mut AppState) {
        self.preview_version += 1;
        if self.settings.remember_window_state {
            self.settings.history_list_width = self.list_width.max(500.0) as f64;
        }
        self.save_settings(state);
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl HistoryWindow {
    /// The list panel: column headers, rows (only the visible ones are laid out).
    fn list(&mut self, ui: &mut Ui, state: &mut AppState, rect: Rect) {
        let colors = state.colors.clone();
        kit::panel(ui, &colors, rect);
        let header = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(rect.width() - 2.0, 36.0));
        ui.painter().rect_filled(
            header,
            egui::CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            },
            colors.control_background(),
        );
        ui.painter().hline(
            header.x_range(),
            header.bottom(),
            egui::Stroke::new(1.0, colors.border()),
        );
        let columns = |r: Rect| {
            kit::columns(
                r.shrink2(vec2(10.0, 0.0)),
                &[
                    Track::Px(18.0),
                    Track::Px(105.0),
                    Track::Star(0.9),
                    Track::Star(1.5),
                ],
                8.0,
            )
        };
        let heads = columns(header);
        for (index, label) in [
            t(hl::HistoryWindows_Date),
            t(hl::HistoryWindows_File_name),
            t(hl::HistoryWindows_URL),
        ]
        .iter()
        .enumerate()
        {
            kit::text_at(
                ui,
                heads[index + 1],
                Align2::LEFT_CENTER,
                label,
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
            );
        }
        let body = Rect::from_min_max(
            pos2(rect.left() + 1.0, header.bottom() + 1.0),
            pos2(rect.right() - 1.0, rect.bottom() - 1.0),
        );
        if !self.data.loaded {
            shared::state_message(
                ui,
                &colors,
                body,
                icons::LOADER_CIRCLE,
                t(hl::HistoryWindows_Loading),
                true,
            );
            return;
        }
        if self.filtered.is_empty() {
            let text = if self.data.items.is_empty() {
                t(hl::HistoryWindows_No_history_items)
            } else {
                t(hl::HistoryWindow_NoItemsMatchCurrentFilters)
            };
            shared::state_message(ui, &colors, body, icons::HISTORY, text, false);
            return;
        }
        let mut area = kit::column(ui, body);
        area.set_clip_rect(body.intersect(ui.clip_rect()));
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("history-list", self.key))
            .auto_shrink([false, false]);
        if let Some(row) = self.scroll_to.take() {
            // Keep the moved-to row in view.
            let top = row as f32 * 34.0;
            let offset = area
                .ctx()
                .data(|d| d.get_temp::<f32>(Id::new(("history-list-offset", self.key))))
                .unwrap_or(0.0);
            let visible = body.height();
            if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + 34.0 > offset + visible {
                scroll = scroll.vertical_scroll_offset(top + 34.0 - visible);
            }
        }
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut double: Option<usize> = None;
        let mut menu: Option<Act> = None;
        let total = self.filtered.len();
        let output = scroll.show_rows(&mut area, 34.0, total, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in range {
                let (r, response) =
                    ui.allocate_exact_size(vec2(body.width(), 34.0), Sense::click());
                let Some(item) = self.item_at(row).cloned() else {
                    continue;
                };
                let selected = self.selected.contains(&item.id);
                if selected {
                    ui.painter()
                        .rect_filled(r, 0.0, colors.accent_start().gamma_multiply(0.22));
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(r, 0.0, colors.control_background_hover());
                }
                let c = columns(r);
                crate::widgets::paint_icon(
                    ui,
                    ui.painter(),
                    c[0].center(),
                    logic::icon(&item),
                    16.0,
                    colors.text_secondary(),
                );
                let cell = |ui: &Ui, rect: Rect, text: &str, color| {
                    let galley = crate::widgets::layout(
                        ui,
                        crate::widgets::trimmed(
                            text,
                            13.0,
                            Weight::Normal,
                            color,
                            rect.width().max(1.0),
                            1,
                        ),
                    );
                    ui.painter().galley(
                        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                };
                cell(ui, c[1], &logic::date_text(&item), colors.text());
                cell(ui, c[2], &item.file_name, colors.text());
                cell(ui, c[3], &item.url, colors.text_secondary());
                if response.clicked() {
                    clicked = Some((row, ui.input(|i| i.modifiers)));
                }
                if response.double_clicked() {
                    double = Some(row);
                }
                if response.secondary_clicked() && !selected {
                    clicked = Some((row, egui::Modifiers::NONE));
                }
                let selection: Vec<HistoryItem> = if selected {
                    self.selected
                        .iter()
                        .filter_map(|id| self.data.find(*id).cloned())
                        .collect()
                } else {
                    vec![item.clone()]
                };
                if let Some(act) = shared::context_menu(&colors, &response, || {
                    let refs: Vec<&HistoryItem> = selection.iter().collect();
                    shared::list_menu(&refs, true)
                }) {
                    menu = Some(act);
                }
            }
        });
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                Id::new(("history-list-offset", self.key)),
                output.state.offset.y,
            )
        });
        if let Some((row, modifiers)) = clicked {
            self.click_row(state, row, modifiers);
        }
        if let Some(row) = double {
            self.select_only(state, row);
            if let Some(item) = self.item_at(row) {
                match logic::open_target(item) {
                    Some(logic::OpenTarget::Url(url)) => state.open_url(&url),
                    Some(logic::OpenTarget::File(path)) => state
                        .effects
                        .push(crate::state::Effect::OpenFile(path.into())),
                    None => {}
                }
            }
        }
        if let Some(act) = menu {
            shared::run(self, state, act);
        }
    }
}

impl AppState {
    /// `TaskHelpers.OpenHistory`.
    pub fn open_history(&mut self) {
        let settings: HistorySettings =
            modlight_tools::settings::read(self.config.get("HistorySettings"));
        let store = self.history_store();
        self.tools
            .open_keyed(|key| Box::new(HistoryWindow::new(key, store, settings)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_dates() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 4).unwrap();
        assert_eq!(parse_date("3/4/2026"), Some(date));
        assert_eq!(parse_date("04.03.2026"), Some(date));
        assert_eq!(parse_date("2026-03-04"), Some(date));
        assert_eq!(parse_date("nope"), None);
        modlight_core::culture::set_thread_culture(Some(modlight_core::culture::Culture::polish()));
        assert_eq!(short_date(date), "04.03.2026");
        modlight_core::culture::set_thread_culture(
            Some(modlight_core::culture::Culture::english()),
        );
        assert_eq!(short_date(date), "3/4/2026");
        modlight_core::culture::set_thread_culture(None);
    }
}

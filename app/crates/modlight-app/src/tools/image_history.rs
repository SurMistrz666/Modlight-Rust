//! `ImageHistoryWindow`: the history's pictures as thumbnails in rows, newest first, in
//! batches (the next one near the end of the scroll), searched by name and tags,
//! favourites, statistics, folder import, its own settings (thumbnail size and caption,
//! batch size, missing files, pictures only), and the menu and keys of `history.rs`. A
//! click shows the picture in the image viewer with its neighbours.
//!
//! Thumbnails are decoded from the files (upstream asks the Windows shell first; macOS
//! Quick Look thumbnails are not used), only for the cards on screen, four at a time.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, Id, Rect, Sense, TextureHandle, Ui, Vec2, pos2, vec2};
use modlight_core::history::{HistoryItem, HistoryStore};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, history as hl, t};
use modlight_tools::history as logic;
use modlight_tools::settings::{ImageHistorySettings, Size, ThumbnailHeaderPosition};

use super::history::{self as shared, Act, HistoryData, HistoryHost, Modal};
use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::AppState;
use crate::theme::Weight;

/// A card's picture.
#[derive(Clone)]
pub enum Thumb {
    Loading,
    Ready(TextureHandle),
    Failed,
}

/// The settings dialog's fields while it is open.
#[derive(Clone, Debug, Default, PartialEq)]
struct Draft {
    width: i64,
    height: i64,
    header: usize,
    batch: i64,
    auto_load: bool,
    filter_missing: bool,
    image_only: bool,
    remember_search: bool,
    remember_window: bool,
}

/// Margin, border and padding around a thumbnail, both sides (`cardChrome`).
const CARD_CHROME: f32 = 18.0;
const CAPTION: f32 = 17.0;
/// `ImageHistoryThumbnailLoader`'s workers.
const WORKERS: usize = 4;

pub struct ImageHistoryWindow {
    pub key: u64,
    pub data: HistoryData,
    pub settings: ImageHistorySettings,
    pub search: String,
    pending_filter: Option<f64>,
    pub filtered: Vec<usize>,
    /// How many of `filtered` are shown (the batches so far).
    pub loaded: usize,
    columns: usize,
    pub selected: Vec<i64>,
    anchor: Option<usize>,
    thumbs: HashMap<i64, Thumb>,
    in_flight: usize,
    thumb_version: u64,
    started: bool,
    draft: Draft,
    scroll_to_row: Option<usize>,
}

impl ImageHistoryWindow {
    pub fn new(
        key: u64,
        store: Option<Arc<HistoryStore>>,
        settings: ImageHistorySettings,
    ) -> ImageHistoryWindow {
        let search = if settings.remember_search_text {
            settings.search_text.clone()
        } else {
            String::new()
        };
        ImageHistoryWindow {
            key,
            data: HistoryData::new(store, true),
            settings,
            search,
            pending_filter: None,
            filtered: Vec::new(),
            loaded: 0,
            columns: 1,
            selected: Vec::new(),
            anchor: None,
            thumbs: HashMap::new(),
            in_flight: 0,
            thumb_version: 0,
            started: false,
            draft: Draft::default(),
            scroll_to_row: None,
        }
    }

    /// QA renders: rows given.
    pub fn qa_load(&mut self, state: &mut AppState, items: Vec<HistoryItem>) {
        self.started = true;
        self.data.set_items(items);
        self.refilter(state);
    }

    fn thumb_size(&self) -> (f32, f32) {
        (
            self.settings.thumbnail_size.width.max(32) as f32,
            self.settings.thumbnail_size.height.max(32) as f32,
        )
    }

    fn card_size(&self) -> Vec2 {
        let (w, h) = self.thumb_size();
        let caption = if self.settings.thumbnail_header_position == ThumbnailHeaderPosition::None {
            0.0
        } else {
            CAPTION + 3.0
        };
        vec2(w + CARD_CHROME, h + caption + CARD_CHROME)
    }

    fn item_at(&self, index: usize) -> Option<&HistoryItem> {
        self.filtered
            .get(index)
            .and_then(|i| self.data.items.get(*i))
    }

    /// `LoadNextBatch`.
    fn load_next_batch(&mut self) {
        if self.loaded >= self.filtered.len() {
            return;
        }
        let batch = if self.settings.max_item_count <= 0 {
            self.filtered.len() - self.loaded
        } else {
            self.settings.max_item_count as usize
        };
        self.loaded = (self.loaded + batch).min(self.filtered.len());
    }

    fn select_only(&mut self, index: usize) {
        if let Some(id) = self.item_at(index).map(|i| i.id) {
            self.selected = vec![id];
            self.anchor = Some(index);
        }
    }

    /// `SelectRange`.
    fn select_range(&mut self, a: usize, b: usize) {
        let (start, end) = (a.min(b), a.max(b));
        self.selected = (start..=end.min(self.loaded.saturating_sub(1)))
            .filter_map(|i| self.item_at(i).map(|item| item.id))
            .collect();
    }

    /// `SelectIndex`: arrows, Home, End.
    fn select_index(&mut self, index: i64, extend: bool) {
        if self.loaded == 0 {
            return;
        }
        let index = index.clamp(0, self.loaded as i64 - 1) as usize;
        match (extend, self.anchor) {
            (true, Some(anchor)) => self.select_range(anchor, index),
            _ => self.select_only(index),
        }
        self.scroll_to_row = Some(index / self.columns.max(1));
    }

    fn save_settings(&self, state: &mut AppState) {
        state.save_app_options("ImageHistorySettings", &self.settings);
    }

    /// Start loading the thumbnails of these cards (on screen), at most four at a time.
    fn request_thumbs(&mut self, state: &mut AppState, ui: &Ui, indices: &[usize]) {
        let (w, h) = self.thumb_size();
        let headless = state.env.qa || state.mode != crate::app::Mode::Normal;
        for &index in indices {
            let Some(item) = self.item_at(index).cloned() else {
                continue;
            };
            if self.thumbs.contains_key(&item.id) {
                continue;
            }
            if headless {
                // QA renders draw once: the picture now.
                let thumb = match thumbnail(&item.file_path, w as u32, h as u32) {
                    Some(image) => Thumb::Ready(kit::texture(
                        ui.ctx(),
                        &format!("history-thumb-{}", item.id),
                        &image,
                        false,
                    )),
                    None => Thumb::Failed,
                };
                self.thumbs.insert(item.id, thumb);
                continue;
            }
            if self.in_flight >= WORKERS {
                break;
            }
            self.in_flight += 1;
            self.thumbs.insert(item.id, Thumb::Loading);
            let (key, version, id) = (self.key, self.thumb_version, item.id);
            let path = item.file_path.clone();
            state.rt.spawn_blocking(move || {
                let image = thumbnail(&path, w as u32, h as u32);
                move |state: &mut AppState, ctx: &egui::Context| {
                    super::with_window::<ImageHistoryWindow, _>(state, key, |window, _| {
                        window.in_flight = window.in_flight.saturating_sub(1);
                        if window.thumb_version != version {
                            return;
                        }
                        let thumb = match image {
                            Some(image) => Thumb::Ready(kit::texture(
                                ctx,
                                &format!("history-thumb-{id}"),
                                &image,
                                false,
                            )),
                            None => Thumb::Failed,
                        };
                        window.thumbs.insert(id, thumb);
                    });
                }
            });
        }
    }

    fn settings_dialog(&mut self, ui: &mut Ui, state: &mut AppState, full: Rect) {
        let colors = state.colors.clone();
        let rows_h = 40.0 * 4.0 + 30.0 * 5.0 + 10.0 * 8.0;
        let height = (18.0_f32 * 2.0 + 20.0 + 14.0 * 2.0 + rows_h + 40.0)
            .min(650.0)
            .min(full.height() - 32.0);
        let card = Rect::from_center_size(full.center(), vec2(480.0, height));
        kit::panel(ui, &colors, card);
        let rows = kit::rows(
            card.shrink(18.0),
            &[Track::Px(20.0), Track::Star(1.0), Track::Px(40.0)],
            14.0,
        );
        kit::heading(
            ui,
            &colors,
            rows[0],
            t(hl::HistoryWindows_Image_history_settings),
        );
        let key = self.key;
        let d = &mut self.draft;
        let mut area = kit::column(ui, rows[1]);
        area.set_clip_rect(rows[1].intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt(("image-history-settings", key))
            .auto_shrink([false, false])
            .show(&mut area, |ui| {
                let width = rows[1].width() - 8.0;
                let (space, _) = ui.allocate_exact_size(vec2(width, rows_h), Sense::hover());
                let mut y = space.top();
                let line =
                    |y: f32, h: f32| Rect::from_min_size(pos2(space.left(), y), vec2(width, h));
                let number = |ui: &mut Ui,
                              y: f32,
                              label: &str,
                              name: &str,
                              value: &mut i64,
                              min: i64,
                              max: i64,
                              step: i64| {
                    let c =
                        kit::columns(line(y, 40.0), &[Track::Px(170.0), Track::Star(1.0)], 10.0);
                    kit::label(ui, &colors, c[0], label);
                    kit::int_at(
                        ui,
                        &colors,
                        c[1],
                        &format!("image-history-{name}-{key}"),
                        value,
                        min,
                        max,
                        step,
                        true,
                    );
                };
                number(
                    ui,
                    y,
                    t(hl::HistoryWindows_Thumbnail_width),
                    "width",
                    &mut d.width,
                    32,
                    800,
                    10,
                );
                y += 50.0;
                number(
                    ui,
                    y,
                    t(hl::HistoryWindows_Thumbnail_height),
                    "height",
                    &mut d.height,
                    32,
                    800,
                    10,
                );
                y += 50.0;
                let c = kit::columns(line(y, 40.0), &[Track::Px(170.0), Track::Star(1.0)], 10.0);
                kit::label(ui, &colors, c[0], t(hl::HistoryWindows_Thumbnail_header));
                let options: Vec<(usize, String)> = [
                    hl::HistoryWindows_None,
                    hl::HistoryWindows_Top,
                    hl::HistoryWindows_Bottom,
                ]
                .iter()
                .enumerate()
                .map(|(i, k)| (i, t(*k).to_string()))
                .collect();
                if let Some(i) = kit::combo_at(
                    ui,
                    &colors,
                    c[1],
                    &format!("image-history-header-{key}"),
                    &d.header,
                    &options,
                    true,
                ) {
                    d.header = i;
                }
                y += 50.0;
                number(
                    ui,
                    y,
                    t(hl::HistoryWindows_Items_per_batch),
                    "batch",
                    &mut d.batch,
                    0,
                    100_000,
                    50,
                );
                y += 50.0;
                for (label, value) in [
                    (
                        t(hl::HistoryWindows_Load_the_next_batch_near_the_end),
                        &mut d.auto_load,
                    ),
                    (
                        t(hl::HistoryWindows_Hide_missing_files),
                        &mut d.filter_missing,
                    ),
                    (
                        t(hl::HistoryWindows_Show_image_files_only),
                        &mut d.image_only,
                    ),
                    (
                        t(hl::HistoryWindows_Remember_search_text),
                        &mut d.remember_search,
                    ),
                    (
                        t(hl::HistoryWindows_Remember_window_size_and_position),
                        &mut d.remember_window,
                    ),
                ] {
                    let r = line(y, 30.0);
                    kit::text_at(
                        ui,
                        r,
                        Align2::LEFT_CENTER,
                        label,
                        13.0,
                        Weight::Normal,
                        colors.text(),
                    );
                    kit::toggle_at(
                        ui,
                        &colors,
                        r,
                        Id::new(("image-history-toggle", key, label)),
                        value,
                        true,
                    );
                    y += 40.0;
                }
            });
        let done = t(hl::HistoryWindows_Done);
        let r = kit::right_aligned(ui, &colors, rows[2], &[(None, Some(done))], 8.0)[0];
        if kit::button_at(ui, &colors, r, None, Some(done), true).clicked() {
            // `OnSaveSettingsClick`.
            let d = self.draft.clone();
            self.settings.thumbnail_size = Size {
                width: d.width,
                height: d.height,
            };
            self.settings.thumbnail_header_position = match d.header {
                0 => ThumbnailHeaderPosition::None,
                1 => ThumbnailHeaderPosition::Top,
                _ => ThumbnailHeaderPosition::Bottom,
            };
            self.settings.max_item_count = d.batch;
            self.settings.auto_load_more_items = d.auto_load;
            self.settings.filter_missing_files = d.filter_missing;
            self.settings.image_only = d.image_only;
            self.settings.remember_search_text = d.remember_search;
            self.settings.remember_window_state = d.remember_window;
            self.settings.search_text = if d.remember_search {
                self.search.clone()
            } else {
                String::new()
            };
            self.save_settings(state);
            self.data.modal = None;
            self.refilter(state);
        }
    }

    /// QA renders: the settings dialog open.
    pub fn qa_open_settings(&mut self) {
        self.open_settings();
    }

    fn open_settings(&mut self) {
        let s = &self.settings;
        self.draft = Draft {
            width: s.thumbnail_size.width,
            height: s.thumbnail_size.height,
            header: match s.thumbnail_header_position {
                ThumbnailHeaderPosition::None => 0,
                ThumbnailHeaderPosition::Top => 1,
                ThumbnailHeaderPosition::Bottom => 2,
            },
            batch: s.max_item_count,
            auto_load: s.auto_load_more_items,
            filter_missing: s.filter_missing_files,
            image_only: s.image_only,
            remember_search: s.remember_search_text,
            remember_window: s.remember_window_state,
        };
        self.data.modal = Some(Modal::Settings);
    }
}

/// `ImageHistoryThumbnailLoader.LoadCore`: the picture scaled down to the width, then to
/// the height if it is still taller (never up).
pub fn thumbnail(path: &str, width: u32, height: u32) -> Option<image::RgbaImage> {
    if path.trim().is_empty() || !std::path::Path::new(path).is_file() {
        return None;
    }
    let image = super::files::load_image(std::path::Path::new(path))?;
    let (w, h) = image.dimensions();
    let mut scale = (width as f32 / w as f32).min(1.0);
    if (h as f32 * scale) > height as f32 {
        scale = (height as f32 / h as f32).min(1.0);
    }
    if scale >= 1.0 {
        return Some(image);
    }
    Some(image::imageops::thumbnail(
        &image,
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    ))
}

impl HistoryHost for ImageHistoryWindow {
    fn data(&mut self) -> &mut HistoryData {
        &mut self.data
    }

    /// `ApplyFilterAsync`, then `ResetLoadedEntries` and the first batch.
    fn refilter(&mut self, _state: &mut AppState) {
        self.filtered = logic::image_filter(
            &self.data.items,
            &self.search,
            self.settings.favorites,
            self.settings.image_only,
            self.settings.filter_missing_files,
        );
        self.loaded = 0;
        self.anchor = None;
        self.selected.retain(|_| false);
        self.thumbs.clear();
        self.thumb_version += 1;
        self.in_flight = 0;
        self.scroll_to_row = Some(0);
        self.load_next_batch();
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

impl ToolWindow for ImageHistoryWindow {
    fn spec(&self) -> WindowSpec {
        let title = if self.data.loaded {
            format!(
                "{} ({})",
                t(hl::HistoryWindows_ShareX_Image_History),
                format(
                    t(hl::ImageHistoryWindow_ItemsFormat),
                    &[Arg::Int(self.filtered.len() as i64)]
                )
            )
        } else {
            t(hl::HistoryWindows_ShareX_Image_History).to_string()
        };
        shared::remembered_spec(
            title,
            vec2(1100.0, 720.0),
            vec2(720.0, 500.0),
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
        if self.in_flight > 0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        kit::fill(ui, &colors);
        let modal_open = self.data.modal.is_some();
        let mut close = false;
        if !modal_open {
            let (escape, f5) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::F5),
                )
            });
            if escape {
                close = true;
            }
            if f5 {
                shared::refresh(self, state);
            }
            if !shared::typing(ui) {
                let keys = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::A) && i.modifiers.command,
                        i.key_pressed(egui::Key::ArrowLeft),
                        i.key_pressed(egui::Key::ArrowRight),
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                        i.key_pressed(egui::Key::Home),
                        i.key_pressed(egui::Key::End),
                        i.modifiers.shift,
                        i.key_pressed(egui::Key::Enter)
                            && !i.modifiers.command
                            && !i.modifiers.shift,
                    )
                });
                let (all, left, right, up, down, home, end, shift, enter) = keys;
                let current = self
                    .selected
                    .first()
                    .and_then(|id| {
                        (0..self.loaded).find(|i| self.item_at(*i).is_some_and(|it| it.id == *id))
                    })
                    .unwrap_or(0) as i64;
                let columns = self.columns.max(1) as i64;
                if all {
                    self.selected = (0..self.loaded)
                        .filter_map(|i| self.item_at(i).map(|it| it.id))
                        .collect();
                } else if left {
                    self.select_index(current - 1, shift);
                } else if right {
                    self.select_index(current + 1, shift);
                } else if up {
                    self.select_index(current - columns, shift);
                } else if down {
                    self.select_index(current + columns, shift);
                } else if home {
                    self.select_index(0, shift);
                } else if end {
                    self.select_index(self.loaded as i64 - 1, shift);
                } else if enter {
                    shared::run(self, state, Act::OpenItem);
                } else if let Some(act) = shared::key_act(ui) {
                    shared::run(self, state, act);
                }
            }
        }

        let full = ui.max_rect();
        // The toolbar.
        let bar = Rect::from_min_size(
            pos2(full.left() + 10.0, full.top()),
            vec2(full.width() - 20.0, 12.0 * 2.0 + 40.0),
        );
        kit::panel(ui, &colors, bar);
        let inner = bar.shrink(12.0);
        let search = Rect::from_min_size(inner.min, vec2(280.0, 40.0));
        if shared::search_box(
            ui,
            &colors,
            search,
            Id::new(("image-history-search", self.key)),
            &mut self.search,
        ) {
            if self.settings.remember_search_text {
                self.settings.search_text = self.search.clone();
            }
            self.pending_filter = Some(now + 0.22);
        }
        let items = [
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
            icons::STAR,
            items[0].1.unwrap_or(""),
            self.settings.favorites,
        ) {
            self.settings.favorites = !self.settings.favorites;
            self.save_settings(state);
            self.refilter(state);
        }
        if kit::button_at(ui, &colors, rects[1], items[1].0, items[1].1, true).clicked() {
            shared::open_stats(self, state);
        }
        if kit::button_at(ui, &colors, rects[2], items[2].0, items[2].1, true).clicked() {
            shared::open_import(self, true);
        }
        if kit::button_at(ui, &colors, rects[3], items[3].0, items[3].1, true).clicked() {
            self.open_settings();
        }

        let grid = Rect::from_min_max(
            pos2(full.left() + 6.0, bar.bottom() + 6.0),
            pos2(full.right() - 6.0, full.bottom() - 6.0),
        );
        self.grid(ui, state, grid);

        if modal_open {
            let mut child = kit::child(ui, full);
            shared::modals(self, &mut child, state, |window, ui, state, full| {
                window.settings_dialog(ui, state, full)
            });
        }
        close
    }

    fn closed(&mut self, state: &mut AppState) {
        self.thumb_version += 1;
        self.save_settings(state);
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl ImageHistoryWindow {
    /// The thumbnail rows (only the visible ones are laid out).
    fn grid(&mut self, ui: &mut Ui, state: &mut AppState, rect: Rect) {
        let colors = state.colors.clone();
        if !self.data.loaded {
            shared::state_message(
                ui,
                &colors,
                rect,
                icons::LOADER_CIRCLE,
                t(hl::HistoryWindows_Loading),
                true,
            );
            return;
        }
        if self.filtered.is_empty() {
            let text = if self.data.items.is_empty() {
                t(hl::HistoryWindows_No_image_history_items)
            } else {
                t(hl::ImageHistoryWindow_NoItemsMatchCurrentFilter)
            };
            shared::state_message(ui, &colors, rect, icons::IMAGES, text, false);
            return;
        }
        let card = self.card_size();
        // `UpdateColumnCount`.
        self.columns = ((rect.width() - 12.0).max(1.0) / card.x.max(50.0))
            .floor()
            .max(1.0) as usize;
        let columns = self.columns;
        let rows = self.loaded.div_ceil(columns);
        let mut area = kit::column(ui, rect);
        area.set_clip_rect(rect.intersect(ui.clip_rect()));
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("image-history-grid", self.key))
            .auto_shrink([false, false]);
        if let Some(row) = self.scroll_to_row.take() {
            let offset = area
                .ctx()
                .data(|d| d.get_temp::<f32>(Id::new(("image-history-offset", self.key))))
                .unwrap_or(0.0);
            let top = row as f32 * card.y;
            if row == 0 && self.selected.is_empty() {
                scroll = scroll.vertical_scroll_offset(0.0);
            } else if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + card.y > offset + rect.height() {
                scroll = scroll.vertical_scroll_offset(top + card.y - rect.height());
            }
        }
        let mut visible: Vec<usize> = Vec::new();
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut double: Option<usize> = None;
        let mut secondary: Option<usize> = None;
        let mut menu: Option<Act> = None;
        let (tw, th) = self.thumb_size();
        let header = self.settings.thumbnail_header_position;
        let output = scroll.show_rows(&mut area, card.y, rows, |ui, range| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for row in range {
                let (line, _) = ui.allocate_exact_size(vec2(rect.width(), card.y), Sense::hover());
                for column in 0..columns {
                    let index = row * columns + column;
                    if index >= self.loaded {
                        break;
                    }
                    let Some(item) = self.item_at(index).cloned() else {
                        continue;
                    };
                    visible.push(index);
                    let outer = Rect::from_min_size(
                        pos2(line.left() + column as f32 * card.x, line.top()),
                        card,
                    );
                    let border = outer.shrink(4.0);
                    let response = ui.interact(
                        border,
                        Id::new(("image-history-card", self.key, item.id)),
                        Sense::click(),
                    );
                    let selected = self.selected.contains(&item.id);
                    let hot = selected || response.hovered();
                    ui.painter().rect_filled(
                        border,
                        6.0,
                        if hot {
                            colors.control_background_hover()
                        } else {
                            colors.control_background()
                        },
                    );
                    ui.painter().rect_stroke(
                        border,
                        6.0,
                        egui::Stroke::new(
                            1.0,
                            if hot {
                                colors.accent_start()
                            } else {
                                colors.border()
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let content = border.shrink(5.0);
                    let name = if item.file_name.trim().is_empty() {
                        std::path::Path::new(&item.file_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    } else {
                        item.file_name.clone()
                    };
                    let mut y = content.top();
                    let caption = |ui: &Ui, y: f32| {
                        let r = Rect::from_min_size(pos2(content.left(), y), vec2(tw, CAPTION));
                        let galley = crate::widgets::layout(
                            ui,
                            crate::widgets::trimmed(
                                &name,
                                13.0,
                                Weight::Normal,
                                colors.text(),
                                tw,
                                1,
                            ),
                        );
                        ui.painter().galley(
                            pos2(
                                r.center().x - galley.size().x / 2.0,
                                r.center().y - galley.size().y / 2.0,
                            ),
                            galley,
                            colors.text(),
                        );
                    };
                    if header == ThumbnailHeaderPosition::Top {
                        caption(ui, y);
                        y += CAPTION + 3.0;
                    }
                    let picture = Rect::from_min_size(pos2(content.left(), y), vec2(tw, th));
                    ui.painter()
                        .rect_filled(picture, 4.0, colors.background_panel());
                    match self.thumbs.get(&item.id) {
                        Some(Thumb::Ready(texture)) => {
                            kit::image_uniform(ui, texture, picture, false);
                        }
                        other => {
                            let loading = matches!(other, Some(Thumb::Loading) | None);
                            let text = if loading {
                                t(hl::HistoryWindows_Loading)
                            } else {
                                ""
                            };
                            let icon_y = if loading {
                                picture.center().y - 10.0
                            } else {
                                picture.center().y
                            };
                            crate::widgets::paint_icon(
                                ui,
                                ui.painter(),
                                pos2(picture.center().x, icon_y),
                                icons::IMAGE,
                                30.0,
                                colors.text_secondary(),
                            );
                            if loading {
                                kit::text_at(
                                    ui,
                                    Rect::from_center_size(
                                        pos2(picture.center().x, icon_y + 26.0),
                                        vec2(tw, 18.0),
                                    ),
                                    Align2::CENTER_CENTER,
                                    text,
                                    13.0,
                                    Weight::Normal,
                                    colors.text_secondary(),
                                );
                            }
                        }
                    }
                    if item.favorite() {
                        crate::widgets::paint_icon(
                            ui,
                            ui.painter(),
                            pos2(picture.right() - 6.0 - 8.0, picture.top() + 6.0 + 8.0),
                            icons::STAR,
                            16.0,
                            colors.accent_start(),
                        );
                    }
                    if header == ThumbnailHeaderPosition::Bottom {
                        caption(ui, picture.bottom() + 3.0);
                    }
                    if response.clicked() {
                        clicked = Some((index, ui.input(|i| i.modifiers)));
                    }
                    if response.double_clicked() {
                        double = Some(index);
                    }
                    if response.secondary_clicked() {
                        secondary = Some(index);
                    }
                    if let Some(act) = shared::context_menu(&colors, &response, shared::image_menu)
                    {
                        menu = Some(act);
                    }
                }
            }
        });
        let offset = output.state.offset.y;
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new(("image-history-offset", self.key)), offset));
        self.request_thumbs(state, ui, &visible);
        // Thumbnails far from the screen go (`Unrealize`), keeping memory bounded.
        if self.thumbs.len() > 400 {
            let keep: std::collections::HashSet<i64> = visible
                .iter()
                .filter_map(|i| self.item_at(*i).map(|it| it.id))
                .collect();
            self.thumbs
                .retain(|id, thumb| keep.contains(id) || matches!(thumb, Thumb::Loading));
        }
        // `OnScrollChanged` / `CheckLoadMoreIfViewportNotFilled`.
        if self.settings.auto_load_more_items && self.loaded < self.filtered.len() {
            let content = output.content_size.y;
            let remaining = content - output.inner_rect.height() - offset;
            if remaining <= th.max(120.0) {
                self.load_next_batch();
                ui.ctx().request_repaint();
            }
        }
        if let Some(index) = secondary
            && self
                .item_at(index)
                .is_some_and(|it| !self.selected.contains(&it.id))
        {
            self.select_only(index);
        }
        if let Some((index, modifiers)) = clicked {
            if modifiers.shift
                && let Some(anchor) = self.anchor
            {
                self.select_range(anchor, index);
            } else if modifiers.command {
                if let Some(id) = self.item_at(index).map(|it| it.id) {
                    if let Some(at) = self.selected.iter().position(|s| *s == id) {
                        self.selected.remove(at);
                    } else {
                        self.selected.push(id);
                    }
                    self.anchor = Some(index);
                }
            } else {
                self.select_only(index);
                // A plain click shows the picture (`OnThumbnailPointerReleased`).
                if double.is_none() {
                    shared::run(self, state, Act::ShowImage);
                }
            }
        }
        if let Some(index) = double {
            self.select_only(index);
            shared::run(self, state, Act::OpenItem);
        }
        if let Some(act) = menu {
            shared::run(self, state, act);
        }
    }
}

impl AppState {
    /// `TaskHelpers.OpenImageHistory`.
    pub fn open_image_history(&mut self) {
        let settings: ImageHistorySettings =
            modlight_tools::settings::read(self.config.get("ImageHistorySettings"));
        let store = self.history_store();
        self.tools
            .open_keyed(|key| Box::new(ImageHistoryWindow::new(key, store, settings)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_fit_without_growing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.png");
        modlight_tools::images::save(&image::RgbaImage::new(1000, 200), &path).unwrap();
        let thumb = thumbnail(&path.to_string_lossy(), 250, 150).unwrap();
        assert_eq!(thumb.dimensions(), (250, 50));
        let tall = dir.path().join("t.png");
        modlight_tools::images::save(&image::RgbaImage::new(100, 600), &tall).unwrap();
        assert_eq!(
            thumbnail(&tall.to_string_lossy(), 250, 150)
                .unwrap()
                .dimensions(),
            (25, 150)
        );
        let small = dir.path().join("s.png");
        modlight_tools::images::save(&image::RgbaImage::new(20, 10), &small).unwrap();
        assert_eq!(
            thumbnail(&small.to_string_lossy(), 250, 150)
                .unwrap()
                .dimensions(),
            (20, 10)
        );
        assert!(thumbnail("/nope.png", 10, 10).is_none());
    }
}

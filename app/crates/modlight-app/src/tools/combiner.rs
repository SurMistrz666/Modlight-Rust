//! `ImageCombinerWindow` and `ImageCombinerViewModel`: a list of pictures (add, remove,
//! reorder, drop files on the window), the layout options, a live preview, and "Combine
//! images", which hands the result to the after-capture tasks (`RunImageTask`).

use std::path::PathBuf;

use egui::{Align2, Rect, Sense, TextureHandle, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, t, tools as tl};
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::combine;
use modlight_tools::settings::{
    ImageCombinerAlignment, ImageCombinerOptions, ImageCombinerOrientation,
};

use super::jobs::OptionsHome;
use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::AppState;
use crate::theme::Weight;

const ROW_HEIGHT: f32 = 36.0;

pub struct CombinerWindow {
    pub key: u64,
    pub settings: TaskSettings,
    pub home: OptionsHome,
    pub options: ImageCombinerOptions,
    pub images: Vec<PathBuf>,
    /// Selected rows (`SelectionMode="Multiple"`); the last clicked is `SelectedImage`.
    pub selected: Vec<usize>,
    pub current: Option<usize>,
    preview: Option<(u64, TextureHandle)>,
    preview_version: u64,
    preview_wanted: u64,
    pub busy: bool,
    /// `StatusText` (kept; upstream's window shows no status line).
    pub status: String,
    /// QA renders show these instead of the fixture files' real paths.
    pub qa_names: Vec<String>,
}

impl CombinerWindow {
    pub fn new(key: u64, settings: TaskSettings, files: Vec<PathBuf>) -> CombinerWindow {
        let options: ImageCombinerOptions =
            super::jobs::tool_options(&settings, "ImageCombinerOptions");
        let home = OptionsHome::of(&settings);
        let mut window = CombinerWindow {
            key,
            settings,
            home,
            options,
            images: Vec::new(),
            selected: Vec::new(),
            current: None,
            preview: None,
            preview_version: 0,
            preview_wanted: 0,
            busy: false,
            status: t(tl::ImageCombinerViewModel_Add_two_images).to_string(),
            qa_names: Vec::new(),
        };
        window.add_files(files);
        window
    }

    /// `AddFiles`: the files that exist, in order.
    pub fn add_files(&mut self, files: Vec<PathBuf>) {
        for file in files.into_iter().filter(|f| f.exists()) {
            self.images.push(file);
        }
        self.collection_changed();
    }

    fn collection_changed(&mut self) {
        self.preview_wanted += 1;
    }

    pub fn can_combine(&self) -> bool {
        self.images.len() > 1 && !self.busy
    }

    pub fn can_remove(&self) -> bool {
        !self.selected.is_empty() || self.current.is_some()
    }

    pub fn can_move_up(&self) -> bool {
        self.current.is_some_and(|i| i > 0)
    }

    pub fn can_move_down(&self) -> bool {
        self.current.is_some_and(|i| i + 1 < self.images.len())
    }

    /// `ImageCountText`.
    pub fn count_text(&self) -> String {
        format(
            t(if self.images.len() == 1 {
                tl::ImageCombinerViewModel_One_image
            } else {
                tl::ImageCombinerViewModel_Image_count
            }),
            &[Arg::from(self.images.len())],
        )
    }

    pub fn remove(&mut self) {
        let mut doomed: Vec<usize> = if self.selected.is_empty() {
            self.current.into_iter().collect()
        } else {
            self.selected.clone()
        };
        doomed.sort_unstable();
        doomed.dedup();
        for index in doomed.into_iter().rev() {
            if index < self.images.len() {
                self.images.remove(index);
            }
        }
        self.selected.clear();
        self.current = None;
        self.collection_changed();
    }

    pub fn move_by(&mut self, delta: i64) {
        let Some(index) = self.current else {
            return;
        };
        let target = index as i64 + delta;
        if target < 0 || target as usize >= self.images.len() {
            return;
        }
        self.images.swap(index, target as usize);
        self.current = Some(target as usize);
        self.selected = vec![target as usize];
        self.collection_changed();
    }

    fn options_changed(&mut self, state: &mut AppState) {
        state.save_tool_options(&self.home, "ImageCombinerOptions", &self.options);
        self.preview_wanted += 1;
    }

    /// `RefreshPreviewAsync`, off the UI thread.
    fn refresh_preview(&mut self, state: &mut AppState) {
        if self.preview_version == self.preview_wanted {
            return;
        }
        self.preview_version = self.preview_wanted;
        if self.images.len() < 2 {
            self.preview = None;
            self.status = t(tl::ImageCombinerViewModel_Add_two_images).to_string();
            return;
        }
        let files = self.images.clone();
        let options = self.options.clone();
        let version = self.preview_version;
        let key = self.key;
        state.rt.spawn_blocking(move || {
            let combined = combine::combine_files(&files, &options);
            move |state: &mut AppState, ctx: &egui::Context| {
                if let Some(window) = state.tools.window_mut::<CombinerWindow>(key)
                    && window.preview_version == version
                {
                    window.set_preview(ctx, combined);
                }
            }
        });
    }

    /// The preview made at once (QA renders have no background runtime turn).
    pub fn preview_now(&mut self, ctx: &egui::Context) {
        self.preview_version = self.preview_wanted;
        let combined = combine::combine_files(&self.images, &self.options);
        self.set_preview(ctx, combined);
    }

    fn set_preview(&mut self, ctx: &egui::Context, image: Option<RgbaImage>) {
        match image {
            Some(image) => {
                let texture = kit::texture(ctx, &format!("combiner-{}", self.key), &image, false);
                self.preview = Some((self.preview_version, texture));
                self.status = format(
                    t(tl::ImageCombinerViewModel_Previewing),
                    &[Arg::from(self.count_text())],
                );
            }
            None => self.preview = None,
        }
    }
}

impl ToolWindow for CombinerWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::ImageCombinerWindow_ShareX_Image_Combiner),
            vec2(980.0, 650.0),
            vec2(820.0, 560.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        // Files dropped on the window.
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty() && p.is_file())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_files(dropped);
        }
        self.refresh_preview(state);

        let full = kit::inset(ui.max_rect(), 16.0);
        let rows = kit::rows(full, &[Track::Star(1.0), Track::Px(40.0)], 12.0);
        let cols = kit::columns(rows[0], &[Track::Px(440.0), Track::Star(1.0)], 12.0);

        // The list panel.
        kit::panel(ui, &colors, cols[0]);
        let inner = cols[0].shrink(12.0);
        let parts = kit::rows(inner, &[Track::Px(40.0), Track::Star(1.0)], 10.0);
        let add_w = kit::button_width(
            ui,
            &colors,
            Some(icons::PLUS),
            Some(t(tl::ImageCombinerWindow_Add)),
        );
        let icon_w = kit::button_width(ui, &colors, Some(icons::TRASH_2), None);
        let bar = kit::columns(
            parts[0],
            &[
                Track::Px(add_w),
                Track::Px(icon_w),
                Track::Px(icon_w),
                Track::Px(icon_w),
                Track::Star(1.0),
            ],
            8.0,
        );
        if kit::button_at(
            ui,
            &colors,
            bar[0],
            Some(icons::PLUS),
            Some(t(tl::ImageCombinerWindow_Add)),
            true,
        )
        .clicked()
        {
            let files = state.tool_open_files(
                t(tl::ImageCombinerWindow_Add_images_dialog),
                &[("Images", &super::files::IMAGE_EXTENSIONS)],
            );
            self.add_files(files);
        }
        if kit::button_at(
            ui,
            &colors,
            bar[1],
            Some(icons::TRASH_2),
            None,
            self.can_remove(),
        )
        .clicked()
        {
            self.remove();
        }
        if kit::button_at(
            ui,
            &colors,
            bar[2],
            Some(icons::ARROW_UP),
            None,
            self.can_move_up(),
        )
        .clicked()
        {
            self.move_by(-1);
        }
        if kit::button_at(
            ui,
            &colors,
            bar[3],
            Some(icons::ARROW_DOWN),
            None,
            self.can_move_down(),
        )
        .clicked()
        {
            self.move_by(1);
        }
        kit::text_at(
            ui,
            bar[4],
            Align2::RIGHT_CENTER,
            &self.count_text(),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );

        // The list (`ListBox`, Fluent: a control background, accent selection).
        let list = parts[1];
        ui.painter()
            .rect_filled(list, 4.0, colors.control_background());
        if self.images.is_empty() {
            kit::placeholder(
                ui,
                &colors,
                list,
                icons::SQUARE_DASHED,
                t(tl::ImageCombinerWindow_Drop_images_here_or_use_the_Add_button),
                42.0,
            );
        } else {
            let mut child = kit::column(ui, list.shrink(2.0));
            child.set_clip_rect(list.shrink(2.0));
            let mut clicked = None;
            egui::ScrollArea::vertical()
                .id_salt(("combiner-list", self.key))
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    for (index, path) in self.images.iter().enumerate() {
                        let (r, response) = ui.allocate_exact_size(
                            vec2(ui.available_width(), ROW_HEIGHT),
                            Sense::click(),
                        );
                        let selected = self.selected.contains(&index);
                        if selected {
                            ui.painter().rect_filled(
                                r,
                                4.0,
                                colors.accent_start().gamma_multiply(0.6),
                            );
                        } else if response.hovered() {
                            ui.painter()
                                .rect_filled(r, 4.0, colors.control_background_hover());
                        }
                        let text_color = if selected {
                            colors.accent_foreground()
                        } else {
                            colors.text()
                        };
                        crate::widgets::paint_icon(
                            ui,
                            ui.painter(),
                            pos2(r.left() + 16.0 + 7.0, r.center().y),
                            icons::IMAGE,
                            14.0,
                            if selected {
                                text_color
                            } else {
                                colors.text_secondary()
                            },
                        );
                        kit::text_at(
                            ui,
                            Rect::from_min_max(
                                pos2(r.left() + 16.0 + 14.0 + 10.0, r.top()),
                                pos2(r.right() - 12.0, r.bottom()),
                            ),
                            Align2::LEFT_CENTER,
                            &self
                                .qa_names
                                .get(index)
                                .cloned()
                                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
                            13.0,
                            Weight::Normal,
                            text_color,
                        );
                        if response.clicked() {
                            clicked = Some(index);
                        }
                    }
                });
            if let Some(index) = clicked {
                // `SelectionMode="Multiple"`: a click toggles the row.
                if let Some(at) = self.selected.iter().position(|i| *i == index) {
                    self.selected.remove(at);
                    self.current = self.selected.last().copied();
                } else {
                    self.selected.push(index);
                    self.current = Some(index);
                }
            }
        }

        // The layout panel.
        let right = kit::rows(
            cols[1],
            &[
                Track::Px(16.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 * 3.0 + 32.0 + 40.0 * 2.0),
                Track::Star(1.0),
            ],
            12.0,
        );
        kit::panel(ui, &colors, right[0]);
        let inner = right[0].shrink(16.0);
        let lines = kit::rows(
            inner,
            &[
                Track::Px(kit::HEADING_HEIGHT),
                Track::Px(32.0),
                Track::Px(40.0),
                Track::Px(40.0),
            ],
            10.0,
        );
        kit::heading(ui, &colors, lines[0], t(tl::ImageCombinerWindow_Layout));
        let row1 = kit::columns(lines[1], &[Track::Px(120.0), Track::Star(1.0)], 10.0);
        kit::label(ui, &colors, row1[0], t(tl::ImageCombinerWindow_Orientation));
        let horizontal = self.options.orientation == ImageCombinerOrientation::Horizontal;
        let h_label = t(tl::ImageCombinerWindow_Horizontal);
        let h_w = kit::radio_width(ui, h_label);
        let mut changed = false;
        if kit::radio_at(
            ui,
            &colors,
            Rect::from_min_size(row1[1].min, vec2(h_w, 32.0)),
            horizontal,
            h_label,
            true,
        ) && !horizontal
        {
            self.options.orientation = ImageCombinerOrientation::Horizontal;
            changed = true;
        }
        let v_label = t(tl::ImageCombinerWindow_Vertical);
        let v_w = kit::radio_width(ui, v_label);
        if kit::radio_at(
            ui,
            &colors,
            Rect::from_min_size(
                pos2(row1[1].left() + h_w + 16.0, row1[1].top()),
                vec2(v_w, 32.0),
            ),
            !horizontal,
            v_label,
            true,
        ) && horizontal
        {
            self.options.orientation = ImageCombinerOrientation::Vertical;
            changed = true;
        }
        let row2 = kit::columns(
            lines[2],
            &[
                Track::Px(120.0),
                Track::Star(1.0),
                Track::Px(120.0),
                Track::Star(1.0),
            ],
            10.0,
        );
        kit::label(ui, &colors, row2[0], t(tl::ImageCombinerWindow_Alignment));
        let alignment_texts = if self.options.orientation == ImageCombinerOrientation::Horizontal {
            [
                tl::ImageCombinerViewModel_Top,
                tl::ImageCombinerViewModel_Center,
                tl::ImageCombinerViewModel_Bottom,
            ]
        } else {
            [
                tl::ImageCombinerViewModel_Left,
                tl::ImageCombinerViewModel_Center,
                tl::ImageCombinerViewModel_Right,
            ]
        };
        let alignments: Vec<(ImageCombinerAlignment, String)> = ImageCombinerAlignment::ALL
            .iter()
            .zip(alignment_texts)
            .map(|(a, k)| (*a, t(k).to_string()))
            .collect();
        if let Some(a) = kit::combo_at(
            ui,
            &colors,
            row2[1],
            &format!("combiner-alignment-{}", self.key),
            &self.options.alignment,
            &alignments,
            true,
        ) {
            self.options.alignment = a;
            changed = true;
        }
        kit::label(ui, &colors, row2[2], t(tl::ImageCombinerWindow_Spacing));
        if kit::int_at(
            ui,
            &colors,
            row2[3],
            &format!("combiner-space-{}", self.key),
            &mut self.options.space,
            0,
            1000,
            1,
            true,
        ) {
            changed = true;
        }
        let row3 = kit::columns(
            lines[3],
            &[
                Track::Px(120.0),
                Track::Star(1.0),
                Track::Px(120.0),
                Track::Star(1.0),
            ],
            10.0,
        );
        kit::label(ui, &colors, row3[0], t(tl::ImageCombinerWindow_Wrap_after));
        if kit::int_at(
            ui,
            &colors,
            row3[1],
            &format!("combiner-wrap-{}", self.key),
            &mut self.options.wrap_after,
            0,
            1000,
            1,
            true,
        ) {
            changed = true;
        }
        let check = Rect::from_min_max(row3[2].min, row3[3].max);
        if kit::check_normal_at(
            ui,
            &colors,
            check,
            &mut self.options.auto_fill_background,
            t(tl::ImageCombinerWindow_Auto_fill_background),
            true,
        ) {
            changed = true;
        }
        if changed {
            self.options_changed(state);
        }

        // The preview panel.
        kit::panel(ui, &colors, right[1]);
        let room = right[1].shrink(16.0);
        match &self.preview {
            Some((_, texture)) => {
                let texture = texture.clone();
                kit::image_uniform(ui, &texture, room, true);
            }
            None => kit::placeholder(
                ui,
                &colors,
                room,
                icons::IMAGES,
                t(tl::ImageCombinerWindow_Add_at_least_two_images_to_preview),
                46.0,
            ),
        }

        // Combine.
        let item = [(
            Some(icons::COMBINE),
            Some(t(tl::ImageCombinerWindow_Combine_images)),
        )];
        let rect = kit::right_aligned(ui, &colors, rows[1], &item, 8.0)[0];
        if kit::button_at(ui, &colors, rect, item[0].0, item[0].1, self.can_combine()).clicked() {
            self.busy = true;
            self.status = t(tl::ImageCombinerViewModel_Combining_images).to_string();
            let files = self.images.clone();
            let options = self.options.clone();
            let settings = self.settings.clone();
            let key = self.key;
            state.rt.spawn_blocking(move || {
                let combined = combine::combine_files(&files, &options);
                move |state: &mut AppState, _ctx: &egui::Context| {
                    if let Some(window) = state.tools.window_mut::<CombinerWindow>(key) {
                        window.busy = false;
                        window.status = t(tl::ImageCombinerViewModel_Images_combined).to_string();
                    }
                    if let Some(image) = combined {
                        state.tool_image_task(image, settings);
                    }
                }
            });
        }
        false
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl AppState {
    /// `OpenImageCombiner(imageFiles, taskSettings)`.
    pub fn open_image_combiner(&mut self, files: Vec<PathBuf>, settings: TaskSettings) {
        self.tools
            .open_keyed(|key| Box::new(CombinerWindow::new(key, settings, files)));
    }

    /// `CombineImages(imageFiles, orientation, taskSettings)` (the history windows' and
    /// the file actions' "combine"): no window, straight to the after-capture tasks.
    pub fn combine_images(
        &mut self,
        files: Vec<PathBuf>,
        orientation: ImageCombinerOrientation,
        settings: TaskSettings,
    ) {
        let mut options: ImageCombinerOptions =
            super::jobs::tool_options(&settings, "ImageCombinerOptions");
        options.orientation = orientation;
        self.rt.spawn_blocking(move || {
            let combined = combine::combine_files(&files, &options);
            move |state: &mut AppState, _ctx: &egui::Context| {
                if let Some(image) = combined {
                    state.tool_image_task(image, settings);
                }
            }
        });
    }
}

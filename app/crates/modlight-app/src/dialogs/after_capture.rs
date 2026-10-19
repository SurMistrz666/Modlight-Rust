//! "After capture" (`AfterCaptureWindow`): 1040 by 680 (down to 820 by 540), topmost,
//! centred on the screen. On the left a card with three sections (the after-capture
//! tasks, the destinations, the after-upload tasks), the file name, and Copy, Cancel,
//! Continue; on the right the picture (or the recording's name).

use std::path::Path;
use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{helpers as h, s, t};
use modlight_core::tasks::TaskContext;
use modlight_core::tasks::info::TaskMetadata;
use modlight_core::tasks::settings::{
    AfterCaptureTasks, AfterUploadTasks, FileDestination, ImageDestination, TaskSettings,
};

use super::background;
use super::chrome::header_band;
use super::url_upload::compact_looks;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button_at, layout, paint_icon, paint_text, trimmed, wrapped,
};

pub const WIDTH: f32 = 1040.0;
pub const HEIGHT: f32 = 680.0;
pub const MIN_WIDTH: f32 = 820.0;
pub const MIN_HEIGHT: f32 = 540.0;

/// What the person decided.
#[derive(Clone, Debug, PartialEq)]
pub enum AfterCaptureOutcome {
    Continue {
        settings: Box<TaskSettings>,
        file_name: String,
    },
    Cancel,
}

pub struct AfterCapture {
    pub key: u64,
    pub settings: TaskSettings,
    pub section: usize,
    after_capture: Vec<(AfterCaptureTasks, String, bool)>,
    after_upload: Vec<(AfterUploadTasks, String, bool)>,
    destinations: Vec<(ImageDestination, Option<FileDestination>, String)>,
    pub file_name: String,
    pub image: Option<Arc<image::RgbaImage>>,
    file: Option<String>,
    texture: Option<egui::TextureHandle>,
    can_copy: bool,
}

/// `AfterCaptureTasks` in their declared order, with their texts.
pub fn after_capture_options() -> Vec<(AfterCaptureTasks, &'static str)> {
    use AfterCaptureTasks as A;
    vec![
        (
            A::ShowQuickTaskMenu,
            t(h::AfterCaptureTasks_ShowQuickTaskMenu),
        ),
        (
            A::ShowAfterCaptureWindow,
            t(h::AfterCaptureTasks_ShowAfterCaptureWindow),
        ),
        (A::BeautifyImage, t(h::AfterCaptureTasks_BeautifyImage)),
        (A::AddImageEffects, t(h::AfterCaptureTasks_AddImageEffects)),
        (A::AnnotateImage, t(h::AfterCaptureTasks_AnnotateImage)),
        (
            A::CopyImageToClipboard,
            t(h::AfterCaptureTasks_CopyImageToClipboard),
        ),
        (A::PinToScreen, t(h::AfterCaptureTasks_PinToScreen)),
        (
            A::SendImageToPrinter,
            t(h::AfterCaptureTasks_SendImageToPrinter),
        ),
        (A::SaveImageToFile, t(h::AfterCaptureTasks_SaveImageToFile)),
        (
            A::SaveImageToFileWithDialog,
            t(h::AfterCaptureTasks_SaveImageToFileWithDialog),
        ),
        (
            A::SaveThumbnailImageToFile,
            t(h::AfterCaptureTasks_SaveThumbnailImageToFile),
        ),
        (A::PerformActions, t(h::AfterCaptureTasks_PerformActions)),
        (
            A::CopyFileToClipboard,
            t(h::AfterCaptureTasks_CopyFileToClipboard),
        ),
        (
            A::CopyFilePathToClipboard,
            t(h::AfterCaptureTasks_CopyFilePathToClipboard),
        ),
        (
            A::CopyFolderPathToClipboard,
            t(h::AfterCaptureTasks_CopyFolderPathToClipboard),
        ),
        (A::ShowInExplorer, t(h::AfterCaptureTasks_ShowInExplorer)),
        (A::AnalyzeImage, t(h::AfterCaptureTasks_AnalyzeImage)),
        (A::ScanQRCode, t(h::AfterCaptureTasks_ScanQRCode)),
        (A::DoOCR, t(h::AfterCaptureTasks_DoOCR)),
        (
            A::ShowBeforeUploadWindow,
            t(h::AfterCaptureTasks_ShowBeforeUploadWindow),
        ),
        (
            A::UploadImageToHost,
            t(h::AfterCaptureTasks_UploadImageToHost),
        ),
        (A::DeleteFile, t(h::AfterCaptureTasks_DeleteFile)),
    ]
}

/// `AfterUploadTasks` in their declared order, with their texts.
pub fn after_upload_options() -> Vec<(AfterUploadTasks, &'static str)> {
    use AfterUploadTasks as A;
    vec![
        (
            A::ShowAfterUploadWindow,
            t(h::AfterUploadTasks_ShowAfterUploadWindow),
        ),
        (A::UseURLShortener, t(h::AfterUploadTasks_UseURLShortener)),
        (A::ShareURL, t(h::AfterUploadTasks_ShareURL)),
        (
            A::CopyURLToClipboard,
            t(h::AfterUploadTasks_CopyURLToClipboard),
        ),
        (A::OpenURL, t(h::AfterUploadTasks_OpenURL)),
        (A::ShowQRCode, t(h::AfterUploadTasks_ShowQRCode)),
    ]
}

impl AfterCapture {
    pub fn new(
        key: u64,
        settings: &TaskSettings,
        metadata: &TaskMetadata,
        file: Option<&Path>,
        ctx: &TaskContext,
    ) -> AfterCapture {
        let after_capture = after_capture_options()
            .into_iter()
            .filter(|(task, _)| {
                *task != AfterCaptureTasks::ShowQuickTaskMenu
                    && *task != AfterCaptureTasks::ShowAfterCaptureWindow
            })
            .map(|(task, label)| {
                (
                    task,
                    label.to_string(),
                    settings.after_capture_job.has(task),
                )
            })
            .collect();
        let after_upload = after_upload_options()
            .into_iter()
            .map(|(task, label)| (task, label.to_string(), settings.after_upload_job.has(task)))
            .collect();
        // `UploadersConfigValidator`: Modlight is the one destination; its file uploader
        // form is offered beside it, as upstream lists both.
        let destinations = vec![
            (ImageDestination::Modlight, None, "Modlight".to_string()),
            (
                ImageDestination::FileUploader,
                Some(FileDestination::Modlight),
                "Modlight".to_string(),
            ),
        ];
        let (file_name, image) = match file {
            Some(path) => {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let image =
                    modlight_core::names::file_helpers::is_image_file(&path.to_string_lossy())
                        .then(|| image::open(path).ok().map(|i| Arc::new(i.to_rgba8())))
                        .flatten();
                (stem, image)
            }
            None => {
                let name = ctx.file_name(settings, None, Some(metadata));
                (name, metadata.image.clone())
            }
        };
        AfterCapture {
            key,
            settings: settings.clone(),
            section: 0,
            after_capture,
            after_upload,
            destinations,
            file_name,
            can_copy: metadata.image.is_some(),
            image,
            file: file.map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
            texture: None,
        }
    }

    pub fn title() -> &'static str {
        t(s::AfterCaptureWindow_Title)
    }

    fn selected_destination(&self, index: usize) -> bool {
        let (image, file, _) = &self.destinations[index];
        match file {
            None => self.settings.image_destination == *image,
            Some(file) => {
                self.settings.image_destination == ImageDestination::FileUploader
                    && self.settings.image_file_destination == *file
            }
        }
    }

    /// `OnContinueClick`: the ticked tasks become the task's.
    fn continue_outcome(&self) -> AfterCaptureOutcome {
        let mut settings = self.settings.clone();
        let mut capture = AfterCaptureTasks::None;
        for (task, _, checked) in &self.after_capture {
            if *checked {
                capture = capture.with(*task);
            }
        }
        let mut upload = AfterUploadTasks::None;
        for (task, _, checked) in &self.after_upload {
            if *checked {
                upload = upload.with(*task);
            }
        }
        settings.after_capture_job = capture;
        settings.after_upload_job = upload;
        AfterCaptureOutcome::Continue {
            settings: Box::new(settings),
            file_name: self.file_name.clone(),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<AfterCaptureOutcome> {
        background(ui, colors);
        let full = ui.max_rect();
        let band = header_band(
            ui,
            colors,
            full.min,
            full.width(),
            icons::IMAGE,
            t(s::AfterCaptureWindow_Heading),
            19.0,
            Some(t(s::AfterCaptureWindow_Subtitle)),
            6.0,
            false,
        );
        let body = Rect::from_min_max(
            pos2(full.left() + 18.0, band.bottom() + 14.0),
            pos2(full.right() - 18.0, full.bottom() - 18.0),
        );
        let left = Rect::from_min_max(body.min, pos2(body.left() + 390.0, body.bottom()));
        let preview = Rect::from_min_max(pos2(left.right() + 14.0, body.top()), body.max);
        let painter = ui.painter().clone();
        let mut outcome = None;

        // The buttons row and the file name, bottom up.
        let buttons_top = left.bottom() - 32.0;
        let name_box = Rect::from_min_max(
            pos2(left.left(), buttons_top - 10.0 - 32.0),
            pos2(left.right(), buttons_top - 10.0),
        );
        let name_label = layout(
            ui,
            trimmed(
                t(s::AfterCaptureWindow_FileName),
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                left.width(),
                1,
            ),
        );
        let label_top = name_box.top() - 4.0 - name_label.size().y;
        let card = Rect::from_min_max(left.min, pos2(left.right(), label_top - 10.0));

        // The card: the section switch, then the section's list.
        painter.rect_filled(card, 6.0, colors.background_panel());
        painter.rect_stroke(
            card,
            6.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = card.shrink(8.0);
        let nav = Rect::from_min_size(inner.min, vec2(inner.width(), 4.0 + 32.0 + 4.0));
        painter.rect_filled(nav, 6.0, colors.background_toolbar());
        let sections = [
            (icons::IMAGE, t(s::AfterCaptureWindow_Heading)),
            (icons::UPLOAD, t(s::AfterCaptureWindow_Destinations)),
            (icons::CLIPBOARD_LIST, t(s::AfterCaptureWindow_AfterUpload)),
        ];
        let mut x = nav.left() + 4.0;
        for (index, (glyph, label)) in sections.iter().enumerate() {
            let galley = layout(
                ui,
                trimmed(
                    label,
                    13.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                    220.0,
                    1,
                ),
            );
            let width = 11.0 + 15.0 + 6.0 + galley.size().x + 11.0;
            let item = Rect::from_min_size(pos2(x, nav.top() + 4.0), vec2(width, 32.0));
            let response = ui.interact(
                item,
                egui::Id::new(("after-capture-section", self.key, index)),
                Sense::click(),
            );
            let selected = self.section == index;
            let color = if selected {
                colors.accent_foreground()
            } else {
                colors.text_secondary()
            };
            if selected {
                crate::widgets::gradient_rect(
                    &painter,
                    item,
                    4.0,
                    colors.accent_start(),
                    colors.accent_end(),
                );
            } else if response.hovered() {
                painter.rect_filled(item, 4.0, colors.control_background_hover());
            }
            paint_icon(
                ui,
                &painter,
                pos2(item.left() + 11.0 + 7.5, item.center().y),
                *glyph,
                15.0,
                color,
            );
            painter.galley(
                pos2(
                    item.left() + 11.0 + 15.0 + 6.0,
                    item.center().y - galley.size().y / 2.0,
                ),
                galley,
                color,
            );
            if response.clicked() {
                self.section = index;
            }
            x = item.right() + 3.0;
        }
        let list = Rect::from_min_max(
            pos2(inner.left() + 4.0, nav.bottom() + 6.0 + 2.0),
            pos2(inner.right() - 4.0, inner.bottom() - 8.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        egui::ScrollArea::vertical()
            .id_salt(("after-capture-list", self.key, self.section))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                match self.section {
                    0 => {
                        for (_, label, checked) in &mut self.after_capture {
                            ui.horizontal(|ui| {
                                ui.add_space(2.0 + 7.0);
                                crate::widgets::checkbox_weighted(
                                    ui,
                                    colors,
                                    checked,
                                    Some(label),
                                    true,
                                    Weight::Normal,
                                );
                            });
                        }
                    }
                    1 => {
                        for index in 0..self.destinations.len() {
                            let selected = self.selected_destination(index);
                            let (row, response) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 32.0),
                                Sense::click(),
                            );
                            let circle = pos2(row.left() + 2.0 + 7.0 + 10.0, row.center().y);
                            radio(ui, colors, circle, selected, response.hovered());
                            paint_text(
                                ui,
                                ui.painter(),
                                Rect::from_min_max(pos2(circle.x + 18.0, row.top()), row.max),
                                Align2::LEFT_CENTER,
                                &self.destinations[index].2,
                                13.0,
                                Weight::Normal,
                                colors.text(),
                            );
                            if response.clicked() {
                                let (image, file, _) = self.destinations[index];
                                self.settings.image_destination = image;
                                if let Some(file) = file {
                                    self.settings.image_file_destination = file;
                                }
                            }
                        }
                    }
                    _ => {
                        for (_, label, checked) in &mut self.after_upload {
                            ui.horizontal(|ui| {
                                ui.add_space(2.0 + 7.0);
                                crate::widgets::checkbox_weighted(
                                    ui,
                                    colors,
                                    checked,
                                    Some(label),
                                    true,
                                    Weight::Normal,
                                );
                            });
                        }
                    }
                }
            });

        // The file name.
        painter.galley(
            pos2(left.left(), label_top),
            name_label,
            colors.text_secondary(),
        );
        let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(name_box));
        Field {
            height: 32.0,
            width: Some(name_box.width()),
            padding_x: 8.0,
            ..Field::new(&mut self.file_name)
        }
        .show(&mut field_ui, colors);

        // Copy, Cancel, Continue.
        let (plain, primary) = compact_looks(colors);
        let plain = plain.height(32.0);
        let primary = primary.height(32.0);
        let copy_look = Look::button(colors).height(32.0).width(32.0).padding(0.0);
        if button_at(
            ui,
            colors,
            &copy_look,
            Content::icon(icons::COPY),
            self.can_copy,
            Rect::from_min_size(pos2(left.left(), buttons_top), vec2(32.0, 32.0)),
        )
        .on_hover_text(t(s::AfterCaptureWindow_CopyImage))
        .clicked()
        {
            // `OnCopyClick`: copy, and only copy.
            let mut settings = self.settings.clone();
            settings.after_capture_job = AfterCaptureTasks::CopyImageToClipboard;
            outcome = Some(AfterCaptureOutcome::Continue {
                settings: Box::new(settings),
                file_name: self.file_name.clone(),
            });
        }
        let (cancel, go_on) = (
            t(s::AfterCaptureWindow_Cancel),
            t(s::AfterCaptureWindow_Continue),
        );
        let go_w = crate::widgets::button_width(ui, &primary, Content::label(go_on));
        let cancel_w = crate::widgets::button_width(ui, &plain, Content::label(cancel));
        let go_rect = Rect::from_min_size(pos2(left.right() - go_w, buttons_top), vec2(go_w, 32.0));
        let cancel_rect = Rect::from_min_size(
            pos2(go_rect.left() - 7.0 - cancel_w, buttons_top),
            vec2(cancel_w, 32.0),
        );
        if button_at(
            ui,
            colors,
            &plain,
            Content::label(cancel),
            true,
            cancel_rect,
        )
        .clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            outcome = Some(AfterCaptureOutcome::Cancel);
        }
        if button_at(ui, colors, &primary, Content::label(go_on), true, go_rect).clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            outcome = Some(self.continue_outcome());
        }

        // The preview.
        painter.rect_filled(preview, 6.0, colors.background_panel());
        painter.rect_stroke(
            preview,
            6.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        if let Some(image) = &self.image {
            let texture = self.texture.get_or_insert_with(|| {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                );
                ui.ctx().load_texture(
                    format!("after-capture-{}", self.key),
                    color,
                    egui::TextureOptions::LINEAR,
                )
            });
            let area = preview.shrink(10.0);
            let (w, h) = (image.width() as f32, image.height() as f32);
            let scale = (area.width() / w).min(area.height() / h);
            let rect = Rect::from_center_size(area.center(), vec2(w * scale, h * scale));
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let badge_text = format!("{} × {}", image.width(), image.height());
            let galley = layout(
                ui,
                trimmed(
                    &badge_text,
                    12.0,
                    Weight::SemiBold,
                    Color32::WHITE,
                    200.0,
                    1,
                ),
            );
            let badge = Rect::from_min_size(
                pos2(
                    preview.right() - 8.0 - galley.size().x - 14.0,
                    preview.bottom() - 8.0 - galley.size().y - 6.0,
                ),
                vec2(galley.size().x + 14.0, galley.size().y + 6.0),
            );
            painter.rect_filled(
                badge,
                4.0,
                Color32::from_rgba_unmultiplied(0x20, 0x20, 0x20, 0xCC),
            );
            painter.galley(badge.min + vec2(7.0, 3.0), galley, Color32::WHITE);
        } else {
            let text = self
                .file
                .clone()
                .unwrap_or_else(|| t(s::AfterCaptureWindow_NoPreview).to_string());
            let note = layout(ui, {
                let mut job = wrapped(
                    &text,
                    13.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                    360.0_f32.min(preview.width() - 20.0),
                );
                job.halign = egui::Align::Center;
                job
            });
            let total = 48.0 + 8.0 + note.size().y;
            let top = preview.center().y - total / 2.0;
            let mut faded = painter.clone();
            faded.set_opacity(0.45);
            paint_icon(
                ui,
                &faded,
                pos2(preview.center().x, top + 24.0),
                icons::FILE_VIDEO,
                48.0,
                colors.text_secondary(),
            );
            painter.galley(
                pos2(preview.center().x, top + 48.0 + 8.0),
                note,
                colors.text_secondary(),
            );
        }
        outcome
    }
}

/// A radio button circle (`RadioButton`): 20 px, accent with a dot when checked.
pub fn radio(ui: &Ui, colors: &Colors, center: egui::Pos2, checked: bool, hovered: bool) {
    let painter = ui.painter();
    painter.circle_filled(
        center,
        10.0,
        if checked {
            colors.accent_start()
        } else {
            colors.control_background()
        },
    );
    if checked {
        painter.circle_filled(center, 4.0, colors.accent_foreground());
    } else {
        painter.circle_stroke(
            center,
            9.5,
            Stroke::new(
                1.0,
                if hovered {
                    colors.accent_start()
                } else {
                    colors.border()
                },
            ),
        );
    }
}

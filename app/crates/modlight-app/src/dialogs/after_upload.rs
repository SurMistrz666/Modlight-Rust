//! "After upload" (`AfterUploadWindow`): 940 by 580, resizable down to 760 by 460. The
//! link in every clipboard format on the left, the picture on the right on a checkered
//! surface with its size, and a row of buttons: copy the image, copy the selected
//! format, open the link, open the file, open its folder, close. Closes itself after a
//! minute when `AutoCloseAfterUploadForm` is set.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t, uploaders};
use modlight_core::names::upload_info as formats;
use modlight_core::tasks::info::{DataType, TaskInfo};

use super::background;
use super::chrome::header_band;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, layout, paint_icon, paint_text, trimmed,
};

pub const WIDTH: f32 = 940.0;
pub const HEIGHT: f32 = 580.0;
pub const MIN_WIDTH: f32 = 760.0;
pub const MIN_HEIGHT: f32 = 460.0;

/// One row of the format list (`AfterUploadFormatItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct FormatItem {
    pub group: String,
    pub description: String,
    pub text: String,
}

/// What a click asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum AfterUploadAction {
    CopyImage,
    CopyText(String),
    OpenUrl(String),
    OpenFile(PathBuf),
    RevealFile(PathBuf),
    Close,
}

pub struct AfterUpload {
    pub key: u64,
    pub info: TaskInfo,
    pub formats: Vec<FormatItem>,
    pub selected: usize,
    pub image: Option<Arc<image::RgbaImage>>,
    texture: Option<egui::TextureHandle>,
    pub opened_at: Option<f64>,
    pub auto_close: bool,
    /// The remote picture was asked for (only once).
    pub remote_requested: bool,
}

/// `CreateFormats`: every link format that has a value; the image ones only for an
/// image link, and then the custom clipboard formats too.
pub fn create_formats(
    info: &TaskInfo,
    custom: &[(String, String)],
    parse: &dyn Fn(&str) -> String,
) -> Vec<FormatItem> {
    let is_image_url = modlight_core::names::file_helpers::is_image_file(&info.result.url);
    let links = t(s::AfterUploadWindow_GroupLinks).to_string();
    let forums = t(s::AfterUploadWindow_GroupForums).to_string();
    let local = t(s::AfterUploadWindow_GroupLocal).to_string();
    let rows: [(&str, &'static str, String, bool); 11] = [
        (
            &links,
            t(uploaders::LinkFormatEnum_URL),
            info.result.url.clone(),
            false,
        ),
        (
            &forums,
            t(uploaders::LinkFormatEnum_ForumImage),
            parse(formats::FORUM_IMAGE),
            true,
        ),
        (
            "HTML",
            t(uploaders::LinkFormatEnum_HTMLImage),
            parse(formats::HTML_IMAGE),
            true,
        ),
        (
            "Wiki",
            t(uploaders::LinkFormatEnum_WikiImage),
            parse(formats::WIKI_IMAGE),
            true,
        ),
        (
            &links,
            t(uploaders::LinkFormatEnum_ShortenedURL),
            info.result.shortened_url.clone(),
            false,
        ),
        (
            &forums,
            t(uploaders::LinkFormatEnum_ForumLinkedImage),
            parse(formats::FORUM_LINKED_IMAGE),
            true,
        ),
        (
            "HTML",
            t(uploaders::LinkFormatEnum_HTMLLinkedImage),
            parse(formats::HTML_LINKED_IMAGE),
            true,
        ),
        (
            "Wiki",
            t(uploaders::LinkFormatEnum_WikiLinkedImage),
            parse(formats::WIKI_LINKED_IMAGE),
            true,
        ),
        (
            &links,
            t(uploaders::LinkFormatEnum_ThumbnailURL),
            info.result.thumbnail_url.clone(),
            false,
        ),
        (
            &local,
            t(uploaders::LinkFormatEnum_LocalFilePath),
            info.file_path().to_string(),
            false,
        ),
        (
            &local,
            t(uploaders::LinkFormatEnum_LocalFilePathUri),
            file_uri(info.file_path()),
            false,
        ),
    ];
    let mut out = Vec::new();
    for (group, description, text, image_only) in rows {
        if image_only && !is_image_url {
            continue;
        }
        if !text.is_empty() {
            out.push(FormatItem {
                group: group.to_string(),
                description: description.to_string(),
                text,
            });
        }
    }
    if is_image_url {
        for (description, format) in custom {
            let text = parse(format);
            if !text.is_empty() {
                out.push(FormatItem {
                    group: t(s::AfterUploadWindow_GroupCustom).to_string(),
                    description: description.clone(),
                    text,
                });
            }
        }
    }
    out
}

/// `GetLocalFilePathAsUri`: `file:///...` for a file that exists.
fn file_uri(path: &str) -> String {
    if path.is_empty() || !Path::new(path).is_file() {
        return String::new();
    }
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .unwrap_or_default()
}

impl AfterUpload {
    pub fn new(
        key: u64,
        info: TaskInfo,
        formats: Vec<FormatItem>,
        auto_close: bool,
    ) -> AfterUpload {
        // The picture: the file when it is an image on disk, else the capture in memory.
        let mut image = None;
        if info.data_type == DataType::Image {
            let path = info.file_path();
            if !path.is_empty()
                && Path::new(path).is_file()
                && modlight_core::names::file_helpers::is_image_file(path)
            {
                image = image::open(path).ok().map(|i| Arc::new(i.to_rgba8()));
            } else if let Some(picture) = info.metadata.image.clone() {
                image = Some(picture);
            }
        }
        AfterUpload {
            key,
            info,
            formats,
            selected: 0,
            image,
            texture: None,
            opened_at: None,
            auto_close,
            remote_requested: false,
        }
    }

    /// The window title: "Modlight - After upload - `<file>`".
    pub fn title(&self) -> String {
        let path = self.info.file_path();
        let name = if !path.is_empty() && Path::new(path).is_file() {
            path.to_string()
        } else {
            self.info.file_name.clone()
        };
        format!("{} - {name}", t(s::AfterUploadWindow_Title))
    }

    /// Whether the remote picture should be fetched now (once, for an image link with
    /// nothing local to show).
    pub fn wants_remote_preview(&mut self) -> Option<String> {
        if self.remote_requested
            || self.image.is_some()
            || self.info.data_type != DataType::Image
            || self.info.result.url.is_empty()
        {
            return None;
        }
        self.remote_requested = true;
        Some(self.info.result.url.clone())
    }

    pub fn set_image(&mut self, image: Arc<image::RgbaImage>) {
        self.image = Some(image);
        self.texture = None;
    }

    fn selected_text(&self) -> Option<String> {
        self.formats
            .get(self.selected)
            .or_else(|| self.formats.first())
            .map(|f| f.text.clone())
            .filter(|t| !t.is_empty())
    }

    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors, now: f64) -> Option<AfterUploadAction> {
        let opened = *self.opened_at.get_or_insert(now);
        if self.auto_close {
            if now - opened >= 60.0 {
                return Some(AfterUploadAction::Close);
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        }
        background(ui, colors);
        let full = ui.max_rect();
        let result_text = if !self.info.result.url.is_empty() {
            self.info.result.url.clone()
        } else {
            self.info.file_name.clone()
        };
        let band = header_band(
            ui,
            colors,
            full.min,
            full.width(),
            icons::CIRCLE_CHECK,
            t(s::AfterUploadWindow_UploadComplete),
            19.0,
            Some(&result_text),
            6.0,
            false,
        );

        // Body: margin 18,14,18,18; two columns (*, 380) 14 apart; buttons under both.
        let body = Rect::from_min_max(
            pos2(full.left() + 18.0, band.bottom() + 14.0),
            pos2(full.right() - 18.0, full.bottom() - 18.0),
        );
        let buttons_top = body.bottom() - 40.0;
        let preview = Rect::from_min_max(
            pos2(body.right() - 380.0, body.top()),
            pos2(body.right(), buttons_top - 10.0),
        );
        let list = Rect::from_min_max(body.min, pos2(preview.left() - 14.0, buttons_top - 10.0));
        let mut action = None;

        // The format list in a panel card.
        let painter = ui.painter().clone();
        painter.rect_filled(list, 6.0, colors.background_panel());
        painter.rect_stroke(
            list,
            6.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = list.shrink(8.0);
        let head = Rect::from_min_size(inner.min, vec2(inner.width(), 7.0 + 18.0 + 7.0));
        painter.rect_filled(head, 6.0, colors.background_toolbar());
        paint_icon(
            ui,
            &painter,
            pos2(head.left() + 10.0 + 8.0, head.center().y),
            icons::LINK,
            16.0,
            colors.accent_start(),
        );
        paint_text(
            ui,
            &painter,
            Rect::from_min_max(pos2(head.left() + 10.0 + 16.0 + 7.0, head.top()), head.max),
            Align2::LEFT_CENTER,
            t(s::AfterUploadWindow_ClipboardFormats),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        let rows_rect = Rect::from_min_max(pos2(inner.left(), head.bottom() + 6.0), inner.max);
        let mut rows_ui = ui.new_child(egui::UiBuilder::new().max_rect(rows_rect));
        egui::ScrollArea::vertical()
            .id_salt(("after-upload-formats", self.key))
            .auto_shrink([false, false])
            .show(&mut rows_ui, |ui| {
                for (index, format) in self.formats.iter().enumerate() {
                    ui.add_space(1.0);
                    let (row, response) = ui.allocate_exact_size(
                        vec2(rows_rect.width(), 6.0 + 30.0 + 6.0),
                        Sense::click(),
                    );
                    ui.add_space(1.0);
                    let selected = index == self.selected;
                    // Unstyled TextBlocks in the item template: the global style
                    // (SemiBold, Text.Secondary) beats the selected item's Foreground.
                    let text_color = colors.text_secondary();
                    if selected {
                        crate::widgets::gradient_rect(
                            ui.painter(),
                            row,
                            4.0,
                            colors.accent_start(),
                            colors.accent_end(),
                        );
                    } else if response.hovered() {
                        ui.painter()
                            .rect_filled(row, 4.0, colors.control_background_hover());
                    }
                    let group = layout(
                        ui,
                        trimmed(&format.group, 11.0, Weight::SemiBold, text_color, 130.0, 1),
                    );
                    let description = layout(
                        ui,
                        trimmed(
                            &format.description,
                            13.0,
                            Weight::SemiBold,
                            text_color,
                            130.0,
                            1,
                        ),
                    );
                    let stack = group.size().y + 1.0 + description.size().y;
                    let top = row.center().y - stack / 2.0;
                    let mut faded = ui.painter().clone();
                    faded.set_opacity(0.72);
                    let group_h = group.size().y;
                    faded.galley(pos2(row.left() + 8.0, top), group, text_color);
                    ui.painter().galley(
                        pos2(row.left() + 8.0, top + group_h + 1.0),
                        description,
                        text_color,
                    );
                    let text_left = row.left() + 8.0 + 130.0 + 10.0;
                    let text = ui.fonts_mut(|f| {
                        let mut job = egui::text::LayoutJob::simple_singleline(
                            format.text.clone(),
                            egui::FontId::monospace(12.0),
                            text_color,
                        );
                        job.wrap = egui::text::TextWrapping {
                            max_width: (row.right() - 8.0 - text_left).max(10.0),
                            max_rows: 1,
                            break_anywhere: true,
                            overflow_character: Some('…'),
                        };
                        f.layout_job(job)
                    });
                    crate::widgets::paint_text_left(
                        ui.painter(),
                        text_left,
                        row.center().y,
                        text,
                        text_color,
                    );
                    let response = response.on_hover_text(&format.text);
                    if response.clicked() {
                        self.selected = index;
                    }
                    if response.double_clicked() {
                        self.selected = index;
                        action = self.selected_text().map(AfterUploadAction::CopyText);
                    }
                }
            });

        // The preview on a checkered surface, or the "no preview" note.
        // `panel-card checkered-surface`: the window's panel-card background wins.
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
                    format!("after-upload-{}", self.key),
                    color,
                    egui::TextureOptions::LINEAR,
                )
            });
            let area = preview.shrink(10.0);
            let (w, h) = (image.width() as f32, image.height() as f32);
            let scale = (area.width() / w).min(area.height() / h);
            let size = vec2(w * scale, h * scale);
            let rect = Rect::from_center_size(area.center(), size);
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
            let surface = ui.interact(
                preview,
                egui::Id::new(("after-upload-preview", self.key)),
                Sense::click(),
            );
            egui::Popup::context_menu(&surface)
                .frame(crate::menus::frame(colors))
                .show(|ui| {
                    let entries = [crate::menus::Entry::item(
                        t(s::AfterUploadWindow_CopyPreviewImage),
                        icons::IMAGE,
                        crate::commands::Command::NotYet,
                    )];
                    if crate::menus::entries(ui, colors, &entries).is_some() {
                        action = Some(AfterUploadAction::CopyImage);
                    }
                });
        } else {
            let note = layout(
                ui,
                trimmed(
                    t(s::AfterUploadWindow_NoPreview),
                    13.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                    preview.width() - 20.0,
                    1,
                ),
            );
            let total = 48.0 + 8.0 + note.size().y;
            let top = preview.center().y - total / 2.0;
            let mut faded = painter.clone();
            faded.set_opacity(0.45);
            paint_icon(
                ui,
                &faded,
                pos2(preview.center().x, top + 24.0),
                icons::LINK,
                48.0,
                colors.text_secondary(),
            );
            let note_w = note.size().x;
            painter.galley(
                pos2(preview.center().x - note_w / 2.0, top + 48.0 + 8.0),
                note,
                colors.text_secondary(),
            );
        }

        // The buttons.
        let file = self.info.file_path().to_string();
        let file_exists = !file.is_empty() && Path::new(&file).is_file();
        let mut icon_look = Look::button(colors);
        icon_look.width = Some(40.0);
        icon_look.padding_x = 0.0;
        let icon_buttons: [(char, &str, bool, u8); 5] = [
            (
                icons::IMAGE,
                t(s::AfterUploadWindow_CopyImage),
                self.image.is_some(),
                0,
            ),
            (
                icons::COPY,
                t(s::AfterUploadWindow_CopySelectedFormat),
                !self.formats.is_empty(),
                1,
            ),
            (
                icons::EXTERNAL_LINK,
                t(s::AfterUploadWindow_OpenLink),
                !self.info.result.url.is_empty(),
                2,
            ),
            (
                icons::FILE,
                t(s::AfterUploadWindow_OpenFile),
                file_exists,
                3,
            ),
            (
                icons::FOLDER,
                t(s::AfterUploadWindow_OpenContainingFolder),
                file_exists,
                4,
            ),
        ];
        let mut x = body.left();
        for (glyph, tip, enabled, which) in icon_buttons {
            let rect = Rect::from_min_size(pos2(x, buttons_top), vec2(40.0, 40.0));
            if button_at(ui, colors, &icon_look, Content::icon(glyph), enabled, rect)
                .on_hover_text(tip)
                .clicked()
            {
                action = match which {
                    0 => Some(AfterUploadAction::CopyImage),
                    1 => self.selected_text().map(AfterUploadAction::CopyText),
                    2 => Some(AfterUploadAction::OpenUrl(self.info.result.url.clone())),
                    3 => Some(AfterUploadAction::OpenFile(PathBuf::from(&file))),
                    _ => Some(AfterUploadAction::RevealFile(PathBuf::from(&file))),
                };
            }
            x += 40.0 + 6.0;
        }
        let mut close_look = Look::button(colors);
        close_look.string_text = Some(colors.text());
        close_look.min_width = 96.0;
        close_look.padding_x = 12.0;
        let close_label = t(s::AfterUploadWindow_Close);
        let close_w = button_width(ui, &close_look, Content::label(close_label));
        let close = Rect::from_min_size(
            pos2(body.right() - close_w, buttons_top),
            vec2(close_w, 40.0),
        );
        if button_at(
            ui,
            colors,
            &close_look,
            Content::label(close_label),
            true,
            close,
        )
        .clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            action = Some(AfterUploadAction::Close);
        }
        action
    }
}

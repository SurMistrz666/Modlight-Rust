//! "This file already exists" (`FileExistWindow`): 580 wide, a toolbar header, the
//! existing path, a new name to type, and four wide buttons (use the new name,
//! overwrite, use a unique name, cancel). Closing the window keeps the path as it was,
//! which overwrites, exactly as upstream's window does.

use std::path::{Path, PathBuf};

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t};

use super::background;
use crate::theme::{Colors, Weight};
use crate::widgets::{Field, layout, paint_icon, paint_text, paint_text_left, trimmed};

pub const WIDTH: f32 = 580.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FileExists {
    pub path: PathBuf,
    /// `FileHelpers.GetUniqueFilePath`, worked out when the window opened.
    pub unique_path: PathBuf,
    pub new_name: String,
    original_stem: String,
    pub content_height: f32,
    focused: bool,
}

impl FileExists {
    pub fn new(path: PathBuf, unique_path: PathBuf) -> FileExists {
        let original_stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        FileExists {
            new_name: original_stem.clone(),
            original_stem,
            path,
            unique_path,
            content_height: 420.0,
            focused: false,
        }
    }

    fn extension(&self) -> String {
        self.path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default()
    }

    /// The typed name with the original extension, or nothing when nothing was typed.
    fn new_file_name(&self) -> String {
        if self.new_name.is_empty() {
            String::new()
        } else {
            format!("{}{}", self.new_name, self.extension())
        }
    }

    fn new_name_allowed(&self) -> bool {
        !self.new_name.is_empty() && !self.new_name.eq_ignore_ascii_case(&self.original_stem)
    }

    fn with_new_name(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(self.new_file_name())
    }

    /// The path to write, `None` to not write the file; nothing until a choice is made.
    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<Option<PathBuf>> {
        background(ui, colors);
        let full = ui.max_rect();
        let width = full.width();
        let painter = ui.painter().clone();

        // Header band: toolbar fill, 18,12 padding, a 40 px tile and two lines.
        let title = layout(
            ui,
            trimmed(
                t(s::FileExistWindow_FileAlreadyExists),
                19.0,
                Weight::SemiBold,
                colors.accent_start(),
                width - 18.0 - 40.0 - 10.0 - 18.0,
                1,
            ),
        );
        let instruction = layout(
            ui,
            trimmed(
                t(s::FileExistWindow_Instruction),
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                width - 18.0 - 40.0 - 10.0 - 18.0,
                1,
            ),
        );
        let text_height = title.size().y + 2.0 + instruction.size().y;
        let header_height = 12.0 + text_height.max(40.0) + 12.0;
        let header = Rect::from_min_size(full.min, vec2(width, header_height));
        painter.rect_filled(header, 0.0, colors.background_toolbar());
        let tile = Rect::from_min_size(
            pos2(header.left() + 18.0, header.center().y - 20.0),
            vec2(40.0, 40.0),
        );
        painter.rect_filled(tile, 6.0, colors.background_panel());
        paint_icon(
            ui,
            &painter,
            tile.center(),
            icons::TRIANGLE_ALERT,
            20.0,
            colors.accent_start(),
        );
        let text_left = tile.right() + 10.0;
        let text_top = header.center().y - text_height / 2.0;
        let title_h = title.size().y;
        painter.galley(pos2(text_left, text_top), title, colors.accent_start());
        painter.galley(
            pos2(text_left, text_top + title_h + 2.0),
            instruction,
            colors.text_secondary(),
        );

        let left = full.left() + 18.0;
        let inner = width - 36.0;
        let mut y = header.bottom() + 14.0;

        // The existing path: a panel card with a file icon and the path, monospace.
        let card = Rect::from_min_size(pos2(left, y), vec2(inner, 8.0 + 18.0 + 8.0));
        painter.rect_filled(card, 6.0, colors.background_panel());
        painter.rect_stroke(
            card,
            6.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        paint_icon(
            ui,
            &painter,
            pos2(card.left() + 10.0 + 8.0, card.center().y),
            icons::FILE,
            16.0,
            colors.accent_start(),
        );
        let path_text = self.path.to_string_lossy().into_owned();
        let galley = ui.fonts_mut(|f| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                path_text.clone(),
                egui::FontId::monospace(13.0),
                colors.text(),
            );
            job.wrap = egui::text::TextWrapping {
                max_width: card.width() - 10.0 - 16.0 - 8.0 - 10.0,
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            f.layout_job(job)
        });
        paint_text_left(
            &painter,
            card.left() + 10.0 + 16.0 + 8.0,
            card.center().y,
            galley,
            colors.text(),
        );
        let path_hover = ui.interact(card, ui.id().with("path"), Sense::hover());
        path_hover.on_hover_text(&path_text);
        y = card.bottom() + 8.0;

        // The new name.
        paint_text(
            ui,
            &painter,
            Rect::from_min_size(pos2(left, y), vec2(inner, 18.0)),
            Align2::LEFT_TOP,
            t(s::FileExistWindow_NewFileName),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        y += 18.0 + 4.0;
        let mut field_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(pos2(left, y), vec2(inner, 32.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let id = egui::Id::new("file-exists-new-name");
        let response = Field {
            height: 32.0,
            width: Some(inner),
            id: Some(id),
            ..Field::new(&mut self.new_name)
        }
        .show(&mut field_ui, colors);
        if !self.focused {
            response.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let range = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.new_name.chars().count()),
                );
                state.cursor.set_char_range(Some(range));
                state.store(ui.ctx(), id);
            }
            self.focused = true;
        }
        y += 32.0 + 8.0;

        let mut choice = None;
        let new_file_name = self.new_file_name();
        let actions: [(char, &str, Option<String>, bool, u8); 4] = [
            (
                icons::PENCIL,
                t(s::FileExistWindow_UseNewName),
                Some(new_file_name),
                self.new_name_allowed(),
                0,
            ),
            (
                icons::FILE,
                t(s::FileExistWindow_Overwrite),
                self.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned()),
                true,
                1,
            ),
            (
                icons::COPY_PLUS,
                t(s::FileExistWindow_UseUniqueName),
                self.unique_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned()),
                true,
                2,
            ),
            (icons::X, t(s::FileExistWindow_Cancel), None, true, 3),
        ];
        for (glyph, title, file, enabled, which) in actions {
            let rect = Rect::from_min_size(pos2(left, y), vec2(inner, 52.0));
            if action_button(
                ui,
                colors,
                rect,
                glyph,
                title,
                file.as_deref(),
                enabled,
                which == 3,
            ) {
                choice = Some(which);
            }
            y += 52.0 + 8.0;
        }
        y -= 8.0;
        ui.allocate_rect(
            Rect::from_min_max(full.min, pos2(full.right(), y + 18.0)),
            Sense::hover(),
        );
        self.content_height = y + 18.0 - full.top();

        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if enter && !self.new_name.is_empty() {
            choice = Some(if self.new_name.eq_ignore_ascii_case(&self.original_stem) {
                1
            } else {
                0
            });
        }
        if escape {
            choice = Some(3);
        }
        match choice {
            Some(0) if self.new_name_allowed() => Some(Some(self.with_new_name())),
            Some(1) => Some(Some(self.path.clone())),
            Some(2) => Some(Some(self.unique_path.clone())),
            Some(3) => Some(None),
            _ => None,
        }
    }

    /// Closing the window without a choice keeps the original path (overwrite).
    pub fn closed(&self) -> Option<PathBuf> {
        Some(self.path.clone())
    }
}

/// `Button.action-button`: 52 tall, the icon, a semibold title and the file it means.
#[allow(clippy::too_many_arguments)]
fn action_button(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    glyph: char,
    title: &str,
    file: Option<&str>,
    enabled: bool,
    plain_icon: bool,
) -> bool {
    let response = ui.allocate_rect(
        rect,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    let fill = if hovered {
        colors.control_background_hover()
    } else {
        colors.control_background()
    };
    let mut faded = painter.clone();
    if !enabled {
        faded.set_opacity(0.35);
    }
    faded.rect_filled(rect, 10.0, fill);
    faded.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let icon_color = if plain_icon {
        colors.text_secondary()
    } else {
        colors.accent_start()
    };
    let icon_center = pos2(rect.left() + 11.0 + 9.0, rect.center().y);
    paint_icon(ui, &faded, icon_center, glyph, 17.0, icon_color);
    let text_left = rect.left() + 11.0 + 18.0 + 10.0;
    let text_width = rect.right() - 11.0 - text_left;
    let title_galley = layout(
        ui,
        trimmed(
            title,
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
            text_width,
            1,
        ),
    );
    match file {
        Some(file) => {
            let file_galley = layout(
                ui,
                trimmed(
                    file,
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    text_width,
                    1,
                ),
            );
            let total = title_galley.size().y + 1.0 + file_galley.size().y;
            let top = rect.center().y - total / 2.0;
            let title_h = title_galley.size().y;
            faded.galley(pos2(text_left, top), title_galley, colors.text_secondary());
            faded.galley(
                pos2(text_left, top + title_h + 1.0),
                file_galley,
                colors.text_secondary(),
            );
            if enabled {
                response.clone().on_hover_text(file);
            }
        }
        None => {
            paint_text_left(
                &faded,
                text_left,
                rect.center().y,
                title_galley,
                colors.text_secondary(),
            );
        }
    }
    enabled && response.clicked()
}

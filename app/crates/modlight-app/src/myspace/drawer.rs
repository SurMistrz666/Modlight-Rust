//! The details drawer (`ModlightDetailsDrawer.axaml`, `ModlightDetailsModel.cs`): 404 px
//! over the right of the listing, sliding in, following the selection, with the
//! preview, the actions, every link format, placement, statistics, gates and the way
//! out.

use egui::{Color32, Id, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t};
use modlight_core::library::{CloudFile, LinkFormat};

use crate::commands::Command;
use crate::myspace::details::{self, COPIED_SECONDS};
use crate::myspace::drag::paint_contain;
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, button_width, icon_galley, job, layout, paint_icon, trimmed, wrapped,
};

pub const WIDTH: f32 = 404.0;

impl AppState {
    /// Open the drawer on a file (`ShowDetailsAsync`).
    pub fn show_details(&mut self, id: &str, ctx: &egui::Context) {
        if self.space.file(id).is_none() {
            return;
        }
        let same = self.details.file.as_deref() == Some(id);
        self.details.open_on(id, self.time, self.env.reduce_motion);
        if !same || self.media.preview.is_none() {
            self.load_details_preview(ctx, id);
        }
        ctx.request_repaint();
    }

    /// Per frame: the single click that waited, following the selection, letting go of
    /// a file that left the listing, and retiring after the slide out.
    pub fn details_tick(&mut self, ctx: &egui::Context) {
        if let Some((id, due)) = self.details.pending_open.clone() {
            if self.time >= due {
                self.details.pending_open = None;
                if !self.drag.is_active() && !self.details.present && self.space.file(&id).is_some()
                {
                    self.show_details(&id, ctx);
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(due - self.time));
            }
        }
        if !self.details.present {
            return;
        }
        if let Some(id) = self.details.file.clone() {
            if self.space.file(&id).is_none() {
                self.details.close(self.time);
            } else if self.details.open {
                let marked = self.space.selected_ids();
                if !marked.is_empty()
                    && (marked.len() == 1 || !marked.contains(&id))
                    && marked[0] != id
                {
                    let target = marked[0].clone();
                    self.details.file = Some(target.clone());
                    self.details.primary_copied_until = 0.0;
                    self.details.row_copied = None;
                    self.load_details_preview(ctx, &target);
                }
            }
        }
        if !self.details.open && self.details.is_settled(self.time) {
            self.details.retire();
            self.media.preview = None;
        }
        if !self.details.is_settled(self.time) {
            ctx.request_repaint();
        }
    }

    /// Walk the listing without leaving the drawer (`Previous`, `Next`).
    pub fn details_step(&mut self, delta: i64, ctx: &egui::Context) {
        let Some(id) = self.details.file.clone() else {
            return;
        };
        let Some(at) = self.space.index_of(&id) else {
            return;
        };
        let next = at as i64 + delta;
        if next < 0 || next >= self.space.files.len() as i64 {
            return;
        }
        let target = self.space.files[next as usize].file.id.clone();
        self.space.select_file(&target);
        self.details.file = Some(target.clone());
        self.details.primary_copied_until = 0.0;
        self.details.row_copied = None;
        self.load_details_preview(ctx, &target);
    }
}

/// What the drawer's buttons asked for.
enum Ask {
    Command(Command),
    Previous,
    Next,
    Close,
    CopyPrimary,
    CopyRow(LinkFormat),
}

/// Draw the drawer over `list` when it is out.
pub fn ui(ui: &mut Ui, state: &mut AppState, list: Rect) {
    if !state.details.present {
        return;
    }
    let Some(file) = state
        .details
        .file
        .as_deref()
        .and_then(|id| state.space.file(id))
        .cloned()
    else {
        return;
    };
    let colors = state.colors.clone();
    let progress = state.details.progress(state.time);
    let left = list.right() - WIDTH * progress;
    let rect = Rect::from_min_size(pos2(left, list.top()), vec2(WIDTH, list.height()));
    let ctx = ui.ctx().clone();
    let mut asked: Vec<Ask> = Vec::new();
    let folder_anchor = std::cell::Cell::new(None::<egui::Response>);

    egui::Area::new(Id::new("details-drawer"))
        .order(Order::Middle)
        .fixed_pos(rect.min)
        .constrain(false)
        .movable(false)
        .show(&ctx, |ui| {
            ui.set_clip_rect(list);
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            let painter = ui.painter().clone();
            painter.rect_filled(rect, 0.0, colors.background_panel());
            painter.vline(
                rect.left() + 0.5,
                rect.y_range(),
                Stroke::new(1.0, colors.border()),
            );
            // Swallow clicks on the drawer's empty parts so they never reach the grid.
            let _ = ui.interact(rect, Id::new("details-drawer-surface"), Sense::click());

            let header_h = header(ui, state, &colors, &file, rect, &mut asked);
            let body = Rect::from_min_max(pos2(rect.left() + 1.0, rect.top() + header_h), rect.max);
            let mut body_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            body_ui.set_clip_rect(body.intersect(list));
            egui::ScrollArea::vertical()
                .id_salt(("details-body", file.id.as_str()))
                .auto_shrink([false, false])
                .show(&mut body_ui, |ui| {
                    let width = body.width();
                    ui.set_width(width);
                    sections(ui, state, &colors, &file, width, &mut asked, &folder_anchor);
                });
        });

    // The "Change" button's menu: the move menu for this one file.
    if let Some(anchor) = folder_anchor.take() {
        let ids = vec![file.id.clone()];
        if let Some(command) = crate::menus::popup_below(&colors, &anchor, || {
            crate::myspace::menus::move_to(state, &ids)
        }) {
            state.push(command);
        }
    }

    for ask in asked {
        match ask {
            Ask::Command(command) => state.push(command),
            Ask::Previous => state.details_step(-1, &ctx),
            Ask::Next => state.details_step(1, &ctx),
            Ask::Close => state.details.close(state.time),
            Ask::CopyPrimary => {
                if file.has_link() {
                    state.copy_text(&file.copy_link);
                    state.details.primary_copied_until = state.time + COPIED_SECONDS;
                }
            }
            Ask::CopyRow(format) => {
                let value = file.link_format(format);
                if !value.is_empty() {
                    state.copy_text(&value);
                    state.details.row_copied = Some((format, state.time + COPIED_SECONDS));
                }
            }
        }
    }
    if state.details.primary_copied_until > state.time
        || state
            .details
            .row_copied
            .is_some_and(|(_, until)| until > state.time)
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// A square glyph button of the drawer (`Button.dd-icon`).
fn dd_icon(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    glyph: char,
    tip: &str,
    enabled: bool,
) -> egui::Response {
    let look = Look::icon_button(colors, 30.0, 8.0, colors.control_background()).icon(15.0, None);
    let response = button_at(ui, colors, &look, Content::icon(glyph), enabled, rect);
    response.on_hover_text(tip)
}

/// The toolbar band: arrows, position, close; the name; badge, size, dimensions.
fn header(
    ui: &mut Ui,
    state: &AppState,
    colors: &Colors,
    file: &CloudFile,
    rect: Rect,
    asked: &mut Vec<Ask>,
) -> f32 {
    let inner_left = rect.left() + 1.0 + 14.0;
    let inner_right = rect.right() - 10.0;
    let width = inner_right - inner_left;
    let title = layout(
        ui,
        trimmed(
            &file.name,
            16.0,
            Weight::SemiBold,
            colors.text_secondary(),
            width,
            2,
        ),
    );
    let badge = details::badge(file);
    let size_text = file.size_text();
    let dimensions = details::dimensions(file).unwrap_or_default();
    let hint_h =
        ui.fonts_mut(|f| f.row_height(&egui::FontId::new(12.0, egui::FontFamily::Proportional)));
    let badge_h =
        ui.fonts_mut(|f| f.row_height(&egui::FontId::new(11.0, crate::theme::semibold()))) + 4.0;
    let facts_h = hint_h.max(if badge.is_empty() { 0.0 } else { badge_h });
    let height = 12.0 + 30.0 + 8.0 + title.size().y + 8.0 + facts_h + 13.0 + 1.0;
    let band = Rect::from_min_size(rect.min + vec2(1.0, 0.0), vec2(rect.width() - 1.0, height));
    let painter = ui.painter().clone();
    painter.rect_filled(band, 0.0, colors.background_toolbar());
    painter.hline(
        band.x_range(),
        band.bottom() - 0.5,
        Stroke::new(1.0, colors.border()),
    );

    let mut y = band.top() + 12.0;
    let index = state.space.index_of(&file.id);
    let total = state.space.files.len();
    let can_previous = index.is_some_and(|i| i > 0);
    let can_next = index.is_some_and(|i| i + 1 < total);
    let r = |x: f32| Rect::from_min_size(pos2(x, y), vec2(30.0, 30.0));
    if dd_icon(
        ui,
        colors,
        r(inner_left),
        icons::CHEVRON_LEFT,
        t(s::MySpace_DetailsPrevious),
        can_previous,
    )
    .clicked()
    {
        asked.push(Ask::Previous);
    }
    if dd_icon(
        ui,
        colors,
        r(inner_left + 30.0),
        icons::CHEVRON_RIGHT,
        t(s::MySpace_DetailsNext),
        can_next,
    )
    .clicked()
    {
        asked.push(Ask::Next);
    }
    let position = details::position_text(index, total);
    let pos_g = layout(
        ui,
        job(&position, 12.0, Weight::Normal, colors.text_secondary()),
    );
    painter.galley(
        pos2(inner_left + 60.0 + 8.0, y + 15.0 - pos_g.size().y / 2.0),
        pos_g,
        colors.text_secondary(),
    );
    if dd_icon(
        ui,
        colors,
        r(inner_right - 30.0),
        icons::X,
        t(s::MySpace_Close),
        true,
    )
    .clicked()
    {
        asked.push(Ask::Close);
    }
    y += 30.0 + 8.0;
    let title_h = title.size().y;
    let title_rect = Rect::from_min_size(pos2(inner_left, y), title.size());
    painter.galley(title_rect.min, title, colors.text_secondary());
    let _ = ui
        .interact(title_rect, Id::new("details-title"), Sense::hover())
        .on_hover_text(&file.name);
    y += title_h + 8.0;
    let mut x = inner_left;
    let cy = y + facts_h / 2.0;
    if !badge.is_empty() {
        let g = layout(
            ui,
            job(&badge, 11.0, Weight::SemiBold, colors.text_secondary()),
        );
        let b = Rect::from_min_size(
            pos2(x, cy - badge_h / 2.0),
            vec2(g.size().x + 14.0, badge_h),
        );
        painter.rect_filled(b, 6.0, colors.control_background());
        painter.galley(
            pos2(b.left() + 7.0, cy - g.size().y / 2.0),
            g,
            colors.text_secondary(),
        );
        x = b.right() + 8.0;
    }
    for text in [size_text.as_str(), dimensions.as_str()] {
        if text.is_empty() {
            continue;
        }
        let g = layout(ui, job(text, 12.0, Weight::Normal, colors.text_secondary()));
        let w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, colors.text_secondary());
        x += w + 8.0;
    }
    height
}

fn section_title(ui: &mut Ui, colors: &Colors, text: &str) {
    crate::widgets::text(ui, text, 11.0, Weight::SemiBold, colors.text_secondary());
}

fn rule(ui: &mut Ui, colors: &Colors, width: f32, top: f32, bottom: f32) {
    ui.add_space(top);
    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().hline(
        (r.left() + 16.0)..=(r.right() - 16.0),
        r.center().y,
        Stroke::new(1.0, colors.separator()),
    );
    ui.add_space(bottom);
}

/// A padded block: 16 px either side, 8 above and below.
fn block(ui: &mut Ui, width: f32, spacing: f32, add: impl FnOnce(&mut Ui, f32)) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width(width - 32.0);
            ui.spacing_mut().item_spacing.y = spacing;
            add(ui, width - 32.0);
        });
    });
    ui.add_space(8.0);
}

/// `Button.dd-small`: 30 tall, 10 px padding, 8 px corners, 12 px text.
fn dd_small(colors: &Colors) -> Look {
    Look::button(colors)
        .height(30.0)
        .padding(10.0)
        .radius(8.0)
        .font(12.0, Weight::Normal)
        .icon(13.0, None)
}

#[allow(clippy::too_many_arguments)]
fn sections(
    ui: &mut Ui,
    state: &AppState,
    colors: &Colors,
    file: &CloudFile,
    width: f32,
    asked: &mut Vec<Ask>,
    folder_anchor: &std::cell::Cell<Option<egui::Response>>,
) {
    let live = !file.is_trashed;
    let now = state.time;

    // The picture, never cropped.
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let (preview, _) = ui.allocate_exact_size(vec2(width - 32.0, 212.0), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(preview, 10.0, colors.control_background());
        let picture = state
            .media
            .preview
            .as_ref()
            .filter(|(id, _)| *id == file.id)
            .map(|(_, texture)| texture.clone());
        match picture {
            Some(texture) => {
                let inner = preview.shrink(1.0);
                let clip = painter.with_clip_rect(inner);
                paint_contain(&clip, inner, &texture);
            }
            None => {
                let label = details::badge(file);
                let g = layout(
                    ui,
                    job(&label, 12.0, Weight::Normal, colors.text_secondary()),
                );
                let stack = 54.0 + 9.0 + g.size().y;
                let top = preview.center().y - stack / 2.0;
                let kind =
                    Rect::from_min_size(pos2(preview.center().x - 27.0, top), vec2(54.0, 54.0));
                painter.rect_filled(kind, 10.0, colors.background_panel());
                paint_icon(
                    ui,
                    &painter,
                    kind.center(),
                    file.icon,
                    22.0,
                    colors.text_secondary(),
                );
                painter.galley(
                    pos2(preview.center().x - g.size().x / 2.0, kind.bottom() + 9.0),
                    g,
                    colors.text_secondary(),
                );
            }
        }
        painter.rect_stroke(
            preview,
            10.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
    });

    // Copy link, open, download.
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let (row, _) = ui.allocate_exact_size(vec2(width - 32.0, 36.0), Sense::hover());
        let action = Look::button(colors).height(36.0).padding(11.0);
        let square = Rect::from_min_size(pos2(row.right() - 38.0, row.top()), vec2(38.0, 36.0));
        let download = square;
        let open = square.translate(vec2(-38.0 - 8.0, 0.0));
        let copy = Rect::from_min_max(row.min, pos2(open.left() - 8.0, row.bottom()));
        let copied = state.details.primary_copied_until > now;
        let primary = Look::primary(colors)
            .height(36.0)
            .padding(11.0)
            .icon(14.0, None);
        let primary = Look {
            spacing: 7.0,
            min_width: 38.0,
            ..primary
        };
        if button_at(
            ui,
            colors,
            &primary,
            Content::both(
                if copied { icons::CHECK } else { icons::COPY },
                t(s::MySpace_CopyLink),
            ),
            file.has_link(),
            copy,
        )
        .clicked()
        {
            asked.push(Ask::CopyPrimary);
        }
        let icon_look = action.clone().icon(15.0, None);
        if button_at(
            ui,
            colors,
            &icon_look,
            Content::icon(icons::EXTERNAL_LINK),
            file.can_open(),
            open,
        )
        .on_hover_text(t(s::MySpace_OpenInBrowser))
        .clicked()
        {
            asked.push(Ask::Command(Command::OpenInBrowser(file.id.clone())));
        }
        if button_at(
            ui,
            colors,
            &icon_look,
            Content::icon(icons::DOWNLOAD),
            file.has_media_base(),
            download,
        )
        .on_hover_text(t(s::MySpace_Download))
        .clicked()
        {
            asked.push(Ask::Command(Command::Download(file.id.clone())));
        }
    });
    ui.add_space(4.0);
    rule(ui, colors, width, 12.0, 4.0);

    // About.
    block(ui, width, 7.0, |ui, inner| {
        section_title(ui, colors, t(s::MySpace_DetailsAbout));
        for fact in details::facts(file) {
            ui.add_space(2.0);
            let label = layout(
                ui,
                wrapped(
                    &fact.label,
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    94.0,
                ),
            );
            let value_w = inner - 94.0 - 12.0;
            let value_job = if fact.mono {
                let mut j = job(&fact.value, 12.0, Weight::Normal, colors.text());
                j.sections[0].format.font_id = egui::FontId::monospace(12.0);
                j.wrap.max_width = value_w;
                j.halign = egui::Align::RIGHT;
                j
            } else {
                let mut j = wrapped(&fact.value, 13.0, Weight::Normal, colors.text(), value_w);
                j.halign = egui::Align::RIGHT;
                j
            };
            let value = layout(ui, value_job);
            let h = label.size().y.max(value.size().y);
            let (r, _) = ui.allocate_exact_size(vec2(inner, h), Sense::hover());
            ui.painter().galley(r.min, label, colors.text_secondary());
            ui.painter()
                .galley(pos2(r.right(), r.top()), value, colors.text());
            ui.add_space(2.0);
        }
    });
    rule(ui, colors, width, 4.0, 4.0);

    // Placement: the folder and the visibility, each with its button.
    block(ui, width, 7.0, |ui, inner| {
        section_title(ui, colors, t(s::MySpace_DetailsPlacement));
        let folder_name = state.space.folder_name_for(file.folder_id.as_deref());
        let (folder_icon, folder_text) = if folder_name.is_empty() {
            (icons::INBOX, t(s::MySpace_Unfiled).to_string())
        } else {
            (icons::FOLDER, folder_name)
        };
        let small = dd_small(colors);
        let change = placement_row(
            ui,
            colors,
            inner,
            folder_icon,
            t(s::MySpace_DetailsFolder),
            &folder_text,
            &small,
            icons::FOLDER_INPUT,
            t(s::MySpace_DetailsChange),
            live,
        );
        // The menu itself opens outside this area, anchored to the button.
        folder_anchor.set(Some(change));
        let (vis_icon, vis_text, action, action_icon) = if file.published {
            (
                icons::GLOBE,
                t(s::MySpace_VisibilityPublic),
                t(s::MySpace_MakePrivate),
                icons::GLOBE_LOCK,
            )
        } else {
            (
                icons::LOCK_KEYHOLE,
                t(s::MySpace_VisibilityPrivate),
                t(s::MySpace_MakePublic),
                icons::GLOBE,
            )
        };
        if placement_row(
            ui,
            colors,
            inner,
            vis_icon,
            t(s::MySpace_DetailsVisibility),
            vis_text,
            &small,
            action_icon,
            action,
            live,
        )
        .clicked()
        {
            asked.push(Ask::Command(Command::SetPublished(
                vec![file.id.clone()],
                !file.published,
            )));
        }
    });

    if live {
        rule(ui, colors, width, 4.0, 4.0);
        block(ui, width, 7.0, |ui, inner| {
            section_title(ui, colors, t(s::MySpace_DetailsLinks));
            for format in LinkFormat::ALL {
                let value = file.link_format(format);
                let copied = state
                    .details
                    .row_copied
                    .is_some_and(|(f, until)| f == format && until > now);
                let (r, response) = ui.allocate_exact_size(vec2(inner, 34.0), Sense::click());
                let hovered = response.hovered();
                let painter = ui.painter();
                painter.rect_filled(
                    r,
                    8.0,
                    if hovered {
                        colors.control_background_hover()
                    } else {
                        colors.control_background()
                    },
                );
                painter.rect_stroke(
                    r,
                    8.0,
                    Stroke::new(1.0, colors.border()),
                    StrokeKind::Inside,
                );
                let label = layout(
                    ui,
                    trimmed(
                        format.label(),
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        72.0,
                        1,
                    ),
                );
                painter.galley(
                    pos2(r.left() + 10.0, r.center().y - label.size().y / 2.0),
                    label,
                    colors.text_secondary(),
                );
                let icon = icon_galley(
                    ui,
                    if copied { icons::CHECK } else { icons::COPY },
                    13.0,
                    colors.text_secondary(),
                );
                let icon_w = icon.size().x;
                painter.galley(
                    pos2(
                        r.right() - 10.0 - icon_w,
                        r.center().y - icon.size().y / 2.0,
                    ),
                    icon,
                    colors.text_secondary(),
                );
                let value_left = r.left() + 10.0 + 72.0 + 8.0;
                let value_w = (r.right() - 10.0 - icon_w - 8.0 - value_left).max(10.0);
                let mut j = job(&value, 11.0, Weight::Normal, colors.text_secondary());
                j.sections[0].format.font_id = egui::FontId::monospace(11.0);
                j.wrap = egui::text::TextWrapping {
                    max_width: value_w,
                    max_rows: 1,
                    break_anywhere: true,
                    overflow_character: Some('…'),
                };
                let g = layout(ui, j);
                painter.galley(
                    pos2(value_left, r.center().y - g.size().y / 2.0),
                    g,
                    colors.text_secondary(),
                );
                if response.on_hover_text(&value).clicked() {
                    asked.push(Ask::CopyRow(format));
                }
                ui.add_space(4.0);
            }
        });
    }
    rule(ui, colors, width, 4.0, 4.0);

    // Statistics.
    block(ui, width, 7.0, |ui, inner| {
        section_title(ui, colors, t(s::MySpace_DetailsStats));
        let w = (inner - 8.0) / 2.0;
        let hint = ui
            .fonts_mut(|f| f.row_height(&egui::FontId::new(12.0, egui::FontFamily::Proportional)));
        let value =
            ui.fonts_mut(|f| f.row_height(&egui::FontId::new(14.0, crate::theme::semibold())));
        let h = 9.0 + hint + value + 9.0;
        let (row, _) = ui.allocate_exact_size(vec2(inner, h), Sense::hover());
        for (i, (glyph, label, text)) in [
            (
                icons::EYE,
                t(s::MySpace_DetailsViews),
                details::views_text(file),
            ),
            (
                icons::DOWNLOAD,
                t(s::MySpace_DetailsDownloads),
                details::downloads_text(file),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let r = Rect::from_min_size(
                pos2(row.left() + i as f32 * (w + 8.0), row.top()),
                vec2(w, h),
            );
            let painter = ui.painter();
            painter.rect_filled(r, 10.0, colors.control_background());
            painter.rect_stroke(
                r,
                10.0,
                Stroke::new(1.0, colors.border()),
                StrokeKind::Inside,
            );
            let icon = icon_galley(ui, glyph, 15.0, colors.text_secondary());
            let icon_w = icon.size().x;
            painter.galley(
                pos2(r.left() + 12.0, r.center().y - icon.size().y / 2.0),
                icon,
                colors.text_secondary(),
            );
            let x = r.left() + 12.0 + icon_w + 10.0;
            let l = layout(
                ui,
                job(label, 12.0, Weight::Normal, colors.text_secondary()),
            );
            painter.galley(pos2(x, r.top() + 9.0), l, colors.text_secondary());
            let v = layout(
                ui,
                job(&text, 14.0, Weight::SemiBold, colors.text_secondary()),
            );
            painter.galley(pos2(x, r.top() + 9.0 + hint), v, colors.text_secondary());
        }
    });

    if live {
        rule(ui, colors, width, 4.0, 4.0);
        block(ui, width, 9.0, |ui, inner| {
            section_title(ui, colors, t(s::MySpace_LinkSettings));
            let gates = details::gates(file);
            if gates.is_empty() {
                crate::widgets::text_wrapped(
                    ui,
                    t(s::MySpace_DetailsNoGates),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
            } else {
                // A wrap of chips.
                let mut x = 0.0;
                let mut rows: Vec<Vec<(f32, char, String)>> = vec![Vec::new()];
                for (glyph, text) in gates {
                    let g = layout(
                        ui,
                        job(&text, 12.0, Weight::SemiBold, colors.text_secondary()),
                    );
                    let icon_w = icon_galley(ui, glyph, 12.0, colors.text_secondary())
                        .size()
                        .x;
                    let w = 8.0 + icon_w + 6.0 + g.size().x + 8.0;
                    if x + w > inner && x > 0.0 {
                        rows.push(Vec::new());
                        x = 0.0;
                    }
                    rows.last_mut().unwrap().push((w, glyph, text));
                    x += w + 6.0;
                }
                let chip_h = ui.fonts_mut(|f| {
                    f.row_height(&egui::FontId::new(12.0, crate::theme::semibold()))
                }) + 8.0;
                for row in rows {
                    let (r, _) = ui.allocate_exact_size(vec2(inner, chip_h + 6.0), Sense::hover());
                    let mut cx = r.left();
                    for (w, glyph, text) in row {
                        let chip = Rect::from_min_size(pos2(cx, r.top()), vec2(w, chip_h));
                        let painter = ui.painter();
                        painter.rect_filled(chip, 6.0, colors.control_background());
                        painter.rect_stroke(
                            chip,
                            6.0,
                            Stroke::new(1.0, colors.border()),
                            StrokeKind::Inside,
                        );
                        let icon = icon_galley(ui, glyph, 12.0, colors.text_secondary());
                        let icon_w = icon.size().x;
                        painter.galley(
                            pos2(chip.left() + 8.0, chip.center().y - icon.size().y / 2.0),
                            icon,
                            colors.text_secondary(),
                        );
                        let g = layout(
                            ui,
                            job(&text, 12.0, Weight::SemiBold, colors.text_secondary()),
                        );
                        painter.galley(
                            pos2(
                                chip.left() + 8.0 + icon_w + 6.0,
                                chip.center().y - g.size().y / 2.0,
                            ),
                            g,
                            colors.text_secondary(),
                        );
                        cx += w + 6.0;
                    }
                }
            }
            let small = dd_small(colors);
            let content = Content::both(icons::SETTINGS_2, t(s::MySpace_DetailsEditLinkSettings));
            let small = Look {
                spacing: 6.0,
                ..small
            };
            if crate::widgets::button(ui, colors, &small, content, true).clicked() {
                asked.push(Ask::Command(Command::LinkSettings(file.id.clone())));
            }
        });
    }
    rule(ui, colors, width, 4.0, 4.0);

    // The way out.
    block(ui, width, 7.0, |ui, inner| {
        section_title(ui, colors, t(s::MySpace_DetailsDanger));
        let danger = Look {
            spacing: 6.0,
            ..dd_small(colors).danger(colors)
        };
        let plain = Look {
            spacing: 6.0,
            ..dd_small(colors)
        };
        if live {
            let button_content = Content::both(icons::TRASH_2, t(s::MySpace_DetailsTrashButton));
            let bw = button_width(ui, &danger, button_content);
            let text_w = inner - 24.0 - 12.0 - bw;
            let title = layout(
                ui,
                wrapped(
                    t(s::MySpace_DetailsTrashTitle),
                    13.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                    text_w,
                ),
            );
            let hint = layout(
                ui,
                wrapped(
                    t(s::MySpace_DetailsTrashHint),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    text_w,
                ),
            );
            let text_h = title.size().y + 2.0 + hint.size().y;
            let h = 12.0 + text_h.max(30.0) + 12.0;
            let (card, _) = ui.allocate_exact_size(vec2(inner, h), Sense::hover());
            ui.painter().rect_stroke(
                card,
                10.0,
                Stroke::new(1.0, colors.cancel_border()),
                StrokeKind::Inside,
            );
            let top = card.center().y - text_h / 2.0;
            let title_h = title.size().y;
            ui.painter().galley(
                pos2(card.left() + 12.0, top),
                title,
                colors.text_secondary(),
            );
            ui.painter().galley(
                pos2(card.left() + 12.0, top + title_h + 2.0),
                hint,
                colors.text_secondary(),
            );
            let b = Rect::from_min_size(
                pos2(card.right() - 12.0 - bw, card.center().y - 15.0),
                vec2(bw, 30.0),
            );
            if button_at(ui, colors, &danger, button_content, true, b).clicked() {
                asked.push(Ask::Command(Command::Trash(vec![file.id.clone()])));
            }
        } else {
            let hint = layout(
                ui,
                wrapped(
                    t(s::MySpace_DetailsTrashedHint),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    inner - 24.0,
                ),
            );
            let h = 12.0 + hint.size().y + 9.0 + 30.0 + 12.0;
            let (card, _) = ui.allocate_exact_size(vec2(inner, h), Sense::hover());
            ui.painter().rect_stroke(
                card,
                10.0,
                Stroke::new(1.0, colors.cancel_border()),
                StrokeKind::Inside,
            );
            let hint_h = hint.size().y;
            ui.painter().galley(
                pos2(card.left() + 12.0, card.top() + 12.0),
                hint,
                colors.text_secondary(),
            );
            let y = card.top() + 12.0 + hint_h + 9.0;
            let restore = Content::both(icons::UNDO_2, t(s::MySpace_Restore));
            let rw = button_width(ui, &plain, restore);
            if button_at(
                ui,
                colors,
                &plain,
                restore,
                true,
                Rect::from_min_size(pos2(card.left() + 12.0, y), vec2(rw, 30.0)),
            )
            .clicked()
            {
                asked.push(Ask::Command(Command::Restore(vec![file.id.clone()])));
            }
            let forever = Content::both(icons::TRASH_2, t(s::MySpace_DeleteForever));
            let fw = button_width(ui, &danger, forever);
            let fr = Rect::from_min_size(pos2(card.left() + 12.0 + rw + 8.0, y), vec2(fw, 30.0));
            if button_at(ui, colors, &danger, forever, true, fr).clicked() {
                asked.push(Ask::Command(Command::DeleteForever(vec![file.id.clone()])));
            }
        }
    });
    ui.add_space(18.0);
    let _ = Color32::TRANSPARENT;
}

/// A fact with the button that changes it beside it.
#[allow(clippy::too_many_arguments)]
fn placement_row(
    ui: &mut Ui,
    colors: &Colors,
    inner: f32,
    glyph: char,
    label: &str,
    value: &str,
    look: &Look,
    action_icon: char,
    action: &str,
    enabled: bool,
) -> egui::Response {
    let look = Look {
        spacing: 6.0,
        ..look.clone()
    };
    let content = Content::both(action_icon, action);
    let bw = button_width(ui, &look, content);
    let hint =
        ui.fonts_mut(|f| f.row_height(&egui::FontId::new(12.0, egui::FontFamily::Proportional)));
    let text = ui.fonts_mut(|f| f.row_height(&egui::FontId::new(13.0, crate::theme::semibold())));
    let h = (hint + text).max(30.0);
    ui.add_space(2.0);
    let (row, _) = ui.allocate_exact_size(vec2(inner, h), Sense::hover());
    ui.add_space(2.0);
    let painter = ui.painter().clone();
    let icon = icon_galley(ui, glyph, 15.0, colors.text_secondary());
    let icon_w = icon.size().x;
    painter.galley(
        pos2(row.left(), row.center().y - icon.size().y / 2.0),
        icon,
        colors.text_secondary(),
    );
    let x = row.left() + icon_w + 10.0;
    let text_w = (row.right() - bw - 10.0 - x).max(10.0);
    let top = row.center().y - (hint + text) / 2.0;
    let l = layout(
        ui,
        job(label, 12.0, Weight::Normal, colors.text_secondary()),
    );
    painter.galley(pos2(x, top), l, colors.text_secondary());
    let v = layout(
        ui,
        trimmed(
            value,
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
            text_w,
            1,
        ),
    );
    painter.galley(pos2(x, top + hint), v, colors.text_secondary());
    let b = Rect::from_min_size(
        pos2(row.right() - bw, row.center().y - 15.0),
        vec2(bw, 30.0),
    );
    button_at(ui, colors, &look, content, enabled, b)
}

//! My space on screen (the `IsMySpaceSelected` grid of `MainWindow.axaml` and
//! `CloudFileTile.axaml`): the header with the path, the sliding library panel, the
//! folder shelf and the file grid sharing one scroller, the signed-out, loading, empty
//! and error panels, the details drawer and the pager bar.

use egui::epaint::{CornerRadius, PathShape, PathStroke};
use egui::{Align2, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t};
use modlight_core::library::{CloudFile, CloudFolder, LIBRARY_VIEWS};

use crate::commands::Command;
use crate::myspace::drag::{CLICK_SLOP, Press, paint_cover};
use crate::state::AppState;
use crate::theme::{Colors, Weight, rgba};
use crate::widgets::{
    Content, Fill, Look, button_at, button_width, icon_galley, job, layout, paint_icon,
    paint_progress, paint_surface, trimmed, wrapped,
};

pub const HEADER_HEIGHT: f32 = 52.0;
pub const PANEL_WIDTH: f32 = 236.0;
pub const PANEL_SLIDE: f64 = 0.16;
pub const FOLDER_WIDTH: f32 = 238.0;
pub const FOLDER_HEIGHT: f32 = 64.0;
pub const CARD_WIDTH: f32 = 180.0;
pub const THUMB_HEIGHT: f32 = 126.0;
pub const GAP: f32 = 12.0;
/// The grid's margin inside the scroller: 16,16,4,0.
pub const GRID_MARGIN_LEFT: f32 = 16.0;
pub const GRID_MARGIN_TOP: f32 = 16.0;
pub const GRID_MARGIN_RIGHT: f32 = 4.0;
/// The Avalonia default button height, which the section headers keep.
const SECTION_HEADER_HEIGHT: f32 = 40.0;
/// How long a single click waits for a second one before the drawer opens.
pub const DOUBLE_CLICK_WINDOW: f64 = 0.5;

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl AppState {
    /// Where the library panel is: (in the layout at all, how far out 0..1).
    pub fn panel_slide(&self) -> (bool, f32) {
        if self.env.reduce_motion {
            return (
                self.library_panel,
                if self.library_panel { 1.0 } else { 0.0 },
            );
        }
        let t = ((self.time - self.panel_moved_at) / PANEL_SLIDE).clamp(0.0, 1.0) as f32;
        let eased = ease_out_cubic(t);
        if self.library_panel {
            (true, eased)
        } else {
            // The column stays until the panel has slid out (plus a beat).
            (
                self.time - self.panel_moved_at < PANEL_SLIDE + 0.04,
                1.0 - eased,
            )
        }
    }
}

/// The whole tab, filling `ui`'s rect.
pub fn ui(ui: &mut Ui, state: &mut AppState) {
    let full = ui.max_rect();
    let colors = state.colors.clone();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let mut top = full.top();
    if state.space.has_account {
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
        header_ui(ui, state, header);
        top = header.bottom();
    }
    if state.space.is_busy {
        let bar = Rect::from_min_size(pos2(full.left(), top), vec2(full.width(), 2.0));
        paint_progress(ui, &colors, bar, None);
        top += 2.0;
    }
    let body = Rect::from_min_max(pos2(full.left(), top), full.max);

    let (present, out) = state.panel_slide();
    let mut main_left = body.left();
    if present {
        let host = Rect::from_min_size(body.min, vec2(PANEL_WIDTH, body.height()));
        library_panel(ui, state, host, out);
        main_left += PANEL_WIDTH;
        if out < 1.0 && out > 0.0 {
            ui.ctx().request_repaint();
        }
    }
    let main = Rect::from_min_max(pos2(main_left, body.top()), body.max);
    let pager_height = crate::myspace::pager::height(ui, state);
    let list = Rect::from_min_max(main.min, pos2(main.right(), main.bottom() - pager_height));

    let drawn = if state.space.has_content() {
        grid(ui, state, list)
    } else {
        Vec::new()
    };
    // A card that was not drawn this frame (scrolled away, its section folded, the grid
    // gone) is not under the pointer: its moving preview stops, as a PointerExited
    // would stop it, instead of ticking on unseen.
    if let Some((hovered, _)) = state.media.motion.hovered.clone()
        && !drawn.contains(&hovered)
    {
        state.unhover_file(&hovered);
    }
    if !state.space.has_content() && state.space.has_account {
        // The empty scroller still answers a right click with the background menu.
        let response = ui.interact(list, Id::new("cloud-empty-background"), Sense::click());
        if response.clicked() {
            state.space.clear_selection();
        }
        if let Some(command) = crate::menus::context(&colors, &response, || {
            crate::myspace::menus::background(state)
        }) {
            state.push(command);
        }
    }
    overlays(ui, state, list);
    crate::myspace::drawer::ui(ui, state, list);
    if pager_height > 0.0 {
        let bar = Rect::from_min_max(pos2(main.left(), list.bottom()), main.max);
        crate::myspace::pager::ui(ui, state, bar);
    }
}

// ---------------------------------------------------------------------- header

fn strip_look(colors: &Colors) -> Look {
    Look::strip(colors)
}

/// A 32 px square strip button carrying one glyph.
fn strip_icon(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    glyph: char,
    size: f32,
    tip: &str,
    enabled: bool,
) -> egui::Response {
    let look = strip_look(colors).width(rect.width()).icon(size, None);
    let response = button_at(ui, colors, &look, Content::icon(glyph), enabled, rect);
    if tip.is_empty() {
        response
    } else {
        response.on_hover_text(tip)
    }
}

fn header_ui(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let colors = state.colors.clone();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_panel());
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + 14.0, rect.top()),
        pos2(rect.right() - 14.0, rect.bottom() - 1.0),
    );
    let cy = inner.center().y;
    let spacing = 6.0;
    let square = |x: f32| Rect::from_center_size(pos2(x + 16.0, cy), vec2(32.0, 32.0));

    // Left: the hamburger, up one level, the title.
    let mut x = inner.left();
    if strip_icon(
        ui,
        &colors,
        square(x),
        icons::MENU,
        16.0,
        t(s::MySpace_LibraryPanel),
        true,
    )
    .clicked()
    {
        state.push(Command::ToggleLibraryPanel);
    }
    x += 32.0 + spacing;
    if state.space.can_go_up() {
        if strip_icon(
            ui,
            &colors,
            square(x),
            icons::CORNER_LEFT_UP,
            16.0,
            t(s::MySpace_Up),
            true,
        )
        .clicked()
        {
            state.push(Command::GoUp);
        }
        x += 32.0;
    }
    x += spacing;
    let crumbs = state.space.crumbs();
    let show_crumbs = crumbs.len() > 1;
    if !show_crumbs {
        let glyph = state.space.current_folder_icon();
        let icon = icon_galley(ui, glyph, 17.0, colors.accent_start());
        let icon_w = icon.size().x;
        painter.galley(
            pos2(x, cy - icon.size().y / 2.0),
            icon,
            colors.accent_start(),
        );
        let name = state.space.current_folder_name();
        let galley = layout(
            ui,
            job(&name, 17.0, Weight::SemiBold, colors.text_secondary()),
        );
        let name_w = galley.size().x;
        painter.galley(
            pos2(x + icon_w + 8.0, cy - galley.size().y / 2.0),
            galley,
            colors.text_secondary(),
        );
        x += icon_w + 8.0 + name_w;
    }
    x += spacing;

    // Right, from the edge inwards: refresh, duplicates, upload here / empty trash, count.
    let mut right = inner.right();
    if strip_icon(
        ui,
        &colors,
        Rect::from_center_size(pos2(right - 16.0, cy), vec2(32.0, 32.0)),
        icons::REFRESH_CW,
        16.0,
        t(s::MySpace_Refresh),
        true,
    )
    .clicked()
    {
        state.push(Command::Reload);
    }
    right -= 32.0 + spacing;
    let look = strip_look(&colors).icon(15.0, Some(colors.accent_start()));
    if state.space.trashed {
        let label = t(s::MySpace_Duplicates);
        let w = button_width(ui, &look, Content::both(icons::COPY, label));
        let r = Rect::from_min_size(pos2(right - w, cy - 16.0), vec2(w, 32.0));
        if button_at(
            ui,
            &colors,
            &look,
            Content::both(icons::COPY, label),
            !state.scanning_duplicates,
            r,
        )
        .on_hover_text(t(s::MySpace_DuplicatesTip))
        .clicked()
        {
            state.push(Command::ScanDuplicates);
        }
        right -= w;
    }
    right -= spacing;
    if state.space.can_upload() {
        let label = t(s::MySpace_UploadHere);
        let w = button_width(ui, &look, Content::both(icons::UPLOAD, label));
        let r = Rect::from_min_size(pos2(right - w, cy - 16.0), vec2(w, 32.0));
        if button_at(
            ui,
            &colors,
            &look,
            Content::both(icons::UPLOAD, label),
            true,
            r,
        )
        .on_hover_text(label)
        .clicked()
        {
            state.push(Command::UploadTo(state.space.current_folder.clone()));
        }
        right -= w;
    } else if state.space.trashed {
        let plain = strip_look(&colors).icon(15.0, None);
        let label = t(s::MySpace_EmptyTrash);
        let w = button_width(ui, &plain, Content::both(icons::TRASH_2, label));
        let r = Rect::from_min_size(pos2(right - w, cy - 16.0), vec2(w, 32.0));
        if button_at(
            ui,
            &colors,
            &plain,
            Content::both(icons::TRASH_2, label),
            !state.emptying_trash,
            r,
        )
        .on_hover_text(t(s::MySpace_EmptyTrashTip))
        .clicked()
        {
            state.push(Command::EmptyTrash);
        }
        right -= w;
    }
    right -= spacing;
    if !state.space.count_text.is_empty() {
        let galley = layout(
            ui,
            job(
                &state.space.count_text,
                11.0,
                Weight::Normal,
                colors.text_secondary(),
            ),
        );
        let w = galley.size().x;
        let mut layer = painter.clone();
        layer.set_opacity(0.75);
        layer.galley(
            pos2(right - 6.0 - w, cy - galley.size().y / 2.0),
            galley,
            colors.text_secondary(),
        );
        right -= w + 12.0;
    }
    right -= spacing;

    // The crumbs, in the space between.
    if show_crumbs {
        let clip = Rect::from_min_max(pos2(x, inner.top()), pos2(right.max(x), inner.bottom()));
        let mut cx = x;
        for crumb in &crumbs {
            if crumb.show_separator {
                let sep = icon_galley(ui, icons::CHEVRON_RIGHT, 12.0, colors.text_secondary());
                let w = sep.size().x;
                let mut faint = painter.with_clip_rect(clip);
                faint.set_opacity(0.5);
                faint.galley(
                    pos2(cx, cy - sep.size().y / 2.0),
                    sep,
                    colors.text_secondary(),
                );
                cx += w + 1.0;
            }
            let weight = if crumb.is_current {
                Weight::SemiBold
            } else {
                Weight::Normal
            };
            let color = if crumb.is_current {
                colors.text()
            } else {
                colors.text_secondary()
            };
            let galley = layout(ui, trimmed(&crumb.name, 13.0, weight, color, 220.0, 1));
            let w = galley.size().x + 16.0;
            let r = Rect::from_min_size(pos2(cx, cy - 14.0), vec2(w, 28.0));
            let visible = r.intersect(clip);
            if visible.width() > 0.0 {
                let response = ui.interact(
                    visible,
                    Id::new(("crumb", crumb.id.clone())),
                    Sense::click(),
                );
                let crumb_painter = painter.with_clip_rect(clip);
                if response.hovered() {
                    crumb_painter.rect_filled(r, 6.0, colors.control_background_hover());
                }
                crumb_painter.galley(
                    pos2(r.left() + 8.0, cy - galley.size().y / 2.0),
                    galley,
                    color,
                );
                if response.clicked() && !crumb.is_current {
                    state.push(Command::NavigateTo(crumb.id.clone()));
                }
            }
            cx += w + 1.0;
        }
    }
}

// ---------------------------------------------------------------------- library panel

fn library_panel(ui: &mut Ui, state: &mut AppState, host: Rect, out: f32) {
    let colors = state.colors.clone();
    let offset = -PANEL_WIDTH * (1.0 - out);
    let rect = host.translate(vec2(offset, 0.0));
    let clip = host;
    let painter = ui.painter().with_clip_rect(clip);
    painter.rect_filled(rect, 0.0, colors.background_panel());
    painter.vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, colors.border()),
    );

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(clip);
    let ui = &mut child;

    // Heading row: margin 12,10,8,6.
    let heading = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.top() + 10.0),
        vec2(rect.width() - 20.0, 26.0),
    );
    let galley = layout(
        ui,
        job(
            t(s::MySpace_Library),
            11.0,
            Weight::SemiBold,
            colors.text_secondary(),
        ),
    );
    ui.painter().galley(
        pos2(heading.left(), heading.center().y - galley.size().y / 2.0),
        galley,
        colors.text_secondary(),
    );
    let close = Rect::from_min_size(
        pos2(heading.right() - 26.0, heading.top()),
        vec2(26.0, 26.0),
    );
    if strip_icon(
        ui,
        &colors,
        close,
        icons::PANEL_LEFT_CLOSE,
        14.0,
        t(s::MySpace_LibraryPanel),
        true,
    )
    .clicked()
    {
        state.push(Command::ToggleLibraryPanel);
    }

    let list = Rect::from_min_max(
        pos2(rect.left(), heading.bottom() + 6.0),
        pos2(rect.right() - 1.0, rect.bottom()),
    );
    let mut list_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(list)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    list_ui.set_clip_rect(list.intersect(clip));
    egui::ScrollArea::vertical()
        .id_salt("library-panel")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            let width = list.width() - 16.0;
            ui.add_space(0.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    for (index, view) in LIBRARY_VIEWS.iter().enumerate() {
                        let current = state.space.is_view_current(view);
                        let response = folder_row(
                            ui,
                            &colors,
                            width,
                            view.icon,
                            t(view.name),
                            current,
                            false,
                            0,
                        );
                        if response.clicked() {
                            state.push(Command::SelectView(index));
                        }
                    }
                    // Rule: margin 2,8.
                    ui.add_space(8.0);
                    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
                    ui.painter().hline(
                        (r.left() + 2.0)..=(r.right() - 2.0),
                        r.center().y,
                        Stroke::new(1.0, colors.border()),
                    );
                    ui.add_space(8.0);
                    // "Folders" with the new-folder button: margin 8,2,0,4.
                    ui.add_space(2.0);
                    let (row, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
                    let galley = layout(
                        ui,
                        job(
                            t(s::MySpace_Folders),
                            11.0,
                            Weight::SemiBold,
                            colors.text_secondary(),
                        ),
                    );
                    ui.painter().galley(
                        pos2(row.left() + 8.0, row.center().y - galley.size().y / 2.0),
                        galley,
                        colors.text_secondary(),
                    );
                    let plus =
                        Rect::from_min_size(pos2(row.right() - 24.0, row.top()), vec2(24.0, 24.0));
                    if strip_icon(
                        ui,
                        &colors,
                        plus,
                        icons::FOLDER_PLUS,
                        13.0,
                        t(s::MySpace_NewFolder),
                        true,
                    )
                    .clicked()
                    {
                        state.push(Command::NewFolder(state.space.current_folder.clone()));
                    }
                    ui.add_space(4.0);
                    let folders: Vec<CloudFolder> =
                        state.space.panel_folders().into_iter().cloned().collect();
                    for folder in &folders {
                        let current =
                            state.space.current_folder.as_deref() == Some(folder.id.as_str());
                        let dropping = state.is_drop_target(&folder.id);
                        let response = folder_row(
                            ui,
                            &colors,
                            width,
                            folder.icon(),
                            &folder.name,
                            current,
                            dropping,
                            folder.depth,
                        );
                        let response = response.on_hover_text(folder.tooltip());
                        if state.drag.is_active() && response.contains_pointer() {
                            state.note_drag_over(folder);
                        }
                        if response.clicked() {
                            state.push(Command::NavigateTo(Some(folder.id.clone())));
                        }
                        if let Some(command) = crate::menus::context(&colors, &response, || {
                            crate::myspace::menus::folder(state, folder)
                        }) {
                            state.push(command);
                        }
                    }
                    ui.add_space(8.0);
                });
            });
        });
}

/// `Button.folder-row`: 32 tall, 2 apart, the icon and the name, indented by depth.
#[allow(clippy::too_many_arguments)]
fn folder_row(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    glyph: char,
    name: &str,
    current: bool,
    dropping: bool,
    depth: usize,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    ui.add_space(2.0);
    let hovered = response.hovered();
    let painter = ui.painter();
    if dropping {
        painter.rect_filled(rect, 6.0, colors.control_background_hover());
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, colors.accent_start()),
            StrokeKind::Inside,
        );
    } else if current {
        painter.rect_filled(rect, 6.0, colors.control_background_hover());
    } else if hovered {
        painter.rect_filled(rect, 6.0, colors.control_background());
    }
    let strong = current || hovered || dropping;
    let text_color = if strong {
        colors.text()
    } else {
        colors.text_secondary()
    };
    let icon_color = if current {
        colors.accent_start()
    } else {
        text_color
    };
    let x = rect.left() + 8.0 + depth as f32 * 14.0;
    let icon = icon_galley(ui, glyph, 14.0, icon_color);
    let icon_w = icon.size().x;
    painter.galley(
        pos2(x, rect.center().y - icon.size().y / 2.0),
        icon,
        icon_color,
    );
    let weight = if current {
        Weight::SemiBold
    } else {
        Weight::Normal
    };
    let max = (rect.right() - 8.0 - (x + icon_w + 9.0)).max(10.0);
    let galley = layout(ui, trimmed(name, 13.0, weight, text_color, max, 1));
    painter.galley(
        pos2(x + icon_w + 9.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    response
}

// ---------------------------------------------------------------------- the grid

/// How many items of `stride` fit a WrapPanel `available` wide (at least one).
fn columns(available: f32, stride: f32) -> usize {
    ((available + 0.01) / stride).floor().max(1.0) as usize
}

/// The height of one card, from the fonts it is drawn with.
pub fn card_height(ui: &Ui) -> f32 {
    let row = |size: f32, weight: Weight| {
        ui.fonts_mut(|f| f.row_height(&egui::FontId::new(size, weight.family())))
    };
    let name = row(13.0, Weight::Medium);
    let meta = row(11.0, Weight::Normal);
    let stats = meta
        .max(ui.fonts_mut(|f| f.row_height(&egui::FontId::new(12.0, crate::theme::icon_family()))));
    (1.0 + THUMB_HEIGHT + 8.0 + name + 2.0 + meta + 6.0 + stats + 10.0 + 1.0).round()
}

fn section_header(
    ui: &mut Ui,
    colors: &Colors,
    at: Pos2,
    label: &str,
    open: bool,
    id: &str,
) -> egui::Response {
    // `Button.section-header`: pad 6,3, margin -6 left, 40 tall (the global height).
    let chevron = icon_galley(ui, icons::CHEVRON_DOWN, 12.0, colors.text_secondary());
    let text = layout(
        ui,
        job(label, 11.0, Weight::SemiBold, colors.text_secondary()),
    );
    let w = 6.0 + chevron.size().x + 7.0 + text.size().x + 6.0;
    let rect = Rect::from_min_size(pos2(at.x - 6.0, at.y), vec2(w, SECTION_HEADER_HEIGHT));
    let response = ui.interact(rect, Id::new(("section-header", id)), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    if hovered || response.is_pointer_button_down_on() {
        painter.rect_filled(rect, 6.0, colors.control_background_hover());
    }
    let color = if hovered {
        colors.text()
    } else {
        colors.text_secondary()
    };
    let cy = rect.center().y;
    let center = pos2(rect.left() + 6.0 + chevron.size().x / 2.0, cy);
    let chevron_shape = icon_galley(ui, icons::CHEVRON_DOWN, 12.0, color);
    let angle = if open {
        0.0
    } else {
        -std::f32::consts::FRAC_PI_2
    };
    let size = chevron_shape.size();
    let text_shape = egui::epaint::TextShape::new(center - size / 2.0, chevron_shape, color);
    let rotated = rotate_text(text_shape, center, angle);
    painter.add(rotated);
    painter.galley(
        pos2(center.x + size.x / 2.0 + 7.0, cy - text.size().y / 2.0),
        text,
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn rotate_text(
    mut shape: egui::epaint::TextShape,
    center: Pos2,
    angle: f32,
) -> egui::epaint::TextShape {
    if angle == 0.0 {
        return shape;
    }
    let rot = egui::emath::Rot2::from_angle(angle);
    shape.pos = center + rot * (shape.pos - center);
    shape.angle = angle;
    shape
}

/// Folder shelf and file cards in one scroller.
/// The scroller with the shelf and the cards; returns the ids of the cards drawn.
fn grid(ui: &mut Ui, state: &mut AppState, rect: Rect) -> Vec<String> {
    let colors = state.colors.clone();
    let card_h = card_height(ui);
    let folders: Vec<CloudFolder> = state.space.visible_folders().into_iter().cloned().collect();
    let has_folders = !folders.is_empty();
    let has_files = state.space.has_files();
    let show_note = state.space.show_no_files_note();
    let show_footer = state.space.show_error_footer();

    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("cloud-scroller")
        .auto_shrink([false, false]);
    if state.scroll_to_top {
        scroll = scroll.vertical_scroll_offset(0.0);
        state.scroll_to_top = false;
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let mut drawn: Vec<String> = Vec::new();
    let output = scroll.show_viewport(&mut child, |ui, viewport| {
        let origin = ui.max_rect().min;
        let width = rect.width();
        let inner_width = (width - GRID_MARGIN_LEFT - GRID_MARGIN_RIGHT).max(1.0);
        let folder_cols = columns(inner_width, FOLDER_WIDTH + GAP);
        let card_cols = columns(inner_width, CARD_WIDTH + GAP);

        // Lay everything out top to bottom first, in content coordinates.
        let mut y = GRID_MARGIN_TOP;
        let shelf_top = y;
        let folders_header = if has_folders {
            let at = y;
            y += SECTION_HEADER_HEIGHT + 4.0;
            Some(at)
        } else {
            None
        };
        let mut folder_rects = Vec::new();
        if has_folders && state.folders_section {
            for (index, _) in folders.iter().enumerate() {
                let col = index % folder_cols;
                let row = index / folder_cols;
                folder_rects.push(Rect::from_min_size(
                    pos2(
                        GRID_MARGIN_LEFT + col as f32 * (FOLDER_WIDTH + GAP),
                        y + row as f32 * (FOLDER_HEIGHT + GAP),
                    ),
                    vec2(FOLDER_WIDTH, FOLDER_HEIGHT),
                ));
            }
            let rows = folders.len().div_ceil(folder_cols);
            y += rows as f32 * (FOLDER_HEIGHT + GAP);
        }
        let note_at = if has_folders && show_note {
            // `plan-note`, margin 1,2,12,10.
            let at = y + 2.0;
            y +=
                2.0 + ui.fonts_mut(|f| {
                    f.row_height(&egui::FontId::new(11.0, egui::FontFamily::Proportional))
                }) + 10.0;
            Some(at)
        } else {
            None
        };
        let shelf_height = if has_folders { y - shelf_top } else { 0.0 };
        let files_header = if has_files {
            y += 4.0;
            let at = y;
            y += SECTION_HEADER_HEIGHT + 4.0;
            Some(at)
        } else {
            None
        };
        let grid_top = y;
        let file_count = state.space.files.len();
        if has_files && state.files_section {
            let rows = file_count.div_ceil(card_cols);
            y += rows as f32 * (card_h + GAP);
        }
        // The footer stack: margin 0,4,12,16.
        y += 4.0;
        let footer_at = y;
        let footer_h = if show_footer {
            notice_height(ui, &state.space.error_text, inner_width - 12.0)
        } else {
            0.0
        };
        y += footer_h + 16.0;
        let content_height = y;
        ui.set_min_size(vec2(width, content_height));

        let to_screen = |r: Rect| r.translate(origin.to_vec2());
        let content_rect =
            Rect::from_min_size(origin, vec2(width, content_height.max(viewport.height())));

        // The background: clears the selection, carries the background menu.
        let background = ui.interact(content_rect, Id::new("cloud-background"), Sense::click());
        let pressed_here = background.hovered() && ui.input(|i| i.pointer.primary_pressed());
        let mut item_pressed = false;

        // Section headers.
        if let Some(at) = folders_header {
            let response = section_header(
                ui,
                &colors,
                origin + vec2(GRID_MARGIN_LEFT, at),
                t(s::MySpace_Folders),
                state.folders_section,
                "folders",
            );
            if response.clicked() {
                state.push(Command::ToggleFoldersSection);
            }
        }
        if let Some(at) = files_header {
            let response = section_header(
                ui,
                &colors,
                origin + vec2(GRID_MARGIN_LEFT, at),
                t(s::MySpace_Files),
                state.files_section,
                "files",
            );
            if response.clicked() {
                state.push(Command::ToggleFilesSection);
            }
        }

        // Folder tiles.
        for (folder, local) in folders.iter().zip(folder_rects.iter()) {
            if !local.intersects(viewport) {
                continue;
            }
            if folder_tile(ui, state, &colors, folder, to_screen(*local)) {
                item_pressed = true;
            }
        }
        if let Some(at) = note_at {
            let galley = layout(
                ui,
                wrapped(
                    t(s::MySpace_NoFilesHere),
                    11.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    inner_width - 13.0,
                ),
            );
            let mut faint = ui.painter().clone();
            faint.set_opacity(0.75);
            faint.galley(
                origin + vec2(GRID_MARGIN_LEFT + 1.0, at),
                galley,
                colors.text_secondary(),
            );
        }

        // File cards: only the ones on screen are drawn.
        let mut first_visible = None;
        let mut last_visible = None;
        if has_files && state.files_section {
            let first_row = ((viewport.top() - grid_top) / (card_h + GAP))
                .floor()
                .max(0.0) as usize;
            let last_row = ((viewport.bottom() - grid_top) / (card_h + GAP))
                .ceil()
                .max(0.0) as usize;
            let first = (first_row * card_cols).min(file_count);
            let last = ((last_row + 1) * card_cols).min(file_count);
            for index in first..last {
                let col = index % card_cols;
                let row = index / card_cols;
                let local = Rect::from_min_size(
                    pos2(
                        GRID_MARGIN_LEFT + col as f32 * (CARD_WIDTH + GAP),
                        grid_top + row as f32 * (card_h + GAP),
                    ),
                    vec2(CARD_WIDTH, card_h),
                );
                if !local.intersects(viewport) {
                    continue;
                }
                first_visible.get_or_insert(index);
                last_visible = Some(index);
                if let Some(entry) = state.space.files.get(index) {
                    drawn.push(entry.file.id.clone());
                }
                if card(ui, state, &colors, index, to_screen(local)) {
                    item_pressed = true;
                }
            }
        }
        if let (Some(first), Some(last)) = (first_visible, last_visible)
            && state.visible_range != Some((first, last))
        {
            state.visible_range = Some((first, last));
            state.set_visible_range(first, last);
        } else if first_visible.is_none() && has_files && !state.files_section {
            state.visible_range = None;
        }

        if show_footer {
            let r = Rect::from_min_size(
                origin + vec2(GRID_MARGIN_LEFT, footer_at),
                vec2(inner_width - 12.0, footer_h),
            );
            if error_notice(ui, &colors, r, &state.space.error_text) {
                state.push(Command::Reload);
            }
        }

        if pressed_here && !item_pressed && !state.drag.is_active() {
            state.space.clear_selection();
        }
        if let Some(command) = crate::menus::context(&colors, &background, || {
            crate::myspace::menus::background(state)
        }) {
            state.push(command);
        }
        let _ = shelf_height;
    });
    let _ = output;
    drawn
}

fn notice_height(ui: &Ui, text: &str, width: f32) -> f32 {
    let galley = layout(
        ui,
        wrapped(
            text,
            13.0,
            Weight::SemiBold,
            Color32::WHITE,
            (width - 28.0 - 26.0 - 100.0).max(40.0),
        ),
    );
    (galley.size().y.max(32.0) + 22.0 + 2.0).round()
}

/// `Border.notice` with the cloud-off glyph, the text and a Retry link. Returns retry.
fn error_notice(ui: &mut Ui, colors: &Colors, rect: Rect, text: &str) -> bool {
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 10.0, colors.control_background());
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let inner = rect.shrink2(vec2(14.0, 11.0));
    let icon = icon_galley(ui, icons::CLOUD_OFF, 16.0, colors.text_secondary());
    let icon_w = icon.size().x;
    painter.galley(
        pos2(inner.left(), inner.center().y - icon.size().y / 2.0),
        icon,
        colors.text_secondary(),
    );
    let look = Look::quiet_link(colors);
    let retry = t(s::MySpace_Retry);
    let retry_w = button_width(ui, &look, Content::label(retry));
    let retry_rect = Rect::from_min_size(
        pos2(inner.right() - retry_w, inner.center().y - 16.0),
        vec2(retry_w, 32.0),
    );
    let text_left = inner.left() + icon_w + 10.0;
    let galley = layout(
        ui,
        wrapped(
            text,
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
            (retry_rect.left() - 10.0 - text_left).max(40.0),
        ),
    );
    painter.galley(
        pos2(text_left, inner.center().y - galley.size().y / 2.0),
        galley,
        colors.text_secondary(),
    );
    button_at(ui, colors, &look, Content::label(retry), true, retry_rect).clicked()
}

/// One folder of the shelf (`Border.folder-tile`). Returns whether it was pressed.
fn folder_tile(
    ui: &mut Ui,
    state: &mut AppState,
    colors: &Colors,
    folder: &CloudFolder,
    rect: Rect,
) -> bool {
    let id = Id::new(("folder-tile", folder.id.as_str()));
    let response = ui.interact(rect, id, Sense::click());
    let hovered = response.hovered();
    let selected = state.space.selected_folder.as_deref() == Some(folder.id.as_str());
    let dropping = state.is_drop_target(&folder.id);
    let landed = state.space.is_flashing(&folder.id, state.time);
    if state.drag.is_active() && response.contains_pointer() {
        state.note_drag_over(folder);
    }
    if landed {
        ui.ctx().request_repaint();
    }

    let painter = ui.painter().clone();
    let tile = if landed {
        Rect::from_center_size(rect.center(), rect.size() * 1.035)
    } else {
        rect
    };
    let fill = if selected || dropping || landed {
        colors.control_background_hover()
    } else {
        colors.control_background()
    };
    let border = if selected || dropping || landed || hovered {
        colors.accent_start()
    } else {
        colors.border()
    };
    painter.rect_filled(tile, 10.0, fill);
    painter.rect_stroke(
        tile,
        10.0,
        Stroke::new(if landed { 2.0 } else { 1.0 }, border),
        StrokeKind::Inside,
    );

    let inner = tile.shrink(10.0);
    // The icon square (its own hit target: a click goes straight in).
    let icon_rect = Rect::from_min_size(
        pos2(inner.left(), inner.center().y - 21.0),
        vec2(42.0, 42.0),
    );
    let icon_response = ui.interact(icon_rect, id.with("icon"), Sense::click());
    let icon_scale = if icon_response.hovered() { 1.08 } else { 1.0 };
    let shown_icon = Rect::from_center_size(icon_rect.center(), icon_rect.size() * icon_scale);
    painter.rect_filled(shown_icon, 8.0, colors.control_background_hover());
    let glyph_color = if folder.is_public || selected {
        colors.accent_start()
    } else {
        colors.text_secondary()
    };
    paint_icon(
        ui,
        &painter,
        shown_icon.center(),
        folder.icon(),
        18.0,
        glyph_color,
    );
    if folder.is_public {
        let badge = Rect::from_min_size(
            pos2(
                shown_icon.right() + 3.0 - 15.0,
                shown_icon.bottom() + 3.0 - 15.0,
            ),
            vec2(15.0, 15.0),
        );
        painter.circle_filled(badge.center(), 7.5, colors.accent_start());
        paint_icon(
            ui,
            &painter,
            badge.center(),
            icons::GLOBE,
            9.0,
            colors.accent_foreground(),
        );
    }

    // The send button (real folders only).
    let send_rect = Rect::from_min_size(
        pos2(inner.right() - 30.0, inner.center().y - 15.0),
        vec2(30.0, 30.0),
    );
    let text_right = if folder.is_real_folder() {
        send_rect.left() - 10.0
    } else {
        inner.right()
    };
    let text_left = icon_rect.right() + 10.0;
    let name_weight = if selected {
        Weight::SemiBold
    } else {
        Weight::Normal
    };
    let name = layout(
        ui,
        trimmed(
            &folder.name,
            14.0,
            name_weight,
            colors.text(),
            (text_right - text_left).max(10.0),
            1,
        ),
    );
    let meta = layout(
        ui,
        trimmed(
            &folder.meta(),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            (text_right - text_left).max(10.0),
            1,
        ),
    );
    let stack = name.size().y + 2.0 + meta.size().y;
    let top = inner.center().y - stack / 2.0;
    let name_h = name.size().y;
    painter.galley(pos2(text_left, top), name, colors.text());
    painter.galley(
        pos2(text_left, top + name_h + 2.0),
        meta,
        colors.text_secondary(),
    );

    let mut pressed = false;
    if folder.is_real_folder() {
        let send = ui.interact(send_rect, id.with("send"), Sense::click());
        let mut layer = painter.clone();
        layer.set_opacity(if hovered || send.hovered() { 1.0 } else { 0.4 });
        if send.hovered() {
            layer.rect_filled(send_rect, 8.0, colors.background_popup());
        }
        paint_icon(
            ui,
            &layer,
            send_rect.center(),
            icons::UPLOAD,
            15.0,
            colors.text_secondary(),
        );
        let send = send.on_hover_text(t(s::MySpace_UploadToFolder));
        if send.clicked() {
            state.space.select_folder_tile(Some(&folder.id));
            state.push(Command::UploadTo(Some(folder.id.clone())));
            pressed = true;
        }
    }

    // Presses: mark on press, open on double click or on an icon click.
    let press = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
            i.pointer.interact_pos(),
        )
    });
    if (response.hovered() || icon_response.hovered()) && (press.0 || press.1) {
        state.space.select_folder_tile(Some(&folder.id));
        pressed = true;
    }
    if icon_response.clicked() {
        state.push(Command::NavigateTo(Some(folder.id.clone())));
    }
    if response.double_clicked() || icon_response.double_clicked() {
        state.push(Command::NavigateTo(Some(folder.id.clone())));
    }
    let tooltip = folder.tooltip();
    let response = response.on_hover_text(tooltip);
    for target in [&response, &icon_response] {
        if let Some(command) = crate::menus::context(colors, target, || {
            crate::myspace::menus::folder(state, folder)
        }) {
            state.push(command);
        }
    }
    pressed
}

/// One card (`Border.cloud-card` + `CloudFileTile`). Returns whether it was pressed.
fn card(ui: &mut Ui, state: &mut AppState, colors: &Colors, index: usize, rect: Rect) -> bool {
    let Some(entry) = state.space.files.get(index) else {
        return false;
    };
    let file = entry.file.clone();
    let selected = entry.selected;
    let id = Id::new(("cloud-card", file.id.as_str()));
    let response = ui.interact(rect, id, Sense::click());
    let hovered = response.hovered();
    let mut pressed = false;

    // Motion preview: enter and leave.
    let hover_now = response.contains_pointer() && !state.drag.is_active();
    let was = state
        .media
        .motion
        .hovered
        .as_ref()
        .is_some_and(|(h, _)| *h == file.id);
    if hover_now && !was {
        state.hover_file(&file.id);
    } else if !hover_now && was {
        state.unhover_file(&file.id);
    }

    // The frame.
    let painter = ui.painter().clone();
    let fill = if selected {
        colors.control_background_hover()
    } else {
        colors.control_background()
    };
    let border = if selected || hovered {
        colors.accent_start()
    } else {
        colors.border()
    };
    painter.rect_filled(rect, 10.0, fill);

    // The picture.
    let thumb = Rect::from_min_size(
        rect.min + vec2(1.0, 1.0),
        vec2(rect.width() - 2.0, THUMB_HEIGHT),
    );
    let thumb_radius = CornerRadius {
        nw: 9,
        ne: 9,
        sw: 0,
        se: 0,
    };
    painter.rect_filled(
        thumb,
        thumb_radius,
        if selected {
            colors.background_main()
        } else {
            colors.background_panel()
        },
    );
    let still = state.media.thumbs.get(&file.id).cloned();
    match &still {
        Some(texture) => paint_cover(&painter, thumb, texture, thumb_radius),
        None => type_mark(ui, &painter, colors, &file, thumb.center()),
    }
    if let Some((playing, reel)) = &state.media.motion.playing
        && *playing == file.id
    {
        paint_cover(&painter, thumb, reel.frame(), thumb_radius);
    }
    // Badges: password, expiry.
    let mut bx = thumb.right() - 8.0;
    for (shown, glyph) in [
        (file.expires, icons::CLOCK),
        (file.has_password, icons::LOCK_KEYHOLE),
    ] {
        if !shown {
            continue;
        }
        let g = icon_galley(ui, glyph, 14.0, colors.text_secondary());
        let size = vec2(g.size().x + 10.0, g.size().y + 6.0);
        let badge = Rect::from_min_size(pos2(bx - size.x, thumb.top() + 8.0), size);
        painter.rect_filled(badge, 6.0, colors.background_popup());
        painter.galley(badge.min + vec2(5.0, 3.0), g, colors.text_secondary());
        bx = badge.left() - 4.0;
    }

    // The caption: name and meta, the menu handle.
    let caption_left = rect.left() + 1.0 + 10.0;
    let menu_rect = Rect::from_min_size(
        pos2(rect.right() - 1.0 - 6.0 - 26.0, thumb.bottom() + 8.0 - 2.0),
        vec2(26.0, 26.0),
    );
    let text_w = (menu_rect.left() - caption_left).max(10.0);
    let name = layout(
        ui,
        trimmed(&file.name, 13.0, Weight::Medium, colors.text(), text_w, 1),
    );
    let meta_text = file.meta(state.env.now());
    let meta = layout(
        ui,
        trimmed(
            &meta_text,
            11.0,
            Weight::Normal,
            colors.text_secondary(),
            text_w,
            1,
        ),
    );
    let name_top = thumb.bottom() + 8.0;
    let name_h = name.size().y;
    let meta_h = meta.size().y;
    painter.galley(pos2(caption_left, name_top), name, colors.text());
    painter.galley(
        pos2(caption_left, name_top + name_h + 2.0),
        meta,
        colors.text_secondary(),
    );

    // Stats: views, folder.
    let stats_top = name_top + name_h + 2.0 + meta_h + 6.0;
    let stats_bottom = rect.bottom() - 1.0 - 10.0;
    let cy = (stats_top + stats_bottom) / 2.0;
    let mut sx = caption_left;
    let mut faint = painter.clone();
    faint.set_opacity(0.8);
    let eye = icon_galley(ui, icons::EYE, 12.0, colors.text_secondary());
    let eye_w = eye.size().x;
    faint.galley(
        pos2(sx, cy - eye.size().y / 2.0),
        eye,
        colors.text_secondary(),
    );
    sx += eye_w + 4.0;
    let views = layout(
        ui,
        job(
            &file.views_text(),
            11.0,
            Weight::Normal,
            colors.text_secondary(),
        ),
    );
    let views_w = views.size().x;
    painter.galley(
        pos2(sx, cy - views.size().y / 2.0),
        views,
        colors.text_secondary(),
    );
    sx += views_w + 10.0;
    let folder_name = state.space.folder_name_for(file.folder_id.as_deref());
    if file.folder_id.is_some() && !folder_name.is_empty() {
        let folder_icon = icon_galley(ui, icons::FOLDER, 12.0, colors.text_secondary());
        let fw = folder_icon.size().x;
        faint.galley(
            pos2(sx, cy - folder_icon.size().y / 2.0),
            folder_icon,
            colors.text_secondary(),
        );
        sx += fw + 4.0;
        let right = rect.right() - 1.0 - 10.0;
        let label = layout(
            ui,
            trimmed(
                &folder_name,
                11.0,
                Weight::Normal,
                colors.text_secondary(),
                (right - sx).max(10.0),
                1,
            ),
        );
        painter.galley(
            pos2(sx, cy - label.size().y / 2.0),
            label,
            colors.text_secondary(),
        );
    }

    painter.rect_stroke(rect, 10.0, Stroke::new(1.0, border), StrokeKind::Inside);

    // The menu handle: always drawn, half strength until the card is under the pointer.
    let menu = ui.interact(menu_rect, id.with("menu"), Sense::click());
    {
        let mut layer = painter.clone();
        layer.set_opacity(if hovered || menu.hovered() { 1.0 } else { 0.5 });
        if menu.hovered() || menu.is_pointer_button_down_on() {
            layer.rect_filled(menu_rect, 8.0, colors.background_popup());
        }
        paint_icon(
            ui,
            &layer,
            menu_rect.center(),
            icons::ELLIPSIS,
            16.0,
            colors.text_secondary(),
        );
    }
    if menu.clicked() && !state.space.files.get(index).is_some_and(|f| f.selected) {
        state.space.select_file(&file.id);
    }
    let file_id = file.id.clone();
    if let Some(command) = crate::menus::popup_below(colors, &menu, || {
        crate::myspace::menus::file(state, &file_id)
    }) {
        state.push(command);
    }
    if menu.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
        pressed = true;
    }

    // Presses on the card itself (the handle is on top and takes its own).
    let (primary, secondary, modifiers, pointer) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
            i.modifiers,
            i.pointer.interact_pos(),
        )
    });
    if hovered && !menu.hovered() && (primary || secondary) {
        pressed = true;
        state.details.pending_open = None;
        state.drag.press = None;
        if secondary {
            if !selected {
                state.space.select_file(&file.id);
            }
        } else {
            if modifiers.shift {
                state.space.select_file_range(&file.id);
            } else if modifiers.command {
                state.space.toggle_file(&file.id);
            } else {
                state.space.select_file(&file.id);
            }
            if let Some(at) = pointer {
                state.drag.press = Some(Press {
                    id: file.id.clone(),
                    at,
                    modifiers,
                });
            }
        }
    }
    if response.clicked() && !menu.hovered() {
        let press_mods = state
            .drag
            .press
            .as_ref()
            .map(|p| p.modifiers)
            .unwrap_or(modifiers);
        if !press_mods.command
            && !press_mods.shift
            && !state.drag.is_active()
            && !state.details.present
        {
            state.details.pending_open = Some((file.id.clone(), state.time + DOUBLE_CLICK_WINDOW));
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    DOUBLE_CLICK_WINDOW + 0.02,
                ));
        }
    }
    if response.double_clicked() {
        state.details.pending_open = None;
        if file.can_open() {
            state.open_url(&file.open_link);
        }
    }
    if let Some(command) = crate::menus::context(colors, &response, || {
        crate::myspace::menus::file(state, &file.id)
    }) {
        state.push(command);
    }
    let _ = CLICK_SLOP;
    pressed
}

/// The mark of a file with no picture (`CloudFileTile` type plate): a sheet with a
/// folded corner in the family's colour, its emblem, and the extension on a chip.
fn type_mark(ui: &Ui, painter: &egui::Painter, colors: &Colors, file: &CloudFile, center: Pos2) {
    // The 64 x 80 sheet, drawn 56 x 70.
    let scale = 56.0 / 64.0;
    let origin = center - vec2(28.0, 35.0);
    let at = |x: f32, y: f32| origin + vec2(x * scale, y * scale);
    let mut sheet = vec![at(8.0, 0.0), at(40.0, 0.0), at(64.0, 24.0)];
    sheet.push(at(64.0, 72.0));
    arc(&mut sheet, at(56.0, 72.0), 8.0 * scale, 0.0, 90.0);
    sheet.push(at(8.0, 80.0));
    arc(&mut sheet, at(8.0, 72.0), 8.0 * scale, 90.0, 180.0);
    sheet.push(at(0.0, 8.0));
    arc(&mut sheet, at(8.0, 8.0), 8.0 * scale, 180.0, 270.0);
    let plate = rgba(file.family.plate());
    painter.add(egui::Shape::convex_polygon(
        sheet.clone(),
        plate,
        Stroke::NONE,
    ));
    let mut fold = vec![at(40.0, 0.0), at(64.0, 24.0), at(48.0, 24.0)];
    arc(&mut fold, at(48.0, 16.0), 8.0 * scale, 90.0, 180.0);
    fold.push(at(40.0, 0.0));
    painter.add(egui::Shape::convex_polygon(
        fold,
        Color32::from_rgba_unmultiplied(255, 255, 255, 0x42),
        Stroke::NONE,
    ));
    // The rim: the path's own edge, half of it clipped away, at 35%.
    let rim = colors.text().gamma_multiply(0.35);
    painter.add(PathShape::closed_line(sheet, PathStroke::new(1.0, rim)));

    // The emblem and the extension chip, 3 apart, 8 down from centre.
    let emblem = icon_galley(
        ui,
        file.family.emblem(),
        17.0,
        Color32::from_rgba_unmultiplied(255, 255, 255, 0xD9),
    );
    let label_size = modlight_core::library::files::type_label_size(&file.type_label);
    let chip_text = layout(
        ui,
        job(
            &file.type_label,
            label_size,
            Weight::SemiBold,
            Color32::WHITE,
        ),
    );
    let chip = vec2(chip_text.size().x + 6.0, chip_text.size().y + 1.0);
    let stack = emblem.size().y + 3.0 + chip.y;
    let top = center.y + 4.0 - stack / 2.0;
    painter.galley(
        pos2(center.x - emblem.size().x / 2.0, top),
        emblem.clone(),
        Color32::from_rgba_unmultiplied(255, 255, 255, 0xD9),
    );
    let chip_rect = Rect::from_min_size(
        pos2(center.x - chip.x / 2.0, top + emblem.size().y + 3.0),
        chip,
    );
    painter.rect_filled(chip_rect, 3.0, Color32::from_black_alpha(0x40));
    painter.galley(chip_rect.min + vec2(3.0, 0.0), chip_text, Color32::WHITE);
}

fn arc(points: &mut Vec<Pos2>, center: Pos2, radius: f32, from_deg: f32, to_deg: f32) {
    for step in 1..=6 {
        let a = (from_deg + (to_deg - from_deg) * step as f32 / 6.0).to_radians();
        points.push(pos2(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
        ));
    }
}

// ---------------------------------------------------------------------- overlays

/// The signed-out, loading, empty and error panels, centred in the list area.
fn overlays(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let colors = state.colors.clone();
    if !state.space.has_account {
        if plan_card(
            ui,
            &colors,
            rect,
            icons::LOG_IN,
            colors.accent_start(),
            t(s::MySpace_SignedOutTitle),
            t(s::MySpace_SignedOutBody),
            Some((icons::LOG_IN, t(s::MainWindow_SignIn))),
            None,
        ) {
            state.push(Command::ShowSignIn);
        }
        return;
    }
    if state.space.show_loading_panel() {
        let painter = ui.painter();
        let icon = icon_galley(ui, icons::CLOUD, 22.0, colors.text_secondary());
        let note = layout(
            ui,
            job(
                t(s::MySpace_Loading),
                11.0,
                Weight::Normal,
                colors.text_secondary(),
            ),
        );
        let h = icon.size().y + 12.0 + note.size().y;
        let top = rect.center().y - h / 2.0;
        let mut faint = painter.clone();
        faint.set_opacity(0.5);
        let icon_h = icon.size().y;
        faint.galley(
            pos2(rect.center().x - icon.size().x / 2.0, top),
            icon,
            colors.text_secondary(),
        );
        let mut note_painter = painter.clone();
        note_painter.set_opacity(0.75);
        note_painter.galley(
            pos2(rect.center().x - note.size().x / 2.0, top + icon_h + 12.0),
            note,
            colors.text_secondary(),
        );
        ui.ctx().request_repaint();
    }
    if state.space.show_empty_panel()
        && plan_card(
            ui,
            &colors,
            rect,
            icons::INBOX,
            colors.accent_start(),
            t(s::MySpace_EmptyTitle),
            t(s::MySpace_EmptyBody),
            Some((icons::SPARKLES, t(s::MySpace_EmptyAction))),
            None,
        )
    {
        state.push(Command::SelectTab(crate::state::Tab::Create));
    }
    if state.space.show_error_panel() {
        let text = state.space.error_text.clone();
        if plan_card(
            ui,
            &colors,
            rect,
            icons::CLOUD_OFF,
            colors.cancel_border(),
            t(s::MySpace_ErrorTitle),
            &text,
            Some((icons::REFRESH_CW, t(s::MySpace_Retry))),
            None,
        ) {
            state.push(Command::Reload);
        }
    }
}

/// `Border.plan-card`: 560 wide, centred, a badge, a title, a body, maybe a note and
/// a button. Returns whether the button was clicked.
#[allow(clippy::too_many_arguments)]
pub fn plan_card(
    ui: &mut Ui,
    colors: &Colors,
    area: Rect,
    glyph: char,
    glyph_color: Color32,
    title: &str,
    body: &str,
    action: Option<(char, &str)>,
    note: Option<&str>,
) -> bool {
    let width = 560.0_f32.min(area.width() - 8.0).max(200.0);
    let inner_w = width - 44.0;
    let text_w = inner_w - 42.0 - 13.0;
    let title_g = layout(
        ui,
        wrapped(title, 17.0, Weight::SemiBold, colors.text(), text_w),
    );
    let body_g = layout(
        ui,
        wrapped(body, 13.0, Weight::Normal, colors.text_secondary(), text_w),
    );
    let note_g = note.map(|n| {
        layout(
            ui,
            wrapped(n, 11.0, Weight::Normal, colors.text_secondary(), inner_w),
        )
    });
    let text_h = title_g.size().y + 6.0 + body_g.size().y;
    let head_h = text_h.max(42.0);
    // Graphite draws the card's one action as its primary button.
    let look = if colors.graphite {
        Look::primary(colors)
    } else {
        Look::button(colors)
    };
    let mut h = 22.0 + head_h;
    if let Some(g) = &note_g {
        h += 14.0 + g.size().y;
    }
    if action.is_some() {
        h += 14.0 + 40.0;
    }
    h += 22.0;
    let rect = Rect::from_center_size(area.center(), vec2(width, h));
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 10.0, colors.background_panel());
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let left = rect.left() + 22.0;
    let mut y = rect.top() + 22.0;
    let badge = Rect::from_min_size(pos2(left, y), vec2(42.0, 42.0));
    painter.rect_filled(badge, 8.0, colors.control_background());
    paint_icon(ui, &painter, badge.center(), glyph, 20.0, glyph_color);
    let text_top = y + (head_h - text_h) / 2.0;
    let title_h = title_g.size().y;
    painter.galley(pos2(badge.right() + 13.0, text_top), title_g, colors.text());
    painter.galley(
        pos2(badge.right() + 13.0, text_top + title_h + 6.0),
        body_g,
        colors.text_secondary(),
    );
    y += head_h;
    if let Some(g) = note_g {
        y += 14.0;
        let gh = g.size().y;
        let mut faint = painter.clone();
        faint.set_opacity(0.75);
        faint.galley(pos2(left, y), g, colors.text_secondary());
        y += gh;
    }
    let mut clicked = false;
    if let Some((icon, label)) = action {
        y += 14.0;
        let content = Content::both(icon, label);
        let w = button_width(ui, &look, content);
        let r = Rect::from_min_size(pos2(left, y), vec2(w, 40.0));
        clicked = button_at(ui, colors, &look, content, true, r).clicked();
    }
    let _ = (
        Fill::None,
        Align2::CENTER_CENTER,
        paint_surface as fn(&egui::Painter, &Colors, Rect, f32, Fill, Option<Color32>),
    );
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{Animation, Reel};

    /// A state with one video card whose moving preview is playing under the pointer.
    fn playing(ctx: &egui::Context) -> AppState {
        let mut state = crate::myspace::actions::tests::offline_state();
        crate::myspace::actions::tests::listing(&mut state, 1, 30, 1, &["v"]);
        let frame = egui::ColorImage::filled([4, 4], Color32::RED);
        let reel = Reel::new(
            ctx,
            "test-reel",
            Animation {
                frames: vec![frame.clone(), frame],
                delays_ms: vec![80, 80],
            },
            0.0,
        );
        state.media.motion.hovered = Some(("v".into(), 0.0));
        state.media.motion.playing = Some(("v".into(), reel));
        state
    }

    /// A card that is no longer drawn (scrolled out of view, its section folded) is not
    /// under the pointer any more: its moving preview stops. It used to play on, asking
    /// for a frame every 80 ms, until the pointer happened to reach another card.
    #[test]
    fn a_card_that_is_not_drawn_stops_moving() {
        let ctx = egui::Context::default();
        let mut state = playing(&ctx);
        state.files_section = false;
        // The first pass runs while the harness is built, before the app's fonts are in.
        let fonts_in = std::rc::Rc::new(std::cell::Cell::new(false));
        let ready = fonts_in.clone();
        let mut harness = egui_kittest::Harness::new_ui_state(
            move |ui, state: &mut AppState| {
                if ready.get() {
                    super::ui(ui, state);
                }
            },
            state,
        );
        harness.ctx.set_fonts(crate::theme::fonts());
        harness.step();
        fonts_in.set(true);
        harness.step();
        let state = harness.state();
        assert!(state.media.motion.hovered.is_none());
        assert!(state.media.motion.playing.is_none());
    }

    /// Nor does a reel go on ticking behind another tab.
    #[test]
    fn another_tab_stops_the_moving_preview() {
        let ctx = egui::Context::default();
        let mut state = playing(&ctx);
        state.tab = crate::state::Tab::Upload;
        state.tick_motion(&ctx);
        assert!(state.media.motion.hovered.is_none());
        assert!(state.media.motion.playing.is_none());
    }
}

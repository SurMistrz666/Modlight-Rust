//! The pager bar under the grid (`LibraryPagerBar.axaml`): where in the library the
//! page is, how many per page, and the strip of pages with a jump box.

use egui::{Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t};
use modlight_core::library::{Pager, pager::PAGE_SIZES};

use crate::commands::Command;
use crate::menus::Entry;
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{Field, icon_galley, job, layout};

const CONTROL_HEIGHT: f32 = 28.0;

/// The bar's height, or 0 when there is nothing to page through.
pub fn height(ui: &Ui, state: &AppState) -> f32 {
    let pager = &state.space.pager;
    if !pager.is_visible() {
        return 0.0;
    }
    let text =
        ui.fonts_mut(|f| f.row_height(&egui::FontId::new(12.0, egui::FontFamily::Proportional)));
    let controls = if pager.show_sizes() || pager.has_pages() {
        CONTROL_HEIGHT
    } else {
        0.0
    };
    (1.0 + 9.0 + text.max(controls) + 9.0).round()
}

/// One `Button.pager-step`: 30 x 28, transparent until hovered; the current page on
/// the control surface in accent.
#[allow(clippy::too_many_arguments)]
fn step(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: Id,
    label: Option<&str>,
    glyph: Option<char>,
    current: bool,
    enabled: bool,
) -> egui::Response {
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    if current {
        painter.rect_filled(rect, 8.0, colors.control_background());
    } else if hovered {
        painter.rect_filled(rect, 8.0, colors.control_background_hover());
    }
    let color = if current {
        colors.accent_start()
    } else if hovered {
        colors.text()
    } else {
        colors.text_secondary()
    };
    let alpha = if enabled { 1.0 } else { 0.4 };
    if let Some(label) = label {
        let weight = if current {
            Weight::SemiBold
        } else {
            Weight::Normal
        };
        let galley = layout(ui, job(label, 12.0, weight, color));
        painter.galley(
            rect.center() - galley.size() / 2.0,
            galley,
            color.gamma_multiply(alpha),
        );
    }
    if let Some(glyph) = glyph {
        let galley = icon_galley(ui, glyph, 13.0, color.gamma_multiply(alpha));
        painter.galley(
            rect.center() - galley.size() / 2.0,
            galley,
            color.gamma_multiply(alpha),
        );
    }
    response
}

pub fn ui(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let colors = state.colors.clone();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_panel());
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + 16.0, rect.top() + 1.0 + 9.0),
        pos2(rect.right() - 16.0, rect.bottom() - 9.0),
    );
    let cy = inner.center().y;

    // Left: "1-120 of 6 926" and the size picker.
    let mut x = inner.left();
    let pager = state.space.pager.clone();
    if !pager.showing_text.is_empty() {
        let galley = layout(
            ui,
            job(
                &pager.showing_text,
                12.0,
                Weight::SemiBold,
                colors.text_secondary(),
            ),
        );
        let w = galley.size().x;
        painter.galley(
            pos2(x, cy - galley.size().y / 2.0),
            galley,
            colors.text_secondary(),
        );
        x += w + 10.0;
    }
    if pager.show_sizes() {
        let label = Pager::size_label(pager.per_page);
        let text = layout(ui, job(&label, 12.0, Weight::Normal, colors.text()));
        let chevron = icon_galley(ui, icons::CHEVRON_DOWN, 12.0, colors.text_secondary());
        let w = 9.0 + text.size().x + 8.0 + chevron.size().x + 9.0;
        let r = Rect::from_min_size(pos2(x, cy - CONTROL_HEIGHT / 2.0), vec2(w, CONTROL_HEIGHT));
        let response = ui.interact(r, Id::new("pager-size"), Sense::click());
        painter.rect_filled(
            r,
            8.0,
            if response.hovered() {
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
        painter.galley(
            pos2(r.left() + 9.0, cy - text.size().y / 2.0),
            text,
            colors.text(),
        );
        painter.galley(
            pos2(
                r.right() - 9.0 - chevron.size().x,
                cy - chevron.size().y / 2.0,
            ),
            chevron,
            colors.text_secondary(),
        );
        let response = response.on_hover_text(t(s::MySpace_PerPageHint));
        let current = pager.per_page;
        let picked = egui::Popup::menu(&response)
            .frame(crate::menus::frame(&colors))
            .align(egui::RectAlign::TOP_START)
            .gap(4.0)
            .show(|ui| {
                let entries: Vec<Entry> = PAGE_SIZES
                    .iter()
                    .map(|size| Entry::Item {
                        label: Pager::size_label(*size),
                        icon: None,
                        enabled: true,
                        toggle: crate::menus::Toggle::Check(*size == current),
                        command: Some(Command::SetPageSize(*size)),
                    })
                    .collect();
                crate::menus::entries(ui, &colors, &entries)
            })
            .and_then(|r| r.inner);
        if let Some(command) = picked {
            state.push(command);
        }
    }

    // Right: previous, the strip, next, the jump box.
    if !pager.has_pages() {
        return;
    }
    let mut widths = Vec::new();
    for page_step in &pager.steps {
        widths.push(if page_step.is_gap() { 18.0 } else { 30.0 });
    }
    let jump_w = if pager.show_jump() { 6.0 + 58.0 } else { 0.0 };
    let strip_w: f32 = 30.0 + 2.0 + widths.iter().map(|w| w + 2.0).sum::<f32>() + 30.0 + jump_w;
    let mut sx = inner.right() - strip_w;
    let row = |x: f32, w: f32| {
        Rect::from_min_size(pos2(x, cy - CONTROL_HEIGHT / 2.0), vec2(w, CONTROL_HEIGHT))
    };

    let previous = step(
        ui,
        &colors,
        row(sx, 30.0),
        Id::new("pager-previous"),
        None,
        Some(icons::CHEVRON_LEFT),
        false,
        pager.can_previous(),
    );
    if previous.on_hover_text(t(s::MySpace_PreviousPage)).clicked()
        && let Some(target) = pager.go_to(pager.page - 1)
    {
        state.push(Command::GoToPage(target));
    }
    sx += 32.0;
    for (page_step, w) in pager.steps.iter().zip(widths) {
        if page_step.is_gap() {
            let galley = layout(
                ui,
                job(
                    &page_step.label,
                    12.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                ),
            );
            let mut faint = painter.clone();
            faint.set_opacity(0.7);
            faint.galley(
                pos2(sx + (w - galley.size().x) / 2.0, cy - galley.size().y / 2.0),
                galley,
                colors.text_secondary(),
            );
        } else {
            let response = step(
                ui,
                &colors,
                row(sx, w),
                Id::new(("pager-step", page_step.page)),
                Some(&page_step.label),
                None,
                page_step.is_current,
                true,
            );
            if response.clicked()
                && let Some(target) = pager.go_to(page_step.page)
            {
                state.push(Command::GoToPage(target));
            }
        }
        sx += w + 2.0;
    }
    let next = step(
        ui,
        &colors,
        row(sx, 30.0),
        Id::new("pager-next"),
        None,
        Some(icons::CHEVRON_RIGHT),
        false,
        pager.can_next(),
    );
    if next.on_hover_text(t(s::MySpace_NextPage)).clicked()
        && let Some(target) = pager.go_to(pager.page + 1)
    {
        state.push(Command::GoToPage(target));
    }
    sx += 30.0;
    if pager.show_jump() {
        sx += 6.0;
        let r = row(sx, 58.0);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let response = Field {
            height: CONTROL_HEIGHT,
            width: Some(58.0),
            hint: &pager.page_of_text,
            font_size: 12.0,
            padding_x: 8.0,
            id: Some(Id::new("pager-jump")),
            ..Field::new(&mut state.space.pager.jump_text)
        }
        .show(&mut child, &colors);
        let response = response.on_hover_text(t(s::MySpace_JumpTo));
        if response.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(target) = state.space.pager.jump()
        {
            state.push(Command::GoToPage(target));
        }
        // Escape gives up on the typed page (`OnJumpKeyDown`).
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.space.pager.jump_text.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `OnJumpKeyDown`: Enter takes the typed page, Escape gives up on it and empties
    /// the box. Escape used to leave the number standing in it.
    #[test]
    fn escape_gives_up_on_the_typed_page() {
        let mut state = crate::myspace::actions::tests::offline_state();
        crate::myspace::actions::tests::listing(&mut state, 1, 30, 1000, &["a"]);
        assert!(state.space.pager.show_jump());
        // The first pass runs while the harness is built, before the app's fonts are in.
        let fonts_in = std::rc::Rc::new(std::cell::Cell::new(false));
        let ready = fonts_in.clone();
        let mut harness = egui_kittest::Harness::new_ui_state(
            move |ui, state: &mut AppState| {
                if ready.get() {
                    let rect = ui.max_rect();
                    super::ui(ui, state, rect);
                }
            },
            state,
        );
        harness.ctx.set_fonts(crate::theme::fonts());
        harness.step();
        fonts_in.set(true);
        harness.step();
        harness
            .ctx
            .memory_mut(|m| m.request_focus(Id::new("pager-jump")));
        harness.step();
        harness.event(egui::Event::Text("7".into()));
        harness.step();
        assert_eq!(harness.state().space.pager.jump_text, "7");
        harness.key_press(egui::Key::Escape);
        harness.step();
        assert_eq!(harness.state().space.pager.jump_text, "");
        assert!(harness.state().pending.is_empty(), "no page was asked for");
    }
}

//! `EffectBrowserPanel`: the floating panel over a soft scrim with the search box, the
//! Recent and Favourites groups and every category of the registry. A click runs an
//! effect (its dialog opens on the right, or it applies at once); a right click removes
//! a recent one or toggles a favourite.

use egui::{Align2, Id, Rect, Sense, Ui, Vec2, pos2, vec2};

use super::style::{self, ButtonKind, Colors, Weight};
use crate::editor::Editor;
use crate::effects::{self, Category, ImageEffect};
use modlight_core::l10n::{editor as s, t, tf};

const PANEL_WIDTH: f32 = 800.0;

/// The panel's own state (the search text).
#[derive(Default)]
pub struct EffectBrowser {
    pub search: String,
    focus_search: bool,
}

struct Group {
    header: String,
    hint: Option<&'static str>,
    effects: Vec<std::sync::Arc<dyn ImageEffect>>,
    recent: bool,
}

impl EffectBrowser {
    pub fn show(&mut self, ui: &mut Ui, colors: &Colors, editor: &mut Editor, view: Rect) {
        // The scrim; a click on it closes the browser.
        let scrim = ui.interact(view, Id::new("editor-effects-scrim"), Sense::click());
        ui.painter().rect_filled(view, 0.0, colors.overlay_soft());
        if scrim.clicked() {
            editor.toggle_effects_panel();
            return;
        }
        let width = PANEL_WIDTH.min(view.width() - 32.0).max(200.0);
        let max_height = (view.height() - 250.0).max(120.0);
        let registry = effects::registry();
        let total: usize = Category::ALL
            .iter()
            .map(|c| registry.by_category(*c).len())
            .sum();
        let search = self.search.trim().to_lowercase();
        let mut groups = vec![
            Group {
                header: t(s::EffectBrowserPanel_Recent).to_string(),
                hint: Some(t(s::EffectBrowserPanel_RecentHint)),
                effects: editor
                    .recent_effects()
                    .iter()
                    .filter_map(|id| registry.get(id))
                    .collect(),
                recent: true,
            },
            Group {
                header: t(s::EffectBrowserPanel_Favorites).to_string(),
                hint: Some(t(s::EffectBrowserPanel_FavoritesHint)),
                effects: editor
                    .favorite_effects()
                    .iter()
                    .filter_map(|id| registry.get(id))
                    .collect(),
                recent: false,
            },
        ];
        for c in Category::ALL {
            let mut list = registry.by_category(c);
            list.sort_by_key(|e| effects::browser_label(e.as_ref()).to_lowercase());
            groups.push(Group {
                header: c.display_name(),
                hint: None,
                effects: list,
                recent: false,
            });
        }

        let panel_rect = Rect::from_min_size(
            pos2(view.center().x - width / 2.0, view.top() + 125.0),
            vec2(width, max_height),
        );
        let area = egui::Area::new(Id::new("editor-effect-browser"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel_rect.min)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(colors.background_panel())
                    .stroke(egui::Stroke::new(1.0, colors.border()))
                    .corner_radius(egui::CornerRadius::same(8))
                    .shadow(egui::Shadow {
                        offset: [0, 22],
                        blur: 40,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(0x40),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width - 2.0);
                        ui.set_max_height(max_height);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.add_space(16.0);
                        let (row, _) =
                            ui.allocate_exact_size(vec2(width - 2.0, 40.0), Sense::hover());
                        let field = row.shrink2(vec2(16.0, 0.0));
                        let hint = tf(s::EffectBrowserPanel_SearchCount, &[total.into()]);
                        let search_id = Id::new("editor-effect-search");
                        let resp = style::text_box(
                            ui,
                            colors,
                            field,
                            &mut self.search,
                            &hint,
                            true,
                            search_id,
                        );
                        if !self.focus_search {
                            self.focus_search = true;
                            resp.request_focus();
                        }
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("editor-effect-scroll")
                            .max_height(max_height - 64.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing = Vec2::ZERO;
                                for group in groups.iter() {
                                    let visible: Vec<_> = group
                                        .effects
                                        .iter()
                                        .filter(|e| {
                                            search.is_empty()
                                                || effects::matches_search(e.as_ref(), &search)
                                        })
                                        .cloned()
                                        .collect();
                                    if visible.is_empty() {
                                        continue;
                                    }
                                    self.group(ui, colors, editor, group, &visible, width - 2.0);
                                }
                                ui.add_space(16.0);
                            });
                    });
            });
        let _ = area;
    }

    fn group(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        editor: &mut Editor,
        group: &Group,
        visible: &[std::sync::Arc<dyn ImageEffect>],
        width: f32,
    ) {
        let inner = width - 32.0;
        // `category-header`: 18 points, margin 8,8,0,8, the count after the name.
        let header = format!("{} ({})", group.header, group.effects.len());
        let g = style::galley(ui, &header, 18.0, Weight::SemiBold, colors.text_secondary());
        let (row, _) = ui.allocate_exact_size(vec2(width, g.size().y + 16.0), Sense::hover());
        let left = row.left() + 16.0 + 8.0;
        ui.painter().galley(
            pos2(left, row.top() + 8.0),
            g.clone(),
            colors.text_secondary(),
        );
        if let Some(hint) = group.hint {
            let tip = Rect::from_min_size(
                pos2(left + g.size().x + 6.0, row.center().y - 11.0),
                vec2(22.0, 22.0),
            );
            ui.painter()
                .rect_filled(tip, 8.0, colors.background_panel());
            style::paint_icon(
                ui,
                ui.painter(),
                tip.center() + vec2(0.0, 1.0),
                crate::ui::icons::LIGHTBULB,
                18.0,
                colors.text_secondary(),
            );
            ui.interact(tip, Id::new(("effect-tip", &group.header)), Sense::hover())
                .on_hover_text(hint);
        }
        // The wrap panel of `effect-item` buttons (margin 4, padding 10 by 8).
        let mut x = 16.0;
        let mut line: Option<Rect> = None;
        let item_h = 34.0;
        for effect in visible {
            let label = effects::browser_label(effect.as_ref());
            let w = 10.0
                + 20.0
                + 8.0
                + style::galley(ui, &label, 13.0, Weight::Normal, colors.text())
                    .size()
                    .x
                + 10.0;
            let full = w + 8.0;
            if line.is_none() || x + full > inner + 16.0 {
                let (r, _) = ui.allocate_exact_size(vec2(width, item_h + 8.0), Sense::hover());
                line = Some(r);
                x = 16.0;
            }
            let Some(l) = line else { continue };
            let rect =
                Rect::from_min_size(pos2(l.left() + x + 4.0, l.top() + 4.0), vec2(w, item_h));
            let (resp, paint) =
                style::button_at(ui, colors, rect, ButtonKind::EffectItem, 8.0, false, true);
            style::paint_icon(
                ui,
                ui.painter(),
                pos2(paint.rect.left() + 10.0 + 10.0, paint.rect.center().y),
                effect.icon(),
                16.0,
                colors.text(),
            );
            style::paint_text(
                ui,
                ui.painter(),
                Rect::from_min_max(
                    pos2(paint.rect.left() + 10.0 + 28.0, paint.rect.top()),
                    paint.rect.max,
                ),
                Align2::LEFT_CENTER,
                &label,
                13.0,
                Weight::Normal,
                colors.text(),
            );
            let id = effect.id().to_string();
            let description = effect.description().to_string();
            let resp = if description.is_empty() {
                resp
            } else {
                resp.on_hover_text(description)
            };
            if resp.secondary_clicked() {
                editor.effect_right_clicked(&id, group.recent);
            } else if resp.clicked() {
                self.search.clear();
                self.focus_search = false;
                editor.run_effect(&id);
            }
            x += full;
        }
        ui.add_space(8.0);
    }

    /// The browser closed: next time the search box takes the focus again.
    pub fn reset(&mut self) {
        self.focus_search = false;
    }
}

//! The Server tab (`AdminView` + `AdminDiskViewModel`): what the server's storage holds,
//! read from `/api/admin/disk` when the tab is opened and on Refresh. The words come
//! from `modlight_core::admin`.

use std::time::{Duration, Instant};

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};

use modlight_api::ApiError;
use modlight_core::admin::{AdminState, DiskView, FileRow, Pressure, Share};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};

use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{layout, paint_icon, trimmed, wrapped};

/// `Freshness`: a report younger than this is shown again without asking.
const FRESHNESS: Duration = Duration::from_secs(120);

/// The tab's state.
#[derive(Clone, Debug)]
pub struct AdminTab {
    pub state: AdminState,
    pub host: String,
    pub view: Option<DiskView>,
    pub read_at: Option<Instant>,
    generation: u64,
}

impl Default for AdminTab {
    fn default() -> Self {
        AdminTab {
            state: AdminState::Idle,
            host: String::new(),
            view: None,
            read_at: None,
            generation: 0,
        }
    }
}

impl AppState {
    /// `Show`: when the tab is opened; a fresh report stays.
    pub fn server_tab_show(&mut self) {
        let tab = &self.server_tab;
        if tab.state == AdminState::Loading {
            return;
        }
        if tab.state == AdminState::Ready && tab.read_at.is_some_and(|at| at.elapsed() < FRESHNESS)
        {
            return;
        }
        self.server_tab_refresh();
    }

    /// `Refresh`.
    pub fn server_tab_refresh(&mut self) {
        self.server_tab.generation += 1;
        let Some(credential) = self.credential() else {
            self.server_tab.host.clear();
            self.server_tab.state = AdminState::SignedOut;
            return;
        };
        if self.mode == crate::app::Mode::Shot {
            return;
        }
        self.server_tab.host = credential.host.clone();
        self.server_tab.state = AdminState::Loading;
        let generation = self.server_tab.generation;
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_disk_report(&credential).await;
            // `DiagnoseNotFoundAsync`: 404 means no such endpoint or not an
            // administrator; /api/me says which (and drops the tab for a lost role).
            let diagnosis = match &answer {
                Err(ApiError::NotSupported) => Some(api.get_account(&credential).await),
                _ => None,
            };
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                if state.server_tab.generation != generation {
                    return;
                }
                let tab_state = match answer {
                    Ok(report) => {
                        let now = chrono::Utc::now();
                        let clock = modlight_core::culture::local_time(&now);
                        state.server_tab.view = Some(DiskView::build(&report, now, &clock));
                        state.server_tab.read_at = Some(Instant::now());
                        AdminState::Ready
                    }
                    Err(ApiError::Timeout) => AdminState::TimedOut,
                    Err(ApiError::Unauthorized) => AdminState::Unauthorized,
                    Err(ApiError::AdminForbidden) => AdminState::Refused,
                    Err(ApiError::NotSupported) => match diagnosis {
                        Some(Ok(None)) => {
                            state.note_token_refused(&credential);
                            AdminState::Unauthorized
                        }
                        Some(Ok(Some(info))) => {
                            let admin = info.role.as_deref() == Some("admin");
                            state.note_account(&credential, &info);
                            if admin {
                                AdminState::Unsupported
                            } else {
                                AdminState::NotAllowed
                            }
                        }
                        _ => AdminState::Unsupported,
                    },
                    Err(error) => AdminState::Failed(format(
                        t(s::ModlightSignIn_Unreachable),
                        &[
                            Arg::from(credential.host.as_str()),
                            Arg::from(error.to_string()),
                        ],
                    )),
                };
                state.server_tab.state = tab_state;
            }) as Box<dyn FnOnce(&mut AppState, &egui::Context) + Send>
        });
    }

    /// `Forget`: the account went away.
    pub fn server_tab_forget(&mut self) {
        self.server_tab = AdminTab {
            generation: self.server_tab.generation + 1,
            ..AdminTab::default()
        };
    }
}

// ---------------------------------------------------------------------- drawing

fn ad_card(ui: &mut Ui, colors: &Colors, pressure: Pressure, add: impl FnOnce(&mut Ui)) {
    let (border, width) = match pressure {
        Pressure::Ok => (colors.border(), 1.0),
        Pressure::Low => (colors.cancel_border(), 1.0),
        Pressure::Critical => (colors.cancel_border(), 2.0),
    };
    let full = ui.available_width();
    egui::Frame::new()
        .fill(colors.control_background())
        .stroke(Stroke::new(width, border))
        .corner_radius(10)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_width(full - 32.0 - 2.0 * width);
            ui.spacing_mut().item_spacing.y = 12.0;
            add(ui);
        });
}

fn text_block(ui: &mut Ui, text: &str, size: f32, weight: Weight, color: Color32, wrap: bool) {
    let width = ui.available_width();
    let job = if wrap {
        wrapped(text, size, weight, color, width)
    } else {
        trimmed(text, size, weight, color, width, 1)
    };
    let galley = layout(ui, job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
}

/// A card's heading: an icon (15, secondary), the title and its description.
fn card_heading(ui: &mut Ui, colors: &Colors, glyph: char, title: &str, desc: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 20.0), Sense::hover());
        paint_icon(
            ui,
            ui.painter(),
            r.center(),
            glyph,
            15.0,
            colors.text_secondary(),
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            text_block(ui, title, 15.0, Weight::SemiBold, colors.text(), true);
            text_block(
                ui,
                desc,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                true,
            );
        });
    });
}

/// `Border.ad-notice` with the warning icon.
fn notice(ui: &mut Ui, colors: &Colors, text: &str) {
    let full = ui.available_width();
    egui::Frame::new()
        .fill(colors.control_background_hover())
        .stroke(Stroke::new(1.0, colors.cancel_border()))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(full - 30.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                let (r, _) = ui.allocate_exact_size(vec2(15.0, 16.0), Sense::hover());
                paint_icon(
                    ui,
                    ui.painter(),
                    r.center(),
                    icons::TRIANGLE_ALERT,
                    15.0,
                    colors.cancel_border(),
                );
                text_block(
                    ui,
                    text,
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    true,
                );
            });
        });
}

/// `Button.ad-quiet`: transparent until hovered; the label 13, secondary.
fn quiet_button(
    ui: &mut Ui,
    colors: &Colors,
    glyph: Option<char>,
    label: &str,
    enabled: bool,
) -> bool {
    let galley = layout(
        ui,
        crate::widgets::job(label, 13.0, Weight::Normal, colors.text_secondary()),
    );
    let icon_w = if glyph.is_some() { 14.0 + 7.0 } else { 0.0 };
    let size = vec2(10.0 + icon_w + galley.size().x + 10.0, 32.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, colors.control_background_hover());
    }
    let alpha = if enabled { 1.0 } else { 0.5 };
    let color = colors.text_secondary().gamma_multiply(alpha);
    if let Some(glyph) = glyph {
        paint_icon(
            ui,
            ui.painter(),
            pos2(rect.left() + 10.0 + 7.0, rect.center().y),
            glyph,
            14.0,
            color,
        );
    }
    ui.painter().galley(
        pos2(
            rect.left() + 10.0 + icon_w,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
    enabled && response.clicked()
}

/// The tab's content.
pub fn ui(ui: &mut Ui, state: &mut AppState, rect: Rect) {
    let colors = state.colors.clone();
    ui.painter()
        .rect_filled(rect, 0.0, colors.background_main());
    // The header band.
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 12.0 + 40.0 + 12.0 + 1.0));
    let painter = ui.painter().clone();
    painter.rect_filled(header, 0.0, colors.background_toolbar());
    painter.hline(
        header.x_range(),
        header.bottom() - 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let mut band = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header.shrink2(vec2(16.0, 12.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    paint_icon(
        &band,
        &painter,
        pos2(header.left() + 16.0 + 10.0, header.center().y),
        icons::HARD_DRIVE,
        19.0,
        colors.accent_start(),
    );
    band.add_space(19.0 + 10.0);
    let tab = state.server_tab.clone();
    let loading = tab.state == AdminState::Loading;
    let refresh_w = 10.0
        + 21.0
        + layout(
            &band,
            crate::widgets::job(t(s::Admin_Refresh), 13.0, Weight::Normal, colors.text()),
        )
        .size()
        .x
        + 10.0;
    band.allocate_ui_with_layout(
        vec2(band.available_width() - refresh_w - 8.0, 40.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            text_block(
                ui,
                t(s::Admin_Title),
                15.0,
                Weight::SemiBold,
                colors.text(),
                false,
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                crate::widgets::text(ui, &tab.host, 11.0, Weight::Normal, colors.text_secondary());
                if let (AdminState::Ready, Some(view)) = (&tab.state, &tab.view) {
                    crate::widgets::text(
                        ui,
                        &view.checked,
                        11.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                }
            });
        },
    );
    band.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if quiet_button(
            ui,
            &colors,
            Some(icons::REFRESH_CW),
            t(s::Admin_Refresh),
            !loading,
        ) {
            state.server_tab_refresh();
        }
    })
    .response
    .on_hover_text(t(s::Admin_Refresh));

    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(body);
    let mut retry = false;
    egui::ScrollArea::vertical()
        .id_salt("server-tab")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            egui::Frame::NONE.inner_margin(16.0).show(ui, |ui| {
                ui.set_width(body.width() - 32.0);
                ui.spacing_mut().item_spacing.y = 12.0;
                match &tab.state {
                    AdminState::Loading => ad_card(ui, &colors, Pressure::Ok, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                let text_w = layout(
                                    ui,
                                    crate::widgets::job(
                                        t(s::Admin_Loading),
                                        12.0,
                                        Weight::Normal,
                                        colors.text(),
                                    ),
                                )
                                .size()
                                .x;
                                ui.add_space(
                                    ((ui.available_width() - text_w - 26.0) / 2.0).max(0.0),
                                );
                                crate::widgets::icon(
                                    ui,
                                    icons::HARD_DRIVE,
                                    16.0,
                                    colors.text_secondary(),
                                );
                                crate::widgets::text(
                                    ui,
                                    t(s::Admin_Loading),
                                    12.0,
                                    Weight::Normal,
                                    colors.text_secondary(),
                                );
                            });
                        });
                    }),
                    problem if problem.is_problem() => ad_card(ui, &colors, Pressure::Ok, |ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            crate::widgets::icon(ui, problem.icon(), 18.0, colors.text_secondary());
                            text_block(
                                ui,
                                problem.title(),
                                15.0,
                                Weight::SemiBold,
                                colors.text(),
                                true,
                            );
                        });
                        text_block(
                            ui,
                            &problem.body(&tab.host),
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                            true,
                        );
                        if problem.can_retry()
                            && quiet_button(ui, &colors, None, t(s::Admin_Retry), true)
                        {
                            retry = true;
                        }
                    }),
                    AdminState::Ready => {
                        if let Some(view) = &tab.view {
                            report(ui, &colors, view);
                        }
                    }
                    _ => {}
                }
            });
        });
    if retry {
        state.server_tab_refresh();
    }
}

fn report(ui: &mut Ui, colors: &Colors, view: &DiskView) {
    // The machine.
    ad_card(ui, colors, view.pressure, |ui| {
        ui.horizontal_top(|ui| {
            let badge = (!view.free_badge.is_empty() && view.has_filesystem).then(|| {
                layout(
                    ui,
                    crate::widgets::job(&view.free_badge, 13.0, Weight::SemiBold, colors.text()),
                )
            });
            let badge_w = badge
                .as_ref()
                .map(|g| g.size().x + 18.0 + 12.0)
                .unwrap_or(0.0);
            let width = ui.available_width() - badge_w;
            ui.allocate_ui_with_layout(
                vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(width);
                    text_block(
                        ui,
                        &view.source,
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        true,
                    )
                },
            );
            if let Some(galley) = badge {
                ui.add_space(12.0);
                let (r, _) =
                    ui.allocate_exact_size(galley.size() + vec2(18.0, 6.0), Sense::hover());
                ui.painter()
                    .rect_filled(r, 9.0, colors.control_background_hover());
                ui.painter()
                    .galley(r.min + vec2(9.0, 3.0), galley, colors.text());
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let figure = layout(
                ui,
                crate::widgets::job(&view.used, 26.0, Weight::SemiBold, colors.text()),
            );
            let (r, _) = ui.allocate_exact_size(figure.size(), Sense::hover());
            let figure_bottom = r.bottom();
            ui.painter().galley(r.min, figure, colors.text());
            if !view.of_total.is_empty() {
                let of = layout(
                    ui,
                    crate::widgets::job(
                        &view.of_total,
                        12.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    ),
                );
                let (r2, _) = ui.allocate_exact_size(vec2(of.size().x, r.height()), Sense::hover());
                ui.painter().galley(
                    pos2(r2.left(), figure_bottom - 4.0 - of.size().y),
                    of,
                    colors.text_secondary(),
                );
            }
        });
        if view.has_filesystem {
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
            ui.painter()
                .rect_filled(bar, 5.0, colors.control_background_hover());
            let total: i64 = view.weights.iter().sum::<i64>().max(1);
            let fills = [
                colors.accent_start(),
                colors.cancel_border(),
                colors.text_secondary().gamma_multiply(0.35),
                Color32::TRANSPARENT,
            ];
            let painter = ui.painter().with_clip_rect(bar);
            let mut x = bar.left();
            for (weight, fill) in view.weights.iter().zip(fills) {
                let w = bar.width() * (*weight as f32 / total as f32);
                if w > 0.0 && fill != Color32::TRANSPARENT {
                    painter.rect_filled(
                        Rect::from_min_size(pos2(x, bar.top()), vec2(w, bar.height())),
                        0.0,
                        fill,
                    );
                }
                x += w;
            }
            // The rounded ends over the segments.
            ui.painter().rect_stroke(
                bar.expand(0.5),
                5.5,
                Stroke::new(1.0, colors.control_background()),
                egui::StrokeKind::Outside,
            );
        }
        legend(ui, colors, view);
        if !view.pressure_note.is_empty() {
            notice(ui, colors, &view.pressure_note);
        }
    });
    // Where the space went.
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: 2,
            right: 2,
            top: 4,
            bottom: 0,
        })
        .show(ui, |ui| {
            text_block(
                ui,
                &view.insights,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                true,
            )
        });
    // Trash.
    ad_card(ui, colors, Pressure::Ok, |ui| {
        card_heading(
            ui,
            colors,
            icons::TRASH_2,
            t(s::Admin_TrashTitle),
            &view.trash_desc,
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 26.0;
            stat(
                ui,
                colors,
                t(s::Admin_TrashHeld),
                &view.trash_held,
                &view.trash_held_hint,
            );
            stat(
                ui,
                colors,
                &view.trash_due_soon_label,
                &view.trash_due_soon,
                &view.trash_due_soon_hint,
            );
            stat(
                ui,
                colors,
                t(s::Admin_TrashOldest),
                &view.trash_oldest,
                t(s::Admin_TrashOldestHint),
            );
        });
        if let Some(text) = &view.trash_overdue {
            notice(ui, colors, text);
        }
    });
    // By file type.
    ad_card(ui, colors, Pressure::Ok, |ui| {
        card_heading(
            ui,
            colors,
            icons::SHAPES,
            t(s::Admin_KindsTitle),
            t(s::Admin_KindsDesc),
        );
        if view.kinds.is_empty() {
            text_block(
                ui,
                t(s::Admin_KindsEmpty),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                true,
            );
        }
        shares(ui, colors, &view.kinds);
    });
    // By account.
    ad_card(ui, colors, Pressure::Ok, |ui| {
        card_heading(
            ui,
            colors,
            icons::USERS,
            t(s::Admin_AccountsTitle),
            t(s::Admin_AccountsDesc),
        );
        if view.accounts.is_empty() {
            text_block(
                ui,
                t(s::Admin_AccountsEmpty),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                true,
            );
        }
        shares(ui, colors, &view.accounts);
    });
    // Biggest files.
    ad_card(ui, colors, Pressure::Ok, |ui| {
        card_heading(
            ui,
            colors,
            icons::ARROW_DOWN_WIDE_NARROW,
            t(s::Admin_BiggestTitle),
            t(s::Admin_BiggestDesc),
        );
        if view.biggest.is_empty() {
            text_block(
                ui,
                t(s::Admin_BiggestEmpty),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                true,
            );
        }
        files(ui, colors, &view.biggest);
    });
    // Untouched.
    ad_card(ui, colors, Pressure::Ok, |ui| {
        ui.horizontal_top(|ui| {
            let summary = (!view.cold.is_empty()).then(|| {
                layout(
                    ui,
                    crate::widgets::job(&view.cold_summary, 13.0, Weight::SemiBold, colors.text()),
                )
            });
            let summary_w = summary.as_ref().map(|g| g.size().x + 12.0).unwrap_or(0.0);
            let width = ui.available_width() - summary_w;
            ui.allocate_ui_with_layout(
                vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(width);
                    card_heading(
                        ui,
                        colors,
                        icons::SNOWFLAKE,
                        &view.cold_title,
                        t(s::Admin_ColdDesc),
                    )
                },
            );
            if let Some(galley) = summary {
                ui.add_space(12.0);
                let (r, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                ui.painter().galley(r.min, galley, colors.text());
            }
        });
        if view.cold.is_empty() {
            text_block(
                ui,
                &view.cold_empty,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                true,
            );
        } else {
            files(ui, colors, &view.cold);
            text_block(
                ui,
                t(s::Admin_ColdNote),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                true,
            );
        }
    });
}

fn legend(ui: &mut Ui, colors: &Colors, view: &DiskView) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(26.0, 6.0);
        for item in &view.legend {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    let (dot, _) = ui.allocate_exact_size(vec2(9.0, 9.0), Sense::hover());
                    let painter = ui.painter();
                    let fill = match item.key {
                        "trash" => colors.cancel_border(),
                        "other" => colors.text_secondary().gamma_multiply(0.35),
                        "free" => colors.control_background_hover(),
                        _ => colors.accent_start(),
                    };
                    painter.circle_filled(dot.center(), 4.5, fill);
                    if item.key == "free" {
                        painter.circle_stroke(dot.center(), 4.0, Stroke::new(1.0, colors.border()));
                    }
                    crate::widgets::text(ui, &item.label, 12.0, Weight::Normal, colors.text());
                });
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    crate::widgets::text(ui, &item.value, 13.0, Weight::SemiBold, colors.text());
                });
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    crate::widgets::text(
                        ui,
                        &item.hint,
                        11.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                });
            });
        }
    });
}

fn stat(ui: &mut Ui, colors: &Colors, label: &str, value: &str, hint: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        crate::widgets::text(ui, label, 11.0, Weight::Normal, colors.text_secondary());
        crate::widgets::text(ui, value, 13.0, Weight::SemiBold, colors.text());
        crate::widgets::text(ui, hint, 11.0, Weight::Normal, colors.text_secondary());
    });
}

/// Rows with a share bar (`ProgressBar.ad-share`, 5 high).
fn shares(ui: &mut Ui, colors: &Colors, rows: &[Share]) {
    for row in rows {
        ui.horizontal_top(|ui| {
            let value = layout(
                ui,
                crate::widgets::job(&row.value, 13.0, Weight::SemiBold, colors.text()),
            );
            let left_w = ui.available_width() - value.size().x - 14.0;
            let r = ui
                .allocate_ui_with_layout(
                    vec2(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(left_w);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        text_block(ui, &row.label, 12.0, Weight::Normal, colors.text(), false);
                        let (bar, _) =
                            ui.allocate_exact_size(vec2(ui.available_width(), 5.0), Sense::hover());
                        ui.painter()
                            .rect_filled(bar, 2.5, colors.control_background_hover());
                        let filled = Rect::from_min_size(
                            bar.min,
                            vec2(bar.width() * (row.percent as f32 / 100.0), bar.height()),
                        );
                        ui.painter().rect_filled(filled, 2.5, colors.accent_start());
                        text_block(
                            ui,
                            &row.hint,
                            11.0,
                            Weight::Normal,
                            colors.text_secondary(),
                            false,
                        );
                    },
                )
                .response
                .rect;
            ui.add_space(14.0);
            let (vr, _) = ui.allocate_exact_size(vec2(value.size().x, r.height()), Sense::hover());
            ui.painter().galley(
                pos2(vr.left(), vr.center().y - value.size().y / 2.0),
                value,
                colors.text(),
            );
        });
        ui.add_space(-2.0);
    }
}

fn files(ui: &mut Ui, colors: &Colors, rows: &[FileRow]) {
    for row in rows {
        ui.horizontal_top(|ui| {
            let size = layout(
                ui,
                crate::widgets::job(&row.size, 13.0, Weight::SemiBold, colors.text()),
            );
            let left_w = ui.available_width() - size.size().x - 14.0;
            let r = ui
                .allocate_ui_with_layout(
                    vec2(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(left_w);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        text_block(ui, &row.name, 12.0, Weight::Normal, colors.text(), false);
                        text_block(
                            ui,
                            &row.meta,
                            11.0,
                            Weight::Normal,
                            colors.text_secondary(),
                            false,
                        );
                    },
                )
                .response
                .rect;
            ui.add_space(14.0);
            let (vr, _) = ui.allocate_exact_size(vec2(size.size().x, r.height()), Sense::hover());
            ui.painter().galley(
                pos2(vr.left(), vr.center().y - size.size().y / 2.0),
                size,
                colors.text(),
            );
        });
        ui.add_space(-3.0);
    }
}

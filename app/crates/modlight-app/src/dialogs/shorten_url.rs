//! "Shorten URL" (`ShortenURLWindow`): 620 wide, the header band, the address in one
//! input card and the link's options in another (custom name, expiry, password, whether
//! the link lives under the account's tag), then Cancel and Shorten.

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};
use modlight_api::ShortLinkOptions;
use modlight_core::icons;
use modlight_core::l10n::{s, t};

use super::background;
use super::chrome::{header_band, input_card};
use super::url_upload::{compact_looks, select_all};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, button_at, button_width, checkbox_weighted, combo_rounded, layout, paint_text,
    wrapped,
};

pub const WIDTH: f32 = 620.0;

/// The expiry choices: days, or never.
const EXPIRIES: [Option<i64>; 6] = [None, Some(1), Some(7), Some(30), Some(90), Some(365)];

#[derive(Clone, Debug, PartialEq)]
pub struct ShortenUrl {
    pub url: String,
    pub custom_code: String,
    pub expiry: Option<i64>,
    pub password: String,
    pub use_tag: bool,
    select_initial: bool,
    focused: bool,
    pub content_height: f32,
}

/// `ShortenURLRequest`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortenRequest {
    pub url: String,
    pub options: ShortLinkOptions,
}

impl ShortenUrl {
    pub fn new(initial: Option<&str>) -> ShortenUrl {
        ShortenUrl {
            url: initial.unwrap_or_default().to_string(),
            custom_code: String::new(),
            expiry: None,
            password: String::new(),
            use_tag: true,
            select_initial: initial.is_some_and(|i| !i.is_empty()),
            focused: false,
            content_height: 560.0,
        }
    }

    fn expiry_label(days: Option<i64>) -> String {
        t(match days {
            None => s::ShortenURLWindow_ExpiryNever,
            Some(1) => s::ShortenURLWindow_Expiry1Day,
            Some(7) => s::ShortenURLWindow_Expiry7Days,
            Some(30) => s::ShortenURLWindow_Expiry30Days,
            Some(90) => s::ShortenURLWindow_Expiry90Days,
            Some(_) => s::ShortenURLWindow_Expiry1Year,
        })
        .to_string()
    }

    /// `Some(Some(request))` on Shorten, `Some(None)` on Cancel.
    pub fn ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<Option<ShortenRequest>> {
        background(ui, colors);
        let full = ui.max_rect();
        let band = header_band(
            ui,
            colors,
            full.min,
            full.width(),
            icons::LINK_2,
            t(s::ShortenURLWindow_Heading),
            19.0,
            None,
            6.0,
            false,
        );
        let card_left = full.left() + 18.0;
        let card_width = full.width() - 36.0;
        let inner_left = card_left + 12.0;
        let inner_width = card_width - 24.0;
        let hint = |ui: &Ui, text: &str| {
            layout(
                ui,
                // `TextBlock.hint` sets no weight: the global SemiBold stays.
                wrapped(
                    text,
                    12.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                    inner_width,
                ),
            )
        };

        // The address.
        let url_hint = hint(ui, t(s::ShortenURLWindow_URLHint));
        let card = Rect::from_min_size(
            pos2(card_left, band.bottom() + 14.0),
            vec2(
                card_width,
                12.0 + 18.0 + 6.0 + 32.0 + 6.0 + url_hint.size().y + 12.0,
            ),
        );
        input_card(ui, colors, card);
        let mut y = card.top() + 12.0;
        label(
            ui,
            colors,
            pos2(inner_left, y),
            inner_width,
            t(s::ShortenURLWindow_URL),
            true,
        );
        y += 18.0 + 6.0;
        let url_id = egui::Id::new("shorten-url-field");
        let response = field(
            ui,
            colors,
            Rect::from_min_size(pos2(inner_left, y), vec2(inner_width, 32.0)),
            &mut self.url,
            "https://example.com/long-url",
            url_id,
            false,
        );
        if !self.focused {
            response.request_focus();
            if self.select_initial {
                select_all(ui, url_id, &self.url);
            }
            self.focused = true;
        }
        y += 32.0 + 6.0;
        let hint_h = url_hint.size().y;
        ui.painter()
            .galley(pos2(inner_left, y), url_hint, colors.text_secondary());
        let _ = hint_h;

        // The options.
        let code_hint = hint(ui, t(s::ShortenURLWindow_CustomCodeHint));
        let password_hint = hint(ui, t(s::ShortenURLWindow_PasswordHint));
        let tag_hint = hint(ui, t(s::ShortenURLWindow_UseTagHint));
        let options_height = 12.0
            + 18.0
            + 12.0
            + (18.0 + 5.0 + 30.0 + 5.0 + code_hint.size().y)
            + 12.0
            + (18.0 + 5.0 + 30.0)
            + 12.0
            + password_hint.size().y
            + 12.0
            + (32.0 + 4.0 + tag_hint.size().y)
            + 12.0;
        let options = Rect::from_min_size(
            pos2(card_left, card.bottom() + 10.0),
            vec2(card_width, options_height),
        );
        input_card(ui, colors, options);
        let mut y = options.top() + 12.0;
        label(
            ui,
            colors,
            pos2(inner_left, y),
            inner_width,
            t(s::ShortenURLWindow_Options),
            true,
        );
        y += 18.0 + 12.0;
        label(
            ui,
            colors,
            pos2(inner_left, y),
            inner_width,
            t(s::ShortenURLWindow_CustomCode),
            false,
        );
        y += 18.0 + 5.0;
        field(
            ui,
            colors,
            Rect::from_min_size(pos2(inner_left, y), vec2(inner_width, 30.0)),
            &mut self.custom_code,
            "my-link",
            egui::Id::new("shorten-url-code"),
            false,
        );
        y += 30.0 + 5.0;
        let code_hint_h = code_hint.size().y;
        ui.painter()
            .galley(pos2(inner_left, y), code_hint, colors.text_secondary());
        y += code_hint_h + 12.0;

        // Expiry and password, side by side.
        let half = (inner_width - 12.0) / 2.0;
        label(
            ui,
            colors,
            pos2(inner_left, y),
            half,
            t(s::ShortenURLWindow_Expiry),
            false,
        );
        label(
            ui,
            colors,
            pos2(inner_left + half + 12.0, y),
            half,
            t(s::ShortenURLWindow_Password),
            false,
        );
        y += 18.0 + 5.0;
        let choices: Vec<(Option<i64>, String)> = EXPIRIES
            .iter()
            .map(|d| (*d, ShortenUrl::expiry_label(*d)))
            .collect();
        let mut combo_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(pos2(inner_left, y), vec2(half, 30.0))),
        );
        // The global ComboBox: 8 px corners (the window does not override them).
        if let Some(choice) = combo_rounded(
            &mut combo_ui,
            colors,
            "shorten-expiry",
            half,
            30.0,
            8.0,
            &self.expiry,
            &choices,
        ) {
            self.expiry = choice;
        }
        field(
            ui,
            colors,
            Rect::from_min_size(pos2(inner_left + half + 12.0, y), vec2(half, 30.0)),
            &mut self.password,
            "••••••••",
            egui::Id::new("shorten-url-password"),
            true,
        );
        y += 30.0 + 12.0;
        let password_hint_h = password_hint.size().y;
        ui.painter()
            .galley(pos2(inner_left, y), password_hint, colors.text_secondary());
        y += password_hint_h + 12.0;
        let mut check_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(inner_left, y),
                    vec2(inner_width, 32.0),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        checkbox_weighted(
            &mut check_ui,
            colors,
            &mut self.use_tag,
            Some(t(s::ShortenURLWindow_UseTag)),
            true,
            Weight::SemiBold,
        );
        y += 32.0 + 4.0;
        ui.painter()
            .galley(pos2(inner_left, y), tag_hint, colors.text_secondary());

        // Cancel, then Shorten (enabled once there is an address).
        let top = options.bottom() + 14.0;
        let (cancel, shorten) = (
            t(s::ShortenURLWindow_Cancel),
            t(s::ShortenURLWindow_Shorten),
        );
        let (plain, primary) = compact_looks(colors);
        let can_submit = !self.url.trim().is_empty();
        let shorten_w = button_width(ui, &primary, Content::label(shorten));
        let cancel_w = button_width(ui, &plain, Content::label(cancel));
        let shorten_rect = Rect::from_min_size(
            pos2(full.right() - 18.0 - shorten_w, top),
            vec2(shorten_w, 40.0),
        );
        let cancel_rect = Rect::from_min_size(
            pos2(shorten_rect.left() - 7.0 - cancel_w, top),
            vec2(cancel_w, 40.0),
        );
        let mut outcome = None;
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
            outcome = Some(None);
        }
        let submit = button_at(
            ui,
            colors,
            &primary,
            Content::label(shorten),
            can_submit,
            shorten_rect,
        )
        .clicked()
            || (can_submit && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if submit && can_submit {
            let code = self.custom_code.trim().to_string();
            outcome = Some(Some(ShortenRequest {
                url: self.url.trim().to_string(),
                options: ShortLinkOptions {
                    custom_code: (!code.is_empty()).then_some(code),
                    expires_in_days: self.expiry,
                    password: (!self.password.is_empty()).then(|| self.password.clone()),
                    use_user_code: self.use_tag,
                },
            }));
        }
        let bottom = top + 40.0 + 18.0;
        ui.allocate_rect(
            Rect::from_min_max(full.min, pos2(full.right(), bottom)),
            Sense::hover(),
        );
        self.content_height = bottom - full.top();
        outcome
    }
}

/// A caption: `TextBlock.field` and the unclassed ones alike are SemiBold (the global
/// TextBlock style).
fn label(ui: &Ui, colors: &Colors, at: egui::Pos2, width: f32, text: &str, _field: bool) {
    paint_text(
        ui,
        ui.painter(),
        Rect::from_min_size(at, vec2(width, 18.0)),
        Align2::LEFT_CENTER,
        text,
        13.0,
        Weight::SemiBold,
        colors.text_secondary(),
    );
}

fn field(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    text: &mut String,
    hint: &str,
    id: egui::Id,
    password: bool,
) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    Field {
        height: rect.height(),
        width: Some(rect.width()),
        hint,
        id: Some(id),
        password,
        ..Field::new(text)
    }
    .show(&mut child, colors)
}

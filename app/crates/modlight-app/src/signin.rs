//! The sign-in window (`SignInWindow`, `SignInViewModel`): the browser flow, the manual
//! token, the account card with its quota, switching account and signing out.

use egui::{Rect, Sense, Ui, pos2, vec2};
use modlight_api::signin::{self, SignInFailure, SignedIn};
use modlight_api::{AccountInfo, Credential};
use modlight_core::format::cloud_size;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::messages;
use tokio_util::sync::CancellationToken;

use crate::media::initials_of;
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button, button_width, layout, paint_icon, trimmed, wrapped,
};

pub const WIDTH: f32 = 580.0;

/// The port's own texts (security review 2026-10-24, P2), worded per system like the
/// backup report's.
mod texts {
    #[cfg(not(windows))]
    pub use modlight_core::l10n::mac::{SignInConfirmCode, SignInConfirmCodeHint};
    #[cfg(windows)]
    pub use modlight_core::l10n::windows::{SignInConfirmCode, SignInConfirmCodeHint};
}

/// The server a distribution may build in (`ModlightDefaultHost`); empty by default,
/// which also keeps updates off (PORT.md section 5).
pub const DEFAULT_HOST: &str = match option_env!("MODLIGHT_DEFAULT_HOST") {
    Some(host) => host,
    None => "",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Success,
    Error,
}

pub struct SignInWindow {
    pub host: String,
    pub manual_token: String,
    pub show_manual_token: bool,
    pub show_server: bool,
    pub busy: bool,
    pub tone: Tone,
    pub status: String,
    pub has_account: bool,
    pub signed_in: bool,
    pub account_name: String,
    pub account_details: String,
    pub quota_text: String,
    pub quota_percent: f32,
    pub has_quota: bool,
    pub has_quota_bar: bool,
    pub cancel: Option<CancellationToken>,
    /// While the browser sign-in waits: the code the authorize page must show too
    /// (`signin::confirmation_code`, review 2026-10-24 P2). Empty otherwise.
    pub confirm_code: String,
    pub generation: u64,
    pub content_height: f32,
    pub initialized: bool,
}

impl SignInWindow {
    pub fn new(host: &str) -> SignInWindow {
        SignInWindow {
            host: if host.is_empty() {
                DEFAULT_HOST.to_string()
            } else {
                host.to_string()
            },
            manual_token: String::new(),
            show_manual_token: false,
            show_server: DEFAULT_HOST.is_empty(),
            busy: false,
            tone: Tone::Neutral,
            status: String::new(),
            has_account: false,
            signed_in: false,
            account_name: String::new(),
            account_details: String::new(),
            quota_text: String::new(),
            quota_percent: 0.0,
            has_quota: false,
            has_quota_bar: false,
            cancel: None,
            confirm_code: String::new(),
            generation: 0,
            content_height: 300.0,
            initialized: false,
        }
    }

    pub fn has_error(&self) -> bool {
        !self.status.is_empty() && self.tone == Tone::Error
    }

    pub fn has_success(&self) -> bool {
        !self.status.is_empty() && !self.busy && self.tone == Tone::Success
    }

    pub fn is_confirmed(&self) -> bool {
        self.signed_in && !self.has_error()
    }

    pub fn needs_sign_in(&self) -> bool {
        !self.is_confirmed()
    }

    pub fn set_status(&mut self, message: &str, tone: Tone) {
        self.status = message.to_string();
        self.tone = tone;
    }

    /// The card from what the config remembers (`ShowStored`).
    pub fn show_stored(&mut self, state: &AppState) {
        let account = state.account.account();
        self.account_name = if account.name.trim().is_empty() {
            account.email.clone()
        } else {
            account.name.clone()
        };
        self.account_details = details(Some(&account.email), Some(&account.tag), &account.host);
        self.has_quota = false;
        self.has_quota_bar = false;
        self.has_account = true;
        self.signed_in = true;
    }

    /// The card from a fresh `/api/me` answer (`ShowAccount`).
    pub fn show_account(&mut self, info: &AccountInfo, host: &str) {
        self.account_name = signin::display_name(info);
        self.account_details = details(info.email.as_deref(), info.tag.as_deref(), host);
        self.has_account = true;
        self.signed_in = true;
        let Some(quota) = &info.quota else {
            self.has_quota = false;
            self.has_quota_bar = false;
            return;
        };
        match quota.max.filter(|max| !quota.unlimited && *max > 0) {
            None => {
                self.quota_text = format(
                    t(s::SignInWindow_QuotaUnlimited),
                    &[Arg::from(cloud_size(quota.used))],
                );
                self.has_quota_bar = false;
            }
            Some(max) => {
                let remaining = quota.remaining.unwrap_or((max - quota.used).max(0));
                self.quota_text = format(
                    t(s::SignInWindow_QuotaUsed),
                    &[
                        Arg::from(cloud_size(quota.used)),
                        Arg::from(cloud_size(max)),
                        Arg::from(cloud_size(remaining)),
                    ],
                );
                self.quota_percent = (quota.used as f32 * 100.0 / max as f32).min(100.0);
                self.has_quota_bar = true;
            }
        }
        self.has_quota = true;
    }

    fn clear_account(&mut self) {
        self.account_name.clear();
        self.account_details.clear();
        self.quota_text.clear();
        self.quota_percent = 0.0;
        self.has_quota = false;
        self.has_quota_bar = false;
        self.has_account = false;
        self.signed_in = false;
    }
}

/// "e-mail" and "Tag X - host" on two lines.
fn details(email: Option<&str>, tag: Option<&str>, host: &str) -> String {
    let mut lines = Vec::new();
    if let Some(email) = email.filter(|e| !e.trim().is_empty()) {
        lines.push(email.to_string());
    }
    match tag.filter(|t| !t.trim().is_empty()) {
        Some(tag) => lines.push(format(
            t(s::SignInWindow_TagAndServer),
            &[Arg::from(tag), Arg::from(host)],
        )),
        None => lines.push(host.to_string()),
    }
    lines.join("\n")
}

/// What the window's buttons asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInAction {
    SignIn,
    UseToken,
    Cancel,
    SignOut,
    Close,
}

impl AppState {
    /// Open the sign-in window, or bring it forward (`SignInWindowIntegration.Show`).
    pub fn show_sign_in(&mut self) {
        if self.windows.sign_in.is_none() {
            self.windows.sign_in = Some(SignInWindow::new(self.account.host()));
        }
    }

    /// The window just opened (`InitializeAsync`): check the stored account.
    pub fn sign_in_opened(&mut self) {
        let Some(window) = self.windows.sign_in.as_mut() else {
            return;
        };
        if window.initialized {
            return;
        }
        window.initialized = true;
        let Some(credential) = self.account.credential() else {
            return;
        };
        let mut window = self.windows.sign_in.take().unwrap();
        window.host = credential.host.clone();
        window.show_stored(self);
        window.busy = true;
        window.set_status(t(s::SignInWindow_Checking), Tone::Neutral);
        window.generation += 1;
        let generation = window.generation;
        // Cancel (and closing the window) stops this wait too, as every operation of
        // the window (`RunAsync`).
        let cancel = CancellationToken::new();
        window.cancel = Some(cancel.clone());
        self.windows.sign_in = Some(window);
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = tokio::select! {
                answer = api.get_account(&credential) => Some(answer),
                _ = cancel.cancelled() => None,
            };
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                let Some(window) = state.windows.sign_in.as_mut() else {
                    return;
                };
                if window.generation != generation {
                    return;
                }
                window.busy = false;
                window.cancel = None;
                let Some(answer) = answer else {
                    window.set_status(t(s::ModlightSignIn_Canceled), Tone::Neutral);
                    return;
                };
                match answer {
                    Ok(Some(info)) => {
                        window.show_account(&info, &credential.host);
                        let name = signin::display_name(&info);
                        window.set_status(&messages::signed_in_as(&name), Tone::Success);
                        state.note_account(&credential, &info);
                    }
                    Ok(None) => {
                        window.set_status(t(s::ModlightSignIn_TokenRejected), Tone::Error);
                        state.note_token_refused(&credential);
                    }
                    Err(error) => window.set_status(&error.to_string(), Tone::Error),
                }
            })
        });
    }

    pub fn sign_in_action(&mut self, action: SignInAction) {
        match action {
            SignInAction::SignIn => self.start_sign_in(false),
            SignInAction::UseToken => self.start_sign_in(true),
            SignInAction::Cancel => {
                if let Some(cancel) = self.windows.sign_in.as_ref().and_then(|w| w.cancel.clone()) {
                    cancel.cancel();
                }
            }
            SignInAction::SignOut => self.sign_out(),
            SignInAction::Close => {
                if let Some(cancel) = self.windows.sign_in.as_ref().and_then(|w| w.cancel.clone()) {
                    cancel.cancel();
                }
                self.windows.sign_in = None;
            }
        }
    }

    fn start_sign_in(&mut self, manual: bool) {
        let qa = self.env.qa;
        let machine = self.env.machine.clone();
        let Some(window) = self.windows.sign_in.as_mut() else {
            return;
        };
        if window.busy {
            return;
        }
        let cancel = CancellationToken::new();
        window.cancel = Some(cancel.clone());
        window.busy = true;
        window.generation += 1;
        let generation = window.generation;
        let host = window.host.clone();
        let token = window.manual_token.clone();
        // The verifier is made here so the window can show its confirmation code while
        // the person decides in the browser.
        let verifier = signin::new_verifier();
        window.confirm_code = if manual {
            String::new()
        } else {
            signin::confirmation_code(&signin::challenge_for(&verifier))
        };
        window.set_status(
            if manual {
                t(s::SignInWindow_Checking)
            } else {
                t(s::SignInWindow_Opening)
            },
            Tone::Neutral,
        );
        let api = self.api.clone();
        self.rt.spawn(async move {
            let result = if manual {
                signin::use_token(&api, &host, &token, &cancel).await
            } else {
                let name = signin::client_name(modlight_core::APP_NAME, &machine);
                signin::sign_in_with_browser_using(
                    &api,
                    &host,
                    &name,
                    &verifier,
                    |url| {
                        if qa {
                            log::info!("QA mode: the authorize page is not opened");
                        } else {
                            crate::platform::system::open_url(url);
                        }
                    },
                    &cancel,
                )
                .await
            };
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                state.finish_sign_in(generation, result, ctx);
            })
        });
    }

    fn finish_sign_in(
        &mut self,
        generation: u64,
        result: Result<SignedIn, SignInFailure>,
        ctx: &egui::Context,
    ) {
        if let Some(window) = self.windows.sign_in.as_mut() {
            if window.generation != generation {
                return;
            }
            window.busy = false;
            window.cancel = None;
            window.confirm_code.clear();
        }
        match result {
            Ok(signed) => {
                if let Err(error) = self.account.save_sign_in(&signed) {
                    if let Some(window) = self.windows.sign_in.as_mut() {
                        window.set_status(&error, Tone::Error);
                    }
                    return;
                }
                self.note_account(
                    &Credential::new(signed.origin.clone(), signed.token.clone()),
                    &signed.account,
                );
                let name = signin::display_name(&signed.account);
                if let Some(window) = self.windows.sign_in.as_mut() {
                    window.set_status(&messages::signed_in_as(&name), Tone::Success);
                    window.manual_token.clear();
                    window.show_manual_token = false;
                    window.show_account(&signed.account, &signed.origin);
                }
                self.signed_in(ctx);
            }
            Err(failure) => {
                let tone = if failure == SignInFailure::Canceled {
                    Tone::Neutral
                } else {
                    Tone::Error
                };
                if let Some(window) = self.windows.sign_in.as_mut() {
                    window.set_status(&messages::sign_in_failure(&failure), tone);
                }
            }
        }
    }

    /// A token landed: theme, settings, the strip and the library all hear about it.
    pub fn signed_in(&mut self, ctx: &egui::Context) {
        self.sync.refused = false;
        self.schedule.signed_in(std::time::Instant::now());
        self.sync_skin();
        self.refresh_account_strip();
        self.request_avatar(ctx);
        self.space_refresh_account();
    }

    /// Forget the account here; the token stays valid on the server (`SignOut`).
    pub fn sign_out(&mut self) {
        if self.windows.sign_in.as_ref().is_some_and(|w| w.busy) {
            return;
        }
        if let Err(error) = self.account.sign_out() {
            log::warn!("Modlight: the token could not be removed from the keychain: {error}");
        }
        self.admin.forget();
        // The report goes, and the window leaves the Server tab whose button just went.
        self.apply_admin_role();
        self.media.avatar.clear();
        self.strip.clear_storage();
        // A reading still out is about the account just signed out of (`Clear` cancels).
        self.strip.in_flight = None;
        self.space_refresh_account();
        if let Some(window) = self.windows.sign_in.as_mut() {
            window.clear_account();
            window.set_status(t(s::SignInWindow_SignedOut), Tone::Neutral);
        }
        log::info!(
            "Modlight: signed out. The token is forgotten here but is still live on the server."
        );
    }

    /// A credential for the sign-in window's account card picture.
    pub fn avatar_credential(&self) -> Option<Credential> {
        self.account.credential()
    }
}

/// Draw the window. Returns what was clicked.
pub fn ui(ui: &mut Ui, state: &mut AppState) -> Option<SignInAction> {
    let colors = state.colors.clone();
    let avatar_texture = state.media.avatar.texture().cloned();
    let window = state.windows.sign_in.as_mut()?;
    crate::dialogs::background(ui, &colors);
    let full = ui.max_rect();
    let mut action = None;

    // Header: toolbar band with the icon tile, heading and subtitle.
    let confirmed = window.is_confirmed();
    let (heading, subtitle, glyph) = if confirmed {
        (
            t(s::SignInWindow_HeadingSignedIn),
            t(s::SignInWindow_SubtitleSignedIn),
            icons::CIRCLE_CHECK,
        )
    } else {
        (
            t(s::SignInWindow_Heading),
            t(s::SignInWindow_Subtitle),
            icons::LOG_IN,
        )
    };
    let text_left = full.left() + 18.0 + 40.0 + 12.0;
    let text_width = full.right() - 18.0 - text_left;
    let heading_galley = layout(
        ui,
        wrapped(
            heading,
            19.0,
            Weight::SemiBold,
            colors.accent_start(),
            text_width,
        ),
    );
    let subtitle_galley = layout(
        ui,
        wrapped(
            subtitle,
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            text_width,
        ),
    );
    let stack = heading_galley.size().y + 3.0 + subtitle_galley.size().y;
    let header_h = 14.0 + stack.max(40.0) + 14.0;
    let (header, _) = ui.allocate_exact_size(vec2(full.width(), header_h), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(header, 0.0, colors.background_toolbar());
    let tile = Rect::from_min_size(
        pos2(header.left() + 18.0, header.top() + 14.0),
        vec2(40.0, 40.0),
    );
    painter.rect_filled(tile, 6.0, colors.background_panel());
    paint_icon(
        ui,
        &painter,
        tile.center(),
        glyph,
        20.0,
        colors.accent_start(),
    );
    let mut y = header.top() + 14.0 + (stack.max(40.0) - stack) / 2.0;
    let h = heading_galley.size().y;
    painter.galley(pos2(text_left, y), heading_galley, colors.accent_start());
    y += h + 3.0;
    painter.galley(pos2(text_left, y), subtitle_galley, colors.text_secondary());

    // Body.
    let body = Rect::from_min_max(
        pos2(full.left() + 18.0, header.bottom() + 14.0),
        pos2(full.right() - 18.0, full.bottom()),
    );
    let mut body_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let ui = &mut body_ui;
    let width = body.width();
    let idle = !window.busy;

    if window.show_server {
        crate::widgets::card(
            colors.background_panel(),
            colors.border(),
            6,
            egui::Margin::symmetric(14, 12),
        )
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            crate::widgets::text(
                ui,
                t(s::SignInWindow_Server),
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
            );
            ui.add_space(6.0);
            Field {
                height: 32.0,
                hint: "https://files.example.com",
                enabled: idle,
                id: Some(egui::Id::new("signin-host")),
                ..Field::new(&mut window.host)
            }
            .show(ui, &colors);
        });
        ui.add_space(10.0);
    }

    if !window.status.is_empty() {
        crate::widgets::card(
            colors.background_panel(),
            colors.border(),
            6,
            egui::Margin::symmetric(14, 12),
        )
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.horizontal(|ui| {
                let (icon_rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                if window.busy {
                    let angle = (ui.input(|i| i.time) as f32 % 1.0) * std::f32::consts::TAU;
                    paint_spinner(ui, icon_rect.center(), 8.0, angle, colors.text_secondary());
                    ui.ctx().request_repaint();
                } else if window.tone == Tone::Success {
                    paint_icon(
                        ui,
                        ui.painter(),
                        icon_rect.center(),
                        icons::CIRCLE_CHECK,
                        18.0,
                        colors.accent_start(),
                    );
                } else if window.tone == Tone::Error {
                    paint_icon(
                        ui,
                        ui.painter(),
                        icon_rect.center(),
                        icons::TRIANGLE_ALERT,
                        18.0,
                        error_color(&colors),
                    );
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    crate::widgets::text_wrapped(
                        ui,
                        &window.status,
                        13.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                    if window.busy && !window.confirm_code.is_empty() {
                        ui.add_space(8.0);
                        crate::widgets::text(
                            ui,
                            &format(
                                t(texts::SignInConfirmCode),
                                &[Arg::from(window.confirm_code.as_str())],
                            ),
                            17.0,
                            Weight::SemiBold,
                            colors.accent_start(),
                        );
                        ui.add_space(4.0);
                        crate::widgets::text_wrapped(
                            ui,
                            t(texts::SignInConfirmCodeHint),
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                    }
                });
            });
        });
        ui.add_space(10.0);
    }

    if window.has_account {
        crate::widgets::card(
            colors.background_panel(),
            colors.border(),
            6,
            egui::Margin::symmetric(14, 12),
        )
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.horizontal_top(|ui| {
                let (circle, _) = ui.allocate_exact_size(vec2(42.0, 42.0), Sense::hover());
                paint_avatar(
                    ui,
                    &colors,
                    circle,
                    avatar_texture.as_ref(),
                    &window.account_name,
                    22.0,
                    17.0,
                );
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    let galley = layout(
                        ui,
                        trimmed(
                            &window.account_name,
                            15.0,
                            Weight::SemiBold,
                            colors.text_secondary(),
                            ui.available_width(),
                            1,
                        ),
                    );
                    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                    ui.painter()
                        .galley(rect.min, galley, colors.text_secondary());
                    if !window.account_details.is_empty() {
                        ui.add_space(6.0);
                        crate::widgets::text_wrapped(
                            ui,
                            &window.account_details,
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                    }
                    if window.has_quota_bar {
                        ui.add_space(6.0);
                        let w = ui.available_width();
                        crate::widgets::progress(
                            ui,
                            &colors,
                            w,
                            4.0,
                            Some(window.quota_percent / 100.0),
                        );
                    }
                    if window.has_quota {
                        ui.add_space(6.0);
                        crate::widgets::text_wrapped(
                            ui,
                            &window.quota_text,
                            12.0,
                            Weight::Normal,
                            colors.text_secondary(),
                        );
                    }
                });
            });
        });
        ui.add_space(10.0);
    }

    let show_manual_toggle = window.needs_sign_in() || window.show_manual_token;
    if show_manual_toggle && hyperlink(ui, &colors, t(s::SignInWindow_ManualToggle), true).clicked()
    {
        window.show_manual_token = !window.show_manual_token;
    }
    if show_manual_toggle {
        ui.add_space(10.0);
    }

    if window.show_manual_token {
        crate::widgets::card(
            colors.background_panel(),
            colors.border(),
            6,
            egui::Margin::symmetric(14, 12),
        )
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            crate::widgets::text_wrapped(
                ui,
                t(s::SignInWindow_ManualHint),
                12.0,
                Weight::Normal,
                colors.text_secondary(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let look = Look::compact(&colors);
                let label = t(s::SignInWindow_UseToken);
                let button_w = button_width(ui, &look, Content::label(label));
                Field {
                    height: 32.0,
                    width: Some(ui.available_width() - button_w - 8.0),
                    hint: t(s::SignInWindow_Token),
                    enabled: idle,
                    id: Some(egui::Id::new("signin-token")),
                    ..Field::new(&mut window.manual_token)
                }
                .show(ui, &colors);
                ui.add_space(8.0);
                if button(ui, &colors, &look, Content::label(label), idle).clicked() {
                    action = Some(SignInAction::UseToken);
                }
            });
        });
        ui.add_space(10.0);
    }

    // The bottom row: links on the left, buttons on the right.
    ui.add_space(4.0);
    let (row, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let mut row_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    if DEFAULT_HOST.is_empty() {
        // "Other server" only exists when a distribution built a host in.
    } else if idle
        && hyperlink(&mut row_ui, &colors, t(s::SignInWindow_OtherServer), true).clicked()
    {
        window.show_server = !window.show_server;
    }
    if window.is_confirmed() {
        if hyperlink(&mut row_ui, &colors, t(s::SignInWindow_SignInAgain), idle).clicked() {
            action = Some(SignInAction::SignIn);
        }
        row_ui.add_space(14.0);
    }
    if window.signed_in
        && hyperlink(&mut row_ui, &colors, t(s::SignInWindow_SignOut), idle).clicked()
    {
        action = Some(SignInAction::SignOut);
    }

    let compact = Look::compact(&colors);
    let primary = Look::primary(&colors);
    let mut buttons: Vec<(Look, &str, bool, SignInAction)> = Vec::new();
    if window.busy {
        buttons.push((
            compact.clone(),
            t(s::SignInWindow_Cancel),
            true,
            SignInAction::Cancel,
        ));
    }
    if window.needs_sign_in() {
        buttons.push((
            compact.clone(),
            t(s::SignInWindow_Close),
            true,
            SignInAction::Close,
        ));
        buttons.push((
            primary.clone(),
            t(s::SignInWindow_SignIn),
            idle,
            SignInAction::SignIn,
        ));
    } else {
        buttons.push((
            primary.clone(),
            t(s::SignInWindow_Close),
            true,
            SignInAction::Close,
        ));
    }
    let mut x = row.right();
    for (look, label, enabled, what) in buttons.iter().rev() {
        let w = button_width(ui, look, Content::label(label));
        x -= w;
        let rect = Rect::from_min_size(pos2(x, row.top()), vec2(w, 40.0));
        if crate::widgets::button_at(ui, &colors, look, Content::label(label), *enabled, rect)
            .clicked()
        {
            action = Some(*what);
        }
        x -= 7.0;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Enter)) && idle {
        action = Some(if window.needs_sign_in() {
            if window.show_manual_token && !window.manual_token.trim().is_empty() {
                SignInAction::UseToken
            } else {
                SignInAction::SignIn
            }
        } else {
            SignInAction::Close
        });
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some(SignInAction::Close);
    }

    window.content_height = (row.bottom() + 18.0 - full.top()).ceil();
    action
}

/// The error brush of the Fluent theme (`SystemControlErrorTextForegroundBrush`).
pub fn error_color(colors: &Colors) -> egui::Color32 {
    if colors.dark {
        egui::Color32::from_rgb(0xFF, 0x99, 0xA4)
    } else {
        egui::Color32::from_rgb(0xC4, 0x2B, 0x1C)
    }
}

/// A `HyperlinkButton.quiet`: 12 px, accent, underlined.
pub fn hyperlink(ui: &mut Ui, colors: &Colors, text: &str, enabled: bool) -> egui::Response {
    let color = if enabled {
        colors.accent_start()
    } else {
        colors.accent_start().gamma_multiply(0.5)
    };
    let galley = layout(ui, crate::widgets::job(text, 12.0, Weight::Normal, color));
    let (rect, response) = ui.allocate_exact_size(
        galley.size(),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let painter = ui.painter();
    painter.galley(rect.min, galley, color);
    painter.hline(
        rect.left()..=rect.right(),
        rect.bottom() - 1.0,
        egui::Stroke::new(1.0, color),
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn paint_spinner(ui: &Ui, center: egui::Pos2, radius: f32, angle: f32, color: egui::Color32) {
    let points: Vec<egui::Pos2> = (0..=24)
        .map(|i| {
            let a = angle + i as f32 / 24.0 * std::f32::consts::TAU * 0.75;
            pos2(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, egui::Stroke::new(2.0, color)));
}

/// The account's mark in a circle: its picture, its initials, or the anonymous glyph.
pub fn paint_avatar(
    ui: &Ui,
    colors: &Colors,
    circle: Rect,
    texture: Option<&egui::TextureHandle>,
    name: &str,
    glyph_size: f32,
    initials_size: f32,
) {
    let painter = ui.painter();
    let radius = circle.width() / 2.0;
    painter.circle_filled(circle.center(), radius, colors.control_background());
    if let Some(texture) = texture {
        let size = texture.size_vec2();
        let scale = (circle.width() / size.x).max(circle.height() / size.y);
        let shown = size * scale;
        let uv_w = circle.width() / shown.x;
        let uv_h = circle.height() / shown.y;
        let uv = Rect::from_min_size(
            pos2((1.0 - uv_w) / 2.0, (1.0 - uv_h) / 2.0),
            vec2(uv_w, uv_h),
        );
        let mut mesh = egui::epaint::Mesh::with_texture(texture.id());
        let segments = 48;
        let center = circle.center();
        let uv_center = uv.center();
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center,
            uv: uv_center,
            color: egui::Color32::WHITE,
        });
        for i in 0..segments {
            let a = i as f32 / segments as f32 * std::f32::consts::TAU;
            let (sin, cos) = a.sin_cos();
            mesh.vertices.push(egui::epaint::Vertex {
                pos: pos2(center.x + radius * cos, center.y + radius * sin),
                uv: pos2(
                    uv_center.x + uv.width() / 2.0 * cos,
                    uv_center.y + uv.height() / 2.0 * sin,
                ),
                color: egui::Color32::WHITE,
            });
        }
        for i in 0..segments as u32 {
            mesh.indices
                .extend_from_slice(&[0, 1 + i, 1 + (i + 1) % segments as u32]);
        }
        painter.add(egui::Shape::mesh(mesh));
        return;
    }
    let initials = initials_of(name);
    if initials.is_empty() {
        paint_icon(
            ui,
            painter,
            circle.center(),
            icons::CIRCLE_USER,
            glyph_size,
            colors.accent_start(),
        );
    } else {
        let galley = layout(
            ui,
            crate::widgets::job(
                &initials,
                initials_size,
                Weight::SemiBold,
                colors.accent_start(),
            ),
        );
        painter.galley(
            circle.center() - galley.size() / 2.0,
            galley,
            colors.accent_start(),
        );
    }
}

#[cfg(test)]
mod tests {
    use modlight_core::account::{AccountStore, StoredAccount};

    use super::*;
    use crate::qa::{Look, state};

    fn admin_info(tag: &str) -> AccountInfo {
        AccountInfo {
            tag: Some(tag.into()),
            name: Some("Alex Rivera".into()),
            email: Some("alex@example.com".into()),
            role: Some("admin".into()),
            ..Default::default()
        }
    }

    /// An `/api/me` answer that set out before somebody else signed in (the storage chip
    /// refreshes when the window gets the focus back, which is what one does after
    /// approving in the browser) must not rename the new account after the old one: its
    /// tag decides which queued uploads go out with the new token, and whose settings
    /// marker counts.
    #[test]
    fn an_answer_about_a_credential_that_went_away_is_not_kept() {
        let mut app = state(&Look::default(), true);
        let old = app.credential().unwrap();
        app.account = AccountStore::in_memory(
            StoredAccount {
                host: old.host.clone(),
                tag: "sam".into(),
                name: "Sam".into(),
                ..StoredAccount::default()
            },
            "another-token",
        );

        app.note_account(&old, &admin_info("alex"));
        assert_eq!(app.account.account().tag, "sam");
        assert_eq!(app.account.display_name(), "Sam");
        assert!(!app.is_admin());

        // The one signed in now is kept, role included; its refusal forgets the role,
        // the old one's does not.
        let current = app.credential().unwrap();
        app.note_account(&current, &admin_info("sam"));
        assert_eq!(app.account.account().tag, "sam");
        assert!(app.is_admin());
        app.note_token_refused(&old);
        assert!(app.is_admin());
        app.note_token_refused(&current);
        assert!(!app.is_admin());
    }

    /// The window opens on a stored account and checks it with the server
    /// (`InitializeAsync`); Cancel is on screen while it waits and must stop the wait,
    /// as it does upstream, rather than leave everything disabled until the request's
    /// 20 seconds run out.
    #[test]
    fn cancel_stops_the_check_of_the_stored_account() {
        // A server that takes the connection and never answers.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", silent.local_addr().unwrap());
        let ctx = egui::Context::default();
        let mut app = state(&Look::default(), true);
        app.account = AccountStore::in_memory(
            StoredAccount {
                host,
                tag: "alex".into(),
                name: "Alex".into(),
                ..StoredAccount::default()
            },
            "alex-token",
        );

        app.show_sign_in();
        app.sign_in_opened();
        assert!(app.windows.sign_in.as_ref().unwrap().busy);
        app.sign_in_action(SignInAction::Cancel);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.windows.sign_in.as_ref().unwrap().busy && std::time::Instant::now() < deadline {
            for completion in app.rt.drain() {
                completion(&mut app, &ctx);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let window = app.windows.sign_in.as_ref().unwrap();
        assert!(!window.busy, "Cancel ends the check");
        assert_eq!(window.status, t(s::ModlightSignIn_Canceled));
        assert_eq!(window.tone, Tone::Neutral);
        assert!(window.cancel.is_none());
        drop(silent);
    }

    /// Signing out while the storage chip's request is out: its answer must not put the
    /// signed-out account's figure back beside "Sign in" (upstream's `Clear` cancels it).
    #[test]
    fn signing_out_drops_the_chips_reading_in_flight() {
        let ctx = egui::Context::default();
        let mut app = state(&Look::default(), true);
        // A server that is a closed port here: the reading fails at once, locally.
        app.account = AccountStore::in_memory(
            StoredAccount {
                host: "http://127.0.0.1:9".into(),
                tag: "alex".into(),
                ..StoredAccount::default()
            },
            "alex-token",
        );
        app.refresh_account_strip();
        assert!(app.strip.in_flight.is_some());

        app.sign_out();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            for completion in app.rt.drain() {
                completion(&mut app, &ctx);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.account.is_signed_in());
        assert!(
            !app.strip.has_storage,
            "no reading for an account signed out of"
        );
    }
}

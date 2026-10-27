//! Not upstream: My space for an account whose files are encrypted on the server (the
//! server's `docs/ENCRYPTION.md`, section 6). What the app knows about the account's
//! vault, the banner over the files that unlocks them with the file password, "Lock
//! files", and what loads again once the files open.
//!
//! The grant lives in [`GrantKeeper`] (kept like the token, and on this computer only when
//! "Remember on this computer for 30 days" is ticked) and, while its account is signed
//! in, in the [`modlight_api::Api`] every content request goes through. A `423 Locked`
//! never signs anybody out and never counts as a token the server refused: it raises the
//! banner, and a download it stopped starts again once the files open.

use std::path::PathBuf;

use egui::{Id, Key, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_api::{
    ApiError, ApiResult, Credential, EncryptionStatus, FilesLocked, UnlockFailure, UnlockGrant,
    VaultAnswer, VaultState,
};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, port, s, t};
use modlight_core::library::CloudFile;
use modlight_core::vault::{GrantKeeper, KeptGrant};

use crate::commands::Command;
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button_at, button_width, checkbox_weighted, layout, paint_icon, wrapped,
};

/// The width of the banner's column with the field, the button and the check box (the
/// web panel's 22rem).
pub const FORM_WIDTH: f32 = 352.0;
/// The banner's inner padding.
const PADDING: f32 = 16.0;
/// The lock's tile.
const TILE: f32 = 36.0;
/// The field's and the button's height.
const ROW: f32 = 36.0;
/// The check box's row.
const CHECK_ROW: f32 = 32.0;
/// The narrowest text column that still stands beside the form.
const MIN_TEXT_BESIDE: f32 = 260.0;

/// What the last unlock ran into, as the banner says it.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    WrongPassword,
    /// Too many attempts: the server takes the next one at this UI time (seconds).
    RateLimited {
        until: f64,
    },
    /// Anything else, in words.
    Failed(String),
}

/// A download the locked files stopped, done again once they open.
#[derive(Clone, Debug, PartialEq)]
pub enum Waiting {
    /// Refused while it ran: again, to the place the person already chose.
    Save {
        name: String,
        link: String,
        path: PathBuf,
    },
    /// Asked for while the files were locked (or a "Download as" whose conversion was
    /// refused for the lock): asked again, conversion check and save panel included.
    File {
        file: Box<CloudFile>,
        convert_to: Option<String>,
    },
}

/// The vault of the signed-in account, as far as the app knows it.
pub struct Vault {
    /// Whose state this is: everything below is about this credential.
    pub credential: Option<Credential>,
    /// The last `GET /api/encryption` answer for it.
    pub status: Option<EncryptionStatus>,
    /// A content request was answered `423` with no working grant held.
    pub locked_seen: bool,
    /// The number the API gave the grant its content requests carry.
    pub grant: Option<u64>,
    /// The status request in flight.
    pub asking: Option<u64>,
    pub generation: u64,
    /// The banner's form.
    pub password: String,
    pub remember: bool,
    pub unlocking: bool,
    pub problem: Option<Problem>,
    /// The password field takes the keyboard on the next frame.
    pub focus: bool,
    /// Downloads the locked files stopped.
    pub waiting: Vec<Waiting>,
    /// Where the grant is kept.
    pub keeper: GrantKeeper,
}

impl Default for Vault {
    fn default() -> Self {
        Vault {
            credential: None,
            status: None,
            locked_seen: false,
            grant: None,
            asking: None,
            generation: 0,
            password: String::new(),
            // "Remember on this computer for 30 days" starts ticked, as on the web.
            remember: true,
            unlocking: false,
            problem: None,
            focus: false,
            waiting: Vec::new(),
            keeper: GrantKeeper::in_memory(),
        }
    }
}

impl Vault {
    /// The account's files are encrypted, as far as the app knows.
    pub fn is_encrypted(&self) -> bool {
        self.locked_seen || self.status.is_some_and(|s| s.state.is_gated())
    }

    /// Encrypted, and nothing this app holds opens them: the banner is up.
    pub fn is_locked(&self) -> bool {
        self.locked_seen
            || self
                .status
                .is_some_and(|s| s.state.is_gated() && !s.unlocked)
    }

    /// Encrypted and open here: "Lock files" is offered.
    pub fn is_unlocked(&self) -> bool {
        self.is_encrypted() && !self.is_locked() && self.grant.is_some()
    }

    /// Too many attempts, and the server's wait is not over at `now` (UI seconds).
    pub fn waits_until(&self, now: f64) -> Option<f64> {
        match self.problem {
            Some(Problem::RateLimited { until }) if until > now => Some(until),
            _ => None,
        }
    }
}

/// What the account card and the account menu say about the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Locked,
    Unlocked,
}

impl AppState {
    /// The account card's and the account menu's word on the files, when encrypted.
    pub fn vault_badge(&self) -> Option<Badge> {
        if !self.account.is_signed_in() || self.vault.credential != self.credential() {
            return None;
        }
        if self.vault.is_unlocked() {
            Some(Badge::Unlocked)
        } else if self.vault.is_locked() {
            Some(Badge::Locked)
        } else {
            None
        }
    }

    /// The banner is up over My space.
    pub fn vault_shows_banner(&self) -> bool {
        self.space.has_account
            && self.vault.is_locked()
            && self.vault.credential.is_some()
            && self.vault.credential == self.credential()
    }

    /// The account may have changed (`RefreshAccount`: the start, a sign-in, a sign-out).
    /// What was known belongs to the previous one; the grant that opens the new one's
    /// files (if this computer has it) goes with its content requests, and a grant of the
    /// previous account is forgotten and revoked on the server while its token works.
    pub fn vault_follow_account(&mut self) {
        let current = self.credential();
        if current == self.vault.credential {
            return;
        }
        let previous = self.vault.credential.take();
        let keeper = std::mem::replace(&mut self.vault.keeper, GrantKeeper::in_memory());
        let generation = self.vault.generation + 1;
        self.vault = Vault {
            credential: current.clone(),
            generation,
            keeper,
            ..Vault::default()
        };
        self.api.release_grant();
        let Some(credential) = current else {
            // Signed out.
            if let Some(kept) = self.vault.keeper.forget() {
                self.vault_revoke(previous, kept);
            }
            return;
        };
        let host = self.account.host().to_string();
        let tag = self.account.account().tag.clone();
        if self
            .vault
            .keeper
            .kept()
            .is_some_and(|kept| !kept.opens(&host, &tag))
            && let Some(kept) = self.vault.keeper.forget()
        {
            // Another account signed in.
            self.vault_revoke(previous, kept);
        }
        if let Some(grant) = self.vault.keeper.grant_for(&host, &tag, self.env.now()) {
            self.vault.grant = Some(self.api.hold_grant(&credential, &grant));
        }
    }

    /// Revoke a forgotten grant on its server, with the credential of its account. Best
    /// effort: nobody has the grant any more either way.
    fn vault_revoke(&self, credential: Option<Credential>, kept: KeptGrant) {
        let Some(credential) = credential
            .filter(|c| modlight_api::same_origin(&c.host, &kept.host) || c.host == kept.host)
        else {
            return;
        };
        let api = self.api.clone();
        let grant = kept.grant;
        self.rt.spawn(async move {
            if let Err(error) = api.lock_files(&credential, &grant).await {
                log::info!(
                    "Modlight: the file password grant could not be revoked on the server: {error}"
                );
            }
            |_: &mut AppState, _: &egui::Context| {}
        });
    }

    /// Ask the server where the account's encryption stands (`GET /api/encryption`).
    pub fn vault_check(&mut self) {
        let Some(credential) = self.credential() else {
            return;
        };
        if self.vault.credential.as_ref() != Some(&credential) {
            self.vault_follow_account();
        }
        if self.vault.asking.is_some() {
            return;
        }
        self.vault.generation += 1;
        let asking = self.vault.generation;
        self.vault.asking = Some(asking);
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_encryption(&credential).await;
            move |state: &mut AppState, ctx: &egui::Context| {
                state.vault_checked(asking, &credential, answer, ctx);
            }
        });
    }

    fn vault_checked(
        &mut self,
        asking: u64,
        credential: &Credential,
        answer: ApiResult<VaultAnswer>,
        ctx: &egui::Context,
    ) {
        if self.vault.asking == Some(asking) {
            self.vault.asking = None;
        }
        if self.vault.credential.as_ref() != Some(credential) {
            return;
        }
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                log::debug!("Modlight: the file encryption state did not load: {error}");
                return;
            }
        };
        let was_locked = self.vault.is_locked();
        let mut status = answer.status;
        if !status.state.is_gated() {
            // Nothing is locked (any more).
            self.vault.locked_seen = false;
            self.vault.problem = None;
            if status.state == VaultState::Off && self.vault.keeper.kept().is_some() {
                // Switched off: the vault and its grants are gone on the server.
                self.vault_drop_grant();
            }
        } else if answer.grant.is_some() && answer.grant == self.vault.grant {
            if !status.unlocked {
                // The grant held no longer opens the files: it ran out, was revoked, or
                // the file password changed.
                self.vault_drop_grant();
            }
        } else {
            // Asked with another grant than the one held now (or with none): what it
            // says about being unlocked is not about the grant held.
            status.unlocked = self.vault.grant.is_some() && !self.vault.locked_seen;
        }
        self.vault.status = Some(status);
        if was_locked && !self.vault.is_locked() {
            self.vault_reload_contents(ctx);
        }
    }

    /// A content request of `credential` was answered `423 Locked`.
    ///
    /// A refusal of the grant held now shows that grant no longer works: it is forgotten
    /// (here and on this computer). A refusal of a request that carried an older grant,
    /// or none while one is held now, says nothing about the grant held.
    pub fn vault_refused(&mut self, credential: &Credential, locked: &FilesLocked) {
        if self.vault.credential.as_ref() != Some(credential) {
            return;
        }
        match (locked.grant, self.vault.grant) {
            (_, None) => {}
            (Some(sent), Some(held)) if sent == held => {
                self.vault_drop_grant();
            }
            _ => return,
        }
        self.vault.locked_seen = true;
        if let Some(status) = &mut self.vault.status {
            status.unlocked = false;
        }
    }

    /// Stop using the grant: the API stops sending it, this computer forgets it.
    fn vault_drop_grant(&mut self) -> Option<KeptGrant> {
        match self.vault.grant.take() {
            Some(number) => {
                self.api.release_grant_numbered(number);
            }
            None => {
                self.api.release_grant();
            }
        }
        self.vault.keeper.forget()
    }

    /// A download the locked files stopped: done again once they open, or now when the
    /// refusal turned out to be about a grant no longer in use.
    pub fn vault_wait(&mut self, credential: &Credential, waiting: Waiting) {
        if self.vault.credential.as_ref() != Some(credential) {
            return;
        }
        if !self.vault.is_locked() {
            self.vault_resume(waiting);
            return;
        }
        if !self.vault.waiting.contains(&waiting) {
            self.vault.waiting.push(waiting);
        }
        self.vault.focus = true;
        self.notice(
            icons::LOCK_KEYHOLE,
            t(port::VaultFilesLocked),
            t(port::VaultDownloadWaits),
            false,
        );
    }

    fn vault_resume(&mut self, waiting: Waiting) {
        let Some(credential) = self.credential() else {
            return;
        };
        match waiting {
            Waiting::Save { name, link, path } => {
                self.start_download(credential, name, link, path);
            }
            Waiting::File { file, convert_to } => {
                self.download_file(credential, *file, convert_to);
            }
        }
    }

    /// The banner's Unlock: the file password for a grant.
    pub fn vault_unlock(&mut self) {
        if self.vault.unlocking
            || self.vault.password.is_empty()
            || self.vault.waits_until(self.time).is_some()
        {
            return;
        }
        let Some(credential) = self.credential() else {
            self.show_sign_in();
            return;
        };
        if self.vault.credential.as_ref() != Some(&credential) {
            self.vault_follow_account();
        }
        self.vault.unlocking = true;
        self.vault.problem = None;
        let password = self.vault.password.clone();
        let remember = self.vault.remember;
        let label = modlight_api::signin::client_name(modlight_core::APP_NAME, &self.env.machine);
        let api = self.api.clone();
        self.rt.spawn(async move {
            let result = api.unlock_files(&credential, &password, &label).await;
            drop(password);
            move |state: &mut AppState, ctx: &egui::Context| {
                state.vault_unlock_finished(&credential, remember, result, ctx);
            }
        });
    }

    fn vault_unlock_finished(
        &mut self,
        credential: &Credential,
        remember: bool,
        result: Result<UnlockGrant, UnlockFailure>,
        ctx: &egui::Context,
    ) {
        // Another account signed in meanwhile: its files are not these.
        if self.vault.credential.as_ref() != Some(credential) {
            return;
        }
        self.vault.unlocking = false;
        match result {
            Ok(grant) => {
                let host = self.account.host().to_string();
                let tag = self.account.account().tag.clone();
                self.vault.password.clear();
                self.vault.problem = None;
                self.vault.locked_seen = false;
                if let Err(error) = self.vault.keeper.keep(&host, &tag, grant.clone(), remember) {
                    log::warn!(
                        "Modlight: the file password grant could not be kept on this computer: {error}"
                    );
                    if remember {
                        let text = format(t(port::VaultRememberFailed), &[Arg::from(error)]);
                        self.notice(icons::TRIANGLE_ALERT, &text, "", true);
                    }
                }
                self.vault.grant = Some(self.api.hold_grant(credential, &grant));
                let status = self.vault.status.get_or_insert(EncryptionStatus {
                    state: VaultState::On,
                    ..EncryptionStatus::default()
                });
                if !status.state.is_gated() {
                    status.state = VaultState::On;
                }
                status.unlocked = true;
                self.notice(icons::LOCK_KEYHOLE_OPEN, t(port::VaultUnlocked), "", false);
                self.vault_reload_contents(ctx);
            }
            Err(UnlockFailure::WrongPassword | UnlockFailure::Invalid) => {
                self.vault.password.clear();
                self.vault.problem = Some(Problem::WrongPassword);
                self.vault.focus = true;
            }
            Err(UnlockFailure::RateLimited(wait)) => {
                self.vault.password.clear();
                self.vault.problem = Some(Problem::RateLimited {
                    until: self.time + wait.as_secs_f64(),
                });
            }
            Err(UnlockFailure::NotEncrypted) => {
                self.vault.password.clear();
                self.vault.problem = None;
                self.vault.locked_seen = false;
                self.vault_drop_grant();
                self.vault.status = Some(EncryptionStatus::default());
                self.notice(icons::INFO, t(port::VaultNotEncrypted), "", false);
                self.vault_reload_contents(ctx);
            }
            Err(UnlockFailure::Failed(ApiError::Unauthorized)) => {
                // The token itself was refused: the account strip says so and offers the
                // sign-in; nothing signs out here.
                self.vault.problem = Some(Problem::Failed(t(s::MySpace_TokenRejected).into()));
            }
            Err(UnlockFailure::Failed(error)) => {
                let text = format(t(port::VaultUnlockFailed), &[Arg::from(error.to_string())]);
                self.vault.problem = Some(Problem::Failed(text));
            }
        }
    }

    /// "Lock files": the grant is forgotten here and revoked on the server, and the
    /// pictures it opened go from the window.
    pub fn vault_lock(&mut self) {
        let Some(credential) = self.credential() else {
            return;
        };
        if self.vault.credential.as_ref() != Some(&credential) {
            return;
        }
        let kept = self.vault_drop_grant();
        if let Some(status) = &mut self.vault.status {
            status.unlocked = false;
        }
        self.vault.waiting.clear();
        self.vault.problem = None;
        self.vault.password.clear();
        self.media.forget_pictures();
        self.notice(icons::LOCK_KEYHOLE, t(port::VaultLockedHere), "", false);
        if let Some(kept) = kept {
            self.vault_revoke(Some(credential), kept);
        }
    }

    /// The account menu's "Unlock files...": My space, with the password field ready.
    pub fn show_unlock_files(&mut self) {
        self.select_tab(crate::state::Tab::MySpace);
        self.vault.focus = true;
    }

    /// The files opened: the pictures, the drawer's preview and the downloads they
    /// stopped load again.
    fn vault_reload_contents(&mut self, ctx: &egui::Context) {
        self.media.restart_pictures();
        self.load_thumbnails(&[]);
        if self.details.present
            && let Some(id) = self.details.file.clone()
        {
            self.load_details_preview(ctx, &id);
        }
        for waiting in std::mem::take(&mut self.vault.waiting) {
            self.vault_resume(waiting);
        }
    }
}

// ------------------------------------------------------------------ the banner

/// Where the banner goes over the files of `list`: its full width less the grid's
/// margins, as tall as its words need.
pub fn banner_rect(ui: &Ui, state: &AppState, list: Rect) -> Rect {
    let width = (list.width() - 32.0).max(200.0);
    let height = plan(ui, state, width).height;
    Rect::from_min_size(
        pos2(list.left() + 16.0, list.top() + 14.0),
        vec2(width, height),
    )
}

/// The banner's arrangement for a width.
struct Plan {
    height: f32,
    /// The form beside the words (wide) or under them.
    beside: bool,
    text_width: f32,
    form_width: f32,
    title: std::sync::Arc<egui::Galley>,
    body: std::sync::Arc<egui::Galley>,
    problem: Option<std::sync::Arc<egui::Galley>>,
}

fn problem_text(state: &AppState) -> Option<String> {
    match &state.vault.problem {
        Some(Problem::WrongPassword) => Some(t(port::VaultWrongPassword).to_string()),
        Some(Problem::RateLimited { until }) => {
            let minutes = ((until - state.time) / 60.0).ceil().max(1.0) as i64;
            Some(format(t(port::VaultRateLimited), &[Arg::Int(minutes)]))
        }
        Some(Problem::Failed(text)) => Some(text.clone()),
        None => None,
    }
}

fn plan(ui: &Ui, state: &AppState, width: f32) -> Plan {
    let colors = &state.colors;
    let inner = width - 2.0 * PADDING;
    let text_left = TILE + 12.0;
    let beside_width = inner - text_left - 16.0 - FORM_WIDTH;
    let beside = beside_width >= MIN_TEXT_BESIDE;
    let text_width = if beside {
        beside_width
    } else {
        inner - text_left
    }
    .max(40.0);
    let title = layout(
        ui,
        wrapped(
            t(port::VaultBannerTitle),
            13.0,
            Weight::SemiBold,
            colors.text(),
            text_width,
        ),
    );
    let body = layout(
        ui,
        wrapped(
            t(port::VaultBannerText),
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            text_width,
        ),
    );
    let problem = problem_text(state).map(|text| {
        layout(
            ui,
            wrapped(
                &text,
                12.0,
                Weight::Medium,
                colors.cancel_border(),
                text_width,
            ),
        )
    });
    let words = title.size().y
        + 3.0
        + body.size().y
        + problem.as_ref().map(|p| 6.0 + p.size().y).unwrap_or(0.0);
    let form = ROW + 6.0 + CHECK_ROW;
    let form_width = if beside {
        FORM_WIDTH
    } else {
        (inner - text_left).min(FORM_WIDTH)
    };
    let content = if beside {
        words.max(TILE).max(form)
    } else {
        words.max(TILE) + 12.0 + form
    };
    Plan {
        height: (content + 2.0 * PADDING).ceil(),
        beside,
        text_width,
        form_width,
        title,
        body,
        problem,
    }
}

/// `button_at` in a child `Ui`: a placed control must not move the layout's cursor.
fn placed(ui: &mut Ui, rect: Rect) -> Ui {
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    )
}

/// The banner in `rect` (from [`banner_rect`]): the lock, what is going on, the file
/// password, "Remember on this computer for 30 days" and Unlock. Returns the command
/// it raised.
pub fn banner(ui: &mut Ui, state: &mut AppState, rect: Rect) -> Option<Command> {
    let colors: Colors = state.colors.clone();
    // The server's wait ran out: the attempt is open again.
    if let Some(Problem::RateLimited { until }) = state.vault.problem {
        if until <= state.time {
            state.vault.problem = None;
        } else {
            let left = until - state.time;
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64((left % 60.0).max(0.25)));
        }
    }
    let plan = plan(ui, state, rect.width());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 12.0, colors.control_background());
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    // Swallow clicks on the banner, so they do not reach the grid's background.
    let _ = ui.interact(rect, Id::new("vault-banner"), egui::Sense::click());

    let inner = rect.shrink(PADDING);
    let accent = if colors.graphite {
        colors.text()
    } else {
        colors.accent_start()
    };
    let tile = Rect::from_min_size(inner.min, vec2(TILE, TILE));
    painter.rect_filled(tile, 8.0, accent.gamma_multiply(0.12));
    paint_icon(
        ui,
        &painter,
        tile.center(),
        icons::LOCK_KEYHOLE,
        16.0,
        accent,
    );

    let text_left = tile.right() + 12.0;
    let mut y = inner.top();
    if plan.beside {
        // The words sit centred against the form when they are shorter than it.
        let words = plan.title.size().y
            + 3.0
            + plan.body.size().y
            + plan
                .problem
                .as_ref()
                .map(|p| 6.0 + p.size().y)
                .unwrap_or(0.0);
        let form = ROW + 6.0 + CHECK_ROW;
        y += ((form.max(TILE) - words) / 2.0).max(0.0);
    }
    painter.galley(pos2(text_left, y), plan.title.clone(), colors.text());
    y += plan.title.size().y + 3.0;
    painter.galley(
        pos2(text_left, y),
        plan.body.clone(),
        colors.text_secondary(),
    );
    y += plan.body.size().y;
    if let Some(problem) = &plan.problem {
        y += 6.0;
        painter.galley(pos2(text_left, y), problem.clone(), colors.cancel_border());
        y += problem.size().y;
    }
    let _ = plan.text_width;

    let form_left = if plan.beside {
        inner.right() - plan.form_width
    } else {
        text_left
    };
    let form_top = if plan.beside {
        inner.top() + ((TILE.max(y - inner.top()) - (ROW + 6.0 + CHECK_ROW)) / 2.0).max(0.0)
    } else {
        y.max(tile.bottom()) + 12.0
    };
    let idle = !state.vault.unlocking && state.vault.waits_until(state.time).is_none();
    let look = Look::primary(&colors).height(ROW).padding(14.0);
    let label = if state.vault.unlocking {
        t(port::VaultUnlocking)
    } else {
        t(port::VaultUnlock)
    };
    let content = Content::both(icons::LOCK_KEYHOLE_OPEN, label);
    let button_w = button_width(ui, &look, content);
    let field_rect = Rect::from_min_size(
        pos2(form_left, form_top),
        vec2((plan.form_width - button_w - 8.0).max(80.0), ROW),
    );
    let button_rect = Rect::from_min_size(
        pos2(field_rect.right() + 8.0, form_top),
        vec2(button_w, ROW),
    );
    let mut command = None;
    let field_id = Id::new("vault-password");
    let mut field_ui = placed(ui, field_rect);
    let field = Field {
        height: ROW,
        width: Some(field_rect.width()),
        hint: t(port::VaultPassword),
        password: true,
        enabled: idle,
        char_limit: 1024,
        id: Some(field_id),
        ..Field::new(&mut state.vault.password)
    }
    .show(&mut field_ui, &colors);
    if state.vault.focus && idle {
        field.request_focus();
        state.vault.focus = false;
    }
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let can_unlock = idle && !state.vault.password.is_empty();
    let mut button_ui = placed(ui, button_rect);
    let clicked = button_at(
        &mut button_ui,
        &colors,
        &look,
        content,
        can_unlock,
        button_rect,
    )
    .clicked();
    if (clicked || entered) && can_unlock {
        command = Some(Command::UnlockFiles);
    }
    let check_rect = Rect::from_min_size(
        pos2(form_left, form_top + ROW + 6.0),
        vec2(plan.form_width, CHECK_ROW),
    );
    let mut check_ui = placed(ui, check_rect);
    let _ = checkbox_weighted(
        &mut check_ui,
        &colors,
        &mut state.vault.remember,
        Some(t(port::VaultRemember)),
        !state.vault.unlocking,
        Weight::Normal,
    )
    .on_hover_text(t(port::VaultRememberHelp));
    command
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::myspace::actions::tests::{listing, state_on};

    const GRANT: &str =
        "6f1c2a9e-0000-4000-8000-000000000001.c2VjcmV0LW9mLWFsZXgtMDEyMzQ1Njc4OWFiY2RlZg";
    const PASSWORD: &str = "correct horse battery staple";

    /// A tiny PNG, so a thumbnail decodes.
    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// The server of an account whose files are encrypted: [`GRANT`] opens them, and the
    /// file password buys it (until `wrong` attempts run out, then 429). `revoked`
    /// turns the grant away again. Every request is logged as "METHOD path [grant]".
    struct Server {
        origin: String,
        log: Arc<Mutex<Vec<String>>>,
        revoked: Arc<Mutex<bool>>,
    }

    fn vault_server() -> Server {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let revoked = Arc::new(Mutex::new(false));
        let (kept, gone) = (log.clone(), revoked.clone());
        let picture = png();
        std::thread::spawn(move || {
            let mut wrong = 0;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut data = Vec::new();
                let mut buffer = [0u8; 4096];
                let head_end = loop {
                    if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break 0,
                        Ok(n) => data.extend_from_slice(&buffer[..n]),
                    }
                };
                if head_end == 0 {
                    continue;
                }
                let head = String::from_utf8_lossy(&data[..head_end]).to_string();
                let mut lines = head.split("\r\n");
                let first = lines.next().unwrap_or("").to_string();
                let headers: HashMap<String, String> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let length: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                while data.len() < head_end + length {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => data.extend_from_slice(&buffer[..n]),
                    }
                }
                let body = String::from_utf8_lossy(&data[head_end..]).to_string();
                let mut parts = first.split(' ');
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let grant = headers.get("x-modlight-unlock").cloned();
                let open = grant.as_deref() == Some(GRANT) && !*gone.lock().unwrap();
                kept.lock().unwrap().push(format!(
                    "{method} {path}{}",
                    if grant.is_some() { " [grant]" } else { "" }
                ));
                let (status, kind, payload): (&str, &str, Vec<u8>) =
                    match (method.as_str(), path.split('?').next().unwrap_or("")) {
                        ("GET", "/api/encryption") => (
                            "200 OK",
                            "application/json",
                            format!(r#"{{"state":"on","unlocked":{open},"progress":{{"done":1,"total":1}}}}"#)
                                .into_bytes(),
                        ),
                        ("POST", "/api/encryption/unlock") => {
                            if body.contains(PASSWORD) {
                                *gone.lock().unwrap() = false;
                                (
                                    "200 OK",
                                    "application/json",
                                    format!(r#"{{"grant":"{GRANT}","expiresAt":"2026-11-26T12:00:00Z"}}"#)
                                        .into_bytes(),
                                )
                            } else {
                                wrong += 1;
                                if wrong > 2 {
                                    (
                                        "429 Too Many Requests",
                                        "application/json",
                                        br#"{"error":"rate_limited","retryAfter":300}"#.to_vec(),
                                    )
                                } else {
                                    (
                                        "401 Unauthorized",
                                        "application/json",
                                        br#"{"error":"wrong_password"}"#.to_vec(),
                                    )
                                }
                            }
                        }
                        ("POST", "/api/encryption/lock") => {
                            *gone.lock().unwrap() = true;
                            ("204 No Content", "text/plain", Vec::new())
                        }
                        ("GET", p) if p.starts_with("/t/") || p.starts_with("/alex/") => {
                            if open {
                                ("200 OK", "image/png", picture.clone())
                            } else {
                                (
                                    "423 Locked",
                                    "application/json",
                                    br#"{"error":"files_locked","owner":"alex"}"#.to_vec(),
                                )
                            }
                        }
                        _ => ("404 Not Found", "text/plain", Vec::new()),
                    };
                let mut reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    payload.len()
                );
                if status.starts_with("423") {
                    reply.push_str("X-Modlight-Locked: 1\r\n");
                }
                reply.push_str("\r\n");
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.write_all(&payload);
            }
        });
        Server {
            origin,
            log,
            revoked,
        }
    }

    /// Run completions until `done` or ten seconds.
    fn settle(state: &mut AppState, done: impl Fn(&AppState) -> bool) {
        let ctx = egui::Context::default();
        let started = std::time::Instant::now();
        while !done(state) && started.elapsed().as_secs() < 10 {
            for completion in state.rt.drain() {
                completion(state, &ctx);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// A signed-in state on `server`, its listing holding files whose pictures are on it.
    fn signed_in(server: &Server) -> AppState {
        let mut state = state_on(&server.origin);
        listing(&mut state, 1, 30, 2, &["a", "b"]);
        for entry in &mut state.space.files {
            entry.file.thumbnail_url = format!("{}/t/{}.webp", server.origin, entry.file.code);
        }
        state.vault_follow_account();
        state
    }

    /// The locked account's pictures answer 423: the banner goes up, nobody is signed
    /// out, and the token is never called refused.
    #[test]
    fn a_locked_answer_raises_the_banner_and_signs_nobody_out() {
        let server = vault_server();
        let mut state = signed_in(&server);
        state.load_thumbnails(&[]);
        settle(&mut state, |s| s.vault.locked_seen);
        assert!(state.vault.locked_seen);
        assert!(state.vault_shows_banner());
        assert!(state.account.is_signed_in(), "a 423 never signs out");
        assert!(state.credential().is_some());
        assert!(
            state.toasts.iter().all(|t| !t.failed),
            "no failure is said for pictures"
        );
        assert_eq!(state.vault_badge(), Some(Badge::Locked));
        // The status agrees.
        state.vault_check();
        settle(&mut state, |s| s.vault.status.is_some());
        assert_eq!(state.vault.status.unwrap().state, VaultState::On);
        assert!(state.vault.is_locked());
        assert!(state.media.thumbs.is_empty());
    }

    /// The whole banner flow: a wrong password is said under the words, the right one
    /// unlocks, the grant is kept (remembered by default) and goes with the content
    /// requests, the pictures load, and a download the lock stopped starts again.
    #[test]
    fn unlocking_loads_the_pictures_and_the_waiting_download() {
        let server = vault_server();
        let mut state = signed_in(&server);
        let profile = tempfile::tempdir().unwrap();
        state.env.profile = profile.path().to_path_buf();

        // Before anything says the files are locked, a download is refused by the server
        // (423): it waits, to the place already chosen.
        state.download("a", None);
        settle(&mut state, |s| !s.vault.waiting.is_empty());
        assert!(matches!(state.vault.waiting[0], Waiting::Save { .. }));
        assert!(
            state
                .toasts
                .iter()
                .any(|t| t.title == t_(port::VaultFilesLocked))
        );
        assert!(state.vault_shows_banner());
        state.vault_check();
        settle(&mut state, |s| s.vault.status.is_some());
        assert!(state.vault_shows_banner());

        // Once the files are known to be locked, "Download as" waits without asking.
        let asked = server.log.lock().unwrap().len();
        state.download("b", Some("jpg".into()));
        assert_eq!(state.vault.waiting.len(), 2);
        assert!(matches!(state.vault.waiting[1], Waiting::File { .. }));
        assert_eq!(
            server.log.lock().unwrap().len(),
            asked,
            "nothing is asked while locked"
        );

        state.vault.password = "not it".into();
        state.vault_unlock();
        assert!(state.vault.unlocking);
        settle(&mut state, |s| !s.vault.unlocking);
        assert_eq!(state.vault.problem, Some(Problem::WrongPassword));
        assert!(
            state.vault.password.is_empty(),
            "a refused password is cleared"
        );
        assert!(state.vault_shows_banner());

        state.vault.password = PASSWORD.into();
        state.vault_unlock();
        settle(&mut state, |s| {
            !s.vault.unlocking && s.media.thumbs.len() == 2 && s.vault.waiting.is_empty()
        });
        assert_eq!(state.vault.problem, None);
        assert!(!state.vault_shows_banner());
        assert_eq!(state.vault_badge(), Some(Badge::Unlocked));
        assert!(
            state.vault.keeper.kept().unwrap().remembered,
            "remembered by default"
        );
        let credential = state.credential().unwrap();
        assert_eq!(state.api.held_grant(&credential), state.vault.grant);
        assert_eq!(state.media.thumbs.len(), 2, "the pictures came");
        let downloads = profile.path().join("Downloads");
        settle(&mut state, |_| {
            downloads.join("a.png").exists() && downloads.join("b.jpg").exists()
        });
        assert_eq!(
            std::fs::read(downloads.join("a.png")).unwrap(),
            png(),
            "the refused download went through"
        );
        assert!(
            downloads.join("b.jpg").exists(),
            "the conversion was asked about and saved"
        );
        let log = server.log.lock().unwrap().clone();
        assert!(
            log.iter()
                .any(|l| l.starts_with("GET /t/") && l.ends_with("[grant]"))
        );
        assert!(
            log.iter()
                .filter(|l| l.starts_with("POST /api/encryption/unlock"))
                .all(|l| !l.ends_with("[grant]")),
            "asking for a grant sends none"
        );
    }

    #[test]
    fn too_many_attempts_wait_for_the_server() {
        let server = vault_server();
        let mut state = signed_in(&server);
        for _ in 0..3 {
            state.vault.password = "not it".into();
            state.vault_unlock();
            settle(&mut state, |s| !s.vault.unlocking);
        }
        let Some(Problem::RateLimited { until }) = state.vault.problem else {
            panic!("{:?}", state.vault.problem)
        };
        assert_eq!(until, state.time + 300.0);
        assert_eq!(
            problem_text(&state).unwrap(),
            format(t(port::VaultRateLimited), &[Arg::Int(5)])
        );
        // Nothing is sent while the server waits.
        let sent = server.log.lock().unwrap().len();
        state.vault.password = PASSWORD.into();
        state.vault_unlock();
        assert!(!state.vault.unlocking);
        assert_eq!(server.log.lock().unwrap().len(), sent);
        // Once the wait is over, it is.
        state.time = until + 1.0;
        state.vault_unlock();
        settle(&mut state, |s| !s.vault.unlocking);
        assert!(state.vault.is_unlocked());
        assert!(state.vault.grant.is_some());
    }

    /// "Lock files" forgets the grant here and on this computer, revokes it on the
    /// server and takes the pictures off the window.
    #[test]
    fn locking_forgets_the_grant_and_the_pictures() {
        let server = vault_server();
        let mut state = signed_in(&server);
        state.vault.password = PASSWORD.into();
        state.vault_unlock();
        settle(&mut state, |s| {
            !s.vault.unlocking && s.media.thumbs.len() == 2
        });
        assert!(state.vault.is_unlocked());

        state.vault_lock();
        assert!(state.vault.keeper.kept().is_none());
        assert_eq!(state.vault.grant, None);
        assert_eq!(state.api.held_grant(&state.credential().unwrap()), None);
        assert!(state.media.thumbs.is_empty(), "the pictures go");
        assert!(state.vault_shows_banner());
        settle(&mut state, |_| *server.revoked.lock().unwrap());
        assert!(*server.revoked.lock().unwrap(), "revoked on the server");
        let log = server.log.lock().unwrap().clone();
        assert!(log.contains(&"POST /api/encryption/lock [grant]".to_string()));
    }

    /// A 423 for the grant held shows it no longer works (revoked in the panel, say): it
    /// is forgotten, and the banner comes back. A 423 for a request that carried no grant
    /// (sent before the unlock) says nothing about the grant held.
    #[test]
    fn a_grant_the_server_turns_away_is_forgotten() {
        let server = vault_server();
        let mut state = signed_in(&server);
        state.vault.password = PASSWORD.into();
        state.vault_unlock();
        settle(&mut state, |s| {
            !s.vault.unlocking && s.vault.grant.is_some()
        });
        let credential = state.credential().unwrap();

        state.vault_refused(&credential, &FilesLocked::default());
        assert!(
            state.vault.grant.is_some(),
            "a request without a grant says nothing"
        );
        assert!(!state.vault_shows_banner());
        state.vault_refused(
            &credential,
            &FilesLocked {
                owner: None,
                grant: Some(state.vault.grant.unwrap() + 7),
            },
        );
        assert!(
            state.vault.grant.is_some(),
            "nor does an older grant's refusal"
        );

        *server.revoked.lock().unwrap() = true;
        state.media.forget_pictures();
        state.load_thumbnails(&[]);
        settle(&mut state, |s| s.vault.grant.is_none());
        assert!(
            state.vault.keeper.kept().is_none(),
            "forgotten on this computer"
        );
        assert_eq!(state.api.held_grant(&credential), None);
        assert!(state.vault_shows_banner());
        assert!(state.account.is_signed_in());
    }

    /// Without "remember", the grant lives for this run only; with it, the next run has
    /// it, for this account only.
    #[test]
    fn the_grant_is_remembered_only_when_asked() {
        let server = vault_server();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(modlight_core::vault::GRANT_FILE);
        let store = || {
            GrantKeeper::new(Box::new(modlight_core::account::FileCredentialStore::new(
                path.clone(),
            )))
        };

        let mut state = signed_in(&server);
        state.vault.keeper = store();
        state.vault.remember = false;
        state.vault.password = PASSWORD.into();
        state.vault_unlock();
        settle(&mut state, |s| !s.vault.unlocking);
        assert!(state.vault.grant.is_some());
        assert!(!path.exists(), "not on this computer");

        let mut state = signed_in(&server);
        state.vault.keeper = store();
        state.vault.password = PASSWORD.into();
        state.vault_unlock();
        settle(&mut state, |s| !s.vault.unlocking);
        assert!(path.exists(), "remembered");

        // The next run of the same account holds it before any picture is asked for.
        let mut later = state_on(&server.origin);
        later.vault.keeper = store();
        later.vault_follow_account();
        assert!(later.vault.grant.is_some());
        assert_eq!(
            later.api.held_grant(&later.credential().unwrap()),
            later.vault.grant
        );

        // Signing out forgets it, here and on this computer.
        later.sign_out();
        assert!(later.vault.keeper.kept().is_none());
        assert!(!path.exists());
        assert_eq!(later.vault.grant, None);
        settle(&mut later, |_| *server.revoked.lock().unwrap());
        assert!(
            *server.revoked.lock().unwrap(),
            "revoked while the token still works"
        );
    }

    /// Another account signing in never gets the first one's grant.
    #[test]
    fn switching_accounts_forgets_the_grant() {
        let server = vault_server();
        let mut state = signed_in(&server);
        state.vault.password = PASSWORD.into();
        state.vault_unlock();
        settle(&mut state, |s| {
            !s.vault.unlocking && s.vault.grant.is_some()
        });

        state.account = modlight_core::account::AccountStore::in_memory(
            modlight_core::account::StoredAccount {
                host: server.origin.clone(),
                tag: "sam".into(),
                ..Default::default()
            },
            "token-of-sam-0123456789",
        );
        state.space_refresh_account();
        assert!(state.vault.keeper.kept().is_none());
        assert_eq!(state.vault.grant, None);
        assert_eq!(state.api.held_grant(&state.credential().unwrap()), None);
        assert!(
            !state.vault_shows_banner(),
            "nothing is known about sam's files yet"
        );
    }

    fn t_(key: modlight_core::l10n::Key) -> String {
        t(key).to_string()
    }
}

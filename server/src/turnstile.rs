//! Optional Cloudflare Turnstile ("verify you are human") in front of the
//! public entry points. NOT a port of anything upstream: an addition on top of
//! the 1:1 port (SKELETON.md section 9, item 43). With no complete
//! configuration nothing here runs: no check, no outbound call, no change to
//! the CSP, no widget on any page.
//!
//! Protected while it is on:
//! - `POST /api/auth/sign-in/email`, `/api/auth/sign-up/email` and
//!   `/api/auth/request-password-reset`, checked by the router before the
//!   endpoint runs ([`guards_auth_path`], called from `auth::api`), which is
//!   where better-auth's own `captcha` plugin runs (`onRequest`), with that
//!   plugin's error codes ([`auth_refusal`]);
//! - the server actions `login-identifier.signInWithIdentifier` and
//!   `invite.acceptInvitation` ([`check_action`]), refused with their usual
//!   `{status}` / `{ok: false, reason}` shapes.
//!
//! Server-side calls (`auth::server::call_endpoint`, what `auth.api.*` was)
//! never pass through the router, so the sign-in an action performs after its
//! own check is not checked a second time (a token is good for one siteverify).
//!
//! The token travels in the `x-captcha-response` request header, the header
//! better-auth's captcha plugin reads, so no action signature and no request
//! body changes.
//!
//! Configuration: block precedence exactly like the SMTP settings
//! (`mail_settings`). Once the admin panel's card has been saved (the
//! `turnstile_enabled` row holds "on" or "off"), the switch, the site key and
//! the secret all come from the panel and the environment is ignored; until
//! then TURNSTILE_SITE_KEY + TURNSTILE_SECRET_KEY (both set) turn it on. Never
//! a key from each. The secret is stored in the secret box (purpose
//! `turnstile`) and never leaves the server: the panel learns a state.
//!
//! Verification: `POST https://challenges.cloudflare.com/turnstile/v0/siteverify`
//! with the form fields `secret`, `response` and `remoteip` (the caller's
//! address under the same rule the rate limiters use, `password::client_ip`),
//! 10 s for the whole exchange, HTTPS through rustls (ring, Mozilla roots).
//! Only `"success": true` passes. A network failure, a timeout or an answer
//! that is not Cloudflare's JSON refuses the request ("fails closed"). Neither
//! the secret nor the token is ever logged.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::{HeaderMap, Request, StatusCode, header};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;

use crate::auth::error::{ApiError, captcha};
use crate::config::{Config, TurnstileEnv, js_trim};
use crate::secret_box::{SecretPurpose, encrypt_secret, try_decrypt_secret};
use crate::settings::{get_settings, is_on, keys, set_setting};
use crate::state::AppState;

/// Cloudflare's verification endpoint.
pub const SITEVERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
/// Where the widget's script and frames come from (the CSP addition).
pub const CHALLENGES_ORIGIN: &str = "https://challenges.cloudflare.com";
/// The request header that carries the widget's token.
pub const TOKEN_HEADER: &str = "x-captcha-response";
/// The whole siteverify exchange: connect, TLS, request and answer.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);
/// Cloudflare: a token is at most 2048 characters. A longer one is refused
/// without asking.
pub const TOKEN_MAX_LEN: usize = 2048;
/// The token Cloudflare's test site keys produce. Sent by the admin panel's
/// "Test" (and by a save that turns the check on) to learn whether Cloudflare
/// accepts a secret: a real secret answers `invalid-input-response` for it,
/// a wrong one `invalid-input-secret`, a "always passes" test secret success.
pub const PROBE_TOKEN: &str = "XXXX.DUMMY.TOKEN.XXXX";
/// The Better Auth routes the check guards (better-auth's captcha plugin's
/// `defaultEndpoints`).
pub const GUARDED_AUTH_PATHS: &[&str] = &["/sign-up/email", "/sign-in/email", "/request-password-reset"];

/// How long a snapshot is trusted before the table is read again.
const TTL_MS: i64 = 30_000;
/// The largest siteverify answer read (Cloudflare's is about 200 bytes).
const REPLY_MAX: usize = 64 * 1024;

/// The `settings` rows of the block.
pub const SETTING_KEYS: &[&str] = &[keys::TURNSTILE_ENABLED, keys::TURNSTILE_SITE_KEY, keys::TURNSTILE_SECRET_ENC];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnstileSource {
    Panel,
    Env,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretState {
    Unset,
    Set,
    Undecryptable,
}

/// The effective configuration. The secret never leaves the server (and is
/// never printed: see the Debug impl).
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedTurnstile {
    pub source: TurnstileSource,
    /// The panel's switch; for the environment, whether both keys are set.
    pub switch_on: bool,
    pub site_key: String,
    pub secret: String,
    pub secret_state: SecretState,
    /// The check runs: switched on, a site key, and a secret that can be read.
    pub active: bool,
}

impl std::fmt::Debug for ResolvedTurnstile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedTurnstile")
            .field("source", &self.source)
            .field("switch_on", &self.switch_on)
            .field("site_key", &self.site_key)
            .field("secret_state", &self.secret_state)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl ResolvedTurnstile {
    pub fn off() -> ResolvedTurnstile {
        ResolvedTurnstile {
            source: TurnstileSource::None,
            switch_on: false,
            site_key: String::new(),
            secret: String::new(),
            secret_state: SecretState::Unset,
            active: false,
        }
    }

    /// What the browser renders the widget with (`layouts.root.config`).
    pub fn public_site_key(&self) -> Option<&str> {
        self.active.then_some(self.site_key.as_str())
    }
}

/// `resolveTurnstile(rows, env, decrypt)`: pure, every input a parameter.
pub fn resolve_turnstile(
    rows: &HashMap<String, String>,
    env: &TurnstileEnv,
    decrypt: &dyn Fn(&str) -> Option<String>,
) -> ResolvedTurnstile {
    let row = |k: &str| rows.get(k).map(String::as_str);
    // The switch row anchors the panel block: written by every save, cleared
    // (NULL, which reads as missing) by "use the environment".
    if let Some(switch) = row(keys::TURNSTILE_ENABLED) {
        let site_key = js_trim(row(keys::TURNSTILE_SITE_KEY).unwrap_or("")).to_string();
        let boxed = row(keys::TURNSTILE_SECRET_ENC).unwrap_or("");
        let decrypted = if boxed.is_empty() { None } else { decrypt(boxed) };
        let secret_state = match (boxed.is_empty(), &decrypted) {
            (true, _) => SecretState::Unset,
            (false, None) => SecretState::Undecryptable,
            (false, Some(_)) => SecretState::Set,
        };
        let secret = decrypted.unwrap_or_default();
        let switch_on = is_on(Some(switch));
        let active = switch_on && !site_key.is_empty() && !secret.is_empty();
        return ResolvedTurnstile { source: TurnstileSource::Panel, switch_on, site_key, secret, secret_state, active };
    }
    if env.complete() {
        return ResolvedTurnstile {
            source: TurnstileSource::Env,
            switch_on: true,
            site_key: env.site_key.clone(),
            secret: env.secret_key.clone(),
            secret_state: SecretState::Set,
            active: true,
        };
    }
    ResolvedTurnstile::off()
}

/// What the admin panel may know. Never the secret. The site key is public
/// (every protected page carries it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnstileStatus {
    pub source: TurnstileSource,
    /// The switch as stored (the environment: both keys set).
    pub enabled: bool,
    /// The check runs on the protected forms.
    pub active: bool,
    pub site_key: String,
    pub secret_state: SecretState,
    /// TURNSTILE_SITE_KEY and TURNSTILE_SECRET_KEY are both set.
    pub env_available: bool,
}

pub fn to_status(resolved: &ResolvedTurnstile, config: &Config) -> TurnstileStatus {
    TurnstileStatus {
        source: resolved.source,
        enabled: resolved.switch_on,
        active: resolved.active,
        site_key: resolved.site_key.clone(),
        secret_state: resolved.secret_state,
        env_available: config.turnstile.complete(),
    }
}

// ── Process state ───────────────────────────────────────────────────────────

/// One siteverify answer (the fields the check reads).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SiteverifyReply {
    pub success: bool,
    #[serde(rename = "error-codes", default)]
    pub error_codes: Vec<String>,
}

/// Why a siteverify exchange produced no answer. Never carries the secret or
/// the token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SiteverifyError {
    #[error("no answer within {0:?}")]
    Timeout(Duration),
    #[error("{0}")]
    Network(String),
    #[error("HTTP {0}")]
    Status(u16),
    #[error("the answer is not siteverify JSON")]
    NotJson,
}

type HttpClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

struct Snapshot {
    value: ResolvedTurnstile,
    read_at: i64,
}

/// The process-wide part: the 30 s configuration snapshot (read without
/// awaiting by the CSP, which is applied after the handler), the HTTPS client
/// and the endpoint (Cloudflare's; tests point it at a stub).
pub struct Turnstile {
    siteverify_url: String,
    timeout: Duration,
    snapshot: Mutex<Option<Snapshot>>,
    client: OnceLock<Result<HttpClient, String>>,
}

impl Default for Turnstile {
    fn default() -> Self {
        Turnstile::new()
    }
}

impl std::fmt::Debug for Turnstile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Turnstile").field("siteverify_url", &self.siteverify_url).finish_non_exhaustive()
    }
}

fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

/// What a probe of a secret (the panel's "Test") found out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Cloudflare knows the secret. `always_passes`: it accepted the dummy
    /// token, which only Cloudflare's "always passes" test secret does.
    Accepted { always_passes: bool },
    /// `invalid-input-secret` / `missing-input-secret`.
    Rejected,
    /// No usable answer (network, timeout, not JSON).
    Unreachable,
}

impl Turnstile {
    pub fn new() -> Turnstile {
        Turnstile::with_siteverify_url(SITEVERIFY_URL)
    }

    /// A Turnstile whose verification goes to `url` (tests: a stub on
    /// 127.0.0.1; plain `http://` is accepted for that).
    pub fn with_siteverify_url(url: impl Into<String>) -> Turnstile {
        Turnstile {
            siteverify_url: url.into(),
            timeout: VERIFY_TIMEOUT,
            snapshot: Mutex::new(None),
            client: OnceLock::new(),
        }
    }

    /// The same with a shorter deadline than [`VERIFY_TIMEOUT`] (tests).
    pub fn with_timeout(mut self, timeout: Duration) -> Turnstile {
        self.timeout = timeout;
        self
    }

    /// `loadTurnstile(force)`: the effective configuration, refreshing the
    /// snapshot when it is stale (or always, when `force`).
    pub async fn load(&self, db: &PgPool, config: &Config, force: bool) -> sqlx::Result<ResolvedTurnstile> {
        if !force {
            let guard = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = guard.as_ref()
                && now_ms() - s.read_at < TTL_MS
            {
                return Ok(s.value.clone());
            }
        }
        let rows = get_settings(db, SETTING_KEYS).await?;
        let secret = config.auth.secret.clone();
        let decrypt = move |boxed: &str| try_decrypt_secret(&secret, Some(boxed), SecretPurpose::Turnstile);
        let value = resolve_turnstile(&rows, &config.turnstile, &decrypt);
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Snapshot { value: value.clone(), read_at: now_ms() });
        Ok(value)
    }

    /// [`Turnstile::load`], falling back to the last known configuration (or
    /// the environment) when the table cannot be read. The fallback is kept
    /// for one snapshot period, so a database outage costs one failed read
    /// per 30 s here instead of a pool timeout on every request.
    pub async fn current(&self, db: &PgPool, config: &Config) -> ResolvedTurnstile {
        match self.load(db, config, false).await {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!("[turnstile] could not read the settings, using the last known configuration: {err}");
                let value = self.snapshot(config);
                *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Snapshot { value: value.clone(), read_at: now_ms() });
                value
            }
        }
    }

    /// The last known configuration WITHOUT awaiting. Before the first load
    /// it is the environment's.
    pub fn snapshot(&self, config: &Config) -> ResolvedTurnstile {
        let guard = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(s) => s.value.clone(),
            None => resolve_turnstile(&HashMap::new(), &config.turnstile, &|_| None),
        }
    }

    /// Does the CSP have to let the widget in? (the snapshot; page loads
    /// refresh it through the root layout before their headers are applied)
    pub fn csp_on(&self, config: &Config) -> bool {
        self.snapshot(config).active
    }

    fn client(&self) -> Result<&HttpClient, SiteverifyError> {
        let built = self.client.get_or_init(|| {
            let connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_provider_and_webpki_roots(rustls::crypto::ring::default_provider())
                .map_err(|e| format!("TLS setup failed: {e}"))?
                .https_or_http()
                .enable_http1()
                .build();
            Ok(Client::builder(TokioExecutor::new()).build(connector))
        });
        built.as_ref().map_err(|e| SiteverifyError::Network(e.clone()))
    }

    /// One siteverify exchange.
    pub async fn siteverify(
        &self,
        secret: &str,
        token: &str,
        remote_ip: Option<IpAddr>,
    ) -> Result<SiteverifyReply, SiteverifyError> {
        let client = self.client()?;
        let body = {
            let mut form = form_urlencoded::Serializer::new(String::new());
            form.append_pair("secret", secret).append_pair("response", token);
            if let Some(ip) = remote_ip {
                form.append_pair("remoteip", &ip.to_string());
            }
            form.finish()
        };
        let request = Request::post(self.siteverify_url.as_str())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::ACCEPT, "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| SiteverifyError::Network("invalid siteverify URL".into()))?;
        let exchange = async {
            let response = client.request(request).await.map_err(|e| SiteverifyError::Network(describe(&e)))?;
            let status = response.status();
            let body = Limited::new(response.into_body(), REPLY_MAX)
                .collect()
                .await
                .map_err(|_| SiteverifyError::Network("the answer could not be read".into()))?
                .to_bytes();
            if status != StatusCode::OK {
                return Err(SiteverifyError::Status(status.as_u16()));
            }
            serde_json::from_slice::<SiteverifyReply>(&body).map_err(|_| SiteverifyError::NotJson)
        };
        tokio::time::timeout(self.timeout, exchange).await.map_err(|_| SiteverifyError::Timeout(self.timeout))?
    }

    /// Does Cloudflare accept `secret`? (the panel's "Test", and a save that
    /// turns the check on: a wrong secret would refuse every sign-in, the
    /// administrator's own included)
    pub async fn probe_secret(&self, secret: &str) -> Probe {
        match self.siteverify(secret, PROBE_TOKEN, None).await {
            Ok(reply) if reply.success => Probe::Accepted { always_passes: true },
            Ok(reply)
                if reply.error_codes.iter().any(|c| c == "invalid-input-secret" || c == "missing-input-secret") =>
            {
                Probe::Rejected
            }
            Ok(_) => Probe::Accepted { always_passes: false },
            Err(err) => {
                tracing::warn!("[turnstile] secret test: siteverify unreachable: {err}");
                Probe::Unreachable
            }
        }
    }
}

/// A client error and its causes, for the log (hyper's own text plus the IO
/// error under it). Contains no part of the request.
fn describe(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

// ── The check ───────────────────────────────────────────────────────────────

/// The outcome of the check on one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Turnstile is not configured: nothing was checked.
    Off,
    Passed,
    /// No token in the request.
    Missing,
    /// Cloudflare said no (or the token is over Cloudflare's length limit).
    Failed,
    /// Cloudflare could not be asked: refused all the same.
    Unavailable,
}

impl Verdict {
    pub fn allows(self) -> bool {
        matches!(self, Verdict::Off | Verdict::Passed)
    }
}

/// The token a request carries (`x-captcha-response`), trimmed; empty = none.
pub fn request_token(headers: &HeaderMap) -> &str {
    headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok()).map(js_trim).unwrap_or("")
}

/// The address sent as `remoteip`: the caller as the rate limiters see it
/// (`X-Real-IP` only behind TRUST_PROXY), when it is an IP at all.
pub fn remote_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> Option<IpAddr> {
    crate::password::client_ip(headers, peer, trust_proxy).parse::<IpAddr>().ok().map(|ip| ip.to_canonical())
}

/// The check against a known configuration.
pub async fn verify_request(
    turnstile: &Turnstile,
    resolved: &ResolvedTurnstile,
    headers: &HeaderMap,
    remote_ip: Option<IpAddr>,
) -> Verdict {
    if !resolved.active {
        return Verdict::Off;
    }
    let token = request_token(headers);
    if token.is_empty() {
        return Verdict::Missing;
    }
    if token.len() > TOKEN_MAX_LEN {
        return Verdict::Failed;
    }
    match turnstile.siteverify(&resolved.secret, token, remote_ip).await {
        Ok(reply) if reply.success => Verdict::Passed,
        Ok(reply) => {
            // Cloudflare's error codes are safe to log; the token is not.
            tracing::info!("[turnstile] verification refused: {}", reply.error_codes.join(", "));
            Verdict::Failed
        }
        Err(err) => {
            tracing::warn!("[turnstile] siteverify unreachable, request refused: {err}");
            Verdict::Unavailable
        }
    }
}

/// The check for one request to a protected entry point.
pub async fn check_request(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> Verdict {
    let resolved = state.turnstile.current(&state.db, &state.config).await;
    if !resolved.active {
        return Verdict::Off;
    }
    let ip = remote_ip(headers, peer, state.config.trust_proxy);
    verify_request(&state.turnstile, &resolved, headers, ip).await
}

/// The check for a server action.
pub async fn check_action(ctx: &crate::rpc::ActionCtx) -> Verdict {
    check_request(&ctx.state, &ctx.headers, ctx.peer).await
}

/// Is `path` (below `/api/auth`, normalised like the rate limiter's) one the
/// check guards? `includes`, as better-auth's captcha plugin matches.
pub fn guards_auth_path(path: &str) -> bool {
    GUARDED_AUTH_PATHS.iter().any(|p| path.contains(p))
}

/// The refusal of a Better Auth route, in better-call's APIError form with
/// the captcha plugin's codes: 400 `MISSING_RESPONSE`, 403
/// `VERIFICATION_FAILED`, 503 `SERVICE_UNAVAILABLE`.
pub fn auth_refusal(verdict: Verdict) -> Option<ApiError> {
    match verdict {
        Verdict::Off | Verdict::Passed => None,
        Verdict::Missing => Some(ApiError::from("BAD_REQUEST", captcha::MISSING_RESPONSE)),
        Verdict::Failed => Some(ApiError::from("FORBIDDEN", captcha::VERIFICATION_FAILED)),
        Verdict::Unavailable => Some(ApiError::from("SERVICE_UNAVAILABLE", captcha::SERVICE_UNAVAILABLE)),
    }
}

// ── Writing ─────────────────────────────────────────────────────────────────

/// A save of the panel card, already validated.
#[derive(Clone)]
pub struct TurnstilePatch {
    pub enabled: bool,
    pub site_key: String,
    /// A newly typed secret; `None` keeps the stored one while the site key
    /// stays the same, and forgets it when the site key changes (a secret
    /// belongs to one widget).
    pub secret: Option<String>,
}

/// Write the panel block. Returns the names of the rows that changed (never
/// a value) and whether the secret was replaced or forgotten.
pub async fn save_settings(state: &AppState, patch: &TurnstilePatch) -> anyhow::Result<(Vec<String>, bool)> {
    let before = get_settings(&state.db, SETTING_KEYS).await?;
    let mut changed: Vec<String> = Vec::new();
    let switch = if patch.enabled { "on" } else { "off" };
    for (key, value) in [(keys::TURNSTILE_ENABLED, switch), (keys::TURNSTILE_SITE_KEY, patch.site_key.as_str())] {
        if before.get(key).map(String::as_str) != Some(value) {
            changed.push(key.to_string());
        }
        set_setting(&state.db, key, Some(value)).await?;
    }
    let same_widget = before.contains_key(keys::TURNSTILE_ENABLED)
        && before.get(keys::TURNSTILE_SITE_KEY).map(String::as_str).unwrap_or("") == patch.site_key;
    let secret_changed = if let Some(secret) = &patch.secret {
        let boxed = encrypt_secret(&state.config.auth.secret, secret, SecretPurpose::Turnstile)?;
        set_setting(&state.db, keys::TURNSTILE_SECRET_ENC, Some(&boxed)).await?;
        true
    } else if !same_widget && before.contains_key(keys::TURNSTILE_SECRET_ENC) {
        set_setting(&state.db, keys::TURNSTILE_SECRET_ENC, None).await?;
        true
    } else {
        false
    };
    if secret_changed {
        changed.push(keys::TURNSTILE_SECRET_ENC.to_string());
    }
    state.turnstile.load(&state.db, &state.config, true).await?;
    Ok((changed, secret_changed))
}

/// Remove every panel row of the block: the environment decides again.
pub async fn clear_settings(state: &AppState) -> sqlx::Result<ResolvedTurnstile> {
    for key in SETTING_KEYS {
        set_setting(&state.db, key, None).await?;
    }
    state.turnstile.load(&state.db, &state.config, true).await
}

/// Site keys and secrets are Cloudflare identifiers (`0x4AAAAAAA...`, the
/// test keys `1x0000...AA`): letters, digits, `_` and `-`. Anything else is
/// a paste accident, and a site key ends up in every protected page.
pub fn is_key_shaped(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The detail of an audit entry: names only, never a value.
pub fn audit_detail(by: &str, by_email: &str, changed: &[String], secret_changed: bool) -> serde_json::Value {
    json!({ "by": by, "byEmail": by_email, "changed": changed, "secretChanged": secret_changed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(site: &str, secret: &str) -> TurnstileEnv {
        TurnstileEnv { site_key: site.into(), secret_key: secret.into() }
    }

    fn rows(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    /// Stand-in for the AES-GCM box: precedence does not need it.
    fn decrypt(boxed: &str) -> Option<String> {
        if boxed == "BROKEN" { None } else { Some(boxed.trim_start_matches("enc:").to_string()) }
    }

    fn resolve(r: &[(&str, &str)], e: &TurnstileEnv) -> ResolvedTurnstile {
        resolve_turnstile(&rows(r), e, &decrypt)
    }

    #[test]
    fn off_without_any_configuration() {
        let r = resolve(&[], &env("", ""));
        assert_eq!(r.source, TurnstileSource::None);
        assert!(!r.active);
        assert_eq!(r.public_site_key(), None);
    }

    #[test]
    fn half_an_environment_is_off() {
        assert!(!resolve(&[], &env("site", "")).active);
        assert!(!resolve(&[], &env("", "secret")).active);
    }

    #[test]
    fn the_environment_alone_turns_it_on() {
        let r = resolve(&[], &env("site", "secret"));
        assert_eq!(r.source, TurnstileSource::Env);
        assert!(r.active);
        assert_eq!(r.secret, "secret");
        assert_eq!(r.public_site_key(), Some("site"));
    }

    #[test]
    fn a_saved_panel_block_wins_entirely_even_switched_off() {
        let r = resolve(&[("turnstile_enabled", "off")], &env("site", "secret"));
        assert_eq!(r.source, TurnstileSource::Panel);
        assert!(!r.active);
        assert_eq!(r.site_key, "");
        assert_eq!(r.secret, "", "an environment secret never fills a panel block");
    }

    #[test]
    fn a_panel_block_needs_switch_site_key_and_a_readable_secret() {
        let full = [
            ("turnstile_enabled", "on"),
            ("turnstile_site_key", " panel-site "),
            ("turnstile_secret_enc", "enc:panel-secret"),
        ];
        let r = resolve(&full, &env("", ""));
        assert!(r.active);
        assert_eq!(r.site_key, "panel-site");
        assert_eq!(r.secret, "panel-secret");
        assert_eq!(r.secret_state, SecretState::Set);

        let no_secret = resolve(&full[..2], &env("site", "secret"));
        assert!(!no_secret.active);
        assert_eq!(no_secret.secret_state, SecretState::Unset);

        let broken = resolve(
            &[("turnstile_enabled", "on"), ("turnstile_site_key", "panel-site"), ("turnstile_secret_enc", "BROKEN")],
            &env("", ""),
        );
        assert!(!broken.active);
        assert_eq!(broken.secret_state, SecretState::Undecryptable);

        let no_site = resolve(&[("turnstile_enabled", "on"), ("turnstile_secret_enc", "enc:s")], &env("", ""));
        assert!(!no_site.active);
    }

    #[test]
    fn the_debug_form_never_shows_the_secret() {
        let r = resolve(&[], &env("site", "very-secret-value"));
        assert!(!format!("{r:?}").contains("very-secret-value"));
        let config = TurnstileEnv { site_key: "s".into(), secret_key: "very-secret-value".into() };
        assert!(!format!("{config:?}").contains("very-secret-value"));
    }

    #[test]
    fn guarded_paths_are_the_captcha_plugins_defaults() {
        for p in ["/sign-in/email", "/sign-up/email", "/request-password-reset"] {
            assert!(guards_auth_path(p), "{p}");
        }
        for p in ["/get-session", "/reset-password", "/reset-password/abc", "/two-factor/verify-totp", "/sign-out"] {
            assert!(!guards_auth_path(p), "{p}");
        }
    }

    #[test]
    fn refusals_carry_the_captcha_plugins_codes() {
        assert!(auth_refusal(Verdict::Off).is_none());
        assert!(auth_refusal(Verdict::Passed).is_none());
        let missing = auth_refusal(Verdict::Missing).unwrap();
        assert_eq!(missing.status.as_u16(), 400);
        assert_eq!(missing.code(), Some("MISSING_RESPONSE"));
        let failed = auth_refusal(Verdict::Failed).unwrap();
        assert_eq!(failed.status.as_u16(), 403);
        assert_eq!(failed.code(), Some("VERIFICATION_FAILED"));
        let down = auth_refusal(Verdict::Unavailable).unwrap();
        assert_eq!(down.status.as_u16(), 503);
        assert_eq!(down.status_text, "SERVICE_UNAVAILABLE");
        assert_eq!(down.code(), Some("SERVICE_UNAVAILABLE"));
    }

    #[test]
    fn tokens_and_addresses_from_the_request() {
        let mut h = HeaderMap::new();
        assert_eq!(request_token(&h), "");
        h.insert(TOKEN_HEADER, " tok ".parse().unwrap());
        assert_eq!(request_token(&h), "tok");
        h.insert("x-real-ip", "203.0.113.7".parse().unwrap());
        let peer: SocketAddr = "[::ffff:192.0.2.1]:4000".parse().unwrap();
        assert_eq!(remote_ip(&h, Some(peer), false), Some("192.0.2.1".parse().unwrap()));
        assert_eq!(remote_ip(&h, Some(peer), true), Some("203.0.113.7".parse().unwrap()));
        assert_eq!(remote_ip(&HeaderMap::new(), None, true), None);
    }

    #[test]
    fn key_shapes() {
        assert!(is_key_shaped("1x00000000000000000000AA", 128));
        assert!(is_key_shaped("0x4AAAAAAAB_c-d", 128));
        assert!(!is_key_shaped("", 128));
        assert!(!is_key_shaped("has space", 128));
        assert!(!is_key_shaped("quote\"", 128));
        assert!(!is_key_shaped(&"a".repeat(129), 128));
    }

    #[tokio::test]
    async fn disabled_never_reads_the_token_or_calls_out() {
        // An unroutable endpoint: any call would fail as Unavailable.
        let t = Turnstile::with_siteverify_url("http://127.0.0.1:1/siteverify");
        let mut h = HeaderMap::new();
        h.insert(TOKEN_HEADER, "tok".parse().unwrap());
        assert_eq!(verify_request(&t, &ResolvedTurnstile::off(), &h, None).await, Verdict::Off);
    }
}

//! Port of `lib/config.ts`: centralized runtime configuration, sourced from
//! environment variables.
//!
//! Framework-agnostic on purpose, exactly like upstream: branding lives here so
//! the product is rebrandable by changing one env var.
//!
//! Differences from upstream, all additive:
//! - `BIND_ADDR` (default `127.0.0.1`) picks the listen address. Upstream
//!   always listened on every interface; this port never does unless told to
//!   (the Docker image sets `0.0.0.0` inside the container).
//! - The config is built from an explicit variable map ([`Config::from_env_map`])
//!   so tests do not have to mutate the process environment, which is what the
//!   upstream tests needed `vi.resetModules()` for.
//! - A production refusal is returned as [`ConfigError`] instead of being
//!   thrown at module load; `main` prints it and exits non-zero, which is what
//!   the uncaught upstream exception amounted to.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use url::Url;

/// Every variable this module reads, so callers can snapshot exactly these.
pub const ENV_VARS: &[&str] = &[
    "NODE_ENV",
    "NEXT_PHASE",
    "SKIP_ENV_VALIDATION",
    "AUTH_SECRET",
    "DATABASE_URL",
    "APP_URL",
    "APP_DOMAINS",
    "APP_NAME",
    "EMBED_BRAND",
    "SHORTLINK_URL",
    "PORT",
    "BIND_ADDR",
    "TRUST_PROXY",
    "REGISTRATION_ENABLED",
    "STORAGE_DRIVER",
    "STORAGE_LOCAL_PATH",
    "MAX_UPLOAD_SIZE",
    "MIN_FREE_DISK_BYTES",
    "TUS_CHUNK_SIZE",
    "TUS_TMP_PATH",
    "UPLOAD_TMP_PATH",
    "DEFAULT_USER_QUOTA",
    "SMTP_HOST",
    "SMTP_PORT",
    "SMTP_USER",
    "SMTP_PASSWORD",
    "SMTP_SECURE",
    "SMTP_SECURITY",
    "MAIL_FROM",
    "MAIL_FROM_NAME",
];

/// A production refusal. Displays exactly like the upstream `Error` message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

fn fatal(message: &str) -> ConfigError {
    ConfigError(format!("[config] Refusing to start in production: {message}"))
}

// ── JavaScript value semantics ──────────────────────────────────────────────
// The upstream helpers lean on `Number(...)`, `String.prototype.trim()` and a
// regex; these reproduce them so an .env file means the same thing to both
// implementations (`PORT=` is 0 in JavaScript, not "unset").

/// Characters `String.prototype.trim()` removes (WhiteSpace + LineTerminator).
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `String.prototype.trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// `Number(string)`: the StringToNumber conversion of ECMAScript.
pub fn js_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let bytes = t.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] {
            b'x' | b'X' => Some(16),
            b'o' | b'O' => Some(8),
            b'b' | b'B' => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            let digits = &t[2..];
            if digits.chars().all(|c| c.is_digit(radix)) {
                // Accumulate in f64 like the spec (exact up to 2^53).
                return digits
                    .chars()
                    .fold(0.0, |acc, c| acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0)));
            }
            return f64::NAN;
        }
    }
    if !is_js_decimal_literal(t) {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Math.round(n)`: halves round towards +Infinity (`-2.5` -> `-2`), NaN and
/// the infinities pass through.
pub fn js_round(n: f64) -> f64 {
    if !n.is_finite() {
        return n;
    }
    let f = n.floor();
    let r = if n - f >= 0.5 { f + 1.0 } else { f };
    // Math.round(-0.4) is -0; keep the sign of zero like JavaScript.
    if r == 0.0 && n.is_sign_negative() { -0.0 } else { r }
}

/// `[+-]? (digits [. digits?] | . digits) ([eE] [+-]? digits)?`
fn is_js_decimal_literal(t: &str) -> bool {
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - frac_start;
    }
    if int_digits == 0 && frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == b.len()
}

/// `bool(v, fallback)` from lib/config.ts: `/^(1|true|yes|on)$/i` after trim.
/// A variable that is set to anything else is `false`, not the fallback.
pub fn env_bool(v: Option<&str>, fallback: bool) -> bool {
    match v {
        None => fallback,
        Some(v) => matches!(js_trim(v).to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
    }
}

/// `int(v, fallback)` from lib/config.ts: `Number(v)` when finite.
pub fn env_number(v: Option<&str>, fallback: f64) -> f64 {
    match v {
        None => fallback,
        Some(v) => {
            let n = js_number(v);
            if n.is_finite() { n } else { fallback }
        }
    }
}

/// A JavaScript number used as a byte count. Truncates like `Math.trunc`.
fn as_i64(n: f64) -> i64 {
    n as i64
}

// ── Secret validation ───────────────────────────────────────────────────────
// Dev keeps working with zero configuration, production refuses to boot with a
// guessable secret. Booting is the last moment we can still stop that safely.

/// The development fallback for AUTH_SECRET (never accepted in production).
pub const DEV_AUTH_SECRET: &str = "dev-insecure-secret-change-me";
const MIN_AUTH_SECRET_LENGTH: usize = 32;
const GENERATE_SECRET_HINT: &str = "generate one with `openssl rand -base64 32`";

/// Values that ship in .env.example, docs and old tutorials.
const PLACEHOLDER_SECRETS: &[&str] = &[
    DEV_AUTH_SECRET,
    "change-me-to-a-long-random-string",
    "change-me",
    "changeme",
    "please-change-me",
    "secret",
    "supersecret",
];

/// The development fallback for DATABASE_URL.
pub const DEV_DATABASE_URL: &str = "postgresql://modlight:modlight@localhost:5432/modlight";

/// Database passwords that ship as defaults in compose files and examples.
const PLACEHOLDER_DB_PASSWORDS: &[&str] =
    &["modlight", "postgres", "password", "change-me", "changeme", "secret", "admin", "root"];

/// Process-wide memory of the warnings already printed (upstream `warnOnce`).
static WARNED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Registrable-ish labels that are never a brand on their own, so
/// `example.co.uk` yields "example" rather than "co".
const SECOND_LEVEL_SUFFIXES: &[&str] = &["co", "com", "net", "org", "gov", "edu", "ac"];

/// Strip trailing slashes so every origin concatenates cleanly with a path.
pub fn normalize_origin(value: &str) -> String {
    js_trim(value).trim_end_matches('/').to_string()
}

/// Percent-decoded password of a URL, or "" when there is none or it does not
/// parse (upstream `urlPassword`).
fn url_password(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return String::new();
    };
    let Some(raw) = parsed.password() else {
        return String::new();
    };
    percent_encoding::percent_decode_str(raw).decode_utf8().map(|s| s.into_owned()).unwrap_or_default()
}

/// The one word a link preview should carry: the site, not the product.
///
/// Derived from APP_URL (never from the owner's chosen domain), because every
/// hostname of one deployment has to unfurl under the SAME name - otherwise the
/// same file reads as "host.example.com" or "h.example.com" depending on who
/// shared it.
///
/// - `https://host.example.com` -> "example"
/// - `https://h.example.com` -> "example"
/// - `https://files.example.co.uk` -> "example"
///
/// An IP literal or a single-label host has nothing to strip and is returned whole.
pub fn brand_from_url(url: &str) -> String {
    let mut host = Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    // A bare "host.example.com:8420/path" PARSES (as scheme "host.example.com")
    // rather than failing, and yields an empty hostname - so fall back on the
    // result too.
    if host.is_empty() {
        let trimmed = js_trim(url);
        let without_scheme = strip_scheme_prefix(trimmed);
        host = without_scheme.split('/').next().unwrap_or("").split(':').next().unwrap_or("").to_string();
    }
    let mut host = host.as_str();
    if host.len() >= 4 && host[..4].eq_ignore_ascii_case("www.") {
        host = &host[4..];
    }
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host);
    let host = host.trim_end_matches('.').to_lowercase();
    if host.is_empty() {
        return String::new();
    }

    // IPv4/IPv6 literals: no label carries meaning, so don't invent one.
    let numeric = host.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if numeric || host.contains(':') {
        return host;
    }

    let mut labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() <= 1 {
        return labels.first().map(|s| s.to_string()).unwrap_or_default();
    }
    labels.pop(); // TLD
    if labels.len() > 1 && SECOND_LEVEL_SUFFIXES.contains(labels.last().unwrap_or(&"")) {
        labels.pop();
    }
    labels.last().map(|s| s.to_string()).unwrap_or_default()
}

/// `/^[a-z][a-z0-9+.-]*:\/\//i` removed from the front of `s`.
fn strip_scheme_prefix(s: &str) -> &str {
    let b = s.as_bytes();
    if b.is_empty() || !b[0].is_ascii_alphabetic() {
        return s;
    }
    let mut i = 1;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'+' | b'.' | b'-')) {
        i += 1;
    }
    if s[i..].starts_with("://") { &s[i + 3..] } else { s }
}

/// Every hostname this deployment answers on, APP_URL first, deduplicated
/// case-insensitively. See the upstream comment in lib/config.ts: one
/// application, several names, and a user may pick which one their links use.
pub fn app_domains(app_url: &str, app_domains_var: Option<&str>) -> Vec<String> {
    let extra: Vec<String> =
        app_domains_var.unwrap_or("").split(',').map(normalize_origin).filter(|s| !s.is_empty()).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for origin in std::iter::once(app_url.to_string()).chain(extra) {
        let key = origin.to_lowercase();
        if seen.insert(key) {
            out.push(origin);
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Signs session cookies AND the e-mail verification links.
    pub secret: String,
    pub registration_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct DbConfig {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// `local` or `s3`, lower-cased, not validated (as upstream).
    pub driver: String,
    pub local_path: String,
}

#[derive(Debug, Clone)]
pub struct UploadsConfig {
    /// Max upload size in bytes; 0 = unlimited (large files are the point).
    pub max_size: i64,
    /// Refuse uploads that would leave the filesystem with less than this free.
    /// A full disk does not degrade gracefully: it takes down every database on
    /// the host and corrupts any backup caught mid-write. 10 GiB by default.
    pub min_free_bytes: i64,
    /// tus chunk size (bytes) for resumable web uploads.
    pub tus_chunk_size: i64,
    /// Where tus keeps in-progress uploads before finalize (local driver).
    pub tus_tmp_path: String,
    /// Scratch dir for the ShareX streaming path.
    pub tmp_path: String,
}

#[derive(Debug, Clone)]
pub struct MailConfig {
    pub host: String,
    pub port: i64,
    pub user: String,
    pub password: String,
    pub secure: bool,
    /// "tls" | "starttls" | "none". Empty derives the mode from SMTP_SECURE.
    pub security: String,
    pub from: String,
    /// Display name on the From header. Empty falls back to the app name.
    pub from_name: String,
}

impl MailConfig {
    /// Upstream `get enabled()`: `Boolean(process.env.SMTP_HOST)`.
    pub fn enabled(&self) -> bool {
        !self.host.is_empty()
    }
}

/// Port of the upstream `config` object.
#[derive(Debug, Clone)]
pub struct Config {
    pub app_name: String,
    /// Name a link preview signs itself with (og:site_name, oEmbed
    /// provider_name). Deliberately NOT app_name; defaults to the brand inside
    /// APP_URL.
    pub embed_brand: String,
    /// Public base URL, no trailing slash. Used for links, ShareX config, embeds.
    pub app_url: String,
    /// All origins the app is reachable on, `app_url` first.
    pub app_domains: Vec<String>,
    /// Base URL used when GENERATING short links. Falls back to APP_URL.
    pub short_url: String,
    /// TCP port. `int(process.env.PORT, 8420)`; validated like Node's
    /// `server.listen` when the server starts ([`Config::listen_port`]).
    pub port: f64,
    /// Listen address (Rust port only). Default `127.0.0.1`.
    pub bind_addr: String,
    pub is_prod: bool,
    pub trust_proxy: bool,
    pub auth: AuthConfig,
    pub db: DbConfig,
    pub storage: StorageConfig,
    pub uploads: UploadsConfig,
    /// Default per-user disk quota in bytes; -1 = unlimited.
    pub default_user_quota: i64,
    pub mail: MailConfig,
    /// SHORTLINK_URL exactly as set: proxy.ts reads it raw, not through
    /// lib/config, to find the shortener host.
    pub raw_shortlink_url: Option<String>,
    /// Origins for the CSP, from the raw APP_URL, SHORTLINK_URL and
    /// APP_DOMAINS the way next.config.ts read them.
    pub csp_origins: Vec<String>,
}

/// Lookup over an environment snapshot.
struct Env<'a>(&'a HashMap<String, String>);

impl Env<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
    /// `process.env.X?.trim() || ""`
    fn trimmed(&self, key: &str) -> &str {
        self.get(key).map(js_trim).unwrap_or("")
    }
    /// `process.env.X || fallback` (no trim, empty counts as unset)
    fn or(&self, key: &str, fallback: &str) -> String {
        match self.get(key) {
            Some(v) if !v.is_empty() => v.to_string(),
            _ => fallback.to_string(),
        }
    }
}

impl Config {
    /// Build from the real process environment (after `.env` was loaded).
    pub fn from_env() -> Result<Config, ConfigError> {
        let mut map = HashMap::new();
        for key in ENV_VARS {
            if let Some(v) = std::env::var_os(key) {
                map.insert((*key).to_string(), v.to_string_lossy().into_owned());
            }
        }
        Config::from_env_map(&map)
    }

    /// Build from an explicit variable map. Missing keys are unset variables.
    pub fn from_env_map(vars: &HashMap<String, String>) -> Result<Config, ConfigError> {
        let env = Env(vars);
        let is_prod = env.get("NODE_ENV") == Some("production");
        // `next build` evaluated this module without the runtime environment;
        // SKIP_ENV_VALIDATION is the manual escape hatch (CI, image smoke tests).
        let is_build_phase =
            env.get("NEXT_PHASE") == Some("phase-production-build") || env_bool(env.get("SKIP_ENV_VALIDATION"), false);
        let enforce_secrets = is_prod && !is_build_phase;
        let warn = |key: &str, message: &str| {
            if is_build_phase {
                return;
            }
            let mut guard = WARNED.lock().unwrap_or_else(|e| e.into_inner());
            let set = guard.get_or_insert_with(HashSet::new);
            if set.insert(key.to_string()) {
                tracing::warn!("[config] {message}");
            }
        };

        let auth_secret = {
            let secret = env.trimmed("AUTH_SECRET").to_string();
            // AUTH_SECRET signs session cookies AND the e-mail verification
            // links, and verified links auto-sign-in the account they point at
            // (autoSignInAfterVerification) - a known secret means account takeover.
            if enforce_secrets {
                if secret.is_empty() {
                    return Err(fatal(&format!("AUTH_SECRET is not set — {GENERATE_SECRET_HINT}.")));
                }
                if PLACEHOLDER_SECRETS.contains(&secret.to_lowercase().as_str()) {
                    return Err(fatal(&format!("AUTH_SECRET is still a placeholder value — {GENERATE_SECRET_HINT}.")));
                }
                let len = secret.encode_utf16().count();
                if len < MIN_AUTH_SECRET_LENGTH {
                    return Err(fatal(&format!(
                        "AUTH_SECRET is too short ({len} chars, minimum {MIN_AUTH_SECRET_LENGTH}) — {GENERATE_SECRET_HINT}."
                    )));
                }
                secret
            } else if secret.is_empty() {
                warn(
                    "auth-secret",
                    &format!(
                        "AUTH_SECRET is not set — using an insecure development fallback. Set it before deploying ({GENERATE_SECRET_HINT})."
                    ),
                );
                DEV_AUTH_SECRET.to_string()
            } else {
                secret
            }
        };

        let database_url = {
            let url = env.trimmed("DATABASE_URL").to_string();
            if enforce_secrets {
                if url.is_empty() {
                    return Err(fatal("DATABASE_URL is not set."));
                }
                let password = url_password(&url);
                if PLACEHOLDER_DB_PASSWORDS.contains(&password.to_lowercase().as_str()) {
                    return Err(fatal(
                        "DATABASE_URL still uses the default database password — set POSTGRES_PASSWORD (docker-compose) or a full DATABASE_URL with a generated password.",
                    ));
                }
                // No password at all is legitimate for unix-socket/peer auth, so only warn.
                if password.is_empty() {
                    warn(
                        "db-password",
                        "DATABASE_URL has no password — make sure the database is not reachable from the network.",
                    );
                }
                url
            } else if url.is_empty() {
                warn("db-url", "DATABASE_URL is not set — using the local development database.");
                DEV_DATABASE_URL.to_string()
            } else {
                url
            }
        };

        let app_url_raw = env.trimmed("APP_URL");
        let app_url = normalize_origin(if app_url_raw.is_empty() { "http://localhost:8420" } else { app_url_raw });

        let app_name = match env.trimmed("APP_NAME") {
            "" => "Modlight".to_string(),
            v => v.to_string(),
        };
        let embed_brand = {
            let explicit = env.trimmed("EMBED_BRAND");
            if !explicit.is_empty() {
                explicit.to_string()
            } else {
                let brand = brand_from_url(&app_url);
                if !brand.is_empty() {
                    brand
                } else if !env.trimmed("APP_NAME").is_empty() {
                    env.trimmed("APP_NAME").to_string()
                } else {
                    "Modlight".to_string()
                }
            }
        };
        let short_url = {
            let raw = env.trimmed("SHORTLINK_URL");
            normalize_origin(if raw.is_empty() { &app_url } else { raw })
        };
        let bind_addr = match env.trimmed("BIND_ADDR") {
            "" => "127.0.0.1".to_string(),
            v => v.to_string(),
        };

        Ok(Config {
            app_domains: app_domains(&app_url, env.get("APP_DOMAINS")),
            app_url,
            app_name,
            embed_brand,
            short_url,
            port: env_number(env.get("PORT"), 8420.0),
            bind_addr,
            is_prod,
            trust_proxy: env_bool(env.get("TRUST_PROXY"), true),
            auth: AuthConfig {
                secret: auth_secret,
                registration_enabled: env_bool(env.get("REGISTRATION_ENABLED"), false),
            },
            db: DbConfig { url: database_url },
            storage: StorageConfig {
                driver: env.or("STORAGE_DRIVER", "local").to_lowercase(),
                local_path: env.or("STORAGE_LOCAL_PATH", "./data/storage"),
            },
            uploads: UploadsConfig {
                max_size: as_i64(env_number(env.get("MAX_UPLOAD_SIZE"), 0.0)),
                min_free_bytes: as_i64(env_number(env.get("MIN_FREE_DISK_BYTES"), 10.0 * 1024.0 * 1024.0 * 1024.0)),
                tus_chunk_size: as_i64(env_number(env.get("TUS_CHUNK_SIZE"), 50.0 * 1024.0 * 1024.0)),
                tus_tmp_path: env.or("TUS_TMP_PATH", "./data/tus"),
                tmp_path: env.or("UPLOAD_TMP_PATH", "./data/tmp"),
            },
            default_user_quota: as_i64(env_number(env.get("DEFAULT_USER_QUOTA"), -1.0)),
            mail: MailConfig {
                host: env.or("SMTP_HOST", ""),
                port: as_i64(env_number(env.get("SMTP_PORT"), 587.0)),
                user: env.or("SMTP_USER", ""),
                password: env.or("SMTP_PASSWORD", ""),
                secure: env_bool(env.get("SMTP_SECURE"), false),
                security: env.or("SMTP_SECURITY", ""),
                from: env.or("MAIL_FROM", "no-reply@localhost"),
                from_name: env.or("MAIL_FROM_NAME", ""),
            },
            raw_shortlink_url: env.get("SHORTLINK_URL").map(str::to_string),
            csp_origins: crate::http::headers::self_origins(
                env.get("APP_URL"),
                env.get("SHORTLINK_URL"),
                env.get("APP_DOMAINS"),
            ),
        })
    }

    /// The port as Node's `server.listen()` would accept it: an integer in
    /// `0..=65535`, otherwise the same ERR_SOCKET_BAD_PORT wording.
    pub fn listen_port(&self) -> Result<u16, ConfigError> {
        let p = self.port;
        if p.fract() == 0.0 && (0.0..=65535.0).contains(&p) {
            Ok(p as u16)
        } else {
            Err(ConfigError(format!(
                "Port should be >= 0 and < 65536. Received type number ({}).",
                format_js_number(p)
            )))
        }
    }

    /// `true` when APP_URL is https, which is what makes Better Auth put the
    /// `__Secure-` prefix on its cookies.
    pub fn secure_cookies(&self) -> bool {
        self.app_url.starts_with("https://")
    }
}

/// Number formatting for messages (`String(n)` for the values we print).
pub fn format_js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 { format!("{}", n as i64) } else { format!("{n}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        Config::from_env_map(&map)
    }

    const HOST: &str = "https://host.example.com";
    const H: &str = "https://h.example.com";
    const S: &str = "https://s.example.com";

    // tests/unit/naming-domains.test.ts: describe("config.appDomains")
    #[test]
    fn app_domains_falls_back_to_app_url_alone() {
        let c = cfg(&[("APP_URL", HOST)]).unwrap();
        assert_eq!(c.app_domains, vec![HOST]);
    }

    #[test]
    fn app_domains_parses_a_comma_separated_list_with_app_url_first() {
        let c = cfg(&[("APP_URL", HOST), ("APP_DOMAINS", &format!("{H}, {S}"))]).unwrap();
        assert_eq!(c.app_domains, vec![HOST, H, S]);
    }

    #[test]
    fn app_domains_normalises_trailing_slashes_and_drops_duplicates() {
        let c = cfg(&[("APP_URL", &format!("{HOST}/")), ("APP_DOMAINS", &format!("{HOST}//,{H}/,{H}"))]).unwrap();
        assert_eq!(c.app_domains, vec![HOST, H]);
    }

    #[test]
    fn app_domains_ignores_empty_entries_from_a_trailing_comma() {
        let c = cfg(&[("APP_URL", HOST), ("APP_DOMAINS", &format!("{H},,  ,"))]).unwrap();
        assert_eq!(c.app_domains, vec![HOST, H]);
    }

    // tests/unit/embed-tags.test.ts: describe("brandFromUrl")
    #[test]
    fn brand_collapses_every_hostname_of_one_deployment_to_the_same_word() {
        assert_eq!(brand_from_url("https://host.example.com"), "example");
        assert_eq!(brand_from_url("https://h.example.com"), "example");
        assert_eq!(brand_from_url("https://s.example.com"), "example");
        assert_eq!(brand_from_url("https://example.com"), "example");
    }

    #[test]
    fn brand_skips_a_second_level_suffix() {
        assert_eq!(brand_from_url("https://files.example.co.uk"), "example");
        assert_eq!(brand_from_url("https://example.com.au"), "example");
    }

    #[test]
    fn brand_drops_www_and_a_trailing_path_or_port() {
        assert_eq!(brand_from_url("https://www.example.com/files"), "example");
        assert_eq!(brand_from_url("host.example.com:8420/x"), "example");
    }

    #[test]
    fn brand_leaves_hosts_with_nothing_to_strip_alone() {
        assert_eq!(brand_from_url("http://localhost:8420"), "localhost");
        assert_eq!(brand_from_url("http://192.168.1.10"), "192.168.1.10");
        assert_eq!(brand_from_url(""), "");
    }

    #[test]
    fn brand_keeps_ipv6_literals() {
        assert_eq!(brand_from_url("http://[::1]:8420"), "::1");
    }

    // Defaults, exactly as lib/config.ts states them.
    #[test]
    fn defaults_without_any_variable() {
        let c = cfg(&[]).unwrap();
        assert_eq!(c.app_name, "Modlight");
        assert_eq!(c.app_url, "http://localhost:8420");
        assert_eq!(c.app_domains, vec!["http://localhost:8420"]);
        assert_eq!(c.short_url, "http://localhost:8420");
        assert_eq!(c.embed_brand, "localhost");
        assert_eq!(c.port, 8420.0);
        assert_eq!(c.listen_port().unwrap(), 8420);
        assert_eq!(c.bind_addr, "127.0.0.1");
        assert!(!c.is_prod);
        assert!(c.trust_proxy);
        assert_eq!(c.auth.secret, DEV_AUTH_SECRET);
        assert!(!c.auth.registration_enabled);
        assert_eq!(c.db.url, DEV_DATABASE_URL);
        assert_eq!(c.storage.driver, "local");
        assert_eq!(c.storage.local_path, "./data/storage");
        assert_eq!(c.uploads.max_size, 0);
        assert_eq!(c.uploads.min_free_bytes, 10 * 1024 * 1024 * 1024);
        assert_eq!(c.uploads.tus_chunk_size, 50 * 1024 * 1024);
        assert_eq!(c.uploads.tus_tmp_path, "./data/tus");
        assert_eq!(c.uploads.tmp_path, "./data/tmp");
        assert_eq!(c.default_user_quota, -1);
        assert_eq!(c.mail.host, "");
        assert_eq!(c.mail.port, 587);
        assert!(!c.mail.secure);
        assert_eq!(c.mail.security, "");
        assert_eq!(c.mail.from, "no-reply@localhost");
        assert_eq!(c.mail.from_name, "");
        assert!(!c.mail.enabled());
        assert!(!c.secure_cookies());
    }

    #[test]
    fn values_are_read_and_normalised() {
        let c = cfg(&[
            ("APP_URL", " https://files.example.co.uk/ "),
            ("APP_NAME", "  Acme  "),
            ("SHORTLINK_URL", "https://s.example.co.uk///"),
            ("PORT", "8421"),
            ("BIND_ADDR", "0.0.0.0"),
            ("TRUST_PROXY", "off"),
            ("REGISTRATION_ENABLED", " YES "),
            ("STORAGE_DRIVER", "S3"),
            ("MAX_UPLOAD_SIZE", "1e3"),
            ("TUS_CHUNK_SIZE", "0x10"),
            ("DEFAULT_USER_QUOTA", "garbage"),
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_SECURE", "1"),
        ])
        .unwrap();
        assert_eq!(c.app_url, "https://files.example.co.uk");
        assert_eq!(c.app_name, "Acme");
        assert_eq!(c.embed_brand, "example");
        assert_eq!(c.short_url, "https://s.example.co.uk");
        assert_eq!(c.listen_port().unwrap(), 8421);
        assert_eq!(c.bind_addr, "0.0.0.0");
        assert!(!c.trust_proxy);
        assert!(c.auth.registration_enabled);
        assert_eq!(c.storage.driver, "s3");
        assert_eq!(c.uploads.max_size, 1000);
        assert_eq!(c.uploads.tus_chunk_size, 16);
        assert_eq!(c.default_user_quota, -1);
        assert!(c.mail.enabled());
        assert!(c.mail.secure);
        assert!(c.secure_cookies());
    }

    #[test]
    fn embed_brand_override_and_fallbacks() {
        assert_eq!(cfg(&[("EMBED_BRAND", " Acme ")]).unwrap().embed_brand, "Acme");
        // A URL with no brand in it falls back to APP_NAME, then to "Modlight".
        assert_eq!(cfg(&[("APP_URL", "http://.")]).unwrap().embed_brand, "Modlight");
        assert_eq!(cfg(&[("APP_URL", "http://."), ("APP_NAME", "Acme")]).unwrap().embed_brand, "Acme");
    }

    #[test]
    fn javascript_number_semantics() {
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number("   "), 0.0);
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number("-5"), -5.0);
        assert_eq!(js_number("1.5"), 1.5);
        assert_eq!(js_number(".5"), 0.5);
        assert_eq!(js_number("5."), 5.0);
        assert_eq!(js_number("1e3"), 1000.0);
        assert_eq!(js_number("0x1F"), 31.0);
        assert_eq!(js_number("0b101"), 5.0);
        assert_eq!(js_number("0o17"), 15.0);
        assert!(js_number("12abc").is_nan());
        assert!(js_number("inf").is_nan());
        assert!(js_number("NaN").is_nan());
        assert!(js_number("-0x10").is_nan());
        assert!(js_number("1e").is_nan());
        assert_eq!(js_number("Infinity"), f64::INFINITY);
        // `PORT=` is 0 in JavaScript, not "unset".
        assert_eq!(env_number(Some(""), 8420.0), 0.0);
        assert_eq!(env_number(Some("Infinity"), 8420.0), 8420.0);
        assert_eq!(env_number(None, 8420.0), 8420.0);
    }

    #[test]
    fn javascript_bool_semantics() {
        assert!(env_bool(None, true));
        assert!(!env_bool(None, false));
        assert!(!env_bool(Some(""), true));
        assert!(env_bool(Some(" On "), false));
        assert!(!env_bool(Some("enabled"), true));
    }

    #[test]
    fn listen_port_is_validated_like_node() {
        let bad = cfg(&[("PORT", "70000")]).unwrap();
        assert_eq!(bad.listen_port().unwrap_err().0, "Port should be >= 0 and < 65536. Received type number (70000).");
        assert!(cfg(&[("PORT", "1.5")]).unwrap().listen_port().is_err());
        assert_eq!(cfg(&[("PORT", "")]).unwrap().listen_port().unwrap(), 0);
    }

    const STRONG: &str = "0123456789abcdef0123456789abcdef-strong";
    const GOOD_DB: &str = "postgresql://modlight:Zx81kQ@db:5432/modlight";

    #[test]
    fn production_refuses_a_missing_secret() {
        let e = cfg(&[("NODE_ENV", "production"), ("DATABASE_URL", GOOD_DB)]).unwrap_err();
        assert_eq!(
            e.0,
            "[config] Refusing to start in production: AUTH_SECRET is not set — generate one with `openssl rand -base64 32`."
        );
    }

    #[test]
    fn production_refuses_a_placeholder_secret_case_insensitively() {
        let e = cfg(&[
            ("NODE_ENV", "production"),
            ("AUTH_SECRET", "Change-Me-To-A-Long-Random-String"),
            ("DATABASE_URL", GOOD_DB),
        ])
        .unwrap_err();
        assert!(e.0.contains("AUTH_SECRET is still a placeholder value"));
    }

    #[test]
    fn production_refuses_a_short_secret() {
        let e = cfg(&[("NODE_ENV", "production"), ("AUTH_SECRET", "short-but-random"), ("DATABASE_URL", GOOD_DB)])
            .unwrap_err();
        assert_eq!(
            e.0,
            "[config] Refusing to start in production: AUTH_SECRET is too short (16 chars, minimum 32) — generate one with `openssl rand -base64 32`."
        );
    }

    #[test]
    fn production_refuses_missing_or_default_database_credentials() {
        let e = cfg(&[("NODE_ENV", "production"), ("AUTH_SECRET", STRONG)]).unwrap_err();
        assert_eq!(e.0, "[config] Refusing to start in production: DATABASE_URL is not set.");
        let e = cfg(&[
            ("NODE_ENV", "production"),
            ("AUTH_SECRET", STRONG),
            ("DATABASE_URL", "postgresql://modlight:ModLight@db:5432/modlight"),
        ])
        .unwrap_err();
        assert!(e.0.contains("DATABASE_URL still uses the default database password"));
        // Percent-encoded placeholder passwords are decoded before the check.
        let e = cfg(&[
            ("NODE_ENV", "production"),
            ("AUTH_SECRET", STRONG),
            ("DATABASE_URL", "postgresql://modlight:%70ostgres@db/modlight"),
        ])
        .unwrap_err();
        assert!(e.0.contains("default database password"));
    }

    #[test]
    fn production_accepts_good_values_and_passwordless_urls() {
        let c = cfg(&[("NODE_ENV", "production"), ("AUTH_SECRET", STRONG), ("DATABASE_URL", GOOD_DB)]).unwrap();
        assert!(c.is_prod);
        assert_eq!(c.auth.secret, STRONG);
        assert_eq!(c.db.url, GOOD_DB);
        // Unix-socket/peer auth has no password: only a warning.
        let c = cfg(&[
            ("NODE_ENV", "production"),
            ("AUTH_SECRET", STRONG),
            ("DATABASE_URL", "postgresql://modlight@/modlight?host=/run/postgresql"),
        ])
        .unwrap();
        assert!(c.is_prod);
    }

    #[test]
    fn skip_env_validation_disables_the_production_checks() {
        let c = cfg(&[("NODE_ENV", "production"), ("SKIP_ENV_VALIDATION", "1")]).unwrap();
        assert_eq!(c.auth.secret, DEV_AUTH_SECRET);
        assert_eq!(c.db.url, DEV_DATABASE_URL);
    }
}

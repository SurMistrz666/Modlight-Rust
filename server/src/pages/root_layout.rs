//! Loader of `app/layout.tsx`, the root layout (fully ported).
//!
//! Upstream renders, for every page:
//! - `<html lang={locale} data-mode={mode} data-theme={skin}>`;
//! - `NextIntlClientProvider` with `locale`, `messages` (the 17 client
//!   namespaces) and `timeZone` (the server's, `UTC` in the container);
//! - `ThemeProvider` seeded with `defaultTheme={mode}`.
//!
//! `mode` is `resolveMode(ml_mode cookie)`; the skin is
//! `resolveTheme(session?.user.theme ?? ml_skin cookie)`: the stored column
//! beats the cookie for anyone signed in. The session lookup degrades to the
//! cookie when it fails (`getSession().catch(() => null)`).
//!
//! Payload (`layouts.root`):
//! `{ locale, messages, messagesHash, timeZone, mode, theme, config }`, where
//! `config` is the public part of `lib/config.ts` (`appName`, `embedBrand`,
//! `appUrl`, `appDomains`, `shortUrl`) that upstream's client components read
//! from the server-rendered tree (`PublicConfig` in the SPA, CONTRACT 5.7),
//! plus, not upstream, `turnstileSiteKey`: the Cloudflare Turnstile site key
//! while the optional check is on, else `null` (src/turnstile.rs).

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::i18n::Locale;
use crate::pages::{LayoutResult, PageCtx};
use crate::theme_cookies::{MODE_COOKIE, SKIN_COOKIE, resolve_mode};
use crate::themes::resolve_theme;

/// The server's IANA time zone as Node would report it (TZ, else UTC).
pub fn server_time_zone() -> String {
    std::env::var("TZ").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "UTC".to_string())
}

/// A short, stable fingerprint of a locale's client messages, so the SPA can
/// skip re-downloading them (`X-Modlight-Messages: <locale>:<hash>`).
pub fn messages_hash(messages: &Value) -> String {
    let text = serde_json::to_string(messages).unwrap_or_default();
    hex::encode(&Sha256::digest(text.as_bytes())[..8])
}

/// What the root layout computed, typed (also used to theme the shell).
#[derive(Debug, Clone)]
pub struct RootLayout {
    pub locale: Locale,
    pub mode: &'static str,
    pub theme: &'static str,
}

pub async fn compute(ctx: &PageCtx) -> RootLayout {
    let locale = ctx.locale();
    let cookies = ctx.cookies();
    let mode = resolve_mode(cookies.get(MODE_COOKIE).map(String::as_str));
    let session = ctx.session.get_or_none().await;
    let skin_source = session.as_ref().and_then(|s| s.user.theme.clone()).or_else(|| cookies.get(SKIN_COOKIE).cloned());
    let theme = resolve_theme(skin_source.as_deref());
    RootLayout { locale, mode, theme }
}

/// The `LayoutLoader` for [`crate::pages::table::Layout::Root`].
pub async fn load(ctx: PageCtx) -> LayoutResult {
    let root = compute(&ctx).await;
    // Not upstream: the optional Cloudflare Turnstile site key (read before
    // the response's CSP is applied, which follows the same snapshot).
    let turnstile = ctx.state.turnstile.current(&ctx.state.db, &ctx.state.config).await;
    let mut config = public_config(&ctx.state.config);
    if let Value::Object(map) = &mut config {
        map.insert("turnstileSiteKey".into(), json!(turnstile.public_site_key()));
    }
    let messages = ctx.state.messages.client_messages(root.locale);
    let hash = messages_hash(&messages);
    let client_has = ctx
        .headers
        .get("x-modlight-messages")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("{}:{hash}", root.locale.as_str()));
    Ok(json!({
        "locale": root.locale.as_str(),
        "messages": if client_has { Value::Null } else { messages },
        "messagesHash": hash,
        "timeZone": server_time_zone(),
        "mode": root.mode,
        "theme": root.theme,
        "config": config,
    }))
}

/// `PublicConfig` (CONTRACT 5.7): never any other config field.
pub fn public_config(config: &crate::config::Config) -> Value {
    json!({
        "appName": config.app_name,
        "embedBrand": config.embed_brand,
        "appUrl": config.app_url,
        "appDomains": config.app_domains,
        "shortUrl": config.short_url,
    })
}

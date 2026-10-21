//! Shared application state handed to every handler, loader and action.
//!
//! Cheap to clone: everything inside is reference counted. Feature modules
//! that need process-wide state of their own (an in-memory rate limiter, a
//! mail settings snapshot, the tus store) keep it in their module behind a
//! `OnceLock`/`Mutex` the way upstream keeps module-level singletons, or add a
//! field here when several modules must share it.

use std::sync::Arc;

use sqlx::PgPool;

use crate::auth::cookies::AuthCookies;
use crate::config::Config;
use crate::http::headers::SecurityHeaders;
use crate::http::proxy::Proxy;
use crate::http::static_files::StaticFiles;
use crate::i18n::Messages;
use crate::pages::PageRegistry;
use crate::pages::shell::SpaShell;
use crate::paths::Paths;
use crate::rpc::RpcRegistry;
use crate::storage::Storage;
use crate::turnstile::Turnstile;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub paths: Arc<Paths>,
    pub db: PgPool,
    /// Translations of every locale, loaded once at startup.
    pub messages: Arc<Messages>,
    /// The configured storage driver, or why it could not be built.
    pub storage: Storage,
    /// Cookie names and attributes Better Auth would use for this APP_URL.
    pub auth_cookies: Arc<AuthCookies>,
    /// proxy.ts (shortener host, dashboard gate).
    pub proxy: Arc<Proxy>,
    /// next.config.ts header rules (CSP computed from the environment).
    pub security: Arc<SecurityHeaders>,
    /// public/ files and the SPA's assets directory.
    pub statics: Arc<StaticFiles>,
    pub rpc: Arc<RpcRegistry>,
    pub pages: Arc<PageRegistry>,
    pub shell: Arc<SpaShell>,
    /// Not upstream: the optional Cloudflare Turnstile check (its settings
    /// snapshot, HTTPS client and siteverify endpoint).
    pub turnstile: Arc<Turnstile>,
}

impl AppState {
    /// Assemble the state. `db` may be a lazy pool (nothing connects until the
    /// first query), which is what tests without a database use.
    pub fn new(config: Config, paths: Paths, db: PgPool) -> anyhow::Result<AppState> {
        let messages = Messages::load(&paths.messages)?;
        let storage = Storage::from_env(&config);
        let auth_cookies = AuthCookies::for_config(&config);
        let proxy = Proxy::new(config.raw_shortlink_url.as_deref());
        let security = SecurityHeaders::new(&config.csp_origins, !config.is_prod);
        let statics = StaticFiles::scan(&paths.public, &paths.web_dist);
        let mut rpc = RpcRegistry::default();
        crate::actions::register_all(&mut rpc);
        let mut pages = PageRegistry::default();
        crate::pages::register_all(&mut pages);
        let shell = SpaShell::new(paths.web_dist.join("index.html"));
        Ok(AppState {
            config: Arc::new(config),
            paths: Arc::new(paths),
            db,
            messages: Arc::new(messages),
            storage,
            auth_cookies: Arc::new(auth_cookies),
            proxy: Arc::new(proxy),
            security: Arc::new(security),
            statics: Arc::new(statics),
            rpc: Arc::new(rpc),
            pages: Arc::new(pages),
            shell: Arc::new(shell),
            turnstile: Arc::new(Turnstile::new()),
        })
    }
}

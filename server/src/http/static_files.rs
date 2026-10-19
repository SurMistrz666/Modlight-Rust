//! Static files: upstream `public/` (plus `app/icon.svg` and
//! `app/apple-icon.png`, which Next served at the root) and the SPA build's
//! `web/dist/assets/` (the `/_next/static` of this port).
//!
//! Like Next, the set of public files is taken once at startup and each file is
//! a route of its own, so it outranks the dynamic page routes (`/brand/x.png`
//! is a file, not `/{userCode}/{mediaCode}`).
//!
//! Cache headers as upstream: `public, max-age=0` for public files,
//! `public, max-age=0, must-revalidate` for the two app icons; the SPA's hashed
//! assets get `public, max-age=31536000, immutable` (what Next sent for
//! `/_next/static`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use axum::Router;
use axum::http::{HeaderValue, Response, header};
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

pub const PUBLIC_CACHE: &str = "public, max-age=0";
pub const ICON_CACHE: &str = "public, max-age=0, must-revalidate";
pub const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// The public files of this process, by URL path.
#[derive(Debug, Clone, Default)]
pub struct StaticFiles {
    pub public_dir: PathBuf,
    pub assets_dir: PathBuf,
    /// URL paths (`/brand/sharex.png`) of every file under `public/`.
    pub files: BTreeSet<String>,
}

fn walk(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            let url: Vec<String> = rel
                .components()
                .map(|c| crate::http::cookies::encode_uri_component(&c.as_os_str().to_string_lossy()))
                .collect();
            out.insert(format!("/{}", url.join("/")));
        }
    }
}

impl StaticFiles {
    pub fn scan(public_dir: &Path, web_dist: &Path) -> StaticFiles {
        let mut files = BTreeSet::new();
        walk(public_dir, public_dir, &mut files);
        StaticFiles { public_dir: public_dir.to_path_buf(), assets_dir: web_dist.join("assets"), files }
    }
}

/// Is `path` a static file (so `/_page` should tell the SPA to load it)?
pub fn is_static_path(state: &AppState, path: &str) -> bool {
    state.statics.files.contains(path) || path.starts_with("/assets/")
}

fn cache_layer<B>(value: &'static str) -> impl Fn(Response<B>) -> Response<B> + Clone {
    move |mut res: Response<B>| {
        if res.status().is_success() || res.status() == axum::http::StatusCode::NOT_MODIFIED {
            res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
        }
        res
    }
}

/// One route per public file, plus `/assets/*`.
pub fn routes(statics: &StaticFiles) -> Router<AppState> {
    let mut router = Router::new();
    let reserved: BTreeSet<&str> =
        crate::pages::table::APP_ROUTES.iter().map(|r| r.path).filter(|p| !p.contains('[')).chain(["/_page"]).collect();
    for url in &statics.files {
        if reserved.contains(url.as_str()) || url.contains(['{', '}', '*']) {
            tracing::warn!("public file {url} is shadowed by a route and will not be served");
            continue;
        }
        let rel: PathBuf = url
            .trim_start_matches('/')
            .split('/')
            .map(|s| crate::http::cookies::decode_uri_component(s).unwrap_or_else(|| s.to_string()))
            .collect();
        let file = statics.public_dir.join(rel);
        let cache = if url == "/icon.svg" || url == "/apple-icon.png" { ICON_CACHE } else { PUBLIC_CACHE };
        router = router.route_service(
            url,
            tower::ServiceBuilder::new().map_response(cache_layer(cache)).service(ServeFile::new(file)),
        );
    }
    router.nest_service(
        "/assets",
        tower::ServiceBuilder::new()
            .map_response(cache_layer(IMMUTABLE_CACHE))
            .service(ServeDir::new(&statics.assets_dir)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    #[test]
    fn scans_the_shipped_public_directory() {
        let p = Paths::crate_dir();
        let s = StaticFiles::scan(&p.public, &p.web_dist);
        assert!(s.files.contains("/brand/sharex.png"));
        assert!(s.files.contains("/fonts/Geist-Bold.ttf"));
        assert!(s.files.contains("/icon.svg"));
        assert!(s.files.contains("/apple-icon.png"));
    }
}

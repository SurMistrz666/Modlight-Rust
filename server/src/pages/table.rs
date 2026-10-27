//! The upstream `app/` tree as data: EVERY page (`page.tsx`) and route handler
//! (`route.ts`), with route groups stripped and `[params]` kept (plus the one
//! page that is not upstream's: `/dashboard/downloads`, Downloads), plus Next's
//! route matching (`getSortedRoutes` order, first full match wins, static
//! segments before `[slug]` before `[...rest]`, compared case-sensitively on
//! the raw path, params decoded with `decodeURIComponent`).
//!
//! Pages are served by the page-data framework; route handlers by the feature
//! modules' axum routers. Both are listed so that `/_page` and the not-found
//! fallback can tell "a page", "a route handler (maybe not ported yet)" and
//! "nothing" apart exactly the way Next would resolve the URL.

use indexmap::IndexMap;
use serde::Serialize;

use crate::http::cookies::decode_uri_component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    Page,
    Handler,
}

/// A layout on the path of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layout {
    /// `app/layout.tsx`
    Root,
    /// `app/(app)/layout.tsx`
    App,
    /// `app/(auth)/layout.tsx`
    Auth,
}

impl Layout {
    /// The key under `layouts` in a page-data response.
    pub fn key(self) -> &'static str {
        match self {
            Layout::Root => "root",
            Layout::App => "(app)",
            Layout::Auth => "(auth)",
        }
    }

    pub fn file(self) -> &'static str {
        match self {
            Layout::Root => "app/layout.tsx",
            Layout::App => "app/(app)/layout.tsx",
            Layout::Auth => "app/(auth)/layout.tsx",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppRoute {
    /// Next app path, e.g. `/[userCode]/[mediaCode]`.
    pub path: &'static str,
    /// Upstream file that implements it.
    pub file: &'static str,
    pub kind: RouteKind,
    /// Layouts from the outside in (pages only).
    pub layouts: &'static [Layout],
}

const ROOT: &[Layout] = &[Layout::Root];
const APP: &[Layout] = &[Layout::Root, Layout::App];
const AUTH: &[Layout] = &[Layout::Root, Layout::Auth];

const fn page(path: &'static str, file: &'static str, layouts: &'static [Layout]) -> AppRoute {
    AppRoute { path, file, kind: RouteKind::Page, layouts }
}

const fn handler(path: &'static str, file: &'static str) -> AppRoute {
    AppRoute { path, file, kind: RouteKind::Handler, layouts: &[] }
}

/// Every page and route handler under upstream `app/`.
pub const APP_ROUTES: &[AppRoute] = &[
    // ── pages ──
    page("/", "app/page.tsx", ROOT),
    page("/welcome", "app/welcome/page.tsx", ROOT),
    page("/download", "app/download/page.tsx", ROOT),
    page("/u/[userCode]", "app/u/[userCode]/page.tsx", ROOT),
    page("/a/[userCode]/[slug]", "app/a/[userCode]/[slug]/page.tsx", ROOT),
    page("/s/[code]/unlock", "app/s/[code]/unlock/page.tsx", ROOT),
    page("/s/[code]/[name]/unlock", "app/s/[code]/[name]/unlock/page.tsx", ROOT),
    page("/[userCode]", "app/[userCode]/page.tsx", ROOT),
    page("/[userCode]/[mediaCode]", "app/[userCode]/[mediaCode]/page.tsx", ROOT),
    page("/dashboard", "app/(app)/dashboard/page.tsx", APP),
    page("/dashboard/admin", "app/(app)/dashboard/admin/page.tsx", APP),
    page("/dashboard/admin/users/[id]/files", "app/(app)/dashboard/admin/users/[id]/files/page.tsx", APP),
    // Not upstream: Downloads (src/pages/dashboard_downloads.rs; the file is the SPA's own page).
    page("/dashboard/downloads", "app/(app)/dashboard/downloads/page.tsx", APP),
    page("/dashboard/duplicates", "app/(app)/dashboard/duplicates/page.tsx", APP),
    page("/dashboard/folders", "app/(app)/dashboard/folders/page.tsx", APP),
    page("/dashboard/links", "app/(app)/dashboard/links/page.tsx", APP),
    page("/dashboard/paste", "app/(app)/dashboard/paste/page.tsx", APP),
    page("/dashboard/profile", "app/(app)/dashboard/profile/page.tsx", APP),
    page("/dashboard/tokens", "app/(app)/dashboard/tokens/page.tsx", APP),
    page("/dashboard/trash", "app/(app)/dashboard/trash/page.tsx", APP),
    page("/dashboard/upload", "app/(app)/dashboard/upload/page.tsx", APP),
    page("/2fa", "app/(auth)/2fa/page.tsx", AUTH),
    page("/authorize", "app/(auth)/authorize/page.tsx", AUTH),
    page("/forgot", "app/(auth)/forgot/page.tsx", AUTH),
    page("/invite/[token]", "app/(auth)/invite/[token]/page.tsx", AUTH),
    page("/login", "app/(auth)/login/page.tsx", AUTH),
    page("/register", "app/(auth)/register/page.tsx", AUTH),
    page("/reset", "app/(auth)/reset/page.tsx", AUTH),
    // ── route handlers ──
    handler("/[userCode]/[mediaCode]/[action]", "app/[userCode]/[mediaCode]/[action]/route.ts"),
    handler("/[userCode]/[mediaCode]/delete/[token]", "app/[userCode]/[mediaCode]/delete/[token]/route.ts"),
    handler("/a/[userCode]/[slug]/zip", "app/a/[userCode]/[slug]/zip/route.ts"),
    handler("/api/admin/disk", "app/api/admin/disk/route.ts"),
    handler("/api/auth/[...all]", "app/api/auth/[...all]/route.ts"),
    handler("/api/avatar", "app/api/avatar/route.ts"),
    handler("/api/background", "app/api/background/route.ts"),
    handler("/api/client-settings", "app/api/client-settings/route.ts"),
    handler("/api/duplicates", "app/api/duplicates/route.ts"),
    handler("/api/folders", "app/api/folders/route.ts"),
    handler("/api/health", "app/api/health/route.ts"),
    handler("/api/link-info", "app/api/link-info/route.ts"),
    handler("/api/me", "app/api/me/route.ts"),
    handler("/api/releases/app", "app/api/releases/app/route.ts"),
    handler("/api/shortlinks", "app/api/shortlinks/route.ts"),
    handler("/api/theme", "app/api/theme/route.ts"),
    handler("/api/uploads", "app/api/uploads/route.ts"),
    handler("/api/uploads/check", "app/api/uploads/check/route.ts"),
    handler("/avatar/[userCode]", "app/avatar/[userCode]/route.ts"),
    handler("/bg/[userCode]", "app/bg/[userCode]/route.ts"),
    handler("/download/app", "app/download/app/route.ts"),
    handler("/embed/[version]/[card]", "app/embed/[version]/[card]/route.ts"),
    handler("/oembed", "app/oembed/route.ts"),
    handler("/s/[code]", "app/s/[code]/route.ts"),
    handler("/s/[code]/[name]", "app/s/[code]/[name]/route.ts"),
];

/// A page route by its app path.
pub fn find_route(path: &str) -> Option<&'static AppRoute> {
    APP_ROUTES.iter().find(|r| r.path == path)
}

/// `/[userCode]/[mediaCode]` -> `/{userCode}/{mediaCode}`, `[...all]` -> `{*all}`.
pub fn axum_path(next_path: &str) -> String {
    if next_path == "/" {
        return "/".to_string();
    }
    next_path
        .split('/')
        .map(|seg| {
            if let Some(name) = seg.strip_prefix("[...").and_then(|s| s.strip_suffix(']')) {
                format!("{{*{name}}}")
            } else if let Some(name) = seg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                format!("{{{name}}}")
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A param value: one segment, or the segments of a catch-all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ParamValue {
    One(String),
    Many(Vec<String>),
}

impl ParamValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ParamValue::One(s) => Some(s),
            ParamValue::Many(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    pub route: &'static AppRoute,
    pub params: IndexMap<String, ParamValue>,
}

/// A dynamic segment did not survive `decodeURIComponent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedPath;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Static(&'static str),
    Param(&'static str),
    CatchAll(&'static str),
}

fn segments(path: &'static str) -> Vec<Seg> {
    if path == "/" {
        return Vec::new();
    }
    path[1..]
        .split('/')
        .map(|s| {
            if let Some(n) = s.strip_prefix("[...").and_then(|x| x.strip_suffix(']')) {
                Seg::CatchAll(n)
            } else if let Some(n) = s.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
                Seg::Param(n)
            } else {
                Seg::Static(s)
            }
        })
        .collect()
}

/// Next's `getSortedRoutes`: at every level a node's own route first, then
/// static children alphabetically, then `[slug]`, then `[...rest]`.
pub fn sorted_routes() -> Vec<&'static AppRoute> {
    #[derive(Default)]
    struct Node {
        own: Option<&'static AppRoute>,
        statics: std::collections::BTreeMap<&'static str, Node>,
        slug: Option<Box<Node>>,
        rest: Option<Box<Node>>,
    }
    fn insert(node: &mut Node, segs: &[Seg], route: &'static AppRoute) {
        match segs.split_first() {
            None => node.own = Some(route),
            Some((Seg::Static(s), tail)) => insert(node.statics.entry(s).or_default(), tail, route),
            Some((Seg::Param(_), tail)) => insert(node.slug.get_or_insert_with(Default::default), tail, route),
            Some((Seg::CatchAll(_), tail)) => insert(node.rest.get_or_insert_with(Default::default), tail, route),
        }
    }
    fn walk(node: &Node, out: &mut Vec<&'static AppRoute>) {
        if let Some(r) = node.own {
            out.push(r);
        }
        for child in node.statics.values() {
            walk(child, out);
        }
        if let Some(s) = &node.slug {
            walk(s, out);
        }
        if let Some(r) = &node.rest {
            walk(r, out);
        }
    }
    let mut root = Node::default();
    for route in APP_ROUTES {
        insert(&mut root, &segments(route.path), route);
    }
    let mut out = Vec::with_capacity(APP_ROUTES.len());
    walk(&root, &mut out);
    out
}

fn try_match(route: &'static AppRoute, raw: &[&str]) -> Option<Result<RouteMatch, MalformedPath>> {
    let segs = segments(route.path);
    let mut params = IndexMap::new();
    let mut i = 0;
    for (n, seg) in segs.iter().enumerate() {
        match seg {
            Seg::Static(s) => {
                if raw.get(i) != Some(s) {
                    return None;
                }
                i += 1;
            }
            Seg::Param(name) => {
                let v = raw.get(i)?;
                if v.is_empty() {
                    return None;
                }
                let Some(decoded) = decode_uri_component(v) else {
                    return Some(Err(MalformedPath));
                };
                params.insert((*name).to_string(), ParamValue::One(decoded));
                i += 1;
            }
            Seg::CatchAll(name) => {
                debug_assert_eq!(n, segs.len() - 1);
                let rest = &raw[i.min(raw.len())..];
                if rest.is_empty() {
                    return None;
                }
                let mut values = Vec::new();
                for v in rest {
                    let Some(decoded) = decode_uri_component(v) else {
                        return Some(Err(MalformedPath));
                    };
                    values.push(decoded);
                }
                params.insert((*name).to_string(), ParamValue::Many(values));
                i = raw.len();
            }
        }
    }
    if i != raw.len() {
        return None;
    }
    Some(Ok(RouteMatch { route, params }))
}

/// Resolve a raw (still percent-encoded) pathname the way Next would.
pub fn match_path(raw_path: &str) -> Result<Option<RouteMatch>, MalformedPath> {
    let trimmed = raw_path.strip_prefix('/').unwrap_or(raw_path);
    let raw: Vec<&str> = if trimmed.is_empty() { Vec::new() } else { trimmed.split('/').collect() };
    for route in sorted_routes() {
        if let Some(result) = try_match(route, &raw) {
            return result.map(Some);
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(path: &str) -> Option<(&'static str, Vec<(String, ParamValue)>)> {
        match_path(path).unwrap().map(|m| (m.route.path, m.params.into_iter().collect()))
    }

    fn one(s: &str) -> ParamValue {
        ParamValue::One(s.to_string())
    }

    #[test]
    fn the_table_has_every_upstream_page_and_route() {
        let pages = APP_ROUTES.iter().filter(|r| r.kind == RouteKind::Page).count();
        let handlers = APP_ROUTES.iter().filter(|r| r.kind == RouteKind::Handler).count();
        // 27 upstream pages and Downloads.
        assert_eq!(pages, 28);
        assert_eq!(handlers, 25);
    }

    #[test]
    fn static_routes_win_over_dynamic_ones() {
        assert_eq!(resolve("/login").unwrap().0, "/login");
        assert_eq!(resolve("/download").unwrap().0, "/download");
        assert_eq!(resolve("/download/app").unwrap().0, "/download/app");
        assert_eq!(resolve("/api/health").unwrap().0, "/api/health");
        assert_eq!(resolve("/u/alice").unwrap(), ("/u/[userCode]", vec![("userCode".into(), one("alice"))]));
    }

    #[test]
    fn falls_back_across_levels_like_next() {
        // /a/x has no 2-segment route under "a", so it is a media page.
        assert_eq!(resolve("/a/x").unwrap().0, "/[userCode]/[mediaCode]");
        assert_eq!(resolve("/a/x/y").unwrap().0, "/a/[userCode]/[slug]");
        assert_eq!(resolve("/a/x/y/zip").unwrap().0, "/a/[userCode]/[slug]/zip");
        assert_eq!(resolve("/a/x/delete/tok").unwrap().0, "/[userCode]/[mediaCode]/delete/[token]");
        assert_eq!(resolve("/api/foo").unwrap().0, "/[userCode]/[mediaCode]");
        assert_eq!(resolve("/api/foo/bar").unwrap().0, "/[userCode]/[mediaCode]/[action]");
        assert_eq!(resolve("/dashboard/nope").unwrap().0, "/[userCode]/[mediaCode]");
        assert_eq!(resolve("/dashboard/downloads").unwrap().0, "/dashboard/downloads");
        assert_eq!(resolve("/s/promo").unwrap().0, "/s/[code]");
        assert_eq!(resolve("/s/promo/unlock").unwrap().0, "/s/[code]/unlock");
        assert_eq!(resolve("/s/tag/promo").unwrap().0, "/s/[code]/[name]");
        assert_eq!(resolve("/s/tag/promo/unlock").unwrap().0, "/s/[code]/[name]/unlock");
        assert_eq!(resolve("/x/y/z/w"), None);
    }

    #[test]
    fn catch_all_and_decoding() {
        assert_eq!(
            resolve("/api/auth/two-factor/verify-totp").unwrap(),
            (
                "/api/auth/[...all]",
                vec![("all".into(), ParamValue::Many(vec!["two-factor".into(), "verify-totp".into()]))]
            )
        );
        assert_eq!(resolve("/a%20b").unwrap(), ("/[userCode]", vec![("userCode".into(), one("a b"))]));
        assert_eq!(match_path("/a%ZZ/b"), Err(MalformedPath));
        // Static segments compare raw and case-sensitively.
        assert_eq!(resolve("/Login").unwrap().0, "/[userCode]");
        assert_eq!(resolve("/").unwrap().0, "/");
    }

    #[test]
    fn axum_paths() {
        assert_eq!(axum_path("/[userCode]/[mediaCode]"), "/{userCode}/{mediaCode}");
        assert_eq!(axum_path("/api/auth/[...all]"), "/api/auth/{*all}");
        assert_eq!(axum_path("/"), "/");
    }
}

//! The panel shell's server side: the `(app)` layout gate and the admin page
//! gate (CONTRACT 5.2, 5.8, 6.20), and `tests/unit/panel-reachability.test.ts`
//! (every panel screen is linked from the shell).
//!
//! The data these loaders compute needs a database; the parity script
//! `tests/parity/panel.py` compares it with the reference server.

mod common;

use axum::body::Body;
use axum::http::Request;
use common::{call, state};
use modlight_server::http::App;
use modlight_server::pages::table::{APP_ROUTES, RouteKind};
use serde_json::json;

async fn get_with_cookie(app: &App, uri: &str, cookie: &str) -> common::Answer {
    let req = Request::get(uri).header("host", "127.0.0.1:8421").header("cookie", cookie).body(Body::empty()).unwrap();
    call(app, req).await
}

#[tokio::test]
async fn the_app_layout_sends_an_unknown_session_to_login_without_next() {
    let app = App::new(state());
    // The proxy gate only looks for a cookie; the layout's requireUser() finds
    // no session behind a garbage one and redirects to /login (no `next`).
    let page = get_with_cookie(&app, "/_page?path=%2Fdashboard%2Fadmin", "better-auth.session_token=garbage").await;
    assert_eq!(page.json(), json!({ "redirect": "/login", "status": 307 }));
    let doc = get_with_cookie(&app, "/dashboard/admin", "better-auth.session_token=garbage").await;
    assert_eq!(doc.status.as_u16(), 307);
    assert_eq!(doc.header("location").as_deref(), Some("/login"));
    // Every dashboard page goes through the same layout.
    let page = get_with_cookie(&app, "/_page?path=%2Fdashboard", "better-auth.session_token=garbage").await;
    assert_eq!(page.json(), json!({ "redirect": "/login", "status": 307 }));
}

#[tokio::test]
async fn without_any_session_cookie_the_proxy_gate_answers_first() {
    let app = App::new(state());
    let page = common::get(&app, "/_page?path=%2Fdashboard%2Fadmin").await;
    assert_eq!(page.json(), json!({ "redirect": "/login?next=%2Fdashboard%2Fadmin", "status": 307 }));
}

/// The top-level panel screens: static `/dashboard/<x>` pages (dynamic
/// segments are reached from a row or a card, never from the rail).
fn panel_screens() -> Vec<&'static str> {
    APP_ROUTES
        .iter()
        .filter(|r| r.kind == RouteKind::Page)
        .filter_map(|r| r.path.strip_prefix("/dashboard/"))
        .filter(|rest| !rest.contains('/') && !rest.starts_with('['))
        .collect()
}

// tests/unit/panel-reachability.test.ts: "every panel screen is reachable
// from the shell", over the SPA's components/app-shell.tsx and the Rust route
// table. The shell is a port since the Graphite layout
// (web/src/overrides/components/app-shell.tsx); the upstream copy is the
// fallback should the port ever go.
#[test]
fn every_panel_screen_is_reachable_from_the_shell() {
    let web = concat!(env!("CARGO_MANIFEST_DIR"), "/web");
    let shell = std::fs::read_to_string(format!("{web}/src/overrides/components/app-shell.tsx"))
        .or_else(|_| std::fs::read_to_string(format!("{web}/components/app-shell.tsx")))
        .expect("the SPA has components/app-shell.tsx (its port or upstream's copy)");
    let screens = panel_screens();
    assert!(screens.len() > 5, "{screens:?}");
    assert!(shell.contains("nav"));
    for screen in screens {
        // An `href`, specifically, in either of the two shapes the shell uses:
        // a JSX attribute or a field in one of the nav-item tables.
        let attribute = format!("href=\"/dashboard/{screen}\"");
        let field = format!("href: \"/dashboard/{screen}\"");
        assert!(shell.contains(&attribute) || shell.contains(&field), "{attribute}");
        // TITLE_ROUTES is the only thing that says where you are on a phone.
        assert!(shell.contains(&format!("prefix: \"/dashboard/{screen}\"")), "/dashboard/{screen} in TITLE_ROUTES");
    }
}

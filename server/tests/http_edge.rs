//! The HTTP edge end to end (no database): server.ts raw paths, Next's path
//! normalisation, proxy.ts, next.config.ts headers, static files, method
//! handling and the not-ported placeholders.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{call, get, state, state_with};
use modlight_server::http::App;

#[tokio::test]
async fn repeated_and_trailing_slashes_are_308s_without_config_headers() {
    let app = App::new(state());
    let a = get(&app, "/login/?a=1").await;
    assert_eq!(a.status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(a.header("location").as_deref(), Some("/login?a=1"));
    assert_eq!(a.header("refresh").as_deref(), Some("0;url=/login?a=1"));
    assert_eq!(a.text(), "/login?a=1");
    assert!(a.header("content-security-policy").is_none());
    let b = get(&app, "//a//b").await;
    assert_eq!(b.header("location").as_deref(), Some("/a/b"));
}

#[tokio::test]
async fn dashboard_gate_redirects_without_a_session_cookie() {
    let app = App::new(state());
    let a = get(&app, "/dashboard/tokens").await;
    assert_eq!(a.status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(a.header("location").as_deref(), Some("/login?next=%2Fdashboard%2Ftokens"));
    assert_eq!(a.text(), "/login?next=%2Fdashboard%2Ftokens");
    assert!(a.header("content-security-policy").unwrap().starts_with("default-src 'self'"));
    assert_eq!(a.header("x-frame-options").as_deref(), Some("SAMEORIGIN"));
}

#[tokio::test]
async fn csp_follows_the_path_rules() {
    let app = App::new(state());
    let authorize = get(&app, "/authorize").await;
    assert!(authorize.header("content-security-policy").unwrap().ends_with("http://127.0.0.1:*"));
    let raw = get(&app, "/tag/code/raw.png").await;
    assert!(raw.header("content-security-policy").unwrap().ends_with("sandbox"));
    let nothing = get(&app, "/authorize/x").await;
    assert!(nothing.header("content-security-policy").is_none());
    // Production CSP: no 'unsafe-eval', origins from APP_URL.
    let login = get(&app, "/login").await;
    let csp = login.header("content-security-policy").unwrap();
    assert!(!csp.contains("unsafe-eval"));
    assert!(csp.ends_with("connect-src 'self' http://127.0.0.1:8421"));
}

#[tokio::test]
async fn raw_upload_paths_bypass_everything() {
    let app = App::new(state());
    for (method, uri) in [(Method::POST, "/upload"), (Method::POST, "/api/upload"), (Method::PATCH, "/api/files/abc")] {
        let a = call(&app, Request::builder().method(method).uri(uri).body(Body::empty()).unwrap()).await;
        // Answered by the ShareX and tus ports themselves, without the edge.
        assert_ne!(a.status, StatusCode::NOT_IMPLEMENTED, "{uri}");
        assert!(a.header("content-security-policy").is_none(), "{uri}");
        assert!(a.header("x-content-type-options").is_none(), "{uri}");
    }
    // GET /upload is not the ShareX path: it is a page URL (tag "upload").
    let page = get(&app, "/upload").await;
    assert!(page.header("content-security-policy").is_some());
}

#[tokio::test]
async fn shortener_host_rewrites_to_s() {
    let mut state = state_with(&[("APP_URL", "https://host.example.com"), ("SHORTLINK_URL", "https://s.example.com")]);
    // Give up on the unreachable database at once rather than after the
    // pool's 10 s acquire timeout.
    state.db = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgresql://nobody@127.0.0.1:1/none")
        .unwrap();
    let app = App::new(state);
    let req = Request::get("/promo").header("host", "s.example.com").body(Body::empty()).unwrap();
    let a = call(&app, req).await;
    // The rewrite reached the /s/[code] handler (not the 501 placeholder):
    // it looks the code up, and this state's database is unreachable, so the
    // lookup fails as a 500. The redirect itself is checked against a real
    // database in tests/library_routes_db.rs.
    assert_eq!(a.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(a.json(), serde_json::json!({ "message": "Internal server error" }));
    // The same path on the main host is a tagless media page.
    let req = Request::get("/promo").header("host", "host.example.com").body(Body::empty()).unwrap();
    let b = call(&app, req).await;
    assert!(b.header("content-type").unwrap().starts_with("text/html"));
}

#[tokio::test]
async fn static_files_and_their_cache_headers() {
    let app = App::new(state());
    let png = get(&app, "/brand/sharex.png").await;
    assert_eq!(png.status, StatusCode::OK);
    assert_eq!(png.header("cache-control").as_deref(), Some("public, max-age=0"));
    assert_eq!(png.header("content-type").as_deref(), Some("image/png"));
    let icon = get(&app, "/icon.svg").await;
    assert_eq!(icon.header("cache-control").as_deref(), Some("public, max-age=0, must-revalidate"));
}

#[tokio::test]
async fn methods_like_next_route_handlers() {
    let app = App::new(state());
    let options =
        call(&app, Request::builder().method(Method::OPTIONS).uri("/_rpc/a/b").body(Body::empty()).unwrap()).await;
    assert_eq!(options.status, StatusCode::NO_CONTENT);
    assert_eq!(options.header("allow").as_deref(), Some("OPTIONS, POST"));
    // Next sorts the list alphabetically (auto-implement-methods).
    let folders =
        call(&app, Request::builder().method(Method::OPTIONS).uri("/api/folders").body(Body::empty()).unwrap()).await;
    assert_eq!(folders.header("allow").as_deref(), Some("DELETE, GET, HEAD, OPTIONS, PATCH, POST"));
    let wrong = get(&app, "/_rpc/a/b").await;
    assert_eq!(wrong.status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(wrong.header("allow").is_none());
}

#[tokio::test]
async fn unported_handlers_and_malformed_paths() {
    let app = App::new(state());
    // Every upstream route handler is ported now; the placeholder still names
    // the upstream file for one that is not registered.
    let route = modlight_server::pages::table::find_route("/a/[userCode]/[slug]/zip").unwrap();
    let placeholder = modlight_server::pages::not_ported_handler(route);
    assert_eq!(placeholder.status(), StatusCode::NOT_IMPLEMENTED);
    let bad = get(&app, "/a%ZZ/b").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let missing = get(&app, "/no/such/page/here").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.text().contains("<meta name=\"robots\" content=\"noindex\"/>"));
    assert!(missing.text().contains("{\"notFound\":true"));
}

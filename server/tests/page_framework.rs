//! The page-data framework end to end with test loaders (PORT.md section 8):
//! `/_page` envelopes, layout order, the SPA shell, metadata and statuses.

mod common;

use std::sync::Arc;

use common::{get, state};
use modlight_server::http::App;
use modlight_server::pages::meta::{OgMedia, OpenGraph, PageMeta, Title};
use modlight_server::pages::{Layout, PageCtx, PageData, PageError, PageRegistry, PageResult, root_layout};
use serde_json::{Value, json};

async fn gallery(ctx: PageCtx) -> PageResult {
    let tag = ctx.param("userCode").unwrap_or_default().to_string();
    if tag == "nobody" {
        return Err(PageError::NotFound);
    }
    let meta = PageMeta {
        title: Title::Templated(format!("{tag}'s files")),
        open_graph: Some(OpenGraph {
            og_type: Some("profile".into()),
            title: Some(format!("{tag}'s files")),
            images: vec![OgMedia {
                url: "https://x.test/raw?w=1200".into(),
                props: vec![("width".into(), "1200".into())],
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    PageData::new(&json!({ "owner": tag, "q": ctx.search.get("q"), "sp": ctx.search.to_value() }), meta)
}

async fn download(_ctx: PageCtx) -> PageResult {
    Err(PageError::redirect("/welcome"))
}

async fn auth_layout(_ctx: PageCtx) -> Result<Value, PageError> {
    Ok(json!({ "year": 2026 }))
}

async fn login(ctx: PageCtx) -> PageResult {
    Ok(PageData {
        props: json!({ "next": ctx.search.get("next").unwrap_or("/dashboard") }),
        meta: PageMeta::titled("Sign in"),
        skin: None,
    })
}

async fn app_layout(_ctx: PageCtx) -> Result<Value, PageError> {
    Ok(json!({ "theme": null }))
}

async fn admin_page(_ctx: PageCtx) -> PageResult {
    Err(PageError::redirect("/dashboard"))
}

async fn folders_page(_ctx: PageCtx) -> PageResult {
    Err(PageError::NotFound)
}

fn app() -> App {
    let mut state = state();
    let mut reg = PageRegistry::default();
    reg.layout(Layout::Root, root_layout::load);
    reg.layout(Layout::Auth, auth_layout);
    reg.layout(Layout::App, app_layout);
    reg.page("/u/[userCode]", gallery);
    reg.page("/download", download);
    reg.page("/login", login);
    reg.page("/dashboard/admin", admin_page);
    reg.page("/dashboard/folders", folders_page);
    state.pages = Arc::new(reg);
    App::new(state)
}

#[tokio::test]
async fn page_data_carries_route_params_layouts_props_and_meta() {
    let app = app();
    let a = get(&app, "/_page?path=%2Fu%2Falice&q=1&q=2").await;
    assert_eq!(a.status.as_u16(), 200);
    assert_eq!(a.header("cache-control").as_deref(), Some("private, no-cache, no-store, max-age=0, must-revalidate"));
    let d = a.json();
    assert_eq!(d["route"], "/u/[userCode]");
    assert_eq!(d["params"], json!({ "userCode": "alice" }));
    assert_eq!(d["props"], json!({ "owner": "alice", "q": "1", "sp": { "q": ["1", "2"] } }));
    assert_eq!(d["meta"]["title"], "alice's files - Modlight");
    let root = &d["layouts"]["root"];
    assert_eq!(root["locale"], "en");
    assert_eq!(root["theme"], "discord");
    assert_eq!(root["mode"], "system");
    assert!(root["messages"]["common"].is_object());
    assert_eq!(d["layouts"].as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn layouts_run_outside_in_and_auth_pages_get_theirs() {
    let app = app();
    let d = get(&app, "/_page?path=/login&next=%2Fdashboard%2Ftokens").await.json();
    let keys: Vec<&String> = d["layouts"].as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["root", "(auth)"]);
    assert_eq!(d["props"], json!({ "next": "/dashboard/tokens" }));
}

#[tokio::test]
async fn messages_are_skipped_when_the_spa_already_has_them() {
    let app = app();
    let first = get(&app, "/_page?path=/login").await.json();
    let hash = first["layouts"]["root"]["messagesHash"].as_str().unwrap().to_string();
    let req = axum::http::Request::get("/_page?path=/login")
        .header("host", "127.0.0.1:8421")
        .header("x-modlight-messages", format!("en:{hash}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let again = common::call(&app, req).await.json();
    assert!(again["layouts"]["root"]["messages"].is_null());
}

#[tokio::test]
async fn redirects_and_not_found() {
    let app = app();
    assert_eq!(get(&app, "/_page?path=/download").await.json(), json!({ "redirect": "/welcome", "status": 307 }));
    let nf = get(&app, "/_page?path=/u/nobody").await.json();
    assert_eq!(nf["notFound"], true);
    assert_eq!(nf["meta"]["robots"], "noindex");
    let doc = get(&app, "/download").await;
    assert_eq!(doc.status.as_u16(), 307);
    assert_eq!(doc.header("location").as_deref(), Some("/welcome"));
    // Like Next, the 307 carries a whole document (SKELETON.md 9.41): the
    // shell with the redirect payload.
    assert_eq!(doc.header("content-type").as_deref(), Some("text/html; charset=utf-8"));
    assert!(doc.text().contains(r#""redirect":"/welcome""#), "{}", doc.text());
    let doc = get(&app, "/u/nobody").await;
    assert_eq!(doc.status.as_u16(), 404);
}

/// Inside `app/(app)/dashboard/` (its `loading.tsx` boundary) a page's own
/// `redirect()` / `notFound()` answers 200, as on the reference (SKELETON.md
/// 9.41); `/_page` is unchanged.
#[tokio::test]
async fn the_dashboard_boundary_answers_200() {
    use axum::body::Body;
    use axum::http::Request;
    let app = app();
    // proxy.ts lets /dashboard through when a session cookie is present
    // (the test `(app)` layout does not look at it).
    let get = |app: &App, uri: &str| {
        let req = Request::get(uri)
            .header("host", "127.0.0.1:8421")
            .header("cookie", "better-auth.session_token=x")
            .body(Body::empty())
            .unwrap();
        let app = app.clone();
        async move { common::call(&app, req).await }
    };
    let doc = get(&app, "/dashboard/admin").await;
    assert_eq!(doc.status.as_u16(), 200);
    assert_eq!(doc.header("location"), None);
    let html = doc.text();
    assert!(html.contains("<title>Admin - Modlight</title>"), "{html}");
    assert!(html.contains(
        r#"<noscript><meta id="__next-page-redirect" http-equiv="refresh" content="1;url=/dashboard"/></noscript>"#
    ));
    assert!(html.contains(r#""redirect":"/dashboard""#));
    assert_eq!(
        get(&app, "/_page?path=/dashboard/admin").await.json(),
        json!({ "redirect": "/dashboard", "status": 307 })
    );
    let doc = get(&app, "/dashboard/folders").await;
    assert_eq!(doc.status.as_u16(), 200);
    let html = doc.text();
    assert!(html.contains(r#"<meta name="robots" content="noindex"/>"#), "{html}");
    assert!(html.contains(r#""notFound":true"#));
}

/// Documents and page data are compressed as Next compresses them (SKELETON
/// 9.42); route handlers are not.
#[tokio::test]
async fn documents_and_page_data_are_compressed_route_handlers_not() {
    use axum::body::Body;
    use axum::http::Request;
    let app = app();
    for uri in ["/login", "/_page?path=/login", "/u/nobody"] {
        let req = Request::get(uri)
            .header("host", "127.0.0.1:8421")
            .header("accept-encoding", "gzip, deflate, br")
            .body(Body::empty())
            .unwrap();
        let a = common::call(&app, req).await;
        assert_eq!(a.header("content-encoding").as_deref(), Some("gzip"), "{uri}");
        assert_eq!(a.header("vary").as_deref(), Some("Accept-Encoding"), "{uri}");
    }
    let req = Request::get("/api/health")
        .header("host", "127.0.0.1:8421")
        .header("accept-encoding", "gzip")
        .body(Body::empty())
        .unwrap();
    let a = common::call(&app, req).await;
    assert_eq!(a.header("content-encoding"), None);
}

/// The opening `<html …>` tag of a document. The built SPA shell carries its own
/// attributes there (a `class`), the placeholder shell none, so tests check the
/// attributes the server injects one by one instead of the whole tag.
fn html_tag(html: &str) -> &str {
    let start = html.find("<html").expect("an <html> tag");
    &html[start..start + html[start..].find('>').expect("a closed <html> tag") + 1]
}

fn assert_html_attrs(html: &str, attrs: &[&str]) {
    let tag = html_tag(html);
    for attr in attrs {
        assert!(tag.contains(attr), "{attr} missing from {tag}");
    }
}

#[tokio::test]
async fn documents_inline_the_payload_and_the_head() {
    let app = app();
    let a = get(&app, "/u/alice").await;
    assert_eq!(a.status.as_u16(), 200);
    assert_eq!(a.header("content-type").as_deref(), Some("text/html; charset=utf-8"));
    let html = a.text();
    assert_html_attrs(&html, &["lang=\"en\"", "data-mode=\"system\"", "data-theme=\"discord\""]);
    assert!(
        html.contains("<title>alice&#x27;s files - Modlight</title>")
            || html.contains("<title>alice's files - Modlight</title>")
    );
    assert!(html.contains("<meta property=\"og:image\" content=\"https://x.test/raw?w=1200\"/>"));
    assert!(html.contains("<meta property=\"og:type\" content=\"profile\"/>"));
    assert!(html.contains("<meta name=\"twitter:card\" content=\"summary_large_image\"/>"));
    let start = html.find("<script id=\"__MODLIGHT_PAGE__\" type=\"application/json\">").unwrap();
    let rest = &html[start..];
    let json_text = &rest[rest.find('>').unwrap() + 1..rest.find("</script>").unwrap()];
    let d: Value = serde_json::from_str(json_text).unwrap();
    assert_eq!(d["route"], "/u/[userCode]");
}

#[tokio::test]
async fn themes_and_locales_come_from_cookies() {
    let app = app();
    let req = axum::http::Request::get("/u/alice")
        .header("host", "127.0.0.1:8421")
        .header("cookie", "ml_mode=dark; ml_skin=glass; ml_locale=pl")
        .body(axum::body::Body::empty())
        .unwrap();
    let html = common::call(&app, req).await.text();
    assert_html_attrs(&html, &["lang=\"pl\"", "data-mode=\"dark\"", "data-theme=\"glass\""]);
}

#[tokio::test]
async fn pages_without_a_loader_answer_501() {
    let app = app();
    let a = get(&app, "/_page?path=/welcome").await;
    assert_eq!(a.status.as_u16(), 501);
    assert_eq!(a.json()["file"], "app/welcome/page.tsx");
}

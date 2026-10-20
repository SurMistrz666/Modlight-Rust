//! server/docs/COVERAGE.md, the part a test can hold: every upstream page has
//! a loader, every layout on a page's path has one, and every upstream route
//! handler is routed (no URL answers the 501 "not ported" placeholder).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use modlight_server::http::App;
use modlight_server::pages::table::{APP_ROUTES, RouteKind};
use modlight_server::pages::{PageRegistry, register_all};

#[test]
fn every_page_and_layout_has_a_loader() {
    let mut reg = PageRegistry::default();
    register_all(&mut reg);
    let pages: Vec<_> = APP_ROUTES.iter().filter(|r| r.kind == RouteKind::Page).collect();
    assert_eq!(pages.len(), 27, "upstream has 27 page.tsx files");
    for route in pages {
        assert!(reg.has_page(route.path), "no loader for {} ({})", route.path, route.file);
        for layout in route.layouts {
            assert!(reg.has_layout(*layout), "no loader for {}", layout.file());
        }
    }
}

/// A URL for an app path: every `[param]` becomes `x`, `[...all]` `x/y`.
fn sample(path: &str) -> String {
    path.split('/')
        .map(|seg| match seg {
            s if s.starts_with("[...") => "x/y".to_string(),
            s if s.starts_with('[') => "x".to_string(),
            s => s.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[tokio::test]
async fn every_route_handler_is_routed() {
    let mut state = common::state();
    // Handlers that reach the database fail fast (500) instead of waiting.
    state.db = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgresql://nobody@127.0.0.1:1/none")
        .unwrap();
    let app = App::new(state);
    let handlers: Vec<_> = APP_ROUTES.iter().filter(|r| r.kind == RouteKind::Handler).collect();
    assert_eq!(handlers.len(), 25, "upstream has 25 route.ts files");
    for route in handlers {
        for method in [Method::GET, Method::POST] {
            let req = Request::builder()
                .method(method.clone())
                .uri(sample(route.path))
                .header("host", "127.0.0.1:8421")
                .body(Body::empty())
                .unwrap();
            let a = common::call(&app, req).await;
            assert_ne!(a.status, StatusCode::NOT_IMPLEMENTED, "{method} {} ({}) is not routed", route.path, route.file);
        }
    }
}

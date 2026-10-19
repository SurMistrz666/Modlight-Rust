//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use bytes::Bytes;
use http_body_util::BodyExt;
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;

/// A state whose database is never reachable: fine for every test that does
/// not query it (a lazy pool only connects on first use).
pub fn state_with(vars: &[(&str, &str)]) -> AppState {
    let map: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    let config = Config::from_env_map(&map).expect("config");
    let pool = modlight_server::db::connect_lazy("postgresql://nobody@127.0.0.1:1/none").expect("pool");
    AppState::new(config, Paths::crate_dir(), pool).expect("state")
}

pub fn state() -> AppState {
    state_with(&[("APP_URL", "http://127.0.0.1:8421"), ("NODE_ENV", "production"), ("SKIP_ENV_VALIDATION", "1")])
}

pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Answer {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).map(|v| v.to_str().unwrap().to_string())
    }
}

pub async fn call(app: &App, req: Request<Body>) -> Answer {
    let res = app.handle(req).await;
    let (parts, body) = res.into_parts();
    let body = body.collect().await.expect("body").to_bytes();
    Answer { status: parts.status, headers: parts.headers, body }
}

pub async fn get(app: &App, uri: &str) -> Answer {
    call(app, Request::get(uri).header("host", "127.0.0.1:8421").body(Body::empty()).unwrap()).await
}

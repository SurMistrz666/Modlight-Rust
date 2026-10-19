//! The RPC framework end to end, with a tiny example action module that only
//! exists in this test (PORT.md section 7).

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Answer, call, state};
use modlight_server::http::App;
use modlight_server::http::set_cookie::SetCookie;
use modlight_server::rpc::{ActionCtx, ActionError, ActionResult, FormValue, RpcRegistry};
use serde_json::{Value, json};

// ── the example module ("app/actions/example.ts") ───────────────────────────

async fn echo(ctx: ActionCtx) -> ActionResult {
    let name: String = ctx.args.get(0)?;
    let count: Option<i64> = ctx.args.get(1)?;
    ctx.set_cookie(SetCookie::new("ml_example", "a b").path("/").max_age(60));
    ctx.revalidate_path("/dashboard");
    // `tagged_json!`, not `json!`: the date must be tagged.
    Ok(modlight_server::tagged_json!({
        "name": name,
        "count": count,
        "secondWasUndefined": ctx.args.is_undefined(1),
        "when": modlight_server::db::types::JsDate::from_millis(0).unwrap(),
    }))
}

async fn thrown(_ctx: ActionCtx) -> ActionResult {
    Err(ActionError::thrown("Folder name is required"))
}

async fn go_away(_ctx: ActionCtx) -> ActionResult {
    Err(ActionError::Redirect("/dashboard/folders".into()))
}

async fn missing(_ctx: ActionCtx) -> ActionResult {
    Err(ActionError::NotFound)
}

async fn nothing(_ctx: ActionCtx) -> ActionResult {
    modlight_server::rpc::void()
}

async fn form(ctx: ActionCtx) -> ActionResult {
    let form = ctx.form.as_ref().ok_or_else(|| ActionError::thrown("no form"))?;
    let file_len = match form.get("file") {
        Some(FormValue::File { data, .. }) => data.len(),
        _ => 0,
    };
    modlight_server::rpc::ok(&json!({ "title": form.get_text("title"), "fileBytes": file_len }))
}

fn register(reg: &mut RpcRegistry) {
    reg.register("example", "echo", echo);
    reg.register("example", "thrown", thrown);
    reg.register("example", "goAway", go_away);
    reg.register("example", "missing", missing);
    reg.register("example", "nothing", nothing);
    reg.register("example", "form", form);
}

// ── tests ───────────────────────────────────────────────────────────────────

fn app() -> App {
    let mut state = state();
    let mut reg = RpcRegistry::default();
    register(&mut reg);
    state.rpc = Arc::new(reg);
    App::new(state)
}

async fn rpc(app: &App, action: &str, body: Value) -> Answer {
    let req = Request::post(format!("/_rpc/example/{action}"))
        .header("host", "127.0.0.1:8421")
        .header("origin", "http://127.0.0.1:8421")
        .header("x-modlight-rpc", "1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    call(app, req).await
}

#[tokio::test]
async fn returns_the_ok_envelope_with_tagged_values_cookies_and_revalidate() {
    let app = app();
    let a = rpc(&app, "echo", json!(["files", { "$undefined": 1 }])).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(
        a.json(),
        json!({
            "ok": {
                "name": "files",
                "count": null,
                "secondWasUndefined": true,
                "when": { "$date": "1970-01-01T00:00:00.000Z" }
            },
            "revalidate": true
        })
    );
    let cookie = a.header("set-cookie").unwrap();
    assert!(cookie.starts_with("ml_example=a%20b; Path=/; Expires="), "{cookie}");
    assert!(cookie.ends_with("; Max-Age=60"), "{cookie}");
}

#[tokio::test]
async fn redirect_not_found_and_void() {
    let app = app();
    assert_eq!(rpc(&app, "goAway", json!([])).await.json(), json!({ "redirect": "/dashboard/folders" }));
    assert_eq!(rpc(&app, "missing", json!([])).await.json(), json!({ "notFound": true }));
    assert_eq!(rpc(&app, "nothing", json!([])).await.json(), json!({ "ok": { "$undefined": 1 } }));
}

#[tokio::test]
async fn thrown_errors_are_sanitised_in_production() {
    let app = app();
    let a = rpc(&app, "thrown", json!([])).await;
    assert_eq!(a.status, StatusCode::OK);
    let body = a.json();
    assert_eq!(body["error"]["message"], modlight_server::rpc::PRODUCTION_ERROR_MESSAGE);
    assert!(body["error"]["digest"].is_string());

    let mut dev = common::state_with(&[("APP_URL", "http://127.0.0.1:8421")]);
    let mut reg = RpcRegistry::default();
    register(&mut reg);
    dev.rpc = Arc::new(reg);
    let a = rpc(&App::new(dev), "thrown", json!([])).await;
    assert_eq!(a.json(), json!({ "error": { "message": "Folder name is required" } }));
}

#[tokio::test]
async fn refuses_foreign_origins_missing_headers_and_bad_bodies() {
    let app = app();
    let foreign = Request::post("/_rpc/example/echo")
        .header("host", "127.0.0.1:8421")
        .header("origin", "http://evil.test")
        .header("x-modlight-rpc", "1")
        .body(Body::from("[]"))
        .unwrap();
    assert_eq!(call(&app, foreign).await.status, StatusCode::FORBIDDEN);
    let no_header = Request::post("/_rpc/example/echo")
        .header("host", "127.0.0.1:8421")
        .header("origin", "http://127.0.0.1:8421")
        .body(Body::from("[]"))
        .unwrap();
    assert_eq!(call(&app, no_header).await.status, StatusCode::FORBIDDEN);
    assert_eq!(rpc(&app, "echo", json!({ "not": "an array" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(rpc(&app, "nope", json!([])).await.status, StatusCode::NOT_FOUND);
    let huge = Value::String("x".repeat(modlight_server::rpc::RPC_BODY_LIMIT + 1));
    assert_eq!(rpc(&app, "echo", json!([huge])).await.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn multipart_form_actions() {
    let app = app();
    let boundary = "XyZ";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHello\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\n12345\r\n--{boundary}--\r\n"
    );
    let req = Request::post("/_rpc/example/form")
        .header("host", "127.0.0.1:8421")
        .header("origin", "http://127.0.0.1:8421")
        .header("x-modlight-rpc", "form")
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap();
    let a = call(&app, req).await;
    assert_eq!(a.json(), json!({ "ok": { "title": "Hello", "fileBytes": 5 } }));
}

#[tokio::test]
async fn real_registry_lists_the_token_actions() {
    let names = common::state().rpc.names();
    for n in ["tokens/createUploadToken", "tokens/myTokens", "tokens/narrowTokenScope", "tokens/revokeUploadToken"] {
        assert!(names.contains(&n.to_string()), "{n}");
    }
}

//! The optional Cloudflare Turnstile check (src/turnstile.rs) end to end on a
//! real PostgreSQL, with a siteverify stub on 127.0.0.1: the admin panel's
//! card (save, test, clear, the secret box, the audit), block precedence
//! over TURNSTILE_*, the root layout's `turnstileSiteKey`, the CSP, and real
//! sign-ups and sign-ins through the protected entry points.
//!
//! Runs only when `MODLIGHT_TEST_DATABASE_URL` names a server where the user
//! may create databases (see tests/db_compat.rs).

mod common;

use std::str::FromStr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Answer, call, siteverify};
use modlight_server::config::Config;
use modlight_server::http::App;
use modlight_server::paths::Paths;
use modlight_server::state::AppState;
use modlight_server::turnstile::{TOKEN_HEADER, Turnstile};
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgPool};

const SECRET: &str = "turnstile-db-test-secret-0123456789abcdef";
const HOST: &str = "127.0.0.1:8421";
const ORIGIN: &str = "http://127.0.0.1:8421";
const PASSWORD: &str = "correct horse battery";

struct TempDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl TempDb {
    async fn create() -> Option<TempDb> {
        let url = std::env::var("MODLIGHT_TEST_DATABASE_URL").ok()?;
        let admin = PgConnectOptions::from_str(&url).expect("MODLIGHT_TEST_DATABASE_URL");
        let name = format!("modlight_turnstile_{}_{}", std::process::id(), rand::random::<u32>());
        let mut conn = admin.connect().await.expect("connect to the test server");
        sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut conn).await.expect("create database");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect_with(admin.clone().database(&name).options([("TimeZone", "UTC")]))
            .await
            .expect("connect to the temp database");
        modlight_server::db::migrate::run(&pool, &Paths::crate_dir().migrations, true).await.expect("migrate");
        Some(TempDb { admin, name, pool })
    }

    async fn drop(self) {
        self.pool.close().await;
        if let Ok(mut conn) = self.admin.connect().await {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name))
                .execute(&mut conn)
                .await;
        }
    }
}

fn app(pool: &PgPool, stub: &str, env_keys: Option<(&str, &str)>) -> App {
    let mut vars: Vec<(&str, &str)> = vec![("APP_URL", ORIGIN), ("AUTH_SECRET", SECRET), ("TRUST_PROXY", "true")];
    if let Some((site, secret)) = env_keys {
        vars.push(("TURNSTILE_SITE_KEY", site));
        vars.push(("TURNSTILE_SECRET_KEY", secret));
    }
    let map = vars.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut state = AppState::new(Config::from_env_map(&map).unwrap(), Paths::crate_dir(), pool.clone()).unwrap();
    state.turnstile = Arc::new(Turnstile::with_siteverify_url(stub));
    App::new(state)
}

struct Client<'a> {
    app: &'a App,
    ip: &'a str,
    cookie: Option<String>,
}

impl Client<'_> {
    async fn post(&self, uri: &str, token: Option<&str>, body: Value) -> Answer {
        let mut req = Request::post(uri)
            .header("host", HOST)
            .header("origin", ORIGIN)
            .header("content-type", "application/json")
            .header("x-real-ip", self.ip);
        if uri.starts_with("/_rpc/") {
            req = req.header("x-modlight-rpc", "1");
        }
        if let Some(cookie) = &self.cookie {
            req = req.header("cookie", cookie);
        }
        if let Some(token) = token {
            req = req.header(TOKEN_HEADER, token);
        }
        call(self.app, req.body(Body::from(body.to_string())).unwrap()).await
    }

    /// An action's `ok` value (panics on any other envelope).
    async fn rpc(&self, path: &str, token: Option<&str>, args: Value) -> Value {
        let a = self.post(&format!("/_rpc/{path}"), token, args).await;
        let body = a.json();
        body.get("ok").cloned().unwrap_or_else(|| panic!("{path}: {body}"))
    }

    async fn page(&self, path: &str) -> Answer {
        let mut req = Request::get(format!("/_page?path={}", path.replace('/', "%2F"))).header("host", HOST);
        if let Some(cookie) = &self.cookie {
            req = req.header("cookie", cookie);
        }
        call(self.app, req.body(Body::empty()).unwrap()).await
    }
}

/// The session cookie a sign-up or sign-in answered with.
fn session_cookie(a: &Answer) -> String {
    a.headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .find(|c| c.starts_with("better-auth.session_token="))
        .map(|c| c.split(';').next().unwrap().to_string())
        .unwrap_or_else(|| panic!("no session cookie: {} {}", a.status, a.text()))
}

fn sign_up(email: &str) -> Value {
    json!({ "email": email, "password": PASSWORD, "name": email.split('@').next().unwrap() })
}

fn root_site_key(page: &Answer) -> Value {
    page.json()["layouts"]["root"]["config"]["turnstileSiteKey"].clone()
}

fn csp_lets_the_widget_in(page: &Answer) -> bool {
    page.header("content-security-policy").unwrap_or_default().contains("https://challenges.cloudflare.com")
}

async fn turnstile_rows(pool: &PgPool) -> Vec<(String, Option<String>)> {
    sqlx::query_as(r#"select "key", "value" from "settings" where "key" like 'turnstile%' order by "key""#)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_admin_card_the_entry_points_and_precedence() {
    let Some(db) = TempDb::create().await else {
        eprintln!("MODLIGHT_TEST_DATABASE_URL not set: skipped");
        return;
    };
    let outcome = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(run(&db.pool))).await;
    db.drop().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

async fn run(pool: &PgPool) {
    let stub = siteverify::start().await;
    let app = app(pool, &stub.url(), None);
    let anon = Client { app: &app, ip: "192.0.2.10", cookie: None };

    // Off: nothing asked, nothing sent, nothing in the page or the CSP.
    let first = anon.post("/api/auth/sign-up/email", None, sign_up("root@example.test")).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let admin = Client { app: &app, ip: "192.0.2.11", cookie: Some(session_cookie(&first)) };
    let login = anon.page("/login").await;
    assert_eq!(root_site_key(&login), Value::Null);
    assert!(!csp_lets_the_widget_in(&login));
    assert_eq!(stub.calls(), 0);

    let status = admin.rpc("turnstile/loadTurnstileStatus", None, json!([])).await;
    assert_eq!(
        status,
        json!({"source": "none", "enabled": false, "active": false, "siteKey": "", "secretState": "unset", "envAvailable": false})
    );

    // Saves that must not be written.
    let save = |enabled: bool, site: &str, secret: Option<&str>| {
        let mut input = json!({ "enabled": enabled, "siteKey": site });
        if let Some(s) = secret {
            input["secret"] = json!(s);
        }
        json!([input])
    };
    let refused = |reason: &str| json!({ "ok": false, "reason": reason });
    assert_eq!(
        admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "site 1", Some("good-secret"))).await,
        refused("invalid-site-key")
    );
    assert_eq!(
        admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "site-1", Some("bad secret"))).await,
        refused("invalid-secret")
    );
    assert_eq!(
        admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "", Some("good-secret"))).await,
        refused("site-key-required")
    );
    assert_eq!(
        admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "site-1", None)).await,
        refused("secret-required")
    );
    assert_eq!(
        admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "site-1", Some("bad-secret"))).await,
        refused("secret-rejected")
    );
    assert!(turnstile_rows(pool).await.is_empty());

    // Turned on from the panel.
    let saved = admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "site-1", Some("good-secret"))).await;
    assert_eq!(saved["ok"], true, "{saved}");
    assert_eq!(saved["status"]["active"], true);
    assert_eq!(saved["status"]["source"], "panel");
    assert_eq!(saved["status"]["secretState"], "set");
    assert_eq!(saved["changed"], json!(["turnstile_enabled", "turnstile_site_key", "turnstile_secret_enc"]));
    let rows = turnstile_rows(pool).await;
    let boxed = rows.iter().find(|(k, _)| k == "turnstile_secret_enc").and_then(|(_, v)| v.clone()).unwrap();
    assert!(boxed.starts_with("v1.") && !boxed.contains("good-secret"), "stored encrypted");
    let audit: Vec<(String, String)> =
        sqlx::query_as(r#"select "action", "detail"::text from "audit_log" where "action" like 'turnstile.%'"#)
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(audit.len(), 1);
    assert!(audit[0].1.contains("turnstile_secret_enc") && !audit[0].1.contains("good-secret"), "{}", audit[0].1);
    let status = admin.rpc("turnstile/loadTurnstileStatus", None, json!([])).await;
    assert!(!status.to_string().contains("good-secret"), "the secret never travels back: {status}");

    let login = anon.page("/login").await;
    assert_eq!(root_site_key(&login), json!("site-1"));
    assert!(csp_lets_the_widget_in(&login));
    let landing = anon.page("/").await;
    assert!(csp_lets_the_widget_in(&landing), "every page: the SPA reaches /login without a new document");

    // Sign-up.
    let calls = stub.calls();
    let a = anon.post("/api/auth/sign-up/email", None, sign_up("second@example.test")).await;
    assert_eq!((a.status, a.json()["code"].clone()), (StatusCode::BAD_REQUEST, json!("MISSING_RESPONSE")));
    let a = anon.post("/api/auth/sign-up/email", Some("nope"), sign_up("second@example.test")).await;
    assert_eq!((a.status, a.json()["code"].clone()), (StatusCode::FORBIDDEN, json!("VERIFICATION_FAILED")));
    let a = anon.post("/api/auth/sign-up/email", Some("pass-signup"), sign_up("second@example.test")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(stub.calls(), calls + 2);
    assert_eq!(stub.last().get("remoteip").map(String::as_str), Some("192.0.2.10"));
    assert_eq!(stub.last().get("secret").map(String::as_str), Some("good-secret"));
    let users: i64 = sqlx::query_scalar(r#"select count(*) from "user""#).fetch_one(pool).await.unwrap();
    assert_eq!(users, 2);

    // Sign-in through the login form's action: one siteverify per attempt.
    let signin = |token: Option<&'static str>| {
        let anon = &anon;
        async move { anon.rpc("login-identifier/signInWithIdentifier", token, json!(["second", PASSWORD])).await }
    };
    assert_eq!(signin(None).await, json!({"status": "captcha"}));
    assert_eq!(signin(Some("nope")).await, json!({"status": "captcha"}));
    let calls = stub.calls();
    assert_eq!(signin(Some("pass-signin")).await, json!({"status": "ok"}));
    assert_eq!(stub.calls(), calls + 1, "the action's own sign-in is not checked a second time");
    // ... and the HTTP route.
    let body = json!({ "email": "second@example.test", "password": PASSWORD });
    let a = anon.post("/api/auth/sign-in/email", None, body.clone()).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    let a = anon.post("/api/auth/sign-in/email", Some("pass-route"), body).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    let user = Client { app: &app, ip: "192.0.2.12", cookie: Some(session_cookie(&a)) };

    // Password reset request.
    let reset = json!({ "email": "second@example.test", "redirectTo": "/reset" });
    let a = anon.post("/api/auth/request-password-reset", None, reset.clone()).await;
    assert_eq!(a.json()["code"], "MISSING_RESPONSE");
    let a = anon.post("/api/auth/request-password-reset", Some("pass-reset"), reset).await;
    assert!(!a.text().contains("MISSING_RESPONSE") && a.status != StatusCode::FORBIDDEN, "{}", a.text());

    // The test button.
    let test = |args: Value| admin.rpc("turnstile/testTurnstileSecret", None, args);
    let r = test(json!([])).await;
    assert_eq!((r["ok"].clone(), r["alwaysPasses"].clone()), (json!(true), json!(false)), "{r}");
    assert_eq!(stub.last().get("secret").map(String::as_str), Some("good-secret"), "the stored one");
    assert_eq!(test(json!(["bad-secret"])).await["kind"], "rejected");
    assert_eq!(test(json!(["always-secret"])).await["alwaysPasses"], true);
    assert_eq!(test(json!(["has space"])).await["kind"], "invalid-secret");

    // Only an administrator.
    let a = user.post("/_rpc/turnstile/loadTurnstileStatus", None, json!([])).await;
    assert!(a.json().get("ok").is_none(), "{}", a.text());

    // The stored secret belongs to its site key.
    let kept = admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "site-1", None)).await;
    assert_eq!(kept["ok"], true);
    assert_eq!(kept["changed"], json!([]));
    assert_eq!(
        admin.rpc("turnstile/saveTurnstileSettings", None, save(true, "site-2", None)).await,
        refused("secret-required")
    );
    let off = admin.rpc("turnstile/saveTurnstileSettings", None, save(false, "site-2", None)).await;
    assert_eq!(off["ok"], true);
    assert_eq!(off["status"]["active"], false);
    assert_eq!(off["status"]["secretState"], "unset", "a new site key forgets the old secret");

    // Off again: exactly as before.
    let login = anon.page("/login").await;
    assert_eq!(root_site_key(&login), Value::Null);
    assert!(!csp_lets_the_widget_in(&login));
    let calls = stub.calls();
    assert_eq!(signin(None).await, json!({"status": "ok"}));
    assert_eq!(stub.calls(), calls);

    // The environment: ignored while the panel holds a block (even one
    // switched off), in charge once the block is cleared.
    let env_app = self::app(pool, &stub.url(), Some(("env-site", "good-secret")));
    let env_admin = Client { app: &env_app, ip: "192.0.2.13", cookie: admin.cookie.clone() };
    let env_anon = Client { app: &env_app, ip: "192.0.2.14", cookie: None };
    let status = env_admin.rpc("turnstile/loadTurnstileStatus", None, json!([])).await;
    assert_eq!((status["source"].clone(), status["active"].clone()), (json!("panel"), json!(false)));
    assert_eq!(status["envAvailable"], true);
    assert_eq!(root_site_key(&env_anon.page("/login").await), Value::Null);
    let cleared = env_admin.rpc("turnstile/clearTurnstileSettings", None, json!([])).await;
    assert_eq!(
        cleared,
        json!({"source": "env", "enabled": true, "active": true, "siteKey": "env-site", "secretState": "set", "envAvailable": true})
    );
    assert!(turnstile_rows(pool).await.iter().all(|(_, v)| v.is_none()));
    let login = env_anon.page("/login").await;
    assert_eq!(root_site_key(&login), json!("env-site"));
    assert!(csp_lets_the_widget_in(&login));
    let a = env_anon.rpc("login-identifier/signInWithIdentifier", None, json!(["second", PASSWORD])).await;
    assert_eq!(a, json!({"status": "captcha"}));
}

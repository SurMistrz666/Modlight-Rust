//! `api/routes/update-session.mjs`: `/update-session` writes the session's
//! client-settable fields. With lib/auth.ts's plugins the only one is the
//! admin plugin's `impersonatedBy` (declared without `input: false`
//! upstream, so a signed-in user can set it on their own session; kept 1:1).

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

use super::set_session_cookie;
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, base};
use crate::auth::session::{AuthSession, SessionOut};
use crate::auth::validate::Schema;
use crate::db::models::{Columns, Session};
use crate::db::types::JsDate;

fn body() -> Schema {
    Schema::Record(Box::new(Schema::Any))
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![Endpoint {
        path: "/update-session",
        methods: &[Method::POST],
        body: Some(body),
        query: None,
        media_types: &[],
        middleware: &[Middleware::Session],
        handler: update_session,
    }]
}

fn update_session(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let Some(Value::Object(body)) = ctx.body.clone() else {
            return Err(ApiError::from("BAD_REQUEST", base::BODY_MUST_BE_AN_OBJECT).into());
        };
        let session = ctx.session.clone().expect("session middleware ran");
        // parseSessionInput(options, body, "update"): only `impersonatedBy`.
        let mut fields: Map<String, Value> = Map::new();
        if let Some(v) = body.get("impersonatedBy") {
            fields.insert("impersonatedBy".into(), v.clone());
        }
        if fields.is_empty() {
            return Err(ApiError::message_only("BAD_REQUEST", "No fields to update").into());
        }
        let value = crate::auth::adapter::pg_text(&fields["impersonatedBy"]);
        let now = JsDate::now();
        let sql = format!(
            "UPDATE {} SET \"impersonated_by\" = $1::text, \"updated_at\" = $2 WHERE \"token\" = $3 RETURNING {}",
            Session::TABLE,
            Session::COLUMNS
        );
        let updated = sqlx::query_as::<_, Session>(&sql)
            .bind(value.clone())
            .bind(now)
            .bind(&session.session.token)
            .fetch_optional(ctx.db())
            .await?;
        let new_session = updated.unwrap_or_else(|| {
            let mut s = session.session.clone();
            s.impersonated_by = value;
            s.updated_at = now;
            s
        });
        set_session_cookie(ctx, AuthSession { session: new_session.clone(), user: session.user.clone() }, None, None);
        let out = serde_json::to_value(SessionOut::from(&new_session)).map_err(anyhow::Error::from)?;
        ctx.json(json!({ "session": out }))
    })
}

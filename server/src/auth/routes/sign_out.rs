//! `api/routes/sign-out.mjs`: `/sign-out` deletes the session row of the
//! signed cookie (a failure is only logged) and expires the session cookies.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::json;

use super::delete_session_cookie;
use crate::auth::adapter;
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult};

pub fn endpoints() -> Vec<Endpoint> {
    vec![Endpoint {
        path: "/sign-out",
        methods: &[Method::POST],
        body: None,
        query: None,
        media_types: &[],
        middleware: &[],
        handler: sign_out,
    }]
}

fn sign_out(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let name = ctx.state.auth_cookies.session_token.name.clone();
        if let Some(token) = ctx.signed_cookie_value(&name)
            && let Err(e) = adapter::delete_session(ctx.db(), &token).await
        {
            tracing::error!("Failed to delete session from database: {e}");
        }
        delete_session_cookie(ctx, false);
        ctx.json(json!({ "success": true }))
    })
}

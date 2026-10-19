//! Port of `app/actions/shortlink-unlock.ts`: verify a short link's gate
//! password (scrypt, [`crate::password`]). On success a signed unlock token
//! goes into the httpOnly cookie `ml_sl_<id>`, so the `/s/<code>` redirect
//! passes the gate and forwards to the target.
//!
//! Public: no session is read. Attempts are throttled per client IP and code,
//! unknown codes included, so probing for codes costs the same as guessing.

use serde_json::{Value, json};

use crate::actions::js_value::{js_string, truthy};
use crate::password::{
    UnlockScope, check_rate_limit_default, mint_unlock_token, reset_rate_limit, unlock_cookie, unlock_cookie_name,
    verify_password,
};
use crate::rpc::{ActionCtx, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("shortlink-unlock", "verifyShortlinkPassword", verify_shortlink_password);
}

async fn verify_shortlink_password(ctx: ActionCtx) -> ActionResult {
    let code = ctx.args.raw(0);
    let password = match ctx.args.raw(1) {
        Some(Value::String(p)) if !p.is_empty() => p.clone(),
        _ => return Ok(json!({ "ok": false })),
    };
    let Some(code) = code.filter(|c| truthy(Some(c))) else {
        return Ok(json!({ "ok": false }));
    };

    let key = format!("{}|shortlink:{}", ctx.client_ip(), js_string(code));
    if !check_rate_limit_default(&key).allowed {
        return Ok(json!({ "ok": false, "error": "rate-limited" }));
    }

    // `code` is the stored code, possibly scoped as `tag/leaf`. A number is
    // bound as its text, as the driver sends it; nothing else can match a
    // varchar.
    let link: Option<(uuid::Uuid, Option<String>)> = match code {
        Value::String(_) | Value::Number(_) => {
            sqlx::query_as(r#"select "id", "password_hash" from "short_links" where "short_links"."code" = $1 limit 1"#)
                .bind(js_string(code))
                .fetch_optional(&ctx.state.db)
                .await?
        }
        _ => None,
    };
    let Some((id, Some(hash))) = link else {
        return Ok(json!({ "ok": false }));
    };
    // Verified untrimmed (creation trimmed it: an upstream quirk kept 1:1).
    if !verify_password(&password, Some(&hash)).await {
        return Ok(json!({ "ok": false }));
    }

    reset_rate_limit(&key);
    let id = id.to_string();
    let token = mint_unlock_token(&ctx.state.config.auth.secret, UnlockScope::Shortlink, &id, None);
    ctx.set_cookie(unlock_cookie(
        unlock_cookie_name(UnlockScope::Shortlink, &id),
        token,
        UnlockScope::Shortlink,
        None,
        ctx.state.config.is_prod,
    ));
    Ok(json!({ "ok": true }))
}

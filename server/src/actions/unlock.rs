//! Port of `app/actions/unlock.ts`: `verifyMediaPassword(mediaId, password,
//! remember = false)`, the password gate of a file (CONTRACT 7.4, module
//! `unlock`).
//!
//! On success a signed unlock token goes into the httpOnly cookie
//! `ml_unlock_<id>`, so the page, `/raw` and `/download` let the visitor
//! through. What "remember" is worth is decided HERE rather than by the
//! client: a file on the owner's CURRENT default password gets an owner-scoped
//! token too (`ml_dpw_<ownerId>`, the next file that person shares opens
//! without a prompt); anything else gets a longer-lived cookie for this file
//! and nothing more. A recipient let into one file must not thereby be let
//! into a file its owner deliberately gave a different password.

use serde_json::{Value, json};

use crate::js_value::{js_string, truthy};
use crate::password::{
    REMEMBER_TTL, UnlockScope, check_rate_limit_default, mint_owner_unlock_token, mint_unlock_token,
    owner_unlock_cookie_name, reset_rate_limit, unlock_cookie, unlock_cookie_name, verify_password,
};
use crate::rpc::tagged::undefined;
use crate::rpc::{ActionCtx, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("unlock", "verifyMediaPassword", verify_media_password);
}

/// uploads.id is a uuid column; anything else would make Postgres throw.
fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

async fn verify_media_password(ctx: ActionCtx) -> ActionResult {
    let fail = || Ok(json!({ "ok": false }));
    // `!mediaId || !UUID.test(mediaId)`: the test reads the argument as a string.
    let media_id = match ctx.args.raw(0) {
        Some(v) if truthy(Some(v)) && is_uuid(&js_string(v)) => js_string(v),
        _ => return fail(),
    };
    let password = match ctx.args.raw(1) {
        Some(Value::String(p)) if !p.is_empty() => p.clone(),
        _ => return fail(),
    };
    let remember = !ctx.args.is_undefined(2) && truthy(ctx.args.raw(2));

    let key = format!("{}|media:{media_id}", ctx.client_ip());
    if !check_rate_limit_default(&key).allowed {
        return Ok(json!({ "ok": false, "error": "rate-limited" }));
    }

    type Row = (uuid::Uuid, Option<String>, Option<String>, Option<i32>, Option<i32>, Option<String>);
    let id: uuid::Uuid = media_id.parse().map_err(anyhow::Error::from)?;
    let row: Option<Row> = sqlx::query_as(
        r#"select "uploads"."id", "uploads"."password_hash", "uploads"."owner_id", "uploads"."password_version",
                  "user"."default_password_version", "user"."default_password_hash"
           from "uploads" left join "user" on "user"."id" = "uploads"."owner_id"
           where "uploads"."id" = $1 limit 1"#,
    )
    .bind(id)
    .fetch_optional(&ctx.state.db)
    .await?;
    let Some((id, Some(hash), owner_id, password_version, owner_version, owner_hash)) = row else {
        return fail();
    };
    if hash.is_empty() || !verify_password(&password, Some(&hash)).await {
        return fail();
    }

    reset_rate_limit(&key);
    let secret = &ctx.state.config.auth.secret;
    let is_prod = ctx.state.config.is_prod;
    let id = id.to_string();

    // Is this file on the owner's default password as it stands right now? Only
    // then may the consent travel to their other files.
    let on_current_default = owner_id.as_deref().is_some_and(|o| !o.is_empty())
        && owner_hash.as_deref().is_some_and(|h| !h.is_empty())
        && password_version.is_some()
        && password_version == owner_version;

    // Always let the visitor through the file they just unlocked. "Remember"
    // stretches that from a day to a month; it never shortens it.
    ctx.set_cookie(unlock_cookie(
        unlock_cookie_name(UnlockScope::Media, &id),
        mint_unlock_token(secret, UnlockScope::Media, &id, None),
        UnlockScope::Media,
        remember.then_some(REMEMBER_TTL),
        is_prod,
    ));

    if remember
        && on_current_default
        && let (Some(owner), Some(version)) = (owner_id.as_deref(), password_version)
    {
        ctx.set_cookie(unlock_cookie(
            owner_unlock_cookie_name(owner),
            mint_owner_unlock_token(secret, owner, i64::from(version), None)?,
            UnlockScope::Owner,
            None,
            is_prod,
        ));
        return Ok(json!({ "ok": true, "remembered": "owner" }));
    }
    Ok(json!({ "ok": true, "remembered": if remember { json!("file") } else { undefined() } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_guarded_like_the_upstream_pattern() {
        assert!(is_uuid("9d2e1a54-0000-4000-8000-000000000001"));
        assert!(is_uuid("9D2E1A54-0000-4000-8000-00000000000A"));
        assert!(!is_uuid("9d2e1a54-0000-4000-8000-00000000000"));
        assert!(!is_uuid("9d2e1a54x0000-4000-8000-000000000001"));
        assert!(!is_uuid("not-a-uuid"));
        assert!(!is_uuid(""));
    }
}

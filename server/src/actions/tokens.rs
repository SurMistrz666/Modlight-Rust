//! Port of `app/actions/tokens.ts`: the Integrations page's token actions.
//!
//! A server action is a public endpoint: the picker in the dialog is a
//! suggestion, and anything can post any value at these. So the scope is
//! validated here: an unrecognised value falls to the NARROWEST scope when
//! creating, and is refused when narrowing.

use serde_json::{Value, json};

use crate::api_token::{
    NarrowResult, TokenScope, create_api_token, list_api_tokens, narrow_api_token_scope, parse_token_scope,
    revoke_api_token,
};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("tokens", "createUploadToken", create_upload_token);
    reg.register("tokens", "revokeUploadToken", revoke_upload_token);
    reg.register("tokens", "narrowTokenScope", narrow_token_scope);
    reg.register("tokens", "myTokens", my_tokens);
}

/// The scope a client gets when the caller did not choose one: `upload`, the
/// safe end of the range, because an omission is what most tokens end up with.
pub const DEFAULT_SCOPE: TokenScope = TokenScope::Upload;

/// `isTokenScope(scope) ? scope : DEFAULT_SCOPE`
pub fn chosen_scope(scope: Option<&Value>) -> TokenScope {
    scope.and_then(Value::as_str).and_then(parse_token_scope).unwrap_or(DEFAULT_SCOPE)
}

/// What the actions need besides the session (stubbed in tests, as the
/// upstream test mocks `@/lib/api-token` and `@/lib/audit`).
pub trait TokensDeps {
    fn create(
        &self,
        user_id: &str,
        name: &str,
        scope: TokenScope,
    ) -> impl Future<Output = anyhow::Result<String>> + Send;
    fn narrow(
        &self,
        id: &str,
        user_id: &str,
        scope: TokenScope,
    ) -> impl Future<Output = anyhow::Result<NarrowResult>> + Send;
    fn audit(&self, user_id: &str, action: &str, detail: Value) -> impl Future<Output = ()> + Send;
}

struct LiveDeps<'a>(&'a ActionCtx);

impl TokensDeps for LiveDeps<'_> {
    async fn create(&self, user_id: &str, name: &str, scope: TokenScope) -> anyhow::Result<String> {
        create_api_token(&self.0.state.db, user_id, name, scope).await
    }
    async fn narrow(&self, id: &str, user_id: &str, scope: TokenScope) -> anyhow::Result<NarrowResult> {
        narrow_api_token_scope(&self.0.state.db, id, user_id, scope).await
    }
    async fn audit(&self, user_id: &str, action: &str, detail: Value) {
        crate::audit::record_audit(&self.0.state.db, user_id, action, detail, &self.0.client_ip()).await;
    }
}

/// `name?.trim() || "ShareX"` (a non-string name is a TypeError upstream).
fn token_name(name: Option<&Value>) -> Result<String, ActionError> {
    match name {
        None | Some(Value::Null) => Ok("ShareX".to_string()),
        Some(Value::String(s)) => {
            let t = crate::config::js_trim(s);
            Ok(if t.is_empty() { "ShareX".to_string() } else { t.to_string() })
        }
        Some(_) => Err(ActionError::thrown("name?.trim is not a function")),
    }
}

/// `createUploadToken(name, scope?)` after `requireUser()`.
pub async fn create_upload_token_with<D: TokensDeps>(
    deps: &D,
    user_id: &str,
    name: Option<&Value>,
    scope: Option<&Value>,
) -> Result<Value, ActionError> {
    let chosen = chosen_scope(scope);
    let token = deps.create(user_id, &token_name(name)?, chosen).await?;
    Ok(json!({ "token": token, "scope": chosen.as_str() }))
}

/// `narrowTokenScope(id, scope)` after `requireUser()`. Never widens: the
/// refusal is enforced in the store; this does not paper over it.
pub async fn narrow_token_scope_with<D: TokensDeps>(
    deps: &D,
    user_id: &str,
    id: &str,
    scope: Option<&Value>,
) -> Result<(Value, bool), ActionError> {
    let Some(scope) = scope.and_then(Value::as_str).and_then(parse_token_scope) else {
        return Ok((json!({ "ok": false, "reason": "not-narrower" }), false));
    };
    match deps.narrow(id, user_id, scope).await? {
        NarrowResult::Ok(settled) => {
            // A credential's reach changed, so it gets a line in the log.
            deps.audit(user_id, "api_token.narrowed", json!({ "token": id, "scope": settled.as_str() })).await;
            Ok((json!({ "ok": true, "scope": settled.as_str() }), true))
        }
        refused => Ok((json!({ "ok": false, "reason": refused.reason().unwrap_or("not-narrower") }), false)),
    }
}

async fn create_upload_token(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let out = create_upload_token_with(&LiveDeps(&ctx), &session.user.id, ctx.args.raw(0), ctx.args.raw(1)).await?;
    ctx.revalidate_path("/dashboard/tokens");
    Ok(out)
}

async fn revoke_upload_token(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let id: String = ctx.args.get(0)?;
    revoke_api_token(&ctx.state.db, &id, &session.user.id).await?;
    ctx.revalidate_path("/dashboard/tokens");
    crate::rpc::void()
}

async fn narrow_token_scope(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let id: Option<String> = ctx.args.get(0)?;
    let (out, changed) =
        narrow_token_scope_with(&LiveDeps(&ctx), &session.user.id, id.as_deref().unwrap_or(""), ctx.args.raw(1))
            .await?;
    if changed {
        ctx.revalidate_path("/dashboard/tokens");
    }
    Ok(out)
}

async fn my_tokens(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    crate::rpc::ok(&list_api_tokens(&ctx.state.db, &session.user.id).await?)
}

#[cfg(test)]
mod tests {
    //! tests/unit/token-scope-actions.test.ts
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Fake {
        created: Mutex<Vec<(String, String, TokenScope)>>,
        narrow_answer: Mutex<Option<NarrowResult>>,
        narrowed: Mutex<Vec<(String, String, TokenScope)>>,
        audits: Mutex<Vec<(String, String, Value)>>,
    }

    impl TokensDeps for Fake {
        async fn create(&self, user_id: &str, name: &str, scope: TokenScope) -> anyhow::Result<String> {
            self.created.lock().unwrap().push((user_id.into(), name.into(), scope));
            Ok("plaintext-token".into())
        }
        async fn narrow(&self, id: &str, user_id: &str, scope: TokenScope) -> anyhow::Result<NarrowResult> {
            self.narrowed.lock().unwrap().push((id.into(), user_id.into(), scope));
            Ok(self.narrow_answer.lock().unwrap().unwrap_or(NarrowResult::NotNarrower))
        }
        async fn audit(&self, user_id: &str, action: &str, detail: Value) {
            self.audits.lock().unwrap().push((user_id.into(), action.into(), detail));
        }
    }

    #[tokio::test]
    async fn passes_the_chosen_scope_through() {
        let f = Fake::default();
        let out = create_upload_token_with(&f, "u1", Some(&json!("laptop")), Some(&json!("write"))).await.unwrap();
        assert_eq!(out, json!({ "token": "plaintext-token", "scope": "write" }));
        assert_eq!(f.created.lock().unwrap()[0], ("u1".into(), "laptop".into(), TokenScope::Write));
    }

    #[tokio::test]
    async fn falls_to_the_narrowest_scope_when_none_was_given() {
        let f = Fake::default();
        let out = create_upload_token_with(&f, "u1", Some(&json!("ShareX")), None).await.unwrap();
        assert_eq!(out["scope"], "upload");
        assert_eq!(f.created.lock().unwrap()[0], ("u1".into(), "ShareX".into(), TokenScope::Upload));
    }

    #[tokio::test]
    async fn refuses_to_widen_an_unrecognised_value() {
        for junk in ["full-access", "FULL", "admin", "", "  "] {
            let f = Fake::default();
            let out = create_upload_token_with(&f, "u1", Some(&json!("x")), Some(&json!(junk))).await.unwrap();
            assert_eq!(out["scope"], "upload");
            assert_eq!(f.created.lock().unwrap()[0].2, TokenScope::Upload);
        }
    }

    #[tokio::test]
    async fn blank_names_become_sharex() {
        let f = Fake::default();
        create_upload_token_with(&f, "u1", Some(&json!("   ")), None).await.unwrap();
        create_upload_token_with(&f, "u1", None, None).await.unwrap();
        let created = f.created.lock().unwrap();
        assert_eq!(created[0].1, "ShareX");
        assert_eq!(created[1].1, "ShareX");
    }

    #[tokio::test]
    async fn hands_the_request_to_the_store_and_reports_what_it_settled_on() {
        let f = Fake::default();
        *f.narrow_answer.lock().unwrap() = Some(NarrowResult::Ok(TokenScope::Read));
        let (out, _) = narrow_token_scope_with(&f, "u1", "tok-1", Some(&json!("read"))).await.unwrap();
        assert_eq!(out, json!({ "ok": true, "scope": "read" }));
        assert_eq!(f.narrowed.lock().unwrap()[0], ("tok-1".into(), "u1".into(), TokenScope::Read));
    }

    #[tokio::test]
    async fn logs_the_change_because_a_credentials_reach_moved() {
        let f = Fake::default();
        *f.narrow_answer.lock().unwrap() = Some(NarrowResult::Ok(TokenScope::Upload));
        narrow_token_scope_with(&f, "u1", "tok-1", Some(&json!("upload"))).await.unwrap();
        assert_eq!(
            f.audits.lock().unwrap()[0],
            ("u1".into(), "api_token.narrowed".into(), json!({ "token": "tok-1", "scope": "upload" }))
        );
    }

    #[tokio::test]
    async fn never_reaches_the_store_with_a_scope_it_does_not_recognise() {
        let f = Fake::default();
        let (out, _) = narrow_token_scope_with(&f, "u1", "tok-1", Some(&json!("superuser"))).await.unwrap();
        assert_eq!(out, json!({ "ok": false, "reason": "not-narrower" }));
        assert!(f.narrowed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn surfaces_the_stores_refusal_rather_than_reporting_success() {
        let f = Fake::default();
        *f.narrow_answer.lock().unwrap() = Some(NarrowResult::NotNarrower);
        let (out, _) = narrow_token_scope_with(&f, "u1", "tok-1", Some(&json!("full"))).await.unwrap();
        assert_eq!(out, json!({ "ok": false, "reason": "not-narrower" }));
        assert!(f.audits.lock().unwrap().is_empty());
    }
}

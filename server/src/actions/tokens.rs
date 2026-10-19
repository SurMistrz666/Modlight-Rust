//! Port of `app/actions/tokens.ts`: the Integrations page's token actions.
//!
//! A server action is a public endpoint: the picker in the dialog is a
//! suggestion, and anything can post any value at these. So the scope is
//! validated here: an unrecognised value falls to the NARROWEST scope when
//! creating, and is refused when narrowing.

use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api_token::{
    NarrowResult, TokenScope, create_api_token, list_api_tokens, narrow_api_token_scope, parse_token_scope,
    revoke_api_token,
};
use crate::db::models::{ApiToken, JsDate};
use crate::rpc::tagged::is_undefined;
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
        Some(v) if is_undefined(v) => Ok("ShareX".to_string()),
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

/// A token id argument as `eq(apiTokens.id, id)` binds it: node-postgres sends
/// `undefined` and `null` as NULL (`id = NULL` matches no row, so `None` here
/// means "no such token"), a string as text for Postgres to cast, and anything
/// else as text that is never a uuid, so Postgres refuses it (22P02).
fn token_id(id: Option<&Value>) -> Result<Option<&str>, ActionError> {
    match id {
        None | Some(Value::Null) => Ok(None),
        Some(v) if is_undefined(v) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(other) => Err(ActionError::thrown(format!("invalid input syntax for type uuid: \"{other}\""))),
    }
}

/// `narrowTokenScope(id, scope)` after `requireUser()`. Never widens: the
/// refusal is enforced in the store; this does not paper over it. The scope is
/// checked before the id is even looked at, as upstream.
pub async fn narrow_token_scope_with<D: TokensDeps>(
    deps: &D,
    user_id: &str,
    id: Option<&Value>,
    scope: Option<&Value>,
) -> Result<(Value, bool), ActionError> {
    let Some(scope) = scope.and_then(Value::as_str).and_then(parse_token_scope) else {
        return Ok((json!({ "ok": false, "reason": "not-narrower" }), false));
    };
    let outcome = match token_id(id)? {
        Some(id) => deps.narrow(id, user_id, scope).await?,
        None => NarrowResult::NotFound,
    };
    match outcome {
        NarrowResult::Ok(settled) => {
            // A credential's reach changed, so it gets a line in the log.
            let token = id.cloned().unwrap_or(Value::Null);
            deps.audit(user_id, "api_token.narrowed", json!({ "token": token, "scope": settled.as_str() })).await;
            Ok((json!({ "ok": true, "scope": settled.as_str() }), true))
        }
        refused => Ok((json!({ "ok": false, "reason": refused.reason().unwrap_or("not-narrower") }), false)),
    }
}

/// One `api_tokens` row as drizzle's `select()` returns it: the keys in the
/// schema's order (`scope` right after `name`), which is not the table's.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenRow {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub scope: String,
    pub token_hash: String,
    pub preview: String,
    pub last_used_at: Option<JsDate>,
    pub revoked_at: Option<JsDate>,
    pub created_at: JsDate,
}

impl From<ApiToken> for ApiTokenRow {
    fn from(t: ApiToken) -> Self {
        ApiTokenRow {
            id: t.id,
            user_id: t.user_id,
            name: t.name,
            scope: t.scope,
            token_hash: t.token_hash,
            preview: t.preview,
            last_used_at: t.last_used_at,
            revoked_at: t.revoked_at,
            created_at: t.created_at,
        }
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
    // A missing id updates nothing (`WHERE id = NULL`) and still resolves.
    if let Some(id) = token_id(ctx.args.raw(0))? {
        revoke_api_token(&ctx.state.db, id, &session.user.id).await?;
    }
    ctx.revalidate_path("/dashboard/tokens");
    crate::rpc::void()
}

async fn narrow_token_scope(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let (out, changed) =
        narrow_token_scope_with(&LiveDeps(&ctx), &session.user.id, ctx.args.raw(0), ctx.args.raw(1)).await?;
    if changed {
        ctx.revalidate_path("/dashboard/tokens");
    }
    Ok(out)
}

async fn my_tokens(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let rows: Vec<ApiTokenRow> =
        list_api_tokens(&ctx.state.db, &session.user.id).await?.into_iter().map(ApiTokenRow::from).collect();
    crate::rpc::ok(&rows)
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
        let (out, _) = narrow_token_scope_with(&f, "u1", Some(&json!("tok-1")), Some(&json!("read"))).await.unwrap();
        assert_eq!(out, json!({ "ok": true, "scope": "read" }));
        assert_eq!(f.narrowed.lock().unwrap()[0], ("tok-1".into(), "u1".into(), TokenScope::Read));
    }

    #[tokio::test]
    async fn logs_the_change_because_a_credentials_reach_moved() {
        let f = Fake::default();
        *f.narrow_answer.lock().unwrap() = Some(NarrowResult::Ok(TokenScope::Upload));
        narrow_token_scope_with(&f, "u1", Some(&json!("tok-1")), Some(&json!("upload"))).await.unwrap();
        assert_eq!(
            f.audits.lock().unwrap()[0],
            ("u1".into(), "api_token.narrowed".into(), json!({ "token": "tok-1", "scope": "upload" }))
        );
    }

    #[tokio::test]
    async fn never_reaches_the_store_with_a_scope_it_does_not_recognise() {
        let f = Fake::default();
        let (out, _) =
            narrow_token_scope_with(&f, "u1", Some(&json!("tok-1")), Some(&json!("superuser"))).await.unwrap();
        assert_eq!(out, json!({ "ok": false, "reason": "not-narrower" }));
        assert!(f.narrowed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn surfaces_the_stores_refusal_rather_than_reporting_success() {
        let f = Fake::default();
        *f.narrow_answer.lock().unwrap() = Some(NarrowResult::NotNarrower);
        let (out, _) = narrow_token_scope_with(&f, "u1", Some(&json!("tok-1")), Some(&json!("full"))).await.unwrap();
        assert_eq!(out, json!({ "ok": false, "reason": "not-narrower" }));
        assert!(f.audits.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn an_undefined_name_is_sharex_too() {
        let f = Fake::default();
        create_upload_token_with(&f, "u1", Some(&json!({ "$undefined": 1 })), None).await.unwrap();
        assert_eq!(f.created.lock().unwrap()[0].1, "ShareX");
        assert!(create_upload_token_with(&f, "u1", Some(&json!(5)), None).await.is_err());
    }

    #[tokio::test]
    async fn the_scope_is_checked_before_the_id_is_looked_at() {
        let f = Fake::default();
        let (out, _) = narrow_token_scope_with(&f, "u1", Some(&json!(5)), Some(&json!("bogus"))).await.unwrap();
        assert_eq!(out, json!({ "ok": false, "reason": "not-narrower" }));
        assert!(narrow_token_scope_with(&f, "u1", Some(&json!(5)), Some(&json!("read"))).await.is_err());
        assert!(f.narrowed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_missing_id_is_no_such_token() {
        // node-postgres binds undefined and null as NULL: `id = NULL` matches nothing.
        let f = Fake::default();
        for id in [None, Some(json!(null)), Some(json!({ "$undefined": 1 }))] {
            let (out, changed) = narrow_token_scope_with(&f, "u1", id.as_ref(), Some(&json!("read"))).await.unwrap();
            assert_eq!(out, json!({ "ok": false, "reason": "not-found" }));
            assert!(!changed);
        }
        assert!(f.narrowed.lock().unwrap().is_empty());
        assert!(f.audits.lock().unwrap().is_empty());
        assert_eq!(token_id(Some(&json!("abc"))).unwrap(), Some("abc"));
        assert!(token_id(Some(&json!(true))).is_err());
        assert!(token_id(Some(&json!({ "$date": "2026-10-18T12:00:00.000Z" }))).is_err());
    }

    #[test]
    fn my_tokens_rows_keep_drizzles_key_order() {
        let row = ApiTokenRow::from(ApiToken {
            id: Uuid::nil(),
            user_id: "u1".into(),
            name: "ShareX".into(),
            token_hash: "h".into(),
            preview: "abcdef".into(),
            last_used_at: None,
            revoked_at: None,
            created_at: JsDate::from_millis(0).unwrap(),
            scope: "upload".into(),
        });
        let v = crate::rpc::tagged::to_tagged_value(&row).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["id", "userId", "name", "scope", "tokenHash", "preview", "lastUsedAt", "revokedAt", "createdAt"]
        );
        assert_eq!(v["createdAt"], json!({ "$date": "1970-01-01T00:00:00.000Z" }));
    }
}

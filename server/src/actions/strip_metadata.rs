//! Not upstream (SKELETON.md 9.47): the Settings card "Remove metadata from
//! files on upload", module `strip-metadata`, called by the SPA's
//! `src/strip-metadata/actions.ts`.

use serde_json::Value;

use crate::data::strip_metadata::{strip_status, write_switch};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("strip-metadata", "loadStripMetadata", load_strip_metadata);
    reg.register("strip-metadata", "setStripMetadata", set_strip_metadata);
}

/// The card's status (`StripStatus`).
async fn load_strip_metadata(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    crate::rpc::ok(&strip_status(&ctx.state.db, &session.user.id).await?)
}

/// `setStripMetadata(enabled: boolean)`: the status after the change. Files
/// already uploaded are not touched.
async fn set_strip_metadata(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let Some(Value::Bool(enabled)) = ctx.args.raw(0) else {
        return Err(ActionError::thrown("enabled must be true or false"));
    };
    write_switch(&ctx.state.db, &session.user.id, *enabled).await?;
    crate::rpc::ok(&strip_status(&ctx.state.db, &session.user.id).await?)
}

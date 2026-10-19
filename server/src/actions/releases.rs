//! Port of `app/actions/releases.ts`: publishing the desktop app's release,
//! from the panel.
//!
//! Until this existed the only way to point the channel at a build was
//! `POST /api/releases/app` with an admin token. That is fine for a script and
//! wrong for a person: it means anybody cutting a release has to mint, carry and
//! paste a credential to do something they are already signed in to do. The
//! upload itself is a normal upload (the release is one settings row naming its
//! code), so all this needs is a form.
//!
//! Everything here re-checks admin on the server. A server action is a public
//! endpoint with a nicer calling convention; the button being hidden proves
//! nothing.
//!
//! The single argument of `publishAppRelease` is read with JavaScript's
//! semantics, in upstream's order: `lastPathSegment(input.code)` first (a
//! missing `input` or a non-string `code` throws), then `input.version.trim()`
//! (throws for a non-string), and `input.notes?.trim()` only when the pointer
//! is written (see [`ReleaseNotes`]).

use serde::Serialize;
use serde_json::{Value, json};

use crate::config::js_trim;
use crate::releases::{
    APP_NAME, PublishInput, ReleaseManifest, ReleaseNotes, current_release, publish_release, unpublish_release,
};
use crate::rpc::tagged::is_undefined;
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("releases", "loadRelease", load_release);
    reg.register("releases", "publishAppRelease", publish_app_release);
    reg.register("releases", "unpublishAppRelease", unpublish_app_release);
}

/// `ReleaseState`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseState {
    pub manifest: Option<ReleaseManifest>,
    pub app_name: &'static str,
}

/// The pages that show the channel.
const REVALIDATED: [&str; 3] = ["/dashboard/admin", "/dashboard/tokens", "/download"];

async fn load_release(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let manifest = current_release(&ctx.state.db, &ctx.state.config).await?;
    crate::rpc::ok(&ReleaseState { manifest, app_name: APP_NAME })
}

/// `input.<key>`: a missing, `undefined` or `null` input is a TypeError; any
/// other non-object simply has no such property.
fn property<'a>(input: Option<&'a Value>, key: &str) -> Result<Option<&'a Value>, ActionError> {
    match input {
        None => Err(ActionError::thrown(format!("Cannot read properties of undefined (reading '{key}')"))),
        Some(v) if is_undefined(v) => {
            Err(ActionError::thrown(format!("Cannot read properties of undefined (reading '{key}')")))
        }
        Some(Value::Null) => Err(ActionError::thrown(format!("Cannot read properties of null (reading '{key}')"))),
        Some(Value::Object(map)) => Ok(map.get(key)),
        Some(_) => Ok(None),
    }
}

/// `<expr>.trim()` on a value: the string, or the TypeError JavaScript throws.
fn trimmable<'a>(value: Option<&'a Value>, expr: &str) -> Result<&'a str, ActionError> {
    match value {
        Some(Value::String(s)) => Ok(s),
        None => Err(ActionError::thrown("Cannot read properties of undefined (reading 'trim')")),
        Some(v) if is_undefined(v) => Err(ActionError::thrown("Cannot read properties of undefined (reading 'trim')")),
        Some(Value::Null) => Err(ActionError::thrown("Cannot read properties of null (reading 'trim')")),
        Some(_) => Err(ActionError::thrown(format!("{expr}.trim is not a function"))),
    }
}

/// `input.notes` as `publishRelease` will treat it (`input.notes?.trim()`).
fn notes_of(value: Option<&Value>) -> ReleaseNotes<'_> {
    match value {
        None | Some(Value::Null) => ReleaseNotes::Absent,
        Some(v) if is_undefined(v) => ReleaseNotes::Absent,
        Some(Value::String(s)) => ReleaseNotes::Text(s),
        Some(_) => ReleaseNotes::NotAString,
    }
}

/// The code out of whatever was pasted: a bare code, a full link, or a link
/// with an extension. The extension is decorative in this app's URLs (it is
/// trimmed at the dot), so it is trimmed here too rather than being sent on to
/// fail a lookup.
pub fn last_path_segment(raw: &str) -> String {
    let trimmed = js_trim(raw);
    if trimmed.is_empty() {
        return String::new();
    }
    let without_query = trimmed.split(['?', '#']).next().unwrap_or("");
    let segment = without_query.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    match segment.find('.') {
        Some(dot) if dot > 0 => segment[..dot].to_string(),
        _ => segment.to_string(),
    }
}

/// Point the channel at an upload.
///
/// `code` is the short code out of the file's own link, which is what an admin
/// has in front of them after uploading the installer, not an internal id they
/// would have to go looking for.
async fn publish_app_release(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let input = ctx.args.raw(0);

    // Accept a whole link as well as a bare code. Somebody who has just uploaded
    // the installer has the URL on their clipboard, and making them edit it down
    // by hand is the kind of small rudeness that gets a step skipped later.
    let code = last_path_segment(trimmable(property(input, "code")?, "raw")?);
    let version = trimmable(property(input, "version")?, "input.version")?;
    let notes = notes_of(property(input, "notes")?);
    // Second click on "publish anyway" after the file name disagreed: the
    // literal `true` only.
    let allow_version_mismatch = property(input, "allowVersionMismatch")? == Some(&Value::Bool(true));

    let result = publish_release(
        &ctx.state.db,
        &ctx.state.config,
        PublishInput { version, code: &code, notes, allow_version_mismatch },
    )
    .await?;
    if result.is_ok() {
        for path in REVALIDATED {
            ctx.revalidate_path(path);
        }
    }
    Ok(result.to_json())
}

async fn unpublish_app_release(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    unpublish_release(&ctx.state.db).await?;
    for path in REVALIDATED {
        ctx.revalidate_path(path);
    }
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    //! tests/unit/release-action.test.ts. "Checks admin before doing anything"
    //! is asserted on behaviour in tests/releases.rs (each action answers the
    //! `/login` redirect without a session on a server whose database cannot be
    //! reached, so nothing ran before the guard).
    use super::*;

    #[test]
    fn what_counts_as_a_code() {
        // Somebody publishing a release has the file's LINK on their clipboard,
        // not its bare code. Making them edit it by hand is how a step gets skipped.
        let cases = [
            ("abcde12345", "abcde12345"),
            ("https://host.example.com/alicja/abcde12345.exe", "abcde12345"),
            ("https://host.example.com/alicja/abcde12345", "abcde12345"),
            ("  https://h.example.com/alicja/abcde12345.exe?x=1  ", "abcde12345"),
            // The documented edges: a raw link names `raw`, a leading dot stays.
            ("https://h/tag/abc/raw.png", "raw"),
            (".hidden", ".hidden"),
            ("a/b#frag/c", "b"),
            ("   ", ""),
            ("///", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(last_path_segment(input), expected, "{input:?}");
        }
    }

    #[test]
    fn the_input_is_read_like_javascript_reads_it() {
        assert!(property(None, "code").is_err());
        assert!(property(Some(&json!({ "$undefined": 1 })), "code").is_err());
        assert!(property(Some(&Value::Null), "code").is_err());
        assert_eq!(property(Some(&json!("a string")), "code").unwrap(), None);
        assert_eq!(property(Some(&json!([1])), "code").unwrap(), None);
        assert_eq!(property(Some(&json!({ "code": "x" })), "code").unwrap(), Some(&json!("x")));
        assert_eq!(trimmable(Some(&json!("v1")), "input.version").unwrap(), "v1");
        for not_a_string in [json!(5), json!(true), json!({ "$date": "2026-10-18T12:00:00.000Z" }), json!([])] {
            assert!(trimmable(Some(&not_a_string), "input.version").is_err());
        }
        assert_eq!(notes_of(None), ReleaseNotes::Absent);
        assert_eq!(notes_of(Some(&json!({ "$undefined": 1 }))), ReleaseNotes::Absent);
        assert_eq!(notes_of(Some(&Value::Null)), ReleaseNotes::Absent);
        assert_eq!(notes_of(Some(&json!("x"))), ReleaseNotes::Text("x"));
        assert_eq!(notes_of(Some(&json!(1))), ReleaseNotes::NotAString);
    }
}

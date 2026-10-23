//! Loader of `app/(auth)/authorize/page.tsx`: the permission prompt a desktop
//! client sends people to (CONTRACT 6.10).
//!
//! It lives in the (auth) route group rather than under (app) for a reason
//! that is not cosmetic: the app shell's `requireUser()` redirects to a bare
//! `/login`, which would drop `?port=&state=` on the floor, and the request
//! cannot be reconstructed afterwards: the app is already listening on a port
//! nobody remembers. Here the redirect is built by hand and carries the whole
//! request back through the sign-in (and 2FA) round trip.
//!
//! The page has NO side effect beyond one in-memory note: rendering it mints
//! nothing (see [`crate::actions::authorize`]); a request carrying the app's
//! pickup `challenge` (not upstream) is registered as pending, so the app's
//! polls can tell "waiting for the person" from "unknown".
//!
//! Props (`AuthorizePageProps` in the SPA): `parsed` is
//! `parseAuthorizeRequest({ port, state, name })` of the query, `session` is
//! `{ user: { email } }` when the request is valid and somebody is signed in,
//! `null` otherwise (an invalid request never looks the session up; a valid
//! one without a session is a redirect, not a page). Not upstream:
//! `confirmCode` (`XXX-XXX`) when the request carries a pickup challenge, the
//! code the app shows next to its waiting spinner (review 2026-10-24, P2).

use serde_json::{Value, json};

use crate::authorize::loopback::{authorize_path, parse_authorize_request_with_challenge};
use crate::http::cookies::encode_uri_component;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageError, PageResult};

/// `generateMetadata`: a permission prompt has no business in a search index,
/// and the URL it is reached by carries somebody's one-off nonce.
fn metadata(ctx: &PageCtx) -> PageMeta {
    PageMeta::titled(ctx.t("authorize").t("title")).noindex()
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let query = ctx.search.to_value();
    let meta = metadata(&ctx);

    // Validated BEFORE the session check on purpose: a broken link should say
    // so rather than march an anonymous visitor through a sign-in to reach a
    // dead end. Nothing here is account-specific, so it reveals nothing either.
    // `challenge` is not upstream: the app's pickup (SKELETON.md 9).
    let parsed = parse_authorize_request_with_challenge(
        query.get("port"),
        query.get("state"),
        query.get("name"),
        query.get("challenge"),
    );
    let Some(request) = parsed.request() else {
        return PageData::new(&json!({ "parsed": parsed, "session": Value::Null }), meta);
    };

    let Some(session) = ctx.get_session().await? else {
        return Err(PageError::redirect(format!("/login?next={}", encode_uri_component(&authorize_path(request)))));
    };

    // The only thing this page writes, and it hands out nothing: from here on
    // the app's pickup polls read "pending" instead of "not found", and the
    // state is bound to this challenge AND this account: only this account's
    // approval or refusal reaches the app (crate::authorize::pickup, review
    // 2026-10-24 P1). Rate-limited and capped per account (P3).
    let mut props = json!({ "parsed": parsed, "session": { "user": { "email": session.user.email } } });
    if let Some(challenge) = &request.challenge {
        use crate::authorize::pickup::{Registered, confirmation_code, now_ms, register_render, store};
        let registered =
            register_render(store(), &request.state, challenge, &session.user.id, &ctx.client_ip(), now_ms());
        if registered != Registered::Bound {
            tracing::warn!("[authorize] pickup not registered: {registered:?}");
        }
        // P2: the code the app shows, so the person can compare before approving.
        props["confirmCode"] = Value::String(confirmation_code(challenge));
    }

    PageData::new(&props, meta)
}

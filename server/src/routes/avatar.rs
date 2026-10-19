//! Port of `app/avatar/[userCode]/route.ts` and `app/bg/[userCode]/route.ts`:
//! an account's avatar (`GET /avatar/{tag}[?v=token]`) and wallpaper
//! (`GET /bg/{tag}[?v=token]`).
//!
//! Public and unauthenticated (they appear next to the account's name and
//! behind its public pages). The bytes are always a WebP this app encoded
//! itself. Keyed by tag and resolved through `user_code_aliases`, so both
//! survive a rename. `immutable` caching only when `v` names the key's own
//! token: replacing the picture mints a new token, so the old URL is never
//! asked again.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use sqlx::PgPool;

use crate::avatar::{AVATAR_CONTENT_TYPE, avatar_token, is_avatar_key};
use crate::background::{BACKGROUND_CONTENT_TYPE, background_token};
use crate::error::AppResult;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/avatar/{userCode}", get(avatar)).route("/bg/{userCode}", get(background))
}

/// Which column of `user` a picture lives in.
#[derive(Clone, Copy)]
enum Picture {
    Avatar,
    Background,
}

impl Picture {
    fn column(self) -> &'static str {
        match self {
            Picture::Avatar => "image",
            Picture::Background => "background_key",
        }
    }
}

/// Current tag first, then retired ones. A row found by its current tag ends
/// the search even when its column is empty.
async fn lookup_key(db: &PgPool, picture: Picture, user_code: &str) -> sqlx::Result<Option<String>> {
    let col = picture.column();
    let direct: Option<Option<String>> =
        sqlx::query_scalar(&format!(r#"select "{col}" from "user" where "user"."user_code" = $1 limit 1"#))
            .bind(user_code)
            .fetch_optional(db)
            .await?;
    if let Some(key) = direct {
        return Ok(key);
    }
    let via_alias: Option<Option<String>> = sqlx::query_scalar(&format!(
        r#"select "user"."{col}" from "user_code_aliases" inner join "user" on "user"."id" = "user_code_aliases"."user_id"
           where "user_code_aliases"."code" = $1 limit 1"#
    ))
    .bind(user_code)
    .fetch_optional(db)
    .await?;
    Ok(via_alias.flatten())
}

fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

/// `new URL(req.url).searchParams.get("v")`
fn asked_token(query: Option<&str>) -> Option<String> {
    form_urlencoded::parse(query.unwrap_or("").as_bytes()).find(|(k, _)| k == "v").map(|(_, v)| v.into_owned())
}

async fn serve(state: &AppState, key: &str, token: String, asked: Option<String>, content_type: &str) -> Response {
    let immutable = asked.as_deref().is_some_and(|a| !a.is_empty() && a == token);
    let Ok(storage) = state.storage.get() else {
        return not_found();
    };
    let Ok(Some(stat)) = storage.stat(key).await else {
        return not_found();
    };
    let Ok(stream) = storage.read_stream(key, None).await else {
        return not_found();
    };
    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).unwrap_or(HeaderValue::from_static("image/webp")),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(stat.size));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=60, must-revalidate"
        }),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("inline"));
    res
}

async fn avatar(
    State(state): State<AppState>,
    Path(user_code): Path<String>,
    RawQuery(query): RawQuery,
) -> AppResult<Response> {
    let key = lookup_key(&state.db, Picture::Avatar, &user_code).await?;
    // Anything that is not one of our own keys is not ours to stream.
    let Some(key) = key.filter(|k| is_avatar_key(Some(k))) else {
        return Ok(not_found());
    };
    let token = avatar_token(Some(&key));
    Ok(serve(&state, &key, token, asked_token(query.as_deref()), AVATAR_CONTENT_TYPE).await)
}

async fn background(
    State(state): State<AppState>,
    Path(user_code): Path<String>,
    RawQuery(query): RawQuery,
) -> AppResult<Response> {
    let Some(key) = lookup_key(&state.db, Picture::Background, &user_code).await?.filter(|k| !k.is_empty()) else {
        return Ok(not_found());
    };
    let token = background_token(Some(&key));
    Ok(serve(&state, &key, token, asked_token(query.as_deref()), BACKGROUND_CONTENT_TYPE).await)
}

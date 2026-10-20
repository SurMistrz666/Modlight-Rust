//! Port of `app/embed/[version]/[card]/route.ts`: `/embed/v1/archive-ZIP.png`,
//! the picture a link preview shows for a file that has no picture of its own.
//!
//! The URL names a CATEGORY and a TOKEN, never a file, and that is load
//! bearing: nothing here can consume a view (there is no row to consume one
//! from - this module touches neither the database nor the serving module), it is
//! not an enumeration oracle (every ZIP points at the same address), and
//! unfurlers get one cache entry per category instead of one per upload. Both
//! segments are checked against closed lists before anything is drawn.

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;

use crate::http::web_response::put_headers;
use crate::media::card::{EMBED_CARD_VERSION, parse_card_file};
use crate::media::part_a::render_card;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/embed/{version}/{card}", get(card))
}

fn not_found() -> Response {
    let mut res = Response::new(Body::from("Not found"));
    *res.status_mut() = StatusCode::NOT_FOUND;
    put_headers(
        &mut res,
        [
            ("Content-Type", "text/plain;charset=UTF-8".to_string()),
            ("Cache-Control", "public, max-age=3600".to_string()),
        ],
    );
    res
}

async fn card(Path((version, card)): Path<(String, String)>) -> Response {
    if version != EMBED_CARD_VERSION {
        return not_found();
    }
    let Some(choice) = parse_card_file(&card) else {
        return not_found();
    };
    let png = match render_card(&choice).await {
        Ok(png) => png,
        Err(err) => {
            tracing::error!(error = ?err, "card render failed");
            return crate::error::route_handler_error_response();
        }
    };
    let len = png.len();
    let mut res = Response::new(Body::from(png));
    put_headers(
        &mut res,
        [
            ("Content-Type", "image/png".to_string()),
            ("Content-Length", len.to_string()),
            ("Content-Disposition", "inline".to_string()),
            ("X-Content-Type-Options", "nosniff".to_string()),
            // Safe to freeze forever precisely because redrawing the art means
            // bumping EMBED_CARD_VERSION, which mints new URLs.
            ("Cache-Control", "public, max-age=31536000, immutable".to_string()),
        ],
    );
    res
}

#[cfg(test)]
mod tests {
    //! tests/unit/embed-card.test.ts, "the card route cannot touch a file".
    #[test]
    fn imports_neither_the_database_nor_the_media_server() {
        let source = include_str!("embed_card.rs");
        let code = &source[..source.find("#[cfg(test)]").unwrap()];
        for banned in ["crate::db", "sqlx", "media::serve", "get_media", "consume_view", "lookup_media"] {
            assert!(!code.contains(banned), "{banned}");
        }
        assert!(code.contains("max-age=31536000, immutable"));
    }
}

//! Responses built the way a Next route handler builds them with the Fetch
//! `Response` constructor: `new Response("text")` gets the Fetch default
//! `text/plain;charset=UTF-8`, and a headers object is applied key by key.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;

/// `new Response(text, { status })`: the Fetch default content type.
pub fn plain_text(status: StatusCode, body: &str) -> Response {
    let mut res = Response::new(Body::from(body.to_string()));
    *res.status_mut() = status;
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain;charset=UTF-8"));
    res
}

/// Insert `(name, value)` pairs as a JavaScript headers object would (a later
/// key replaces an earlier one).
pub fn put_headers<'a>(res: &mut Response, pairs: impl IntoIterator<Item = (&'a str, String)>) {
    for (name, value) in pairs {
        if let (Ok(n), Ok(v)) = (header::HeaderName::try_from(name), HeaderValue::from_str(&value)) {
            res.headers_mut().insert(n, v);
        }
    }
}

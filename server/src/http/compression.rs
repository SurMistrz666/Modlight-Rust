//! Next's response compression (`compress: true`, the default; Next runs the
//! `compression` 1.7.4 middleware), on the responses Next renders itself.
//!
//! Seen on the reference (127.0.0.1, `Accept-Encoding: gzip, deflate, br`):
//!
//! | Response | Upstream | Here |
//! |---|---|---|
//! | page documents (pages, not-found, redirect and error documents) | gzip | [`Compress::Rendered`] |
//! | RSC payloads (`text/x-component`) | gzip | `/_page` and `/_rpc`, [`Compress::Rendered`] |
//! | `/_next/static/*`, `public/` files of a compressible type | gzip, under 1 KiB not | `/assets/*` and `public/`, [`Compress::Static`] |
//! | route handlers (`/api/*`, `/raw`, `/download`, `/oembed`, `/embed`, `/icon.svg`...) | never | never (no marker) |
//! | `server.ts`' raw upload paths | never | never |
//!
//! The middleware's own rules, in its order: a compressible `Content-Type`
//! (mime-db's `compressible`, or `text/*`, `+json`, `+text`, `+xml`), no
//! `Cache-Control: no-transform`, then `Vary: Accept-Encoding` is added, then
//! the 1 KiB threshold when the length is known (static files; Next streams
//! everything it renders, so those have no length and are always eligible),
//! nothing already encoded, not `HEAD`, and gzip or deflate acceptable
//! (gzip preferred over deflate on a tie; `br` is not offered by 1.7.4).
//! Partial content (`206`, ranges of a static file) is never compressed.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt;
use std::io::Write;

/// Which of Next's own responses this is (an extension on the response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compress {
    /// Rendered by Next (documents, RSC): always eligible.
    Rendered,
    /// A file from disk: eligible from 1 KiB.
    Static,
}

/// The `threshold` default of compression 1.7.4 (`'1kb'`).
pub const THRESHOLD: u64 = 1024;

/// Mark a response as one Next would compress.
pub fn mark(mut res: Response, kind: Compress) -> Response {
    res.extensions_mut().insert(kind);
    res
}

/// `compressible(type)`: mime-db's `compressible: true` types this server
/// can send, and the generic rule of the `compressible` package.
pub fn compressible(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if essence.starts_with("text/")
        || essence.ends_with("+json")
        || essence.ends_with("+text")
        || essence.ends_with("+xml")
    {
        return true;
    }
    matches!(
        essence.as_str(),
        "application/javascript"
            | "application/x-javascript"
            | "application/ecmascript"
            | "application/json"
            | "application/xml"
            | "application/xhtml+xml"
            | "application/manifest+json"
            | "application/wasm"
            | "image/svg+xml"
            | "image/x-icon"
            | "image/vnd.microsoft.icon"
            | "image/bmp"
            | "font/ttf"
            | "font/otf"
            | "application/x-font-ttf"
            | "application/vnd.ms-fontobject"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Gzip,
    Deflate,
}

/// `accepts(req).encoding(['gzip', 'deflate', 'identity'])`, then "we really
/// don't prefer deflate": gzip whenever it is acceptable at the best quality.
fn negotiate(accept_encoding: Option<&HeaderValue>) -> Option<Encoding> {
    let raw = accept_encoding.and_then(|v| v.to_str().ok())?;
    let mut gzip: Option<f32> = None;
    let mut deflate: Option<f32> = None;
    let mut star: Option<f32> = None;
    for part in raw.split(',') {
        let mut pieces = part.split(';');
        let name = pieces.next().unwrap_or("").trim().to_ascii_lowercase();
        let mut q = 1.0f32;
        for p in pieces {
            if let Some(v) = p.trim().strip_prefix("q=").or_else(|| p.trim().strip_prefix("Q=")) {
                q = v.trim().parse().unwrap_or(0.0);
            }
        }
        match name.as_str() {
            "gzip" | "x-gzip" => gzip = Some(q),
            "deflate" => deflate = Some(q),
            "*" => star = Some(q),
            _ => {}
        }
    }
    let gzip = gzip.or(star).unwrap_or(0.0);
    let deflate = deflate.or(star).unwrap_or(0.0);
    if gzip > 0.0 && gzip >= deflate {
        Some(Encoding::Gzip)
    } else if deflate > 0.0 {
        Some(Encoding::Deflate)
    } else {
        None
    }
}

fn append_vary(headers: &mut HeaderMap) {
    let existing: Vec<String> = headers
        .get_all(header::VARY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    if existing.iter().any(|v| v == "*" || v.eq_ignore_ascii_case("accept-encoding")) {
        return;
    }
    let mut all = existing;
    all.push("Accept-Encoding".to_string());
    if let Ok(v) = HeaderValue::from_str(&all.join(", ")) {
        headers.insert(header::VARY, v);
    }
}

/// Compress `res` the way Next would, when it carries a [`Compress`] mark.
pub async fn apply(accept_encoding: Option<&HeaderValue>, method: &Method, res: Response) -> Response {
    let Some(kind) = res.extensions().get::<Compress>().copied() else {
        return res;
    };
    let status = res.status();
    if status == StatusCode::PARTIAL_CONTENT || status == StatusCode::NO_CONTENT || status == StatusCode::NOT_MODIFIED {
        return res;
    }
    let headers = res.headers();
    let Some(content_type) = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return res;
    };
    if !compressible(content_type) {
        return res;
    }
    let no_transform = headers
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(',').any(|d| d.trim().eq_ignore_ascii_case("no-transform")));
    if no_transform {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    append_vary(&mut parts.headers);
    let length = parts.headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    let already = parts.headers.get(header::CONTENT_ENCODING).is_some_and(|v| v.as_bytes() != b"identity");
    let small = kind == Compress::Static && length.is_some_and(|n| n < THRESHOLD);
    let encoding = negotiate(accept_encoding);
    if small || already || method == Method::HEAD || encoding.is_none() {
        return Response::from_parts(parts, body);
    }
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            tracing::warn!("could not read a response body to compress it: {err}");
            return crate::error::route_handler_error_response();
        }
    };
    let compressed = match encoding {
        Some(Encoding::Gzip) => {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(&bytes).and_then(|_| enc.finish())
        }
        _ => {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(&bytes).and_then(|_| enc.finish())
        }
    };
    let Ok(compressed) = compressed else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    parts.headers.insert(
        header::CONTENT_ENCODING,
        HeaderValue::from_static(if encoding == Some(Encoding::Gzip) { "gzip" } else { "deflate" }),
    );
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(compressed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn ae(v: &str) -> Option<HeaderValue> {
        Some(HeaderValue::from_str(v).unwrap())
    }

    #[test]
    fn negotiates_like_compression_1_7() {
        assert_eq!(negotiate(ae("gzip, deflate, br").as_ref()), Some(Encoding::Gzip));
        assert_eq!(negotiate(ae("deflate").as_ref()), Some(Encoding::Deflate));
        assert_eq!(negotiate(ae("br").as_ref()), None);
        assert_eq!(negotiate(ae("gzip;q=0, deflate").as_ref()), Some(Encoding::Deflate));
        assert_eq!(negotiate(ae("*").as_ref()), Some(Encoding::Gzip));
        assert_eq!(negotiate(ae("identity").as_ref()), None);
        assert_eq!(negotiate(None), None);
    }

    #[test]
    fn compressible_types() {
        for t in [
            "text/html; charset=utf-8",
            "application/json",
            "text/x-component",
            "image/svg+xml",
            "font/ttf",
            "application/javascript; charset=UTF-8",
            "text/css",
        ] {
            assert!(compressible(t), "{t}");
        }
        for t in ["image/png", "image/webp", "video/mp4", "application/octet-stream", "font/woff2", "application/zip"] {
            assert!(!compressible(t), "{t}");
        }
    }

    fn response(kind: Option<Compress>, status: StatusCode, ctype: &str, body: Vec<u8>) -> Response {
        let len = body.len();
        let mut res = Response::new(Body::from(body));
        *res.status_mut() = status;
        res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_str(ctype).unwrap());
        res.headers_mut().insert(header::CONTENT_LENGTH, HeaderValue::from(len));
        match kind {
            Some(k) => mark(res, k),
            None => res,
        }
    }

    async fn body(res: Response) -> Vec<u8> {
        res.into_body().collect().await.unwrap().to_bytes().to_vec()
    }

    #[tokio::test]
    async fn compresses_only_what_next_compresses() {
        let html = b"<!doctype html>".repeat(10);
        let gz = ae("gzip, deflate, br");
        // A rendered document: gzip, Vary, no stale length.
        let res = apply(
            gz.as_ref(),
            &Method::GET,
            response(Some(Compress::Rendered), StatusCode::OK, "text/html", html.clone()),
        )
        .await;
        assert_eq!(res.headers()[header::CONTENT_ENCODING], "gzip");
        assert_eq!(res.headers()[header::VARY], "Accept-Encoding");
        assert!(res.headers().get(header::CONTENT_LENGTH).is_none());
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&body(res).await[..]).read_to_end(&mut out).unwrap();
        assert_eq!(out, html);
        // Route handlers (no mark), partial content, media types, HEAD,
        // no-transform and small static files stay as they are.
        let res =
            apply(gz.as_ref(), &Method::GET, response(None, StatusCode::OK, "application/json", html.clone())).await;
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
        let res = apply(
            gz.as_ref(),
            &Method::GET,
            response(Some(Compress::Static), StatusCode::PARTIAL_CONTENT, "text/css", html.clone()),
        )
        .await;
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
        let res = apply(
            gz.as_ref(),
            &Method::GET,
            response(Some(Compress::Static), StatusCode::OK, "font/woff2", html.clone()),
        )
        .await;
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
        assert!(res.headers().get(header::VARY).is_none());
        let res = apply(
            gz.as_ref(),
            &Method::HEAD,
            response(Some(Compress::Rendered), StatusCode::OK, "text/html", html.clone()),
        )
        .await;
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(res.headers()[header::VARY], "Accept-Encoding");
        let small = apply(
            gz.as_ref(),
            &Method::GET,
            response(Some(Compress::Static), StatusCode::OK, "text/css", b"a{}".to_vec()),
        )
        .await;
        assert!(small.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(small.headers()[header::VARY], "Accept-Encoding");
        let mut nt = response(Some(Compress::Rendered), StatusCode::OK, "text/html", html.clone());
        nt.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-transform"));
        assert!(apply(gz.as_ref(), &Method::GET, nt).await.headers().get(header::CONTENT_ENCODING).is_none());
        // A client that offers neither gzip nor deflate.
        let res = apply(
            ae("br").as_ref(),
            &Method::GET,
            response(Some(Compress::Rendered), StatusCode::OK, "text/html", html.clone()),
        )
        .await;
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(body(res).await, html);
    }
}

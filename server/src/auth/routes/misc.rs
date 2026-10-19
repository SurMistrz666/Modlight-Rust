//! `api/routes/ok.mjs` and `api/routes/error.mjs`.
//!
//! `/error` in production (and Modlight always runs with NODE_ENV=production)
//! redirects to `/?error=<code>[&error_description=..]`; with NODE_ENV unset
//! it renders Better Auth's built-in error page instead.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::json;

use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Out};

pub fn endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/ok",
            methods: &[Method::GET],
            body: None,
            query: None,
            media_types: &[],
            middleware: &[],
            handler: ok,
        },
        Endpoint {
            path: "/error",
            methods: &[Method::GET],
            body: None,
            query: None,
            media_types: &[],
            middleware: &[],
            handler: error,
        },
    ]
}

fn ok(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move { ctx.json(json!({ "ok": true })) })
}

/// `sanitize(input)`: HTML-escape, leaving existing entities alone.
fn sanitize(input: &str) -> String {
    let escaped = input.replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;");
    // `&(?!amp;|lt;|gt;|quot;|#39;|#x[0-9a-fA-F]+;|#[0-9]+;)` -> `&amp;`
    let mut out = String::with_capacity(escaped.len());
    for (i, ch) in escaped.char_indices() {
        if ch == '&' {
            let tail = &escaped[i + 1..];
            let is_entity =
                ["amp;", "lt;", "gt;", "quot;", "#39;"].iter().any(|e| tail.starts_with(e)) || entity_number(tail);
            out.push_str(if is_entity { "&" } else { "&amp;" });
        } else {
            out.push(ch);
        }
    }
    out
}

fn entity_number(tail: &str) -> bool {
    if let Some(hex) = tail.strip_prefix("#x") {
        let digits: usize = hex.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        return digits > 0 && hex[digits..].starts_with(';');
    }
    if let Some(dec) = tail.strip_prefix('#') {
        let digits: usize = dec.chars().take_while(|c| c.is_ascii_digit()).count();
        return digits > 0 && dec[digits..].starts_with(';');
    }
    false
}

fn error(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let url = url::Url::parse(&ctx.request_url).map_err(|e| anyhow::anyhow!("Invalid URL: {e}"))?;
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let first = |k: &str| pairs.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        let code = first("error").filter(|c| !c.is_empty()).unwrap_or_else(|| "UNKNOWN".into());
        let description = first("error_description").filter(|d| !d.is_empty());
        let safe_code =
            if !code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '\'' | '_' | '-')) {
                code
            } else {
                "UNKNOWN".to_string()
            };
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.append_pair("error", &safe_code);
        if let Some(d) = &description {
            query.append_pair("error_description", d);
        }
        let query = query.finish();
        if ctx.state.config.is_prod {
            return Ok(Out::Redirect { status: 302, location: format!("/?{query}") });
        }
        Ok(Out::Html(error_page(&ctx.state.config.app_name, &safe_code, description.as_deref().map(sanitize))))
    })
}

/// The development error page (a compact rendition of better-auth's: the
/// page only exists outside production).
fn error_page(app_name: &str, code: &str, description: Option<String>) -> String {
    let code = sanitize(code);
    let description = description.unwrap_or_default();
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"UTF-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\"><title>Error</title></head>\n<body><main><h1>Error</h1><p>{}</p><code>{code}</code><p>{description}</p><a href=\"/\">Go Home</a></main></body>\n</html>",
        crate::pages::meta::escape_attr(app_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_entities() {
        assert_eq!(
            sanitize("<a href='x'>&amp; & &#39; &#x41; &zz</a>"),
            "&lt;a href=&#39;x&#39;&gt;&amp; &amp; &#39; &#x41; &amp;zz&lt;/a&gt;"
        );
    }
}

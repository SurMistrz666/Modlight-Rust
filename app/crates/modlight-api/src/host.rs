//! Addresses and credentials as text: what the sign-in box accepts, which links the
//! token may follow, and how a value is put into a URL.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

/// Everything outside RFC 3986 "unreserved" is escaped, as `Uri.EscapeDataString` does.
const DATA: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `Uri.EscapeDataString`: a value made safe to stand anywhere in a URL.
pub fn escape_data_string(value: &str) -> String {
    utf8_percent_encode(value, DATA).to_string()
}

/// Turn whatever the user typed into an origin: scheme and authority, nothing else.
///
/// Every endpoint hangs off the root, so a pasted "/dashboard" or a trailing slash is
/// dropped rather than concatenated into a 404. A bare `files.example.com` gets https:
/// the alternative is sending somebody's token over plaintext for two saved keystrokes.
/// An address typed WITH `http://` keeps it, which is what the local reference server
/// (`http://127.0.0.1:8420`) and any other plain-HTTP deployment needs; upstream draws
/// the same line.
///
/// The authority is rebuilt from the parsed URL, so user info, path, query and fragment
/// are dropped and a default port is normalised away.
pub fn try_normalize_host(input: &str) -> Option<String> {
    let mut value = input.trim().to_string();
    if value.is_empty() {
        return None;
    }
    if !value.contains("://") {
        value = format!("https://{value}");
    }
    let url = Url::parse(&value).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?;
    if host.is_empty() {
        return None;
    }
    Some(origin_of(&url))
}

fn origin_of(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}

/// Whether a string can be sent as a header value at all.
///
/// The manual paste box is the one place a token arrives as free text, and a value with
/// a newline in it is a request header of somebody else's choosing. Visible ASCII only,
/// at most 512 characters, which every token the server has ever issued already is.
pub fn is_plausible_token(token: &str) -> bool {
    if token.is_empty() || token.chars().count() > 512 {
        return false;
    }
    token.chars().all(|c| c > ' ' && c <= '~')
}

/// Point a link the server built at the deployment we hold a token for.
///
/// Links come back on the account's PREFERRED hostname, which can be a different name
/// from the one signed in to. Every one of them serves the same paths, so swapping the
/// origin costs nothing and buys the rule that matters: the token only ever goes to the
/// host that issued it. A link that is not absolute http(s) is refused, not patched up.
pub fn rewrite_to_host(link: &str, host: &str) -> Option<String> {
    let link = Url::parse(link.trim()).ok()?;
    if link.scheme() != "http" && link.scheme() != "https" {
        return None;
    }
    let origin = Url::parse(host).ok()?;
    origin.host_str()?;
    let mut out = origin_of(&origin);
    out.push_str(link.path());
    if let Some(query) = link.query() {
        out.push('?');
        out.push_str(query);
    }
    Some(out)
}

/// Whether two origins name the same deployment (scheme, host and port).
pub fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_hosts_like_the_windows_client() {
        assert_eq!(
            try_normalize_host("files.example.com").as_deref(),
            Some("https://files.example.com")
        );
        assert_eq!(
            try_normalize_host("  https://Files.Example.com/dashboard?x=1#y ").as_deref(),
            Some("https://files.example.com")
        );
        assert_eq!(
            try_normalize_host("https://files.example.com:443/").as_deref(),
            Some("https://files.example.com")
        );
        assert_eq!(
            try_normalize_host("http://127.0.0.1:8420").as_deref(),
            Some("http://127.0.0.1:8420")
        );
        assert_eq!(
            try_normalize_host("http://user:pass@127.0.0.1:8420/x").as_deref(),
            Some("http://127.0.0.1:8420")
        );
        assert_eq!(
            try_normalize_host("127.0.0.1:8420").as_deref(),
            Some("https://127.0.0.1:8420")
        );
        assert_eq!(try_normalize_host(""), None);
        assert_eq!(try_normalize_host("   "), None);
        assert_eq!(try_normalize_host("ftp://files.example.com"), None);
        assert_eq!(try_normalize_host("javascript://alert(1)"), None);
        assert_eq!(try_normalize_host("https://"), None);
    }

    #[test]
    fn plausible_tokens_are_visible_ascii() {
        assert!(is_plausible_token("abc_DEF-123"));
        assert!(!is_plausible_token(""));
        assert!(!is_plausible_token("abc def"));
        assert!(!is_plausible_token("abc\ndef"));
        assert!(!is_plausible_token("zażółć"));
        assert!(!is_plausible_token(&"a".repeat(513)));
        assert!(is_plausible_token(&"a".repeat(512)));
    }

    #[test]
    fn links_are_moved_onto_the_signed_in_origin() {
        assert_eq!(
            rewrite_to_host(
                "https://vanity.example.org/u/abc.png?w=480&h=480#frag",
                "http://127.0.0.1:8420"
            )
            .as_deref(),
            Some("http://127.0.0.1:8420/u/abc.png?w=480&h=480")
        );
        assert_eq!(
            rewrite_to_host("/relative/path", "http://127.0.0.1:8420"),
            None
        );
        assert_eq!(
            rewrite_to_host("file:///etc/passwd", "http://127.0.0.1:8420"),
            None
        );
        assert_eq!(rewrite_to_host("https://x.example/a", "not a url"), None);
    }

    #[test]
    fn escapes_like_escape_data_string() {
        assert_eq!(
            escape_data_string("Modlight on Mac mini"),
            "Modlight%20on%20Mac%20mini"
        );
        assert_eq!(escape_data_string("a-b.c_d~e"), "a-b.c_d~e");
        assert_eq!(escape_data_string("zó/ł?&="), "z%C3%B3%2F%C5%82%3F%26%3D");
    }
}

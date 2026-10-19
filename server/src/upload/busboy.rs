//! The parts of busboy 1.6.0 (`lib/utils.js`, `lib/types/multipart.js`) whose
//! behaviour `lib/upload/sharex.ts` depends on: which request content types
//! are accepted, how a part's `Content-Disposition` and `Content-Type` are
//! read, and how a part is classified as a file or a field.
//!
//! The boundary scanning itself is `multer` (streaming, constant memory); the
//! classification rules are busboy's, because they decide what a client's
//! request MEANS:
//!
//! - a part without `Content-Disposition`, or whose disposition is not
//!   `form-data`, is skipped;
//! - a part is a FILE when its type is `application/octet-stream` or it has a
//!   (non-empty) `filename` / `filename*`; the name is reduced to its basename;
//! - a part without `Content-Type` is `text/plain`;
//! - header bytes are latin1 and parameter values are decoded as UTF-8
//!   (`defParamCharset: "utf8"` upstream), invalid sequences replaced.

/// `TOKEN` of busboy: RFC 7230 token characters.
fn is_token(c: u8) -> bool {
    matches!(c, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
        || c.is_ascii_alphanumeric()
}

/// `QDTEXT`: tab, space, `!`, `#`-`[`, `]`-`~`, and every byte >= 0x80.
fn is_qdtext(c: u8) -> bool {
    c == b'\t' || c == b' ' || c == 0x21 || (0x23..=0x5b).contains(&c) || (0x5d..=0x7e).contains(&c) || c >= 0x80
}

/// `CHARSET`
fn is_charset(c: u8) -> bool {
    matches!(c, b'!' | b'#' | b'$' | b'%' | b'&' | b'+' | b'-' | b'^' | b'_' | b'`' | b'{' | b'}' | b'~')
        || c.is_ascii_alphanumeric()
}

/// `EXTENDED_VALUE`
fn is_extended_value(c: u8) -> bool {
    matches!(c, b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
        || c.is_ascii_alphanumeric()
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decode raw bytes with a charset busboy knows (`getDecoder`).
fn decode_charset(bytes: &[u8], charset: &str) -> Option<String> {
    match charset.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" => Some(String::from_utf8_lossy(bytes).into_owned()),
        "latin1" | "ascii" | "us-ascii" | "iso-8859-1" | "iso8859-1" | "iso88591" | "iso_8859-1" | "windows-1252"
        | "iso_8859-1:1987" | "cp1252" | "x-cp1252" => Some(bytes.iter().map(|b| *b as char).collect()),
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => {
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    i
}

/// `{ type, params }` of a Content-Disposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disposition {
    pub kind: String,
    /// Lower-cased names; the first occurrence of a name wins.
    pub params: Vec<(String, String)>,
}

impl Disposition {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// `parseDisposition(str, utf8Decoder)`; `None` when malformed.
pub fn parse_disposition(raw: &[u8]) -> Option<Disposition> {
    if raw.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < raw.len() && is_token(raw[i]) {
        i += 1;
    }
    let kind: String = raw[..i].iter().map(|b| *b as char).collect::<String>().to_lowercase();
    let mut params: Vec<(String, String)> = Vec::new();
    if i < raw.len() {
        parse_params(raw, i, &mut params)?;
    }
    Some(Disposition { kind, params })
}

fn parse_params(s: &[u8], mut i: usize, params: &mut Vec<(String, String)>) -> Option<()> {
    while i < s.len() {
        i = skip_ws(s, i);
        if i == s.len() {
            break;
        }
        if s[i] != b';' {
            return None;
        }
        i += 1;
        i = skip_ws(s, i);
        if i == s.len() {
            return None;
        }
        let name_start = i;
        while i < s.len() && is_token(s[i]) {
            i += 1;
        }
        if i == s.len() || s[i] != b'=' {
            return None;
        }
        let name: String = s[name_start..i].iter().map(|b| *b as char).collect();
        let value: String = if name.ends_with('*') {
            // Extended value: charset'lang'percent-encoded
            i += 1;
            let charset_start = i;
            while i < s.len() && is_charset(s[i]) {
                i += 1;
            }
            if i == s.len() || s[i] != b'\'' {
                return None;
            }
            let charset: String = s[charset_start..i].iter().map(|b| *b as char).collect();
            i += 1;
            while i < s.len() && s[i] != b'\'' {
                i += 1;
            }
            if i == s.len() {
                return None;
            }
            i += 1;
            if i == s.len() {
                return None;
            }
            let mut bytes: Vec<u8> = Vec::new();
            while i < s.len() {
                let c = s[i];
                if is_extended_value(c) {
                    bytes.push(c);
                    i += 1;
                } else if c == b'%' {
                    match (s.get(i + 1).and_then(|h| hex_value(*h)), s.get(i + 2).and_then(|h| hex_value(*h))) {
                        (Some(hi), Some(lo)) if i + 2 < s.len() => {
                            bytes.push((hi << 4) + lo);
                            i += 3;
                        }
                        _ => return None,
                    }
                } else {
                    break;
                }
            }
            decode_charset(&bytes, &charset)?
        } else {
            i += 1;
            if i == s.len() {
                return None;
            }
            let mut bytes: Vec<u8> = Vec::new();
            if s[i] == b'"' {
                i += 1;
                let mut escaping = false;
                loop {
                    if i == s.len() {
                        return None; // no end quote
                    }
                    let c = s[i];
                    if c == b'\\' {
                        // `\\` is one backslash; a lone backslash starts an escape.
                        if escaping {
                            bytes.push(b'\\');
                        }
                        escaping = !escaping;
                        i += 1;
                        continue;
                    }
                    if c == b'"' {
                        if escaping {
                            bytes.push(b'"');
                            escaping = false;
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    if escaping {
                        // busboy keeps the backslash of an escape that escapes nothing.
                        bytes.push(b'\\');
                        escaping = false;
                    }
                    if !is_qdtext(c) {
                        return None;
                    }
                    bytes.push(c);
                    i += 1;
                }
                i += 1;
            } else {
                let start = i;
                while i < s.len() && is_token(s[i]) {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                bytes.extend_from_slice(&s[start..i]);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let key = name.to_lowercase();
        if !params.iter().any(|(k, _)| *k == key) {
            params.push((key, value));
        }
    }
    Some(())
}

/// `parseContentType(str)`: `type/subtype` lower-cased, and the charset
/// parameter; `None` when malformed. Only what the upload path needs.
pub fn parse_content_type(raw: &str) -> Option<(String, String, Option<String>)> {
    let s = raw.as_bytes();
    if s.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < s.len() && is_token(s[i]) {
        i += 1;
    }
    if i == s.len() || s[i] != b'/' || i == 0 {
        return None;
    }
    let kind = raw[..i].to_lowercase();
    i += 1;
    let sub_start = i;
    while i < s.len() && is_token(s[i]) {
        i += 1;
    }
    if i == sub_start {
        return None;
    }
    let subtype = raw[sub_start..i].to_lowercase();
    let mut params: Vec<(String, String)> = Vec::new();
    if i < s.len() {
        parse_params(s, i, &mut params)?;
    }
    let charset = params.into_iter().find(|(k, _)| k == "charset").map(|(_, v)| v);
    Some((kind, subtype, charset))
}

/// The body types busboy accepts, decided from the request's Content-Type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyType {
    Multipart { boundary: String },
    UrlEncoded,
}

/// `Busboy({ headers })`: what it would parse, or the error it throws
/// ("Missing Content-Type", "Malformed content type", "Unsupported content
/// type", "Multipart: Boundary not found").
pub fn body_type(content_type: Option<&str>) -> Result<BodyType, &'static str> {
    let Some(ct) = content_type else {
        return Err("Missing Content-Type");
    };
    let Some((kind, subtype, _)) = parse_content_type(ct) else {
        return Err("Malformed content type");
    };
    if kind == "multipart" && subtype == "form-data" {
        // The boundary parameter, as busboy reads it.
        let s = ct.as_bytes();
        let start = ct.find(';').unwrap_or(ct.len());
        let mut params = Vec::new();
        if start < s.len() && parse_params(s, start, &mut params).is_none() {
            return Err("Malformed content type");
        }
        return match params.into_iter().find(|(k, _)| k == "boundary") {
            Some((_, b)) => Ok(BodyType::Multipart { boundary: b }),
            None => Err("Multipart: Boundary not found"),
        };
    }
    if kind == "application" && subtype == "x-www-form-urlencoded" {
        return Ok(BodyType::UrlEncoded);
    }
    Err("Unsupported content type")
}

/// `basename(path)`: the part after the last `/` or `\`; "." and ".." become "".
pub fn basename(path: &str) -> String {
    let tail = match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    };
    if tail == "." || tail == ".." { String::new() } else { tail.to_string() }
}

/// How busboy classifies one part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartKind {
    /// No usable Content-Disposition: busboy skips the part.
    Skip,
    File {
        name: Option<String>,
        filename: Option<String>,
        mime_type: String,
    },
    Field {
        name: Option<String>,
        charset: String,
    },
}

/// Classify a part from its raw headers.
pub fn classify_part(disposition: Option<&[u8]>, content_type: Option<&[u8]>) -> PartKind {
    let Some(disp) = disposition.and_then(parse_disposition) else {
        return PartKind::Skip;
    };
    if disp.kind != "form-data" {
        return PartKind::Skip;
    }
    let name = disp.param("name").filter(|n| !n.is_empty()).map(str::to_string);
    let filename = disp
        .param("filename*")
        .filter(|f| !f.is_empty())
        .or_else(|| disp.param("filename").filter(|f| !f.is_empty()))
        .map(basename);
    let mut part_type = "text/plain".to_string();
    let mut charset = "utf8".to_string();
    if let Some(ct) = content_type
        && let Some((kind, subtype, cs)) = parse_content_type(&ct.iter().map(|b| *b as char).collect::<String>())
    {
        part_type = format!("{kind}/{subtype}");
        if let Some(cs) = cs {
            charset = cs.to_lowercase();
        }
    }
    if part_type == "application/octet-stream" || filename.is_some() {
        PartKind::File { name, filename, mime_type: part_type }
    } else {
        PartKind::Field { name, charset }
    }
}

/// Decode a field's bytes with its charset (utf8 by default).
pub fn decode_field(bytes: &[u8], charset: &str) -> String {
    decode_charset(bytes, charset).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispositions() {
        let d = parse_disposition(br#"form-data; name="file"; filename="C:\dir\Ch\xc5\x82opak.mp4""#).unwrap();
        assert_eq!(d.kind, "form-data");
        assert_eq!(d.param("name"), Some("file"));
        let d = parse_disposition("form-data; name=\"file\"; filename=\"Chłopak.mp4\"".as_bytes()).unwrap();
        assert_eq!(d.param("filename"), Some("Chłopak.mp4"));
        let d = parse_disposition(b"form-data; name=f; filename*=UTF-8''%C5%BC%C3%B3%C5%82w.png").unwrap();
        assert_eq!(d.param("filename*"), Some("żółw.png"));
        assert!(parse_disposition(b"form-data; name").is_none());
        assert!(parse_disposition(br#"form-data; name="unterminated"#).is_none());
        let d = parse_disposition(br#"form-data; name="a\"b""#).unwrap();
        assert_eq!(d.param("name"), Some("a\"b"));
        let d = parse_disposition(br#"form-data; name="f"; filename="C:\dir\shot.png""#).unwrap();
        assert_eq!(d.param("filename"), Some(r"C:\dir\shot.png"));
        assert_eq!(basename(d.param("filename").unwrap()), "shot.png");
    }

    #[test]
    fn classification() {
        let file = classify_part(Some(br#"form-data; name="sharex"; filename="a/b/shot.png""#), Some(b"image/png"));
        assert_eq!(
            file,
            PartKind::File {
                name: Some("sharex".into()),
                filename: Some("shot.png".into()),
                mime_type: "image/png".into()
            }
        );
        // No Content-Type: text/plain. Empty filename: a field.
        assert_eq!(
            classify_part(Some(br#"form-data; name="x"; filename="""#), None),
            PartKind::Field { name: Some("x".into()), charset: "utf8".into() }
        );
        assert!(matches!(
            classify_part(Some(br#"form-data; name="x""#), Some(b"application/octet-stream")),
            PartKind::File { filename: None, .. }
        ));
        assert_eq!(classify_part(None, None), PartKind::Skip);
        assert_eq!(classify_part(Some(b"attachment; name=x"), None), PartKind::Skip);
    }

    #[test]
    fn body_types() {
        assert_eq!(
            body_type(Some("multipart/form-data; boundary=----abc")),
            Ok(BodyType::Multipart { boundary: "----abc".into() })
        );
        assert_eq!(
            body_type(Some("Multipart/Form-Data; boundary=\"q b\"")),
            Ok(BodyType::Multipart { boundary: "q b".into() })
        );
        assert_eq!(body_type(Some("multipart/form-data")), Err("Multipart: Boundary not found"));
        assert_eq!(body_type(Some("application/x-www-form-urlencoded")), Ok(BodyType::UrlEncoded));
        assert_eq!(body_type(Some("application/json")), Err("Unsupported content type"));
        assert_eq!(body_type(Some("garbage")), Err("Malformed content type"));
        assert_eq!(body_type(None), Err("Missing Content-Type"));
        assert_eq!(basename("..\\..\\x"), "x");
        assert_eq!(basename(".."), "");
    }
}

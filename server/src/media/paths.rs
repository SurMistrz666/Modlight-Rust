//! Port of `lib/media/paths.ts`: string surgery on public codes, filenames and
//! storage keys. Dependency-free on purpose (the browser side of the
//! conversion menu shares it upstream).

/// Strip a trailing extension to recover the bare code (codes never contain
/// dots).
///
/// This is what makes the extension in a public URL DECORATIVE: the lookup key
/// is the code alone, so /{tag}/{code}.mp4, /{tag}/{code}.gif and
/// /{tag}/{code} all address the same row.
///
/// `decodeURIComponent` throws a `URIError` on a malformed escape; that is
/// `None` here.
pub fn code_from_media_code(media_code: &str) -> Option<String> {
    let decoded = crate::http::cookies::decode_uri_component(media_code)?;
    Some(decoded.split('.').next().unwrap_or("").to_string())
}

/// Replace the extension of a storage key, keeping its directory prefix.
///
/// Derived from the STORED key rather than rebuilt with `storageKeyFor`: the
/// prefix is the owner's tag AS IT WAS when the file was uploaded.
pub fn swap_key_ext(key: &str, ext: &str) -> String {
    let (dir, name) = split_key(key);
    let base = strip_ext(name);
    if ext.is_empty() { format!("{dir}{base}") } else { format!("{dir}{base}.{ext}") }
}

/// Replace the extension of a display filename: only the LAST segment is an
/// extension, and a name with no extension at all simply gains one.
pub fn swap_filename_ext(filename: &str, ext: &str) -> String {
    let stripped = strip_ext(filename);
    let base = if stripped.is_empty() { filename } else { stripped };
    if ext.is_empty() { base.to_string() } else { format!("{base}.{ext}") }
}

/// Insert a short suffix before the extension: "a/b.png" -> "a/b-x1y2.png".
pub fn suffix_key(key: &str, suffix: &str) -> String {
    let (dir, name) = split_key(key);
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{dir}{}-{suffix}{}", &name[..dot], &name[dot..]),
        _ => format!("{dir}{name}-{suffix}"),
    }
}

fn split_key(key: &str) -> (&str, &str) {
    match key.rfind('/') {
        Some(slash) => (&key[..slash + 1], &key[slash + 1..]),
        None => ("", key),
    }
}

/// Drop a trailing ".ext" (never a leading dot, so ".gitignore" survives whole).
fn strip_ext(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surgery() {
        assert_eq!(code_from_media_code("abc12.png").as_deref(), Some("abc12"));
        assert_eq!(code_from_media_code("abc12").as_deref(), Some("abc12"));
        assert_eq!(code_from_media_code("a%2Eb").as_deref(), Some("a"));
        assert_eq!(code_from_media_code("%E0%A4%A"), None);
        assert_eq!(swap_key_ext("tag/code.mp4", "gif"), "tag/code.gif");
        assert_eq!(swap_key_ext("tag/code", "gif"), "tag/code.gif");
        assert_eq!(swap_key_ext("tag/code.mp4", ""), "tag/code");
        assert_eq!(swap_filename_ext("wakacje.2024.mp4", "gif"), "wakacje.2024.gif");
        assert_eq!(swap_filename_ext("noext", "gif"), "noext.gif");
        assert_eq!(swap_filename_ext(".gitignore", "txt"), ".gitignore.txt");
        assert_eq!(suffix_key("a/b.png", "x1y2"), "a/b-x1y2.png");
        assert_eq!(suffix_key("a/b", "x1y2"), "a/b-x1y2");
        assert_eq!(suffix_key("a/.env", "x"), "a/.env-x");
    }
}

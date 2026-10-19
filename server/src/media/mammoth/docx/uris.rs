//! mammoth `lib/docx/uris.js`.

/// `uriToZipEntryName(base, uri)`.
pub fn uri_to_zip_entry_name(base: &str, uri: &str) -> String {
    match uri.strip_prefix('/') {
        Some(rest) => rest.to_owned(),
        // In general, we should check first and second for trailing and leading slashes,
        // but in our specific case this seems to be sufficient
        None => format!("{base}/{uri}"),
    }
}

/// `replaceFragment(uri, fragment)`.
pub fn replace_fragment(uri: &str, fragment: &str) -> String {
    let uri = match uri.find('#') {
        Some(i) => &uri[..i],
        None => uri,
    };
    format!("{uri}#{fragment}")
}

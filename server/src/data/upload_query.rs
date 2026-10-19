//! Port of `lib/data/upload-query.ts`: turning a query string into a library
//! listing, with nothing in it the database can choke on.
//!
//! Every number is checked for FINITENESS before it is clamped, and the page
//! has a ceiling (`?page=1e30` would otherwise reach Postgres as "3e+31").

use crate::config::{js_number, js_trim};
use crate::naming::js_slice;

/// `SortKey`
pub const SORT_KEYS: &[&str] = &["newest", "oldest", "largest", "smallest", "name", "views"];

/// `FileKind`: coarse file kinds, derived from mimeType.
pub const FILE_KINDS: &[&str] = &["image", "video", "audio", "document", "archive", "paste", "other"];

pub const DEFAULT_PER_PAGE: i64 = 30;

/// The most rows one request may pull; a caller asking for more is answered
/// with a full page and the perPage actually used.
pub const MAX_PER_PAGE: i64 = 120;

/// Highest page number anyone may ask for.
pub const MAX_PAGE: i64 = 1_000_000;

/// Longest search needle we will put inside an ILIKE '%...%'.
const MAX_SEARCH_LENGTH: usize = 200;

pub fn is_sort_key(v: &str) -> bool {
    SORT_KEYS.contains(&v)
}

pub fn is_file_kind(v: &str) -> bool {
    FILE_KINDS.contains(&v)
}

/// `UploadQuery`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadQuery {
    pub page: i64,
    pub per_page: i64,
    pub search: Option<String>,
    pub sort: String,
    pub kind: Option<String>,
    /// List the trash instead of the library.
    pub trashed: bool,
    /// The folder id EXACTLY as the client wrote it, including the "root"
    /// sentinel - validated later against the database and the caller.
    pub folder: Option<String>,
}

/// A whole number >= 1, or the fallback. An absent parameter and an empty one
/// mean the same thing.
fn positive_int(raw: Option<&str>, fallback: i64) -> i64 {
    let Some(raw) = raw else {
        return fallback;
    };
    if js_trim(raw).is_empty() {
        return fallback;
    }
    let n = js_number(raw);
    if !n.is_finite() {
        return fallback;
    }
    (n.floor().max(1.0)).min(i64::MAX as f64) as i64
}

fn one_of(raw: Option<&str>, allowed: &[&str]) -> Option<String> {
    raw.filter(|r| !r.is_empty() && allowed.contains(r)).map(str::to_string)
}

/// `parseUploadQuery(params)`: `get` returns the FIRST value of a key.
pub fn parse_upload_query(params: &[(String, String)]) -> UploadQuery {
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let search = get("search").map(|s| js_slice(js_trim(s), MAX_SEARCH_LENGTH).to_string()).filter(|s| !s.is_empty());
    UploadQuery {
        page: positive_int(get("page"), 1).min(MAX_PAGE),
        per_page: positive_int(get("perPage"), DEFAULT_PER_PAGE).min(MAX_PER_PAGE),
        search,
        // An unknown sort is the default sort, not an error.
        sort: one_of(get("sort"), SORT_KEYS).unwrap_or_else(|| "newest".to_string()),
        kind: one_of(get("kind"), FILE_KINDS),
        // Strict "1", the same literal the panel's own toggles use.
        trashed: get("trashed") == Some("1"),
        folder: get("folderId").map(|f| js_trim(f).to_string()).filter(|f| !f.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/upload-query.test.ts
    use super::*;

    fn q(s: &str) -> UploadQuery {
        let pairs: Vec<(String, String)> = form_urlencoded::parse(s.as_bytes()).into_owned().collect();
        parse_upload_query(&pairs)
    }

    #[test]
    fn defaults_and_clamps() {
        let d = q("");
        assert_eq!((d.page, d.per_page, d.sort.as_str(), d.trashed), (1, 30, "newest", false));
        assert_eq!(d.search, None);
        assert_eq!(q("perPage=abc").per_page, 30);
        assert_eq!(q("perPage=").per_page, 30);
        assert_eq!(q("perPage=0").per_page, 1);
        assert_eq!(q("perPage=5000").per_page, 120);
        assert_eq!(q("perPage=2.9").per_page, 2);
        assert_eq!(q("page=1e30").page, MAX_PAGE);
        assert_eq!(q("page=-4").page, 1);
        assert_eq!(q("page=Infinity").page, 1);
        assert_eq!(q("sort=largest").sort, "largest");
        assert_eq!(q("sort=biggest").sort, "newest");
        assert_eq!(q("kind=image").kind.as_deref(), Some("image"));
        assert_eq!(q("kind=movie").kind, None);
        assert!(q("trashed=1").trashed);
        assert!(!q("trashed=true").trashed);
        assert_eq!(q("folderId=%20root%20").folder.as_deref(), Some("root"));
        assert_eq!(q("folderId=%20").folder, None);
        assert_eq!(q(&format!("search={}", "a".repeat(300))).search.unwrap().len(), 200);
        assert_eq!(q("search=%20%20").search, None);
    }
}

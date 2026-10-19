//! Port of `lib/upload/duplicate-check.ts`: "do I already have this file?"
//! asked BEFORE the bytes are sent - the database-free half of the contract.
//!
//! `uploads.checksum` is NOT populated for every row (old rows, and the tus
//! path never hashes), so "not found" is only reported as `new` when the
//! absence of a match is a FACT about the whole library; otherwise `unknown`.

use serde::Serialize;
use serde_json::Value;

/// Checksums one request may ask about (the same 500 as `MAX_IDS`).
pub const MAX_CHECKSUMS: usize = 500;

/// `/^[0-9a-f]{64}$/`: a stored sha256, lower-case hex.
pub fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `ChecksumListIssue`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumListIssue {
    Empty,
    Malformed,
    TooMany,
}

impl ChecksumListIssue {
    pub fn as_str(self) -> &'static str {
        match self {
            ChecksumListIssue::Empty => "empty",
            ChecksumListIssue::Malformed => "malformed",
            ChecksumListIssue::TooMany => "too-many",
        }
    }
}

/// Validate the list a client asked about, BEFORE it can reach a query:
/// lower-cased, trimmed, deduplicated, every entry a sha256.
pub fn parse_checksums(input: Option<&Value>) -> Result<Vec<String>, ChecksumListIssue> {
    let Some(Value::Array(items)) = input else {
        return Err(ChecksumListIssue::Empty);
    };
    if items.is_empty() {
        return Err(ChecksumListIssue::Empty);
    }
    if items.len() > MAX_CHECKSUMS {
        return Err(ChecksumListIssue::TooMany);
    }
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in items {
        let Some(s) = raw.as_str() else {
            return Err(ChecksumListIssue::Malformed);
        };
        let value = crate::config::js_trim(s).to_lowercase();
        if !is_sha256(&value) {
            return Err(ChecksumListIssue::Malformed);
        }
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    if out.is_empty() {
        return Err(ChecksumListIssue::Empty);
    }
    Ok(out)
}

/// `CheckStatus`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Duplicate,
    New,
    Unknown,
}

/// A match always wins; otherwise `new` only when nothing was invisible.
pub fn status_for(has_match: bool, blind: bool) -> CheckStatus {
    if has_match {
        CheckStatus::Duplicate
    } else if blind {
        CheckStatus::Unknown
    } else {
        CheckStatus::New
    }
}

/// One file the caller already has (`CheckMatch`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckMatch {
    pub id: String,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    /// ISO 8601.
    pub created_at: String,
    pub folder_id: Option<String>,
    pub folder_name: Option<String>,
    pub url: String,
    pub raw_url: String,
    pub copy_url: String,
}

/// The answer for one requested checksum.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResult {
    pub checksum: String,
    pub status: CheckStatus,
    pub matches: Vec<CheckMatch>,
}

/// The whole reply from POST /api/uploads/check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResponse {
    pub message: String,
    pub requested: usize,
    pub unchecksummed: i64,
    pub truncated: bool,
    pub results: Vec<CheckResult>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn checksum_lists() {
        assert_eq!(parse_checksums(None), Err(ChecksumListIssue::Empty));
        assert_eq!(parse_checksums(Some(&json!([]))), Err(ChecksumListIssue::Empty));
        assert_eq!(parse_checksums(Some(&json!("x"))), Err(ChecksumListIssue::Empty));
        assert_eq!(parse_checksums(Some(&json!([1]))), Err(ChecksumListIssue::Malformed));
        assert_eq!(parse_checksums(Some(&json!(["nope"]))), Err(ChecksumListIssue::Malformed));
        let many: Vec<String> = (0..501).map(|_| A.to_string()).collect();
        assert_eq!(parse_checksums(Some(&json!(many))), Err(ChecksumListIssue::TooMany));
        let upper = A.to_uppercase();
        assert_eq!(parse_checksums(Some(&json!([format!(" {upper} "), A]))).unwrap(), vec![A.to_string()]);
        assert_eq!(status_for(true, true), CheckStatus::Duplicate);
        assert_eq!(status_for(false, true), CheckStatus::Unknown);
        assert_eq!(status_for(false, false), CheckStatus::New);
    }
}

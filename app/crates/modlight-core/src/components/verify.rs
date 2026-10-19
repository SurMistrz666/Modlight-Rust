//! Hashing a downloaded file and getting rid of it when the hash is wrong
//! (`ModlightUpdateFile`, `ModlightRelease.IsDigest`).
//!
//! The last gate before a downloaded executable, model or installer is used. Written to
//! be dull: no network, no UI, nothing that needs a window.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Characters in a hex SHA-256.
pub const DIGEST_LENGTH: usize = 64;

/// The result of hashing a file and comparing it with what was promised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    /// True only when a digest was computed and it is the expected one.
    pub matches: bool,
    /// The digest that was computed, or empty when it could not be.
    pub actual: String,
    /// Why it could not be computed or did not match.
    pub error: Option<String>,
}

/// Whether a string is a hex SHA-256. Case is not part of the answer.
pub fn is_digest(value: &str) -> bool {
    value.len() == DIGEST_LENGTH && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether two digests are the same digest: case-insensitive, never empty.
pub fn digests_match(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && a.eq_ignore_ascii_case(b)
}

/// Hash a file. Lowercase hex, the form the publishers print.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Delete a file if it is there, and say whether it is gone now.
pub fn try_delete(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => !path.exists(),
    }
}

/// Hash the file, and delete it unless it is exactly what was promised.
///
/// Deleting on failure is the point: what would otherwise be left behind is an
/// executable of unknown provenance under a name that says it belongs to us. A digest
/// that could not be computed at all counts as a failure for the same reason.
pub fn verify_or_delete(path: &Path, expected: &str) -> Verification {
    if !is_digest(expected) {
        try_delete(path);
        return Verification {
            matches: false,
            actual: String::new(),
            error: Some("no digest to check against".into()),
        };
    }
    let actual = match sha256_file(path) {
        Ok(actual) => actual,
        Err(error) => {
            try_delete(path);
            return Verification {
                matches: false,
                actual: String::new(),
                error: Some(error.to_string()),
            };
        }
    };
    if digests_match(&actual, expected) {
        return Verification {
            matches: true,
            actual,
            error: None,
        };
    }
    try_delete(path);
    Verification {
        matches: false,
        actual,
        error: Some("digest mismatch".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn a_digest_is_sixty_four_hex_characters() {
        assert!(is_digest(HELLO));
        assert!(is_digest(&HELLO.to_uppercase()));
        assert!(!is_digest(&HELLO[1..]));
        assert!(!is_digest(&format!("{}g", &HELLO[1..])));
        assert!(!is_digest(""));
    }

    #[test]
    fn digests_compare_without_case_and_never_empty() {
        assert!(digests_match(HELLO, &HELLO.to_uppercase()));
        assert!(!digests_match("", ""));
        assert!(!digests_match(HELLO, ""));
    }

    #[test]
    fn a_matching_file_is_kept_and_a_wrong_one_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.part");
        std::fs::write(&path, b"hello").unwrap();
        let ok = verify_or_delete(&path, HELLO);
        assert!(ok.matches);
        assert!(path.exists());

        let wrong = "0".repeat(64);
        let bad = verify_or_delete(&path, &wrong);
        assert!(!bad.matches);
        assert_eq!(bad.actual, HELLO);
        assert!(!path.exists(), "a file that disagrees is deleted");
    }

    #[test]
    fn no_digest_means_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.part");
        std::fs::write(&path, b"hello").unwrap();
        let result = verify_or_delete(&path, "");
        assert!(!result.matches);
        assert!(!path.exists());
    }
}

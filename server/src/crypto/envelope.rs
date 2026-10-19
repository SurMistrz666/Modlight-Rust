//! Port of the server-side part of `lib/crypto/envelope.ts` (the MLENC1
//! container): its constants and `isEncryptedUpload`. Building, opening and
//! decrypting containers happens in the browser only (WebCrypto), with
//! upstream's module verbatim in the SPA.

/// Magic bytes every container starts with.
pub const CONTAINER_MAGIC: &str = "MLENC1";

/// How much of a container the viewer reads before it can name the file. Its
/// first request (`Range: bytes=0-4095`) is the one `/raw` counts as the view.
pub const HEADER_PROBE_BYTES: u64 = 4096;

/// Extension every encrypted upload is stored under.
pub const ENCRYPTED_EXT: &str = "mlenc";

/// MIME the upload is declared as: a type the server has never heard of, so
/// `mediaKind()` answers "other", nothing decodes it, and `serveDecision()`
/// hands it over as `application/octet-stream; attachment`.
pub const ENCRYPTED_MIME: &str = "application/vnd.modlight.encrypted";

/// `isEncryptedUpload(mimeType, ext)`: asked of the ROW, never of the bytes.
/// Both are checked because either one alone is guessable - a user can name a
/// plain file `.mlenc`, and a client can claim any MIME - and the answer only
/// decides which viewer renders, never whether anything is decrypted.
pub fn is_encrypted_upload(mime_type: Option<&str>, ext: Option<&str>) -> bool {
    let mime = crate::config::js_trim(mime_type.unwrap_or("").split(';').next().unwrap_or("")).to_lowercase();
    mime == ENCRYPTED_MIME && ext.unwrap_or("").to_lowercase() == ENCRYPTED_EXT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::mime::{MediaKind, media_kind, serve_decision};

    #[test]
    fn both_facts_are_needed() {
        assert!(is_encrypted_upload(Some(ENCRYPTED_MIME), Some("mlenc")));
        assert!(is_encrypted_upload(Some(" Application/VND.Modlight.Encrypted ; x=1"), Some("MLENC")));
        assert!(!is_encrypted_upload(Some(ENCRYPTED_MIME), Some("bin")));
        assert!(!is_encrypted_upload(Some("application/octet-stream"), Some("mlenc")));
        assert!(!is_encrypted_upload(None, None));
    }

    #[test]
    fn the_server_never_presents_a_container_as_anything() {
        // tests/integration/encrypted-upload.test.ts: no decoder, an inert download.
        assert_eq!(media_kind(Some(ENCRYPTED_MIME), Some(ENCRYPTED_EXT)), MediaKind::Other);
        let d = serve_decision(Some(ENCRYPTED_MIME), Some(ENCRYPTED_EXT));
        assert_eq!((d.disposition, d.content_type.as_str()), ("attachment", "application/octet-stream"));
    }
}

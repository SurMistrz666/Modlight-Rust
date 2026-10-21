//! The API token in the Windows Credential Manager (a generic credential, protected by
//! DPAPI for this user, as upstream's `[JsonEncrypt]` token was), the counterpart of the
//! macOS port's login Keychain item.
//!
//! The credential's target is `<service>/<account>` (`Modlight/api-token`; the image
//! analysis keys are `Modlight/ai-...-api-key`), its user name the account, and it is
//! kept on this computer for this user (`CRED_PERSIST_LOCAL_MACHINE`, no roaming). The
//! value is the text the app hands over, stored as its UTF-8 bytes: for the token that is
//! the token together with the origin it was issued for (`account.rs`), exactly what the
//! Keychain item holds on a Mac. Nothing here logs the value.

use modlight_core::account::CredentialStore;
use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
use windows_sys::Win32::Security::Credentials::{
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW,
    CredDeleteW, CredFree, CredReadW, CredWriteW,
};

use crate::system::system_message;
use crate::wide::wide;

/// One generic credential: target `<service>/<account>`.
pub struct SecretStore {
    service: String,
    account: String,
}

impl SecretStore {
    /// The credential the app keeps its token in.
    pub fn for_app() -> SecretStore {
        SecretStore::new("Modlight", "api-token")
    }

    pub fn new(service: &str, account: &str) -> SecretStore {
        SecretStore {
            service: service.to_string(),
            account: account.to_string(),
        }
    }

    fn target(&self) -> String {
        format!("{}/{}", self.service, self.account)
    }
}

/// The stored bytes as text: UTF-8, as this store writes it. A value another program
/// wrote as UTF-16 (as `cmdkey` and Credential Manager's own window do) is read as that.
fn decode(blob: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(blob)
        && !text.contains('\0')
    {
        return Some(text.to_string());
    }
    let (pairs, rest) = blob.as_chunks::<2>();
    if rest.is_empty() {
        let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        return String::from_utf16(&units)
            .ok()
            .map(|text| text.trim_end_matches('\0').to_string());
    }
    None
}

impl CredentialStore for SecretStore {
    fn load(&self) -> Option<String> {
        let target = wide(self.target());
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated; on success the system allocates the
        // credential, which is freed below.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0
            || credential.is_null()
        {
            return None;
        }
        // SAFETY: CredReadW returned a valid credential.
        let found = unsafe { &*credential };
        let blob: &[u8] = if found.CredentialBlob.is_null() || found.CredentialBlobSize == 0 {
            &[]
        } else {
            // SAFETY: the blob holds CredentialBlobSize bytes while the credential lives.
            unsafe {
                std::slice::from_raw_parts(found.CredentialBlob, found.CredentialBlobSize as usize)
            }
        };
        let text = decode(blob);
        // SAFETY: allocated by CredReadW, freed once, not used after.
        unsafe { CredFree(credential.cast()) };
        text.filter(|t| !t.is_empty())
    }

    fn save(&self, token: &str) -> Result<(), String> {
        let blob = token.as_bytes();
        if blob.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            return Err(format!(
                "the secret is longer than the {CRED_MAX_CREDENTIAL_BLOB_SIZE} bytes Credential Manager keeps"
            ));
        }
        let mut target = wide(self.target());
        let mut user = wide(&self.account);
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_ptr().cast_mut(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: every pointer in `credential` is valid for the call; CredWriteW copies
        // what it keeps and does not write through the blob pointer.
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            // SAFETY: reads the calling thread's last error code.
            return Err(system_message(unsafe { GetLastError() }));
        }
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        let target = wide(self.target());
        // SAFETY: `target` is NUL-terminated.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(());
        }
        // SAFETY: reads the calling thread's last error code.
        match unsafe { GetLastError() } {
            ERROR_NOT_FOUND => Ok(()),
            code => Err(system_message(code)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A credential of its own for each test, deleted at the end.
    fn throwaway() -> SecretStore {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        SecretStore::new(
            &format!("ModlightTest-{}-{nanos}", std::process::id()),
            "api-token",
        )
    }

    #[test]
    fn a_secret_is_saved_read_replaced_and_deleted() {
        let store = throwaway();
        assert_eq!(store.load(), None);
        let secret = r#"{"Host":"https://files.example.com","Token":"mlt_zażółć_123"}"#;
        store.save(secret).unwrap();
        assert_eq!(store.load().as_deref(), Some(secret));
        store.save("second").unwrap();
        assert_eq!(store.load().as_deref(), Some("second"));
        store.delete().unwrap();
        assert_eq!(store.load(), None);
        // Nothing to delete is not an error.
        store.delete().unwrap();
    }

    #[test]
    fn an_oversized_secret_is_refused() {
        let store = throwaway();
        let long = "x".repeat(CRED_MAX_CREDENTIAL_BLOB_SIZE as usize + 1);
        assert!(store.save(&long).is_err());
        assert_eq!(store.load(), None);
    }

    #[test]
    fn values_written_by_other_programs_are_read() {
        assert_eq!(decode(b"token").as_deref(), Some("token"));
        let utf16: Vec<u8> = "tóken".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&utf16).as_deref(), Some("tóken"));
        assert_eq!(decode(&[0xff]), None);
    }
}

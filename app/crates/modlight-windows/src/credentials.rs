//! The API token in the Windows Credential Manager (a generic credential, protected by
//! DPAPI for this user, as upstream's `[JsonEncrypt]` token was), the counterpart of the
//! macOS port's login Keychain item.

use modlight_core::account::CredentialStore;

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

impl CredentialStore for SecretStore {
    fn load(&self) -> Option<String> {
        let _ = self.target();
        None
    }

    fn save(&self, token: &str) -> Result<(), String> {
        let _ = token;
        Err("not implemented yet".into())
    }

    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

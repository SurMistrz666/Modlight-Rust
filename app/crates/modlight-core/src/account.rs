//! The account this installation is signed in to (`ModlightAccount`, `ModlightSignIn.Save`,
//! `SignOut`, `Adopt`) and the administrator flag (`ModlightAdmin`).
//!
//! Everything but the token is kept in `UploadersConfig.json`, in the shape the Windows
//! client stores it. The token never goes into that file: it lives in the system's
//! credential store (Credential Manager on Windows, the login Keychain on a Mac; or, in QA
//! mode, a private file of the QA profile), behind [`CredentialStore`].
//!
//! The store keeps the token together with the origin it was issued for. The host in
//! `UploadersConfig.json` can change behind the token's back (an edited file, a settings
//! backup from somebody else), and on Windows that cannot happen because the token sits
//! in the same file. Here a token is only ever paired with the host it was issued for:
//! any other host reads as signed out until the person signs in there.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use modlight_api::{AccountInfo, Credential, SignedIn};
use serde::{Deserialize, Serialize};

use crate::config::write_atomically;
use crate::sync::account_key;

pub const UPLOADERS_CONFIG_FILE: &str = "UploadersConfig.json";

/// Where the API token is kept. Implementations must never log the token.
pub trait CredentialStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, token: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// The token in a file readable by this user only (QA profiles and tests).
pub struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    pub fn new(path: PathBuf) -> Self {
        FileCredentialStore { path }
    }
}

impl CredentialStore for FileCredentialStore {
    fn load(&self) -> Option<String> {
        std::fs::read_to_string(&self.path)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    }

    fn save(&self, token: &str) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("tmp");
        write_private(&tmp, token.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// A store that keeps the token in memory only (previews and tests).
#[derive(Default)]
pub struct MemoryCredentialStore {
    token: std::sync::Mutex<Option<String>>,
}

impl CredentialStore for MemoryCredentialStore {
    fn load(&self) -> Option<String> {
        self.token.lock().unwrap().clone()
    }

    fn save(&self, token: &str) -> Result<(), String> {
        *self.token.lock().unwrap() = Some(token.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        *self.token.lock().unwrap() = None;
        Ok(())
    }
}

/// The account row of `UploadersConfig.json`, without the token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct StoredAccount {
    /// Origin of the server, no trailing slash. Kept on sign-out: it is where the person
    /// signs back in.
    pub host: String,
    pub tag: String,
    pub name: String,
    pub email: String,
    pub avatar: String,
    pub signed_in_at: Option<DateTime<Utc>>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct UploadersFile {
    modlight_account: StoredAccount,
}

/// What the credential store holds: the token and the origin it was issued for.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BoundToken {
    host: String,
    token: String,
}

/// Whether two account hosts name the same server: the same origin, or the same text.
fn same_host(a: &str, b: &str) -> bool {
    let (a, b) = (
        a.trim().trim_end_matches('/'),
        b.trim().trim_end_matches('/'),
    );
    !a.is_empty() && (a.eq_ignore_ascii_case(b) || modlight_api::host::same_origin(a, b))
}

/// The secret written to the credential store for a token issued by `host`.
fn bind(host: &str, token: &str) -> String {
    serde_json::to_string(&BoundToken {
        host: host.to_string(),
        token: token.to_string(),
    })
    .unwrap_or_default()
}

/// The token in a stored secret, when it was issued for `host`. A secret that does not
/// name its host (written by a build before the binding) is not used either: nobody
/// can tell any more which server it belongs to.
fn token_for(secret: &str, host: &str) -> Option<String> {
    let Ok(bound) = serde_json::from_str::<BoundToken>(secret) else {
        log::warn!("Modlight: the saved token does not say which server issued it; sign in again.");
        return None;
    };
    if bound.token.is_empty() {
        return None;
    }
    if !same_host(&bound.host, host) {
        log::warn!(
            "Modlight: the saved token was issued by {}, not by {host}; sign in to {host} to use it.",
            bound.host
        );
        return None;
    }
    Some(bound.token)
}

/// The signed-in account and its token.
pub struct AccountStore {
    path: Option<PathBuf>,
    account: StoredAccount,
    token: String,
    /// The origin `token` was issued for; the token is used only while the account's
    /// host is this origin.
    token_host: String,
    credentials: Box<dyn CredentialStore>,
}

impl AccountStore {
    /// Load the account of `profile`, with its token from `credentials`.
    pub fn load(profile: &Path, credentials: Box<dyn CredentialStore>) -> AccountStore {
        let path = profile.join(UPLOADERS_CONFIG_FILE);
        let account = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<UploadersFile>(&text).ok())
            .map(|file| file.modlight_account)
            .unwrap_or_default();
        let token = if account.host.is_empty() {
            None
        } else {
            credentials
                .load()
                .and_then(|secret| token_for(&secret, &account.host))
        };
        let (token, token_host) = match token {
            Some(token) => (token, account.host.clone()),
            None => (String::new(), String::new()),
        };
        AccountStore {
            path: Some(path),
            account,
            token,
            token_host,
            credentials,
        }
    }

    /// An account that is never written anywhere (previews and tests).
    pub fn in_memory(account: StoredAccount, token: &str) -> AccountStore {
        AccountStore {
            path: None,
            token_host: account.host.clone(),
            account,
            token: token.to_string(),
            credentials: Box::new(MemoryCredentialStore::default()),
        }
    }

    pub fn account(&self) -> &StoredAccount {
        &self.account
    }

    /// The host the sign-in box offers: the last one used.
    pub fn host(&self) -> &str {
        &self.account.host
    }

    pub fn is_signed_in(&self) -> bool {
        !self.account.host.is_empty()
            && !self.token.is_empty()
            && same_host(&self.token_host, &self.account.host)
    }

    /// What to send requests with, when signed in.
    pub fn credential(&self) -> Option<Credential> {
        self.is_signed_in()
            .then(|| Credential::new(self.account.host.clone(), self.token.clone()))
    }

    /// The marker key of this account: host and tag.
    pub fn account_key(&self) -> String {
        account_key(&self.account.host, &self.account.tag)
    }

    /// Keep a token that `/api/me` just accepted. The host is the one we signed in to,
    /// never the one the answer reported.
    pub fn save_sign_in(&mut self, signed: &SignedIn) -> Result<(), String> {
        self.credentials
            .save(&bind(&signed.origin, &signed.token))?;
        self.token = signed.token.clone();
        self.token_host = signed.origin.clone();
        self.account = StoredAccount {
            host: signed.origin.clone(),
            tag: signed.account.tag.clone().unwrap_or_default(),
            name: signed.account.name.clone().unwrap_or_default(),
            email: signed.account.email.clone().unwrap_or_default(),
            avatar: signed.account.avatar.clone().unwrap_or_default(),
            signed_in_at: Some(Utc::now()),
        };
        self.write()
    }

    /// Forget the account on this machine. Local only: the token stays valid on the
    /// server until it is revoked in the panel. The host is kept.
    pub fn sign_out(&mut self) -> Result<(), String> {
        let deleted = self.credentials.delete();
        self.token.clear();
        self.token_host.clear();
        self.account.tag.clear();
        self.account.name.clear();
        self.account.email.clear();
        self.account.signed_in_at = None;
        self.write()?;
        deleted
    }

    /// Take the identity from a fresh `/api/me` answer; writes only when something moved.
    /// Never touches the host or the token: a reply must not rewrite the credential it
    /// was authorised by.
    pub fn adopt(&mut self, info: &AccountInfo) -> bool {
        let tag = info.tag.clone().unwrap_or_default();
        let name = info.name.clone().unwrap_or_default();
        let email = info.email.clone().unwrap_or_default();
        let avatar = info.avatar.clone().unwrap_or_default();
        if self.account.tag == tag
            && self.account.name == name
            && self.account.email == email
            && self.account.avatar == avatar
        {
            return false;
        }
        self.account.tag = tag;
        self.account.name = name;
        self.account.email = email;
        self.account.avatar = avatar;
        let _ = self.write();
        true
    }

    /// Name, else e-mail, else tag: what the account is called on screen.
    pub fn display_name(&self) -> String {
        [&self.account.name, &self.account.email, &self.account.tag]
            .into_iter()
            .find(|v| !v.trim().is_empty())
            .cloned()
            .unwrap_or_default()
    }

    fn write(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let file = UploadersFile {
            modlight_account: self.account.clone(),
        };
        let text = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
        write_atomically(path, &text).map_err(|e| e.to_string())
    }
}

/// Whether the signed-in account administers this server. ADVISORY, for the UI only:
/// the server checks the role on every call. Fails closed, is never persisted, and is
/// tied to WHO it was about, so another account reads as "not an administrator".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdminFlag {
    subject: String,
    answer: bool,
}

impl AdminFlag {
    /// Write down what `/api/me` said (exact `role == "admin"`).
    pub fn record(&mut self, host: &str, info: &AccountInfo) -> bool {
        self.set(
            account_key(host, info.tag.as_deref().unwrap_or("")),
            info.is_admin(),
        )
    }

    /// The token was refused, or the account was signed out of.
    pub fn forget(&mut self) -> bool {
        self.set(String::new(), false)
    }

    /// Whether `account_key` (host + tag of who is signed in now) is an administrator.
    pub fn is_admin(&self, account_key: Option<&str>) -> bool {
        match account_key {
            Some(key) => self.answer && self.subject == key,
            None => false,
        }
    }

    /// Returns whether the answer changed.
    fn set(&mut self, key: String, admin: bool) -> bool {
        let changed = self.answer != admin || self.subject != key;
        self.subject = key;
        self.answer = admin;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(tag: &str) -> SignedIn {
        SignedIn {
            origin: "http://127.0.0.1:8420".into(),
            token: "tok-123".into(),
            account: AccountInfo {
                tag: Some(tag.into()),
                name: Some("Alex Rivera".into()),
                email: Some("alex@example.com".into()),
                host: Some("https://vanity.example".into()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn sign_in_keeps_the_origin_and_hides_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = dir.path().join("credentials");
        let mut store = AccountStore::load(
            dir.path(),
            Box::new(FileCredentialStore::new(credentials.clone())),
        );
        assert!(!store.is_signed_in());
        store.save_sign_in(&signed("ToqOse")).unwrap();
        assert_eq!(
            store.host(),
            "http://127.0.0.1:8420",
            "the host we signed in to, not the link host"
        );

        let file = std::fs::read_to_string(dir.path().join(UPLOADERS_CONFIG_FILE)).unwrap();
        assert!(
            !file.contains("tok-123"),
            "the token never goes into UploadersConfig.json"
        );
        assert!(file.contains("\"Host\""));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&credentials)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let again = AccountStore::load(
            dir.path(),
            Box::new(FileCredentialStore::new(credentials.clone())),
        );
        assert!(again.is_signed_in());
        assert_eq!(again.credential().unwrap().token, "tok-123");
        assert_eq!(again.display_name(), "Alex Rivera");
        assert!(!format!("{:?}", again.credential().unwrap()).contains("tok-123"));
    }

    #[test]
    fn sign_out_keeps_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = dir.path().join("credentials");
        let mut store = AccountStore::load(
            dir.path(),
            Box::new(FileCredentialStore::new(credentials.clone())),
        );
        store.save_sign_in(&signed("ToqOse")).unwrap();
        store.sign_out().unwrap();
        assert!(!store.is_signed_in());
        assert_eq!(store.host(), "http://127.0.0.1:8420");
        assert!(store.account().tag.is_empty());
        assert!(!credentials.exists());
        let again = AccountStore::load(dir.path(), Box::new(FileCredentialStore::new(credentials)));
        assert!(!again.is_signed_in());
        assert_eq!(again.host(), "http://127.0.0.1:8420");
    }

    /// Point `UploadersConfig.json` of `profile` at another host, as an edited file or an
    /// imported backup would, leaving the stored token alone.
    fn swap_host(profile: &Path, host: &str) {
        let path = profile.join(UPLOADERS_CONFIG_FILE);
        let mut file: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        file["ModlightAccount"]["Host"] = serde_json::json!(host);
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    }

    #[test]
    fn a_host_swapped_behind_the_token_never_gets_it() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = dir.path().join("credentials");
        let store = || {
            AccountStore::load(
                dir.path(),
                Box::new(FileCredentialStore::new(credentials.clone())),
            )
        };
        store().save_sign_in(&signed("ToqOse")).unwrap();
        assert!(store().is_signed_in());

        swap_host(dir.path(), "https://attacker.example");
        let swapped = store();
        assert!(
            !swapped.is_signed_in(),
            "a token is never paired with another host"
        );
        assert!(swapped.credential().is_none());
        assert_eq!(
            swapped.host(),
            "https://attacker.example",
            "the sign-in box offers the host, which needs its own sign-in"
        );

        // The same origin written another way is still the host the token came from.
        swap_host(dir.path(), "HTTP://127.0.0.1:8420/");
        assert_eq!(store().credential().unwrap().token, "tok-123");

        // A sign-in to the new host binds the new token to it.
        swap_host(dir.path(), "https://attacker.example");
        let mut again = store();
        let mut other = signed("Other");
        other.origin = "https://files.example.com".into();
        other.token = "tok-456".into();
        again.save_sign_in(&other).unwrap();
        let reloaded = store();
        assert_eq!(reloaded.host(), "https://files.example.com");
        assert_eq!(reloaded.credential().unwrap().token, "tok-456");
    }

    #[test]
    fn a_token_that_does_not_name_its_host_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = dir.path().join("credentials");
        let mut store = AccountStore::load(
            dir.path(),
            Box::new(FileCredentialStore::new(credentials.clone())),
        );
        store.save_sign_in(&signed("ToqOse")).unwrap();
        // What a build before the binding wrote: the bare token.
        std::fs::write(&credentials, "tok-123").unwrap();
        let again = AccountStore::load(dir.path(), Box::new(FileCredentialStore::new(credentials)));
        assert!(!again.is_signed_in());
        assert_eq!(again.host(), "http://127.0.0.1:8420");
    }

    #[test]
    fn adopt_writes_only_what_moved() {
        let mut store = AccountStore::in_memory(StoredAccount::default(), "t");
        let info = AccountInfo {
            tag: Some("T".into()),
            name: Some("N".into()),
            ..Default::default()
        };
        assert!(store.adopt(&info));
        assert!(!store.adopt(&info));
    }

    #[test]
    fn the_admin_flag_is_about_one_account() {
        let mut flag = AdminFlag::default();
        assert!(!flag.is_admin(Some("h\nT")));
        let info = AccountInfo {
            tag: Some("T".into()),
            role: Some("admin".into()),
            ..Default::default()
        };
        assert!(flag.record("h", &info));
        assert!(flag.is_admin(Some(&account_key("h", "T"))));
        assert!(
            !flag.is_admin(Some(&account_key("h", "U"))),
            "another account is not covered"
        );
        assert!(!flag.is_admin(None));
        let not_admin = AccountInfo {
            tag: Some("T".into()),
            role: Some("admins".into()),
            ..Default::default()
        };
        flag.record("h", &not_admin);
        assert!(
            !flag.is_admin(Some(&account_key("h", "T"))),
            "exact match only"
        );
        flag.forget();
        assert!(!flag.is_admin(Some(&account_key("h", "T"))));
    }
}

//! Not upstream: accounts whose files are encrypted on the server (the server's
//! `docs/ENCRYPTION.md`, section 6, "API for bearer clients").
//!
//! Such an account's file CONTENTS (`/raw`, `/download`, `/convert`, thumbnails, moving
//! tiles, album ZIPs) open only for a request that carries a grant, which the file
//! password buys: `POST /api/encryption/unlock` answers with one, valid for 30 days.
//! While the app holds a grant for a credential ([`Api::hold_grant`]), every content
//! request of that credential carries it in `X-Modlight-Unlock`, and no other request
//! does. A content request without a grant that opens the files is answered
//! `423 Locked`, which is [`ApiError::Locked`]: never "the token is invalid" (that stays
//! 401 and keeps signing out), and never a picture that does not exist.
//!
//! Listings, renames, moves, the trash and uploads need nothing new.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::Deserialize;

use crate::client::{Api, Credential, REQUEST_TIMEOUT};
use crate::error::{ApiError, ApiResult, FilesLocked};
use crate::host::is_plausible_token;
use crate::models::parse_time;

/// The vault's state and whether this request's grant opens it.
pub const ENCRYPTION_PATH: &str = "/api/encryption";
/// The file password in, a grant out.
pub const UNLOCK_PATH: &str = "/api/encryption/unlock";
/// Revoke the grant the request carries.
pub const LOCK_PATH: &str = "/api/encryption/lock";
/// The header a grant travels in.
pub const UNLOCK_HEADER: &str = "x-modlight-unlock";
/// The header every locked answer carries.
pub const LOCKED_HEADER: &str = "x-modlight-locked";
/// How long a rate-limited unlock waits when the server did not say.
pub const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Unlocking runs Argon2 on the server and may wait for a free slot there.
pub const UNLOCK_TIMEOUT: Duration = Duration::from_secs(45);

/// The most of a locked answer read to learn whose files they are.
const LOCKED_BODY_LIMIT: usize = 4 * 1024;

/// Where the account's encryption stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultState {
    /// Not encrypted: nothing is ever locked.
    #[default]
    Off,
    /// Being switched on: files are encrypted one by one, and every one of them (done or
    /// not) already needs a grant.
    Encrypting,
    /// Encrypted.
    On,
    /// Being switched off with the password: nobody is asked any more.
    Decrypting,
    /// A state this build does not know. Treated as not asking: a refusal of a file
    /// still arrives as 423 and says so.
    #[serde(other)]
    Unknown,
}

impl VaultState {
    /// Whether reading a file's contents needs a grant in this state.
    pub fn is_gated(self) -> bool {
        matches!(self, VaultState::Encrypting | VaultState::On)
    }
}

/// How far the background pass has come (`encrypting` and `decrypting`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct VaultProgress {
    pub done: i64,
    pub total: i64,
}

/// `GET /api/encryption`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct EncryptionStatus {
    pub state: VaultState,
    /// Whether the grant this request carried opens the files.
    pub unlocked: bool,
    pub progress: VaultProgress,
}

/// [`EncryptionStatus`] and the grant it was asked with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VaultAnswer {
    pub status: EncryptionStatus,
    /// The grant the request carried, by its number (see [`FilesLocked::grant`]).
    pub grant: Option<u64>,
}

/// A grant: the `<id>.<secret>` value `X-Modlight-Unlock` carries, and when the server
/// stops honouring it. A secret like the token: `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct UnlockGrant {
    pub value: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl UnlockGrant {
    pub fn new(value: impl Into<String>, expires_at: Option<DateTime<Utc>>) -> UnlockGrant {
        UnlockGrant {
            value: value.into(),
            expires_at,
        }
    }

    /// Whether the server has stopped honouring it by `now`.
    pub fn has_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|at| at <= now)
    }
}

impl std::fmt::Debug for UnlockGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockGrant")
            .field("value", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Why the file password did not unlock the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockFailure {
    /// 401 `wrong_password`.
    WrongPassword,
    /// 429: too many attempts; the server takes the next one after this long.
    RateLimited(Duration),
    /// 409: the account's files are not encrypted (any more).
    NotEncrypted,
    /// 400: the server did not take the request as it was (an empty password).
    Invalid,
    /// Anything else: the token refused (a 401 that is not about the password, 403), a
    /// server error, an unreadable answer, no answer at all.
    Failed(ApiError),
}

/// The JSON of the vault's refusals: `{"error": "...", "retryAfter": <s>}` or a locked
/// answer's `{"error": "files_locked", "owner": "<tag>"}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct VaultRefusal {
    error: Option<String>,
    retry_after: Option<serde_json::Value>,
    owner: Option<String>,
}

/// `200 {"grant": "...", "expiresAt": "..."}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Granted {
    grant: Option<String>,
    expires_at: Option<String>,
}

/// The grant the app holds, and for which credential. Shared by every clone of an [`Api`].
#[derive(Default)]
pub(crate) struct GrantSlot {
    held: Option<HeldGrant>,
    /// The numbers handed out so far.
    issued: u64,
}

struct HeldGrant {
    credential: Credential,
    value: String,
    number: u64,
}

pub(crate) type SharedGrantSlot = std::sync::Arc<Mutex<GrantSlot>>;

fn slot(shared: &SharedGrantSlot) -> MutexGuard<'_, GrantSlot> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Api {
    /// Send `grant` with every content request of `credential` from now on, in place of
    /// any grant held before (for this or another credential). Returns its number, which
    /// [`FilesLocked::grant`] and [`VaultAnswer::grant`] name it by.
    pub fn hold_grant(&self, credential: &Credential, grant: &UnlockGrant) -> u64 {
        let mut slot = slot(self.grants());
        slot.issued += 1;
        let number = slot.issued;
        slot.held = Some(HeldGrant {
            credential: credential.clone(),
            value: grant.value.clone(),
            number,
        });
        number
    }

    /// Stop sending the grant held, whatever it is. Returns its number.
    pub fn release_grant(&self) -> Option<u64> {
        slot(self.grants()).held.take().map(|held| held.number)
    }

    /// Stop sending grant `number`, if it is still the one held. Returns whether it was.
    pub fn release_grant_numbered(&self, number: u64) -> bool {
        let mut slot = slot(self.grants());
        if slot.held.as_ref().is_some_and(|held| held.number == number) {
            slot.held = None;
            true
        } else {
            false
        }
    }

    /// The number of the grant `credential`'s content requests carry, if any.
    pub fn held_grant(&self, credential: &Credential) -> Option<u64> {
        slot(self.grants())
            .held
            .as_ref()
            .filter(|held| held.credential == *credential)
            .map(|held| held.number)
    }

    /// A request that reads a file's contents: the credential in its header, and the
    /// grant held for that very credential in `X-Modlight-Unlock`. Returns which grant it
    /// carries.
    pub(crate) fn content_request(
        &self,
        method: Method,
        url: &str,
        credential: &Credential,
    ) -> (RequestBuilder, Option<u64>) {
        let request = self.request(method, url, credential);
        let held = slot(self.grants())
            .held
            .as_ref()
            .filter(|held| held.credential == *credential)
            .map(|held| (held.value.clone(), held.number));
        match held {
            Some((value, number)) => (request.header(UNLOCK_HEADER, value), Some(number)),
            None => (request, None),
        }
    }

    /// `GET /api/encryption`: the vault's state, and whether the grant held for
    /// `credential` (sent along) opens it. A server without file encryption (404) reads
    /// as `off`: nothing of it is ever locked.
    pub async fn get_encryption(&self, credential: &Credential) -> ApiResult<VaultAnswer> {
        let (request, grant) =
            self.content_request(Method::GET, &credential.url(ENCRYPTION_PATH), credential);
        let response = request
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ApiError::Unauthorized);
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(VaultAnswer {
                status: EncryptionStatus::default(),
                grant,
            });
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16()));
        }
        let text = response.text().await.map_err(ApiError::from_reqwest)?;
        let status = serde_json::from_str::<EncryptionStatus>(&text)
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(VaultAnswer { status, grant })
    }

    /// `POST /api/encryption/unlock`: the file password for a grant. `label` names this
    /// app in the account's list of browsers and apps that remember the password.
    ///
    /// The password is never logged; the request carries no grant (it asks for one).
    pub async fn unlock_files(
        &self,
        credential: &Credential,
        password: &str,
        label: &str,
    ) -> Result<UnlockGrant, UnlockFailure> {
        let mut body = serde_json::Map::new();
        body.insert("password".into(), serde_json::Value::from(password));
        if !label.trim().is_empty() {
            body.insert("label".into(), serde_json::Value::from(label.trim()));
        }
        let bytes = serde_json::to_vec(&serde_json::Value::Object(body))
            .map_err(|e| UnlockFailure::Failed(ApiError::Decode(e.to_string())))?;
        let response = self
            .request(Method::POST, &credential.url(UNLOCK_PATH), credential)
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .body(bytes)
            .timeout(UNLOCK_TIMEOUT)
            .send()
            .await
            .map_err(|e| UnlockFailure::Failed(ApiError::from_reqwest(e)))?;
        let status = response.status();
        let retry_header = retry_after_header(&response);
        let text = response
            .text()
            .await
            .map_err(|e| UnlockFailure::Failed(ApiError::from_reqwest(e)))?;
        if status.is_success() {
            let granted: Granted = serde_json::from_str(&text)
                .map_err(|e| UnlockFailure::Failed(ApiError::Decode(e.to_string())))?;
            let value = granted.grant.unwrap_or_default();
            // It goes into a header on every content request: a value that is not one
            // the server issues is refused here, not sent.
            if !is_plausible_token(&value) || !value.contains('.') {
                return Err(UnlockFailure::Failed(ApiError::Decode(
                    "the answer holds no usable grant".into(),
                )));
            }
            let expires_at = granted.expires_at.as_deref().and_then(parse_time);
            return Ok(UnlockGrant { value, expires_at });
        }
        let refusal: VaultRefusal = serde_json::from_str(&text).unwrap_or_default();
        Err(match status.as_u16() {
            // A 401 about the password says so; any other 401 is about the token, which
            // keeps meaning what it always meant.
            401 if refusal.error.as_deref() == Some("wrong_password") => {
                UnlockFailure::WrongPassword
            }
            401 | 403 => UnlockFailure::Failed(ApiError::Unauthorized),
            409 => UnlockFailure::NotEncrypted,
            429 => UnlockFailure::RateLimited(
                seconds_of(refusal.retry_after.as_ref())
                    .or(retry_header)
                    .unwrap_or(DEFAULT_RETRY_AFTER),
            ),
            400 => UnlockFailure::Invalid,
            other => UnlockFailure::Failed(ApiError::Status(other)),
        })
    }

    /// `POST /api/encryption/lock`: revoke `grant` on the server. It is sent explicitly
    /// (not from what is held), so a grant already let go of here can still be revoked.
    pub async fn lock_files(&self, credential: &Credential, grant: &UnlockGrant) -> ApiResult<()> {
        let response = self
            .request(Method::POST, &credential.url(LOCK_PATH), credential)
            .header(UNLOCK_HEADER, grant.value.as_str())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16()));
        }
        Ok(())
    }
}

/// Whether a response is the vault's refusal of a content request (`423 Locked`).
pub(crate) fn is_locked(response: &Response) -> bool {
    response.status().as_u16() == 423
}

/// The locked answer of a content request that carried grant `grant`, its body read (a
/// few bytes at most) for whose files they are.
pub(crate) async fn locked(response: Response, grant: Option<u64>) -> FilesLocked {
    let owner = match Api::read_limited(response, LOCKED_BODY_LIMIT).await {
        Ok(Some(bytes)) => owner_of(&bytes),
        _ => None,
    };
    FilesLocked { owner, grant }
}

/// The locked answer of a request whose body was already read as text.
pub(crate) fn locked_from_text(text: &str, grant: Option<u64>) -> FilesLocked {
    FilesLocked {
        owner: owner_of(text.as_bytes()),
        grant,
    }
}

fn owner_of(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<VaultRefusal>(bytes)
        .ok()
        .and_then(|refusal| refusal.owner)
        .filter(|owner| !owner.trim().is_empty())
}

/// `retryAfter` as the body writes it: whole seconds, as a number or a string.
fn seconds_of(value: Option<&serde_json::Value>) -> Option<Duration> {
    let seconds = match value? {
        serde_json::Value::Number(n) => n.as_f64()?,
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs(seconds.ceil() as u64))
}

/// `Retry-After` in seconds (the HTTP date form is not used by this server).
fn retry_after_header(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_read_as_the_server_writes_them() {
        let read = |text: &str| serde_json::from_str::<EncryptionStatus>(text).unwrap();
        let on = read(r#"{"state":"on","unlocked":false,"progress":{"done":3,"total":9}}"#);
        assert_eq!(on.state, VaultState::On);
        assert!(on.state.is_gated());
        assert!(!on.unlocked);
        assert_eq!(on.progress, VaultProgress { done: 3, total: 9 });
        assert!(
            read(r#"{"state":"encrypting","unlocked":true}"#)
                .state
                .is_gated()
        );
        assert!(!read(r#"{"state":"decrypting"}"#).state.is_gated());
        assert!(!read(r#"{"state":"off"}"#).state.is_gated());
        // A state this build does not know asks for nothing; neither does no state.
        assert_eq!(read(r#"{"state":"rotating"}"#).state, VaultState::Unknown);
        assert!(!read(r#"{"state":"rotating"}"#).state.is_gated());
        assert_eq!(read("{}").state, VaultState::Off);
    }

    #[test]
    fn a_grant_is_never_printed() {
        let grant = UnlockGrant::new("0f0e.c2VjcmV0", None);
        assert!(!format!("{grant:?}").contains("c2VjcmV0"));
    }

    #[test]
    fn retry_after_reads_numbers_and_text() {
        assert_eq!(
            seconds_of(Some(&serde_json::json!(90))),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            seconds_of(Some(&serde_json::json!("12"))),
            Some(Duration::from_secs(12))
        );
        assert_eq!(
            seconds_of(Some(&serde_json::json!(1.2))),
            Some(Duration::from_secs(2))
        );
        assert_eq!(seconds_of(Some(&serde_json::json!(-1))), None);
        assert_eq!(seconds_of(Some(&serde_json::json!(null))), None);
        assert_eq!(seconds_of(None), None);
    }

    #[test]
    fn grants_go_only_with_their_credential() {
        let api = Api::new("test").unwrap();
        let mine = Credential::new("https://files.example.com", "token-a");
        let other = Credential::new("https://files.example.com", "token-b");
        assert_eq!(api.held_grant(&mine), None);
        let first = api.hold_grant(&mine, &UnlockGrant::new("1.a", None));
        assert_eq!(api.held_grant(&mine), Some(first));
        assert_eq!(api.held_grant(&other), None, "another token gets nothing");
        // A clone shares what is held.
        assert_eq!(api.clone().held_grant(&mine), Some(first));
        let second = api.hold_grant(&mine, &UnlockGrant::new("2.b", None));
        assert_ne!(first, second);
        assert!(
            !api.release_grant_numbered(first),
            "an older number is not held"
        );
        assert_eq!(api.held_grant(&mine), Some(second));
        assert!(api.release_grant_numbered(second));
        assert_eq!(api.held_grant(&mine), None);
        api.hold_grant(&other, &UnlockGrant::new("3.c", None));
        assert!(api.release_grant().is_some());
        assert_eq!(api.release_grant(), None);
    }
}

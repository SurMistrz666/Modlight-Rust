//! What a request may read of one account's files.
//!
//! The rule (docs/ENCRYPTION.md section 4): while an account's vault is
//! `encrypting` or `on`, its files are readable only by a request carrying a
//! live grant of that account, in the cookie `ml_vault_<ownerId>` (browsers)
//! or the header `X-Modlight-Unlock` (API clients). Nobody is exempt: not the
//! owner's session, not an administrator. While the vault is `decrypting` the
//! pass's key reads the files that are still encrypted.

use std::sync::Arc;

use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::keys::{AccountSecret, server_key, unbox_for_job, unwrap_grant};
use super::{UNLOCK_HEADER, VaultState, cookie_name, store};
use crate::db::models::FileVault;
use crate::http::cookies::parse_cookies_next;
use crate::state::AppState;

/// One account's files, as this request may see them.
#[derive(Clone)]
pub enum Access {
    /// The account has no vault: everything as before.
    Plain,
    /// Readable: a live grant, or the vault is being decrypted.
    Unlocked { secret: Arc<AccountSecret>, grant: Option<Uuid> },
    /// The vault asks for the password and this request has no live grant.
    Locked,
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Access::Plain => f.write_str("Plain"),
            Access::Unlocked { grant, .. } => write!(f, "Unlocked({grant:?})"),
            Access::Locked => f.write_str("Locked"),
        }
    }
}

impl Access {
    pub fn is_locked(&self) -> bool {
        matches!(self, Access::Locked)
    }

    /// The account has a vault that asks for the password (locked or not):
    /// what unfurlers and caches must treat like a password.
    pub fn is_vault(&self) -> bool {
        !matches!(self, Access::Plain)
    }

    pub fn secret(&self) -> Option<&AccountSecret> {
        match self {
            Access::Unlocked { secret, .. } => Some(secret),
            _ => None,
        }
    }
}

/// `<grant id>.<secret, base64url>`
pub fn encode_grant(id: Uuid, secret: &[u8; 32]) -> Zeroizing<String> {
    Zeroizing::new(format!("{id}.{}", URL_SAFE_NO_PAD.encode(secret)))
}

pub fn parse_grant(value: &str) -> Option<(Uuid, Zeroizing<[u8; 32]>)> {
    let (id, secret) = crate::config::js_trim(value).split_once('.')?;
    let id = Uuid::parse_str(id).ok()?;
    let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(secret).ok()?);
    let arr: [u8; 32] = bytes.as_slice().try_into().ok()?;
    Some((id, Zeroizing::new(arr)))
}

/// The grant values a request offers for `owner_id`: its cookie, then the
/// header.
pub fn offered_grants(headers: &HeaderMap, owner_id: &str) -> Vec<Zeroizing<String>> {
    let mut out = Vec::new();
    if let Some(v) = parse_cookies_next(headers).get(&cookie_name(owner_id)) {
        out.push(Zeroizing::new(v.clone()));
    }
    for v in headers.get_all(UNLOCK_HEADER) {
        if let Ok(s) = v.to_str() {
            out.push(Zeroizing::new(s.to_string()));
        }
    }
    out
}

/// Open a grant value for `vault`: the row must be live, of the vault's
/// generation, and open with the secret (the AEAD tag is the check).
pub async fn open_grant(
    state: &AppState,
    vault: &FileVault,
    value: &str,
) -> anyhow::Result<Option<(Uuid, AccountSecret)>> {
    let Some((id, secret)) = parse_grant(value) else {
        return Ok(None);
    };
    let Some(row) = store::live_grant(&state.db, id, &vault.user_id, vault.generation).await? else {
        return Ok(None);
    };
    let server = server_key(&state.config.auth.secret)?;
    match unwrap_grant(&row.wrapped_key, &server, &secret, &id.to_string(), &vault.user_id, &vault.public_key) {
        Ok(account) => {
            store::touch_grant(&state.db, id);
            Ok(Some((id, account)))
        }
        Err(_) => Ok(None),
    }
}

/// What this request may do with `owner_id`'s files, given its vault row.
pub async fn access_with(state: &AppState, headers: &HeaderMap, vault: Option<&FileVault>) -> anyhow::Result<Access> {
    let Some(vault) = vault else {
        return Ok(Access::Plain);
    };
    match store::state_of(vault) {
        Some(VaultState::Decrypting) => {
            // Switched off with the password: the pass's own key reads what is
            // still encrypted, and nobody is asked any more.
            let Some(boxed) = vault.job_key.as_deref() else {
                return Ok(Access::Locked);
            };
            match unbox_for_job(boxed, &state.config.auth.secret, &vault.public_key) {
                Ok(secret) => Ok(Access::Unlocked { secret: Arc::new(secret), grant: None }),
                Err(_) => Ok(Access::Locked),
            }
        }
        Some(_) => {
            for value in offered_grants(headers, &vault.user_id) {
                if let Some((id, secret)) = open_grant(state, vault, &value).await? {
                    return Ok(Access::Unlocked { secret: Arc::new(secret), grant: Some(id) });
                }
            }
            Ok(Access::Locked)
        }
        // A state this build does not know: fail closed.
        None => Ok(Access::Locked),
    }
}

/// [`access_with`] for an owner by id.
pub async fn owner_access(state: &AppState, headers: &HeaderMap, owner_id: &str) -> anyhow::Result<Access> {
    let vault = store::load(&state.db, owner_id).await?;
    access_with(state, headers, vault.as_ref()).await
}

/// The state a request may read `uploads` (all of `owner_id`) with: unchanged
/// for an account without a vault, a decrypting view for an unlocked one,
/// None when the request must be answered with the locked gate.
///
/// `owner_vault` is what the lookup already knows (`MediaLookup::owner_vault`),
/// so an account without a vault costs no query here.
pub async fn open_files(
    state: &AppState,
    headers: &HeaderMap,
    owner_id: &str,
    owner_vault: Option<&str>,
    uploads: &[&crate::db::models::Upload],
) -> anyhow::Result<Option<AppState>> {
    let any_encrypted = uploads.iter().any(|u| u.is_encrypted());
    if owner_vault.is_none() && !any_encrypted {
        return Ok(Some(state.clone()));
    }
    let access = owner_access(state, headers, owner_id).await?;
    if access.is_locked() || (any_encrypted && access.secret().is_none()) {
        return Ok(None);
    }
    Ok(Some(super::view::readable(state, access.secret(), uploads)?))
}

/// The grant id this request's cookie or header names for `owner_id`, valid or
/// not (to revoke "this browser").
pub fn offered_grant_ids(headers: &HeaderMap, owner_id: &str) -> Vec<Uuid> {
    offered_grants(headers, owner_id).iter().filter_map(|v| parse_grant(v).map(|(id, _)| id)).collect()
}

/// Whose files a path of the form is for: `/{tag}/{code}/...` (a file),
/// `/a/{tag}/...` (an album and its ZIP), `/u/{tag}` (a gallery).
pub async fn owner_of_path(state: &AppState, next: &str) -> anyhow::Result<Option<String>> {
    let path = next.split(['?', '#']).next().unwrap_or("");
    let segments: Vec<String> = path
        .trim_start_matches('/')
        .split('/')
        .map(|s| percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned())
        .collect();
    match segments.as_slice() {
        [first, tag, ..] if (first == "a" || first == "u") && !tag.is_empty() => owner_by_tag(state, tag).await,
        [tag, code, ..] if !tag.is_empty() && !code.is_empty() => {
            let code = crate::http::cookies::encode_uri_component(code);
            Ok(match crate::media::serve::lookup_media(&state.db, tag, &code).await {
                Ok(crate::media::serve::Lookup::Live(found) | crate::media::serve::Lookup::Spent(found)) => {
                    Some(found.owner_id)
                }
                _ => None,
            })
        }
        _ => Ok(None),
    }
}

/// An account by its current or a retired tag.
pub async fn owner_by_tag(state: &AppState, tag: &str) -> anyhow::Result<Option<String>> {
    let owner: Option<String> = sqlx::query_scalar(
        r#"select "id" from "user" where "user_code" = $1
           union all select "user_id" from "user_code_aliases" where "code" = $1 limit 1"#,
    )
    .bind(tag)
    .fetch_optional(&state.db)
    .await?;
    Ok(owner)
}

/// What a public gallery or album of `owner_id` may show this request
/// (docs/ENCRYPTION.md section 4): None for an account without a vault that
/// asks; otherwise whether this request is locked out. The page drops its
/// `og:image` either way (an unfurler holds no grant).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryVault {
    pub locked: bool,
    /// The owner's tag, the target of the unlock form.
    pub tag: String,
}

pub async fn gallery_vault(
    state: &AppState,
    headers: &HeaderMap,
    owner_id: &str,
    tag: &str,
) -> anyhow::Result<Option<GalleryVault>> {
    let Some(vault) = store::load(&state.db, owner_id).await? else {
        return Ok(None);
    };
    if !store::state_of(&vault).is_some_and(VaultState::is_gated) {
        return Ok(None);
    }
    let locked = access_with(state, headers, Some(&vault)).await?.is_locked();
    Ok(Some(GalleryVault { locked, tag: tag.to_string() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn grant_values_round_trip_and_junk_is_refused() {
        let id = Uuid::new_v4();
        let secret = [3u8; 32];
        let text = encode_grant(id, &secret);
        let (back, s) = parse_grant(&text).unwrap();
        assert_eq!(back, id);
        assert_eq!(*s, secret);
        for junk in [
            "",
            ".",
            "abc.def",
            &format!("{id}."),
            &format!("{id}.AAAA"),
            "not-a-uuid.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(parse_grant(junk).is_none(), "{junk}");
        }
    }

    #[test]
    fn grants_come_from_the_owners_cookie_or_the_header() {
        let mut h = HeaderMap::new();
        h.insert("cookie", HeaderValue::from_static("ml_vault_u1=a.b; ml_vault_u2=c.d"));
        h.append(UNLOCK_HEADER, HeaderValue::from_static("e.f"));
        let got: Vec<String> = offered_grants(&h, "u1").iter().map(|z| z.to_string()).collect();
        assert_eq!(got, ["a.b", "e.f"]);
        let other: Vec<String> = offered_grants(&h, "u3").iter().map(|z| z.to_string()).collect();
        assert_eq!(other, ["e.f"]);
    }
}

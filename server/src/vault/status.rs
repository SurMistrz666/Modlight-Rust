//! What the panel shows about a file vault: the Settings card's status, the
//! library's banner (docs/ENCRYPTION.md section 8).

use axum::http::HeaderMap;
use serde::Serialize;
use uuid::Uuid;

use super::access::{Access, access_with};
use super::store::{self, Progress};
use super::{MIN_PASSWORD_CHARS, VaultState};
use crate::db::models::VaultGrant;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantView {
    pub id: String,
    /// "owner" | "visitor" | "app"
    pub kind: String,
    pub label: Option<String>,
    pub ip: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: String,
    /// The grant this browser holds.
    pub current: bool,
}

/// The Settings card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// "off" | "encrypting" | "on" | "decrypting"
    pub state: &'static str,
    /// This browser holds a live grant.
    pub unlocked: bool,
    pub progress: Progress,
    /// The pass could not finish some files (it retries).
    pub error: bool,
    pub since: Option<String>,
    pub grants: Vec<GrantView>,
    pub min_length: usize,
}

fn grant_view(g: &VaultGrant, current: Option<Uuid>) -> GrantView {
    GrantView {
        id: g.id.to_string(),
        kind: g.kind.clone(),
        label: g.label.clone(),
        ip: g.ip.clone(),
        created_at: g.created_at.to_iso_string(),
        last_used_at: g.last_used_at.map(|d| d.to_iso_string()),
        expires_at: g.expires_at.to_iso_string(),
        current: Some(g.id) == current,
    }
}

pub async fn vault_status(state: &AppState, headers: &HeaderMap, user_id: &str) -> anyhow::Result<VaultStatus> {
    let Some(vault) = store::load(&state.db, user_id).await? else {
        return Ok(VaultStatus {
            state: "off",
            unlocked: false,
            progress: Progress::default(),
            error: false,
            since: None,
            grants: Vec::new(),
            min_length: MIN_PASSWORD_CHARS,
        });
    };
    let vault_state = store::state_of(&vault).unwrap_or(VaultState::On);
    let current = match access_with(state, headers, Some(&vault)).await? {
        Access::Unlocked { grant, .. } => grant,
        _ => None,
    };
    let grants = store::list_grants(&state.db, user_id).await?;
    Ok(VaultStatus {
        state: vault_state.as_str(),
        unlocked: vault_state.is_gated() && current.is_some(),
        progress: store::progress(&state.db, user_id, vault_state).await?,
        error: vault.job_error.is_some(),
        since: Some(vault.created_at.to_iso_string()),
        grants: grants.iter().map(|g| grant_view(g, current)).collect(),
        min_length: MIN_PASSWORD_CHARS,
    })
}

/// The library's banner: present only while this browser is locked out of
/// the account's own files.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryVault {
    pub state: &'static str,
    pub unlocked: bool,
}

pub async fn library_vault(
    state: &AppState,
    headers: &HeaderMap,
    user_id: &str,
) -> anyhow::Result<Option<LibraryVault>> {
    let Some(vault) = store::load(&state.db, user_id).await? else {
        return Ok(None);
    };
    let vault_state = store::state_of(&vault).unwrap_or(VaultState::On);
    if !vault_state.is_gated() {
        return Ok(None);
    }
    let unlocked = !access_with(state, headers, Some(&vault)).await?.is_locked();
    Ok(Some(LibraryVault { state: vault_state.as_str(), unlocked }))
}

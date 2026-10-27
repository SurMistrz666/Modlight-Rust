//! Loader of `app/(app)/dashboard/profile/page.tsx` (CONTRACT 6.18).
//!
//! Upstream: `requireUser()`, then `userStats`, `getUserPrefs`, `folderTree`,
//! `myUrlPrefs()` and `listMySessions()` in parallel. The page passed most of
//! that to its client components piecemeal; the SPA port takes the same
//! values as one props object (`ProfilePageProps` in
//! `web/src/pages/(app)/dashboard/profile/page.tsx`) and derives the component
//! props itself. Three values were derived from server-only inputs, so they
//! arrive ready: the flattened folder list, the avatar URL (over `user.image`,
//! a storage key the browser must not see) and the wallpaper URL (over
//! `background_key`, which embeds the internal user id).
//!
//! Never sent: `user.id`, `user.image`, `background_key`,
//! `default_password_hash`, any session token.

use serde::Serialize;
use uuid::Uuid;

use crate::actions::account::{UrlPrefs, url_prefs};
use crate::avatar::avatar_url;
use crate::background::background_url;
use crate::data::folders::{FolderNode, folder_tree};
use crate::data::sessions::{SessionView, list_my_sessions};
use crate::data::uploads::{UserStats, user_stats};
use crate::data::user_prefs::get_user_prefs;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageResult};

/// The session user's fields the page reads (raw: the page applies its own
/// defaults).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileUser {
    pub name: String,
    pub email: String,
    pub user_code: Option<String>,
    pub hide_uploads: Option<bool>,
    pub copy_raw: Option<bool>,
    pub used_quota: Option<i64>,
    pub max_quota: Option<i64>,
    pub role: Option<String>,
    pub theme: Option<String>,
    pub locale: Option<String>,
    pub two_factor_enabled: Option<bool>,
    pub email_verified: bool,
}

/// `getUserPrefs(u.id)`, the fields the page reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePrefs {
    pub user_code: Option<String>,
    pub skip_tag_in_url: bool,
    pub default_folder_id: Option<Uuid>,
    pub background_dim: i32,
}

/// One option of the default-folder select (`FolderOption`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FolderOption {
    pub id: Uuid,
    pub name: String,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileProps {
    pub u: ProfileUser,
    pub stats: UserStats,
    pub prefs: Option<ProfilePrefs>,
    pub url_prefs: UrlPrefs,
    pub sessions: Vec<SessionView>,
    pub folders: Vec<FolderOption>,
    pub avatar_url: Option<String>,
    pub background_url: Option<String>,
    /// Not upstream: the "File encryption" card (docs/ENCRYPTION.md).
    pub encryption: crate::vault::status::VaultStatus,
}

/// Depth-first flatten so the select can indent nested folders.
pub fn flatten_folders(nodes: &[FolderNode], depth: usize) -> Vec<FolderOption> {
    nodes
        .iter()
        .flat_map(|n| {
            std::iter::once(FolderOption { id: n.id, name: n.name.clone(), depth })
                .chain(flatten_folders(&n.children, depth + 1))
        })
        .collect()
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_user().await?;
    let db = &ctx.state.db;
    let u = &session.user;

    // `skipTagInUrl` / `defaultFolderId` are not part of Better Auth's session
    // payload, so they are read straight from the row. The sessions are read on
    // every load rather than cached: a list of live sessions that is a minute
    // stale is a list that can show a session somebody just revoked.
    let (stats, prefs, tree, url_prefs, sessions) = tokio::try_join!(
        user_stats(db, &u.id),
        get_user_prefs(db, &u.id),
        folder_tree(db, &u.id),
        url_prefs(db, &ctx.state.config, &u.id),
        list_my_sessions(db, &session),
    )?;
    let encryption = crate::vault::status::vault_status(&ctx.state, &ctx.headers, &u.id).await?;

    let props = ProfileProps {
        u: ProfileUser {
            name: u.name.clone(),
            email: u.email.clone(),
            user_code: u.user_code.clone(),
            hide_uploads: u.hide_uploads,
            copy_raw: u.copy_raw,
            used_quota: u.used_quota,
            max_quota: u.max_quota,
            role: u.role.clone(),
            theme: u.theme.clone(),
            locale: u.locale.clone(),
            two_factor_enabled: u.two_factor_enabled,
            email_verified: u.email_verified,
        },
        stats,
        avatar_url: avatar_url(
            prefs.as_ref().and_then(|p| p.user_code.as_deref()).or(u.user_code.as_deref()),
            u.image.as_deref(),
        ),
        background_url: prefs
            .as_ref()
            .and_then(|p| background_url(p.user_code.as_deref(), p.background_key.as_deref())),
        prefs: prefs.map(|p| ProfilePrefs {
            user_code: p.user_code,
            skip_tag_in_url: p.skip_tag_in_url,
            default_folder_id: p.default_folder_id,
            background_dim: p.background_dim,
        }),
        url_prefs,
        sessions,
        folders: flatten_folders(&tree, 0),
        encryption,
    };
    PageData::new(&props, PageMeta::titled("Profile"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::types::JsDate;

    fn node(id: u128, name: &str, children: Vec<FolderNode>) -> FolderNode {
        FolderNode {
            id: Uuid::from_u128(id),
            parent_id: None,
            name: name.into(),
            slug: name.into(),
            is_public: false,
            sort_order: 0,
            created_at: JsDate::from_millis(0).unwrap(),
            count: 0,
            total_count: 0,
            size: 0,
            total_size: 0,
            children,
        }
    }

    #[test]
    fn flattens_pre_order_with_depths() {
        let tree = vec![
            node(1, "a", vec![node(2, "b", vec![node(3, "c", vec![])]), node(4, "d", vec![])]),
            node(5, "e", vec![]),
        ];
        let flat: Vec<(String, usize)> = flatten_folders(&tree, 0).into_iter().map(|f| (f.name, f.depth)).collect();
        assert_eq!(flat, vec![("a".into(), 0), ("b".into(), 1), ("c".into(), 2), ("d".into(), 1), ("e".into(), 0)]);
        assert_eq!(
            serde_json::to_value(&flatten_folders(&tree, 0)[0]).unwrap(),
            serde_json::json!({ "id": "00000000-0000-0000-0000-000000000001", "name": "a", "depth": 0 })
        );
    }
}

//! Loader of `app/(app)/layout.tsx`: the panel shell around every
//! `/dashboard...` page (CONTRACT 5.8).
//!
//! Upstream renders, after `requireUser()` (signed out -> 307 `/login`):
//! - `<ThemeApplier theme={session.user.theme} background={...}>`: the skin
//!   and wallpaper (the document paints them from this payload,
//!   [`crate::pages::DocumentSkin::from_app_layout`]);
//! - `<TransfersProvider folders defaultFolderId>`: the upload queue that
//!   outlives the page it started on;
//! - `<AppShell user sidebar defaultCollapsed>`;
//! - `<DuplicateReminderSlot userId>` inside the shell.
//!
//! Payload (`layouts["(app)"]`, `AppLayoutData` in the SPA): `{ theme,
//! background, transferFolders, defaultFolderId, user, sidebar,
//! defaultCollapsed, duplicateReminder }`.

use indexmap::IndexMap;
use serde::Serialize;
use uuid::Uuid;

use crate::avatar::avatar_url;
use crate::background::{BackgroundStyle, background_style};
use crate::data::duplicates::{SNOOZE_COOKIE, duplicate_reminder, is_snoozed};
use crate::data::folders::{FolderNode, folder_tree};
use crate::data::uploads::{kind_counts, user_stats};
use crate::data::user_prefs::get_user_prefs;
use crate::db::types::JsDate;
use crate::pages::{LayoutResult, PageCtx};
use crate::rpc::tagged;

/// The cookie the shell's collapse toggle writes.
pub const SIDEBAR_COOKIE: &str = "ml_sidebar";

/// `FolderTreeNode` (components/dashboard/FolderTree.tsx).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderTreeNode {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub is_public: bool,
    pub count: i64,
    pub total_count: i64,
    pub children: Vec<FolderTreeNode>,
}

/// Drop the server-only fields (sortOrder, createdAt, sizes) before crossing
/// to the client.
pub fn to_tree_nodes(nodes: &[FolderNode]) -> Vec<FolderTreeNode> {
    nodes
        .iter()
        .map(|n| FolderTreeNode {
            id: n.id,
            parent_id: n.parent_id,
            name: n.name.clone(),
            slug: n.slug.clone(),
            is_public: n.is_public,
            count: n.count,
            total_count: n.total_count,
            children: to_tree_nodes(&n.children),
        })
        .collect()
}

/// `TransferFolder` (components/transfers/TransfersProvider.tsx).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferFolder {
    pub id: Uuid,
    pub name: String,
}

/// Flatten the tree to (id, name) pairs, all the drop target needs to label
/// itself: pre-order, depth first.
pub fn flatten_folders(nodes: &[FolderNode], out: &mut Vec<TransferFolder>) {
    for node in nodes {
        out.push(TransferFolder { id: node.id, name: node.name.clone() });
        flatten_folders(&node.children, out);
    }
}

/// `ShellUser` (components/app-shell.tsx).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellUser {
    pub name: String,
    pub email: String,
    /// Already resolved through `avatarUrl`: never the storage key.
    pub image: Option<String>,
    pub role: Option<String>,
    pub user_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ShellStorage {
    pub used: i64,
    pub max: i64,
}

/// `ShellSidebarData` (components/app-shell.tsx).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSidebarData {
    pub folders: Vec<FolderTreeNode>,
    /// `Record<FileKind, number>`, every kind, in `FILE_KINDS` order.
    pub counts: IndexMap<&'static str, i64>,
    pub total_files: i64,
    pub unsorted_count: i64,
    pub storage: ShellStorage,
}

/// What `DuplicateReminderSlot` hands to `<DuplicateReminder>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DuplicateReminderProps {
    pub redundant: i64,
    pub reclaimable: i64,
}

/// `AppLayoutData` (the SPA's `contract/layouts.ts`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLayoutData {
    /// `session.user.theme`, unresolved (ThemeApplier resolves it).
    pub theme: Option<String>,
    pub background: Option<BackgroundStyle>,
    pub transfer_folders: Vec<TransferFolder>,
    pub default_folder_id: Option<Uuid>,
    pub user: ShellUser,
    pub sidebar: ShellSidebarData,
    pub default_collapsed: bool,
    pub duplicate_reminder: Option<DuplicateReminderProps>,
}

/// The `LayoutLoader` for [`crate::pages::table::Layout::App`].
pub async fn load(ctx: PageCtx) -> LayoutResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.as_str();
    let db = &ctx.state.db;

    let (tree, counts, stats, prefs) = tokio::try_join!(
        folder_tree(db, user_id),
        kind_counts(db, user_id),
        user_stats(db, user_id),
        // `defaultFolderId` is deliberately not part of the Better Auth
        // session, so the drop target reads it here rather than from session.user.
        get_user_prefs(db, user_id),
    )?;
    let cookies = ctx.cookies();

    // Every file lives in exactly one folder or none, so what the root
    // folders don't account for is precisely the loose pile.
    let in_folders: i64 = tree.iter().map(|n| n.total_count).sum();

    let sidebar = ShellSidebarData {
        folders: to_tree_nodes(&tree),
        counts,
        total_files: stats.count,
        unsorted_count: (stats.count - in_folders).max(0),
        storage: ShellStorage { used: stats.total_size, max: session.user.max_quota.unwrap_or(-1) },
    };

    let prefs_code = prefs.as_ref().and_then(|p| p.user_code.as_deref());
    let background = background_style(
        prefs_code,
        prefs.as_ref().and_then(|p| p.background_key.as_deref()),
        prefs.as_ref().map(|p| p.background_dim),
    );

    let mut transfer_folders = Vec::new();
    flatten_folders(&tree, &mut transfer_folders);

    let user = ShellUser {
        name: session.user.name.clone(),
        email: session.user.email.clone(),
        // `user.image` holds a storage key, not a URL (see lib/avatar.ts). The
        // shell is a client component and must never see one, so it is
        // resolved here, where the account's tag is already loaded.
        image: avatar_url(prefs_code.or(session.user.user_code.as_deref()), session.user.image.as_deref()),
        role: session.user.role.clone(),
        user_code: session.user.user_code.clone(),
    };

    let data = AppLayoutData {
        theme: session.user.theme.clone(),
        background,
        transfer_folders,
        default_folder_id: prefs.as_ref().and_then(|p| p.default_folder_id),
        user,
        sidebar,
        default_collapsed: cookies.get(SIDEBAR_COOKIE).map(String::as_str) == Some("collapsed"),
        duplicate_reminder: duplicate_reminder_slot(&ctx, user_id, cookies.get(SNOOZE_COOKIE).map(String::as_str))
            .await,
    };
    Ok(tagged::to_tagged_value(&data)?)
}

/// `components/duplicates/DuplicateReminderSlot.tsx`: the server half of the
/// reminder, mounted once in the shell. The cheap answers come first:
///
///  1. "not now" still holding (a cookie read, no query),
///  2. under both thresholds (one grouped scan of one account's rows).
///
/// A failure here must not take the panel down with it: there is no version
/// of "we could not count your duplicates" that justifies an error page, so
/// the reminder simply does not appear.
pub async fn duplicate_reminder_slot(
    ctx: &PageCtx,
    user_id: &str,
    snooze_cookie: Option<&str>,
) -> Option<DuplicateReminderProps> {
    if is_snoozed(snooze_cookie, JsDate::now().millis()) {
        return None;
    }
    match duplicate_reminder(&ctx.state.db, user_id).await {
        Ok((totals, true)) => {
            Some(DuplicateReminderProps { redundant: totals.redundant, reclaimable: totals.reclaimable })
        }
        Ok((_, false)) => None,
        Err(err) => {
            tracing::error!(error = ?err, "[duplicates] reminder failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::folders::{FolderRow, build_folder_tree};

    fn row(id: u128, parent: Option<u128>, name: &str, count: i64) -> FolderRow {
        FolderRow {
            id: Uuid::from_u128(id),
            parent_id: parent.map(Uuid::from_u128),
            name: name.into(),
            slug: name.into(),
            is_public: id == 2,
            sort_order: 0,
            created_at: JsDate::from_millis(0).unwrap(),
            count,
            size: count * 10,
        }
    }

    #[test]
    fn tree_nodes_drop_the_server_only_fields_and_flatten_depth_first() {
        let tree = build_folder_tree(&[
            row(1, None, "a", 1),
            row(2, Some(1), "b", 2),
            row(3, Some(2), "c", 3),
            row(4, None, "d", 4),
        ]);
        let nodes = to_tree_nodes(&tree);
        let v = serde_json::to_value(&nodes).unwrap();
        let keys: Vec<&String> = v[0].as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["id", "parentId", "name", "slug", "isPublic", "count", "totalCount", "children"]);
        assert_eq!(v[0]["totalCount"], 6);
        assert_eq!(v[0]["children"][0]["isPublic"], true);
        assert_eq!(v[0]["children"][0]["children"][0]["name"], "c");

        let mut flat = Vec::new();
        flatten_folders(&tree, &mut flat);
        let names: Vec<&str> = flat.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c", "d"]);
        assert_eq!(
            serde_json::to_value(&flat[0]).unwrap(),
            serde_json::json!({ "id": Uuid::from_u128(1), "name": "a" })
        );
    }

    #[test]
    fn layout_payload_keys_are_the_spas() {
        let data = AppLayoutData {
            theme: None,
            background: None,
            transfer_folders: vec![],
            default_folder_id: None,
            user: ShellUser {
                name: "n".into(),
                email: "e@x".into(),
                image: None,
                role: Some("user".into()),
                user_code: None,
            },
            sidebar: ShellSidebarData {
                folders: vec![],
                counts: IndexMap::new(),
                total_files: 0,
                unsorted_count: 0,
                storage: ShellStorage { used: 0, max: -1 },
            },
            default_collapsed: false,
            duplicate_reminder: Some(DuplicateReminderProps { redundant: 11, reclaimable: 5 }),
        };
        let v = tagged::to_tagged_value(&data).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            vec![
                "theme",
                "background",
                "transferFolders",
                "defaultFolderId",
                "user",
                "sidebar",
                "defaultCollapsed",
                "duplicateReminder"
            ]
        );
        let user_keys: Vec<&String> = v["user"].as_object().unwrap().keys().collect();
        assert_eq!(user_keys, vec!["name", "email", "image", "role", "userCode"]);
        let sidebar_keys: Vec<&String> = v["sidebar"].as_object().unwrap().keys().collect();
        assert_eq!(sidebar_keys, vec!["folders", "counts", "totalFiles", "unsortedCount", "storage"]);
        assert_eq!(v["duplicateReminder"], serde_json::json!({ "redundant": 11, "reclaimable": 5 }));
    }
}

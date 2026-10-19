//! Loader of `app/(app)/dashboard/folders/page.tsx`: the folder explorer
//! (CONTRACT 6.12). An unknown, foreign or unreachable folder id is
//! `notFound()`.

use serde::Serialize;
use uuid::Uuid;

use crate::data::folders::{
    CountSize, Crumb, FolderNode, build_folder_tree, folder_path, list_user_folders, unfiled_stats,
};
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageError, PageResult};

/// `BrowserFolder` (components/dashboard/FolderCard.tsx).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFolder {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub is_public: bool,
    pub direct_count: i64,
    pub file_count: i64,
    pub size: i64,
    pub subfolder_count: usize,
    pub descendant_count: usize,
}

/// Folders nested under `node`, at any depth.
fn count_descendants(node: &FolderNode) -> usize {
    node.children.iter().map(|c| 1 + count_descendants(c)).sum()
}

pub fn to_browser_folder(node: &FolderNode) -> BrowserFolder {
    BrowserFolder {
        id: node.id,
        name: node.name.clone(),
        slug: node.slug.clone(),
        is_public: node.is_public,
        direct_count: node.count,
        file_count: node.total_count,
        size: node.total_size,
        subfolder_count: node.children.len(),
        descendant_count: count_descendants(node),
    }
}

/// Depth-first, exact id (as text).
fn find_node<'a>(nodes: &'a [FolderNode], id: &str) -> Option<&'a FolderNode> {
    for n in nodes {
        if n.id.to_string() == id {
            return Some(n);
        }
        if let Some(hit) = find_node(&n.children, id) {
            return Some(hit);
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
struct Current {
    id: Uuid,
    name: String,
    slug: String,
    #[serde(rename = "isPublic")]
    is_public: bool,
    count: i64,
    size: i64,
}

/// `FoldersPageProps`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FoldersProps {
    current: Option<Current>,
    breadcrumb: Vec<Crumb>,
    children: Vec<BrowserFolder>,
    total_folders: usize,
    unsorted: Option<CountSize>,
    user_code: String,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let session = ctx.require_user().await?;
    let user = &session.user;
    let db = &ctx.state.db;
    let folder_id = ctx.search.get("folder").filter(|f| !f.is_empty()).map(str::to_string);

    let (rows, breadcrumb, unsorted) = tokio::try_join!(
        list_user_folders(db, &user.id),
        async {
            match &folder_id {
                Some(id) => folder_path(db, &user.id, id).await,
                None => Ok(Vec::new()),
            }
        },
        async {
            match &folder_id {
                Some(_) => Ok(None),
                None => unfiled_stats(db, &user.id).await.map(Some),
            }
        },
    )?;

    let tree = build_folder_tree(&rows);
    let current = folder_id.as_deref().and_then(|id| find_node(&tree, id));
    if folder_id.is_some() && current.is_none() {
        return Err(PageError::NotFound);
    }
    let children = current.map(|c| c.children.as_slice()).unwrap_or(&tree).iter().map(to_browser_folder).collect();
    let props = FoldersProps {
        current: current.map(|c| Current {
            id: c.id,
            name: c.name.clone(),
            slug: c.slug.clone(),
            is_public: c.is_public,
            count: c.count,
            size: c.size,
        }),
        breadcrumb,
        children,
        total_folders: rows.len(),
        unsorted,
        user_code: user.user_code.clone().unwrap_or_default(),
    };
    PageData::new(&props, PageMeta::default())
}

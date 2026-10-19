//! Port of `lib/data/folders.ts` (the parts the panel shell and the profile
//! page need: `listUserFolders`, `buildFolderTree`, `folderTree`,
//! `unfiledStats`). The rest of the module belongs to the library port.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::types::JsDate;

/// One folder with its direct file count and bytes.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub is_public: bool,
    pub sort_order: i32,
    pub created_at: JsDate,
    pub count: i64,
    pub size: i64,
}

/// A folder in the sidebar tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub is_public: bool,
    pub sort_order: i32,
    pub created_at: JsDate,
    /// Files directly in this folder (excludes descendants).
    pub count: i64,
    /// Files in this folder and everything beneath it.
    pub total_count: i64,
    /// Bytes stored directly in this folder (excludes descendants).
    pub size: i64,
    /// Bytes in this folder and everything beneath it.
    pub total_size: i64,
    pub children: Vec<FolderNode>,
}

/// Flat list of a user's folders with direct file counts and byte totals.
///
/// The aggregate is a single grouped scan of `uploads` joined onto `folders`,
/// not one query per folder: the production account holds ~6800 uploads, so
/// anything per-folder turns the sidebar into an N+1.
pub async fn list_user_folders(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<FolderRow>> {
    // sum(bigint) comes back as numeric; ::text keeps it exact over the wire.
    sqlx::query_as::<_, FolderRow>(
        r#"SELECT f."id", f."parent_id", f."name", f."slug", f."is_public", f."sort_order", f."created_at",
                  coalesce(agg."files", 0)::bigint AS "count",
                  coalesce(agg."bytes", '0')::numeric::bigint AS "size"
           FROM "folders" f
           LEFT JOIN (
             SELECT "folder_id", count(*)::int AS "files", coalesce(sum("size"), 0)::text AS "bytes"
             FROM "uploads"
             WHERE "owner_id" = $1 AND "deleted_at" IS NULL AND "folder_id" IS NOT NULL
             GROUP BY "folder_id"
           ) agg ON agg."folder_id" = f."id"
           WHERE f."owner_id" = $1
           ORDER BY f."sort_order" ASC, f."name" ASC"#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

/// Build the sidebar tree. Rows whose parent is missing (or which sit in a
/// cycle left behind by a bad edit) are attached at the root rather than
/// vanishing: a folder the user cannot see is a folder whose files look lost.
/// (A cycle of two or more nodes is reachable from no root and so stays out,
/// exactly as upstream.)
pub fn build_folder_tree(rows: &[FolderRow]) -> Vec<FolderNode> {
    let index: std::collections::HashMap<Uuid, usize> = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    // children[i] = indices of rows whose parent is row i, in row order.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        match r.parent_id.and_then(|p| index.get(&p).copied()) {
            Some(parent) if rows[parent].id != r.id => children[parent].push(i),
            _ => roots.push(i),
        }
    }
    // Roll direct counts and bytes up into the totals, depth-first, with a
    // per-root `seen` set as the cycle guard.
    fn build(
        i: usize,
        rows: &[FolderRow],
        children: &[Vec<usize>],
        seen: &mut std::collections::HashSet<usize>,
    ) -> (FolderNode, i64, i64) {
        let r = &rows[i];
        let mut node = FolderNode {
            id: r.id,
            parent_id: r.parent_id,
            name: r.name.clone(),
            slug: r.slug.clone(),
            is_public: r.is_public,
            sort_order: r.sort_order,
            created_at: r.created_at,
            count: r.count,
            total_count: r.count,
            size: r.size,
            total_size: r.size,
            children: Vec::new(),
        };
        if !seen.insert(i) {
            // Already rolled up through another path: contributes nothing.
            return (node, 0, 0);
        }
        let (mut count, mut size) = (r.count, r.size);
        for &c in &children[i] {
            let (child, sub_count, sub_size) = build(c, rows, children, seen);
            count += sub_count;
            size += sub_size;
            node.children.push(child);
        }
        node.total_count = count;
        node.total_size = size;
        (node, count, size)
    }
    roots
        .into_iter()
        .map(|r| {
            let mut seen = std::collections::HashSet::new();
            build(r, rows, &children, &mut seen).0
        })
        .collect()
}

pub async fn folder_tree(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<FolderNode>> {
    Ok(build_folder_tree(&list_user_folders(db, user_id).await?))
}

/// Files that belong to no folder at all: the "Unsorted" pile.
pub async fn unfiled_stats(db: &PgPool, user_id: &str) -> sqlx::Result<(i64, i64)> {
    let (count, size): (i64, i64) = sqlx::query_as(
        r#"SELECT count(*)::bigint, coalesce(sum("size"), 0)::bigint FROM "uploads"
           WHERE "owner_id" = $1 AND "deleted_at" IS NULL AND "folder_id" IS NULL"#,
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok((count, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u128, parent: Option<u128>, name: &str, count: i64, size: i64) -> FolderRow {
        FolderRow {
            id: Uuid::from_u128(id),
            parent_id: parent.map(Uuid::from_u128),
            name: name.into(),
            slug: name.into(),
            is_public: false,
            sort_order: 0,
            created_at: JsDate::from_millis(0).unwrap(),
            count,
            size,
        }
    }

    // tests/unit/folders-core.test.ts (buildFolderTree)
    #[test]
    fn tree_rolls_totals_up_and_keeps_orphans_at_the_root() {
        let rows = vec![
            row(1, None, "a", 1, 10),
            row(2, Some(1), "b", 2, 20),
            row(3, Some(2), "c", 3, 30),
            row(4, Some(99), "orphan", 4, 40),
            row(5, Some(5), "self", 5, 50),
        ];
        let tree = build_folder_tree(&rows);
        let names: Vec<&str> = tree.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["a", "orphan", "self"]);
        assert_eq!(tree[0].total_count, 6);
        assert_eq!(tree[0].total_size, 60);
        assert_eq!(tree[0].children[0].total_count, 5);
        assert_eq!(tree[0].children[0].children[0].total_count, 3);
        assert_eq!(tree[1].total_count, 4);
    }

    #[test]
    fn a_two_node_cycle_is_reachable_from_no_root() {
        let rows = vec![row(1, Some(2), "x", 1, 1), row(2, Some(1), "y", 1, 1), row(3, None, "z", 1, 1)];
        let tree = build_folder_tree(&rows);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "z");
    }
}

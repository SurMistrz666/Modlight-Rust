//! Port of `lib/data/folders.ts`: folder listings, the sidebar tree,
//! breadcrumbs, subtrees and the public gallery lookup.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::models::{Columns, Folder, JsDate};

/// One folder with its direct file count and bytes (`FolderRow`).
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

/// A folder in the sidebar tree (`FolderNode`).
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

/// Flat list of a user's folders with direct file counts **and byte totals**,
/// ordered `sort_order, name`.
///
/// The aggregate is a single grouped scan of `uploads` joined onto `folders`,
/// not one query per folder: anything per-folder turns the sidebar into an N+1.
pub async fn list_user_folders(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<FolderRow>> {
    // sum(bigint) comes back as numeric; ::text keeps it exact over the wire.
    type Row = (Uuid, Option<Uuid>, String, String, bool, i32, JsDate, Option<i32>, Option<String>);
    let rows: Vec<Row> =
        sqlx::query_as(
            r#"select "folders"."id", "folders"."parent_id", "folders"."name", "folders"."slug", "folders"."is_public",
                      "folders"."sort_order", "folders"."created_at", "files", "bytes"
               from "folders"
               left join (select "folder_id", count(*)::int as "files", coalesce(sum("uploads"."size"), 0)::text as "bytes"
                          from "uploads"
                          where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null and "uploads"."folder_id" is not null)
                          group by "uploads"."folder_id") "folder_agg" on "folder_agg"."folder_id" = "folders"."id"
               where "folders"."owner_id" = $1
               order by "folders"."sort_order" asc, "folders"."name" asc"#,
        )
        .bind(user_id)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| FolderRow {
            id: r.0,
            parent_id: r.1,
            name: r.2,
            slug: r.3,
            is_public: r.4,
            sort_order: r.5,
            created_at: r.6,
            count: i64::from(r.7.unwrap_or(0)),
            size: r.8.as_deref().map(crate::config::js_number).unwrap_or(0.0) as i64,
        })
        .collect())
}

/// `{ count, size }` of uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CountSize {
    pub count: i64,
    pub size: i64,
}

async fn count_size(db: &PgPool, sql: &str, user_id: &str, folder: Option<&str>) -> sqlx::Result<CountSize> {
    let mut q = sqlx::query_as::<_, (i32, String)>(sql).bind(user_id);
    if let Some(f) = folder {
        q = q.bind(f);
    }
    let (count, size) = q.fetch_one(db).await?;
    Ok(CountSize { count: i64::from(count), size: crate::config::js_number(&size) as i64 })
}

/// The same two numbers `list_user_folders` reports, for ONE folder (a write
/// endpoint answers with a row in the listing's shape without re-running the
/// grouped scan).
pub async fn folder_stats(db: &PgPool, user_id: &str, folder_id: &str) -> sqlx::Result<CountSize> {
    count_size(
        db,
        r#"select count(*)::int, coalesce(sum("size"), 0)::text from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."folder_id" = $2::uuid and "uploads"."deleted_at" is null)"#,
        user_id,
        Some(folder_id),
    )
    .await
}

/// Files that belong to no folder at all - the "Unsorted" pile.
pub async fn unfiled_stats(db: &PgPool, user_id: &str) -> sqlx::Result<CountSize> {
    count_size(
        db,
        r#"select count(*)::int, coalesce(sum("size"), 0)::text from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null and "uploads"."folder_id" is null)"#,
        user_id,
        None,
    )
    .await
}

/// Build the sidebar tree. Rows whose parent is missing (or which point at
/// themselves) are attached at the root rather than vanishing; nodes inside a
/// longer parent cycle are reachable from no root and so are absent.
pub fn build_folder_tree(rows: &[FolderRow]) -> Vec<FolderNode> {
    let ids: HashSet<Uuid> = rows.iter().map(|r| r.id).collect();
    // Map iteration order is insertion order; a duplicate id (impossible for a
    // primary key) would keep the first position with the last value.
    let mut order: Vec<Uuid> = Vec::new();
    let mut by_id: HashMap<Uuid, &FolderRow> = HashMap::new();
    for r in rows {
        if by_id.insert(r.id, r).is_none() {
            order.push(r.id);
        }
    }
    let mut children_of: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    let mut roots: Vec<Uuid> = Vec::new();
    for id in &order {
        let node = by_id[id];
        match node.parent_id {
            Some(p) if ids.contains(&p) && p != node.id => children_of.entry(p).or_default().push(node.id),
            _ => roots.push(node.id),
        }
    }
    // Roll direct counts and bytes up into the totals, depth-first; a per-root
    // `seen` set stops cycles.
    fn build(
        id: Uuid,
        by_id: &HashMap<Uuid, &FolderRow>,
        children_of: &HashMap<Uuid, Vec<Uuid>>,
        seen: &mut HashSet<Uuid>,
    ) -> FolderNode {
        let r = by_id[&id];
        let first_visit = seen.insert(id);
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
        if !first_visit {
            return node;
        }
        let mut count = r.count;
        let mut size = r.size;
        for child in children_of.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
            let sub = build(*child, by_id, children_of, seen);
            count += sub.total_count;
            size += sub.total_size;
            node.children.push(sub);
        }
        node.total_count = count;
        node.total_size = size;
        node
    }
    roots.into_iter().map(|id| build(id, &by_id, &children_of, &mut HashSet::new())).collect()
}

pub async fn folder_tree(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<FolderNode>> {
    Ok(build_folder_tree(&list_user_folders(db, user_id).await?))
}

/// One breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Crumb {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

/// Breadcrumb chain from root down to `folder_id`. Empty when not found (the
/// id is compared as text, so any string is accepted).
pub async fn folder_path(db: &PgPool, user_id: &str, folder_id: &str) -> sqlx::Result<Vec<Crumb>> {
    let rows: Vec<(Uuid, Option<Uuid>, String, String)> =
        sqlx::query_as(r#"select "id", "parent_id", "name", "slug" from "folders" where "folders"."owner_id" = $1"#)
            .bind(user_id)
            .fetch_all(db)
            .await?;
    let by_id: HashMap<String, &(Uuid, Option<Uuid>, String, String)> =
        rows.iter().map(|r| (r.0.to_string(), r)).collect();
    let mut chain: Vec<Crumb> = Vec::new();
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut cur = by_id.get(folder_id).copied();
    while let Some(r) = cur {
        if !seen.insert(r.0) {
            break;
        }
        chain.insert(0, Crumb { id: r.0, name: r.2.clone(), slug: r.3.clone() });
        cur = r.1.and_then(|p| by_id.get(&p.to_string()).copied());
    }
    Ok(chain)
}

/// All folder ids at or beneath `folder_id` (always containing `folder_id`
/// itself, as given).
pub async fn descendant_folder_ids(db: &PgPool, user_id: &str, folder_id: &str) -> sqlx::Result<Vec<String>> {
    let rows: Vec<(Uuid, Option<Uuid>)> =
        sqlx::query_as(r#"select "id", "parent_id" from "folders" where "folders"."owner_id" = $1"#)
            .bind(user_id)
            .fetch_all(db)
            .await?;
    let mut children_of: HashMap<String, Vec<String>> = HashMap::new();
    for (id, parent) in &rows {
        let key = parent.map(|p| p.to_string()).unwrap_or_else(|| "__root__".to_string());
        children_of.entry(key).or_default().push(id.to_string());
    }
    let mut out = Vec::new();
    let mut stack = vec![folder_id.to_string()];
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(children) = children_of.get(&id) {
            stack.extend(children.iter().cloned());
        }
        out.push(id);
    }
    Ok(out)
}

/// True when moving `folder_id` under `new_parent_id` would create a cycle.
pub async fn would_create_cycle(
    db: &PgPool,
    user_id: &str,
    folder_id: &str,
    new_parent_id: Option<&str>,
) -> sqlx::Result<bool> {
    let Some(new_parent) = new_parent_id.filter(|p| !p.is_empty()) else {
        return Ok(false);
    };
    if new_parent == folder_id {
        return Ok(true);
    }
    Ok(descendant_folder_ids(db, user_id, folder_id).await?.iter().any(|d| d == new_parent))
}

/// The owner columns the public gallery needs.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PublicFolderOwner {
    pub id: String,
    pub user_code: Option<String>,
    pub name: String,
    pub theme: Option<String>,
    pub preferred_domain: Option<String>,
    pub background_key: Option<String>,
    pub background_dim: i32,
}

/// One file of a public gallery (no access-gating columns travel with it).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PublicFolderItem {
    pub id: Uuid,
    pub code: String,
    pub ext: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub blurhash: Option<String>,
    pub storage_key: String,
    pub created_at: JsDate,
}

pub struct PublicFolder {
    pub folder: Folder,
    pub owner: PublicFolderOwner,
    pub items: Vec<PublicFolderItem>,
}

const OWNER_COLUMNS: &str =
    r#""id", "user_code", "name", "theme", "preferred_domain", "background_key", "background_dim""#;

/// Public gallery for a shared folder: /a/{userCode}/{slug}. Accepts a
/// retired tag via user_code_aliases. Expired / burned / password-protected
/// files are excluded so the gallery cannot become a side channel around
/// those gates.
pub async fn get_public_folder(db: &PgPool, user_code: &str, slug: &str) -> sqlx::Result<Option<PublicFolder>> {
    let sql = format!(r#"select {OWNER_COLUMNS} from "user" where "user"."user_code" = $1 limit 1"#);
    let mut owner: Option<PublicFolderOwner> = sqlx::query_as(&sql).bind(user_code).fetch_optional(db).await?;
    if owner.is_none() {
        let alias: Option<String> = sqlx::query_scalar(
            r#"select "user_id" from "user_code_aliases" where "user_code_aliases"."code" = $1 limit 1"#,
        )
        .bind(user_code)
        .fetch_optional(db)
        .await?;
        let Some(user_id) = alias else {
            return Ok(None);
        };
        let sql = format!(r#"select {OWNER_COLUMNS} from "user" where "user"."id" = $1 limit 1"#);
        owner = sqlx::query_as(&sql).bind(user_id).fetch_optional(db).await?;
    }
    let Some(owner) = owner else {
        return Ok(None);
    };
    let sql = format!(
        r#"select {} from {} where ("folders"."owner_id" = $1 and "folders"."slug" = $2) limit 1"#,
        Folder::COLUMNS,
        Folder::TABLE
    );
    let folder: Option<Folder> = sqlx::query_as(&sql).bind(&owner.id).bind(slug).fetch_optional(db).await?;
    let Some(folder) = folder.filter(|f| f.is_public) else {
        return Ok(None);
    };
    let items: Vec<PublicFolderItem> = sqlx::query_as(
        r#"select "id", "code", "ext", "filename", "mime_type", "size", "blurhash", "storage_key", "created_at"
           from "uploads"
           where ("uploads"."folder_id" = $1 and "uploads"."published" = true and "uploads"."deleted_at" is null
                  and "uploads"."password_hash" is null
                  and ("uploads"."expires_at" is null or "uploads"."expires_at" > now())
                  and ("uploads"."max_views" is null or "uploads"."views" < "uploads"."max_views"))
           order by "uploads"."created_at" desc"#,
    )
    .bind(folder.id)
    .fetch_all(db)
    .await?;
    Ok(Some(PublicFolder { folder, owner, items }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u128, parent: Option<u128>, count: i64) -> FolderRow {
        FolderRow {
            id: Uuid::from_u128(id),
            parent_id: parent.map(Uuid::from_u128),
            name: format!("f{id}"),
            slug: format!("f{id}"),
            is_public: false,
            sort_order: 0,
            created_at: JsDate::from_millis(0).unwrap(),
            count,
            size: count * 10,
        }
    }

    #[test]
    fn tree_rolls_up_and_keeps_orphans_at_the_root() {
        let rows =
            vec![row(1, None, 1), row(2, Some(1), 2), row(3, Some(2), 3), row(4, Some(99), 4), row(5, Some(5), 5)];
        let tree = build_folder_tree(&rows);
        assert_eq!(tree.iter().map(|n| n.id.as_u128()).collect::<Vec<_>>(), vec![1, 4, 5]);
        assert_eq!(tree[0].total_count, 6);
        assert_eq!(tree[0].total_size, 60);
        assert_eq!(tree[0].children[0].total_count, 5);
        assert_eq!(tree[1].total_count, 4);
    }

    #[test]
    fn nodes_in_a_longer_cycle_are_unreachable() {
        let rows = vec![row(1, Some(2), 1), row(2, Some(1), 1), row(3, None, 1)];
        let tree = build_folder_tree(&rows);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id.as_u128(), 3);
    }
}

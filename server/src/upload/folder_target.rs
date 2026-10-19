//! Port of `lib/upload/folder-target.ts`: which folder an upload asked to land
//! in, and whether it is allowed to.
//!
//! Shared by BOTH upload paths on purpose. The browser (tus) and the API
//! clients (ShareX, the desktop companions) name a folder in different ways -
//! tus metadata versus a multipart field - but the question is identical, and
//! so is the answer that must not be got wrong: the id arrives as a string the
//! client wrote, so a forged UUID must never push a file into somebody else's
//! tree.

use sqlx::PgPool;
use uuid::Uuid;

use super::tus_server::store::Metadata;

/// Metadata / field key naming the drop target.
pub const FOLDER_KEY: &str = "folderId";

/// Sentinel for "no folder", as opposed to an absent key ("use the default").
pub const ROOT_FOLDER: &str = "root";

/// Shape check for any id that will be compared against a `uuid` column:
/// `/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i`.
pub fn is_uuid(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 36 {
        return false;
    }
    b.iter().enumerate().all(|(i, c)| match i {
        8 | 13 | 18 | 23 => *c == b'-',
        _ => c.is_ascii_hexdigit(),
    })
}

/// Raised when a client names a folder it does not own (or that is gone).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Folder not found.")]
pub struct FolderAccessError;

/// The three outcomes of [`resolve_folder_target`]: the difference between
/// the first two matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderTarget {
    /// `undefined`: the client expressed no preference; the account default
    /// applies (finalize's job).
    Default,
    /// `null`: the client explicitly asked for "no folder", i.e. the root.
    Root,
    /// A folder this user owns.
    Folder(Uuid),
}

impl FolderTarget {
    /// The column value finalize writes for an explicit target, `None` for
    /// `Default`.
    pub fn explicit(self) -> Option<Option<Uuid>> {
        match self {
            FolderTarget::Default => None,
            FolderTarget::Root => Some(None),
            FolderTarget::Folder(id) => Some(Some(id)),
        }
    }
}

/// An error looking a folder up: the refusal, or the database failing.
#[derive(Debug, thiserror::Error)]
pub enum FolderTargetError {
    #[error(transparent)]
    Access(#[from] FolderAccessError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Resolve a client-supplied folder id for `owner_id`. An unknown id, another
/// account's id, and a malformed id are ONE answer on purpose (no oracle for
/// "does folder X exist").
pub async fn resolve_folder_target(
    db: &PgPool,
    owner_id: &str,
    raw: Option<&str>,
) -> Result<FolderTarget, FolderTargetError> {
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Ok(FolderTarget::Default);
    };
    if raw == ROOT_FOLDER {
        return Ok(FolderTarget::Root);
    }
    if !is_uuid(raw) {
        return Err(FolderAccessError.into());
    }
    let id: Option<Uuid> = sqlx::query_scalar(
        r#"select "id" from "folders" where ("folders"."id" = $1::uuid and "folders"."owner_id" = $2) limit 1"#,
    )
    .bind(raw)
    .bind(owner_id)
    .fetch_optional(db)
    .await?;
    id.map(FolderTarget::Folder).ok_or_else(|| FolderAccessError.into())
}

/// Same question, asked with a tus metadata bag.
pub async fn resolve_folder_from_metadata(
    db: &PgPool,
    owner_id: &str,
    metadata: Option<&Metadata>,
) -> Result<FolderTarget, FolderTargetError> {
    let raw = metadata.and_then(|m| m.get(FOLDER_KEY)).and_then(|v| v.as_deref());
    resolve_folder_target(db, owner_id, raw).await
}

#[cfg(test)]
mod tests {
    //! tests/unit/folder-target.test.ts (the parts that never reach the database)
    use super::*;

    fn lazy_db() -> PgPool {
        sqlx::postgres::PgPoolOptions::new().connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap()
    }

    #[tokio::test]
    async fn absent_root_and_malformed_ids_never_query() {
        let db = lazy_db();
        for raw in [None, Some("")] {
            assert_eq!(resolve_folder_target(&db, "u", raw).await.unwrap(), FolderTarget::Default);
        }
        assert_eq!(resolve_folder_target(&db, "u", Some(ROOT_FOLDER)).await.unwrap(), FolderTarget::Root);
        for bad in ["nope", "../etc", "123", "00000000-0000-0000-0000-00000000000g", "'; drop table folders; --"] {
            assert!(matches!(
                resolve_folder_target(&db, "u", Some(bad)).await,
                Err(FolderTargetError::Access(FolderAccessError))
            ));
        }
    }

    #[test]
    fn uuid_shape() {
        assert!(is_uuid("9d2e1a54-0000-4000-8000-000000000001"));
        assert!(is_uuid("9D2E1A54-0000-4000-8000-00000000000A"));
        assert!(!is_uuid("9d2e1a540000-4000-8000-000000000001"));
        assert!(!is_uuid(" 9d2e1a54-0000-4000-8000-00000000000"));
    }
}

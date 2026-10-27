//! The JSON shapes of the Modlight API, field for field as the Windows client reads them
//! (`ModlightApi.cs`, `ModlightLibrary.cs`, `ModlightDuplicates.cs`, `ModlightTheme.cs`,
//! `ModlightServerDisk.cs`, the uploader and the URL shortener).
//!
//! Every field is optional on the wire: a server older than a field simply leaves it
//! out, and an explicit `null` where a number is expected reads as zero rather than
//! failing the whole listing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// `null` and absent both become the type's default.
fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A timestamp the server wrote as ISO 8601; anything unreadable is treated as absent.
fn lenient_time<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match raw {
        Some(serde_json::Value::String(text)) => parse_time(&text),
        _ => None,
    })
}

/// Parse an ISO 8601 timestamp, with or without an offset (no offset means UTC).
pub fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Some(parsed.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|naive| naive.and_utc())
}

/// How much room is left on the account. `max` and `remaining` are null when unlimited.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Quota {
    #[serde(deserialize_with = "null_default")]
    pub used: i64,
    pub max: Option<i64>,
    pub remaining: Option<i64>,
    #[serde(deserialize_with = "null_default")]
    pub unlimited: bool,
}

/// The answer to `GET /api/me`: who a token belongs to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountInfo {
    /// The account's short code, which its links are built from.
    pub tag: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    /// Absolute URL of the account's picture, on the LINK host; fetched through
    /// [`crate::host::rewrite_to_host`] like every other link.
    pub avatar: Option<String>,
    /// The hostname this account's links are built on, which need not be the one asked.
    pub host: Option<String>,
    pub quota: Option<Quota>,
    pub default_folder_id: Option<String>,
    /// Whether new uploads are private until published. Shown, never acted on.
    #[serde(deserialize_with = "null_default")]
    pub hide_uploads: bool,
    /// Whether the copied link is the raw file. For display only.
    #[serde(deserialize_with = "null_default")]
    pub copy_raw: bool,
    /// `"admin"` or anything else; advisory, decides only whether admin UI is offered.
    pub role: Option<String>,
}

impl AccountInfo {
    /// Exactly the string the panel tests for; never a prefix or a list.
    pub fn is_admin(&self) -> bool {
        self.role.as_deref() == Some("admin")
    }
}

/// One folder from `GET /api/folders`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Folder {
    pub id: Option<String>,
    pub name: Option<String>,
    pub slug: Option<String>,
    /// The folder this one sits in, or null at the top.
    pub parent_id: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub is_public: bool,
    /// Files sitting DIRECTLY in this folder. Nothing nested is counted.
    #[serde(deserialize_with = "null_default")]
    pub file_count: i64,
    /// Bytes of the files directly in this folder.
    #[serde(deserialize_with = "null_default")]
    pub size: i64,
    #[serde(deserialize_with = "null_default")]
    pub sort_order: i64,
}

/// What is loose in the account: files in no folder at all.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Unfiled {
    #[serde(deserialize_with = "null_default")]
    pub file_count: i64,
    #[serde(deserialize_with = "null_default")]
    pub size: i64,
}

/// The answer to `GET /api/folders`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FolderList {
    /// The literal to send back as `folderId` to mean "files in no folder".
    pub root_value: Option<String>,
    /// Null on a deployment that predates the field: "unknown", not zero.
    pub unfiled: Option<Unfiled>,
    pub folders: Option<Vec<Folder>>,
}

/// One uploaded file from `GET /api/uploads`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Upload {
    pub id: Option<String>,
    /// The short code the links are built from.
    pub code: Option<String>,
    pub filename: Option<String>,
    pub ext: Option<String>,
    pub mime_type: Option<String>,
    /// image, video, audio, document, archive, paste, other: the server's own class.
    pub kind: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    #[serde(deserialize_with = "null_default")]
    pub published: bool,
    #[serde(deserialize_with = "null_default")]
    pub views: i64,
    #[serde(deserialize_with = "null_default")]
    pub downloads: i64,
    #[serde(deserialize_with = "lenient_time")]
    pub created_at: Option<DateTime<Utc>>,
    pub folder_id: Option<String>,
    /// When the link stops working. Null when it does not expire.
    #[serde(deserialize_with = "lenient_time")]
    pub expires_at: Option<DateTime<Utc>>,
    /// How many views the link is allowed in total. Null when unlimited.
    pub max_views: Option<i64>,
    #[serde(deserialize_with = "null_default")]
    pub has_password: bool,
    /// The preview page. Null for a file in the server's trash.
    pub url: Option<String>,
    /// The file itself.
    pub raw_url: Option<String>,
    /// A resized preview, or null when this file cannot have one.
    pub thumb_url: Option<String>,
    /// An animated WebP preview, or null; taken as final, never derived locally.
    pub motion_thumb_url: Option<String>,
    /// Which of the two links belongs on the clipboard, decided by the server.
    pub copy_url: Option<String>,
}

/// One page of the answer to `GET /api/uploads`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UploadPage {
    /// The page actually returned, which need not be the one asked for.
    #[serde(deserialize_with = "null_default")]
    pub page: i64,
    /// The page size actually used: the server clamps it and the clamp is believed.
    #[serde(deserialize_with = "null_default")]
    pub per_page: i64,
    #[serde(deserialize_with = "null_default")]
    pub total: i64,
    pub root_value: Option<String>,
    pub items: Option<Vec<Upload>>,
}

/// The answer to `DELETE /api/uploads`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeleteResult {
    /// What the server actually did: trash, permanent or restore.
    pub mode: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub requested: i64,
    /// How many rows it touched; lower than `requested` is not an error.
    #[serde(deserialize_with = "null_default")]
    pub affected: i64,
}

/// The answer to `POST` or `PATCH /api/folders`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FolderResult {
    pub folder: Option<Folder>,
}

/// The answer to `DELETE /api/folders`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FolderDeleteResult {
    #[serde(deserialize_with = "null_default")]
    pub with_contents: bool,
    /// How many folders went, this one included.
    #[serde(deserialize_with = "null_default")]
    pub folders: i64,
    /// How many files were moved to the trash. Zero in the safe mode.
    #[serde(deserialize_with = "null_default")]
    pub trashed: i64,
}

/// The answer to `PATCH /api/uploads`: the rows as they are NOW.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UploadPatchResult {
    #[serde(deserialize_with = "null_default")]
    pub requested: i64,
    #[serde(deserialize_with = "null_default")]
    pub affected: i64,
    pub items: Option<Vec<Upload>>,
}

/// What one `PATCH /api/uploads` is asking for.
///
/// A field nobody set is left OUT of the body; a field set to "nothing" is sent as JSON
/// null. The server draws exactly that distinction (absent: leave alone, null: clear),
/// so this keeps an ordered list of the fields that were named, values included.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UploadEdit {
    fields: Vec<(String, serde_json::Value)>,
}

impl UploadEdit {
    pub fn new() -> Self {
        Self::default()
    }

    /// A folder id, or `None` for "no folder".
    pub fn move_to(self, folder_id: Option<&str>) -> Self {
        self.set(
            "folderId",
            folder_id
                .map(|id| id.into())
                .unwrap_or(serde_json::Value::Null),
        )
    }

    pub fn set_published(self, published: bool) -> Self {
        self.set("published", published.into())
    }

    /// The vanity slug. One file only: a URL cannot belong to two rows.
    pub fn set_code(self, code: &str) -> Self {
        self.set("code", code.into())
    }

    /// When the link stops working, or `None` to clear the expiry.
    pub fn set_expiry(self, expires: Option<DateTime<Utc>>) -> Self {
        let value = expires
            .map(|at| {
                serde_json::Value::String(at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            })
            .unwrap_or(serde_json::Value::Null);
        self.set("expiresAt", value)
    }

    /// The view budget, or `None` to remove it.
    pub fn set_max_views(self, max_views: Option<i64>) -> Self {
        self.set(
            "maxViews",
            max_views
                .map(|v| v.into())
                .unwrap_or(serde_json::Value::Null),
        )
    }

    /// A per-file password, or `None` to remove the one that is set.
    pub fn set_password(self, password: Option<&str>) -> Self {
        self.set(
            "password",
            password
                .map(|p| p.into())
                .unwrap_or(serde_json::Value::Null),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn fields(&self) -> &[(String, serde_json::Value)] {
        &self.fields
    }

    fn set(mut self, key: &str, value: serde_json::Value) -> Self {
        if let Some(slot) = self.fields.iter_mut().find(|(name, _)| name == key) {
            slot.1 = value;
        } else {
            self.fields.push((key.to_string(), value));
        }
        self
    }
}

/// Whether a conversion would work, asked without doing the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertVerdict {
    pub ok: bool,
    /// The `X-Convert-Refused` token, or empty when the answer was yes.
    pub reason: String,
    /// Not upstream: the account's files are encrypted and locked (`423`), which says
    /// nothing about the conversion (`reason` is `locked`).
    pub locked: Option<crate::error::FilesLocked>,
}

impl ConvertVerdict {
    pub fn yes() -> Self {
        ConvertVerdict {
            ok: true,
            reason: String::new(),
            locked: None,
        }
    }

    pub fn no(reason: &str) -> Self {
        ConvertVerdict {
            ok: false,
            reason: if reason.is_empty() {
                "unknown".to_string()
            } else {
                reason.to_string()
            },
            locked: None,
        }
    }

    /// The files are locked: ask again once they are unlocked.
    pub fn locked(locked: crate::error::FilesLocked) -> Self {
        ConvertVerdict {
            ok: false,
            reason: "locked".to_string(),
            locked: Some(locked),
        }
    }
}

/// What came back when a moving preview was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotionPreview {
    /// The animated WebP.
    Ok(Vec<u8>),
    /// This FILE will never have a moving preview (415/413). Permanent; stop asking.
    Refused,
    /// Not this time. Says nothing about the file; asking again later is fine.
    Unavailable,
}

/// One copy inside a duplicate set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicateFile {
    pub id: Option<String>,
    pub code: Option<String>,
    pub filename: Option<String>,
    pub ext: Option<String>,
    pub mime_type: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub size: i64,
    /// ISO 8601 as the server sends it; parsed for display, never for identity.
    pub created_at: Option<String>,
    pub folder_id: Option<String>,
    /// The folder's name, already resolved by this listing.
    pub folder_name: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub views: i64,
    #[serde(deserialize_with = "null_default")]
    pub downloads: i64,
    pub url: Option<String>,
}

/// Files that are byte-for-byte the same, grouped by their sha256.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicateGroup {
    pub checksum: Option<String>,
    /// Live copies that exist, which can exceed `files` (the server lists at most 50).
    #[serde(deserialize_with = "null_default")]
    pub copies: i64,
    /// Bytes one copy occupies.
    #[serde(deserialize_with = "null_default")]
    pub size: i64,
    /// Bytes the whole set occupies.
    #[serde(deserialize_with = "null_default")]
    pub bytes: i64,
    /// Bytes freed by keeping exactly one copy.
    #[serde(deserialize_with = "null_default")]
    pub reclaimable: i64,
    /// The owner has said these copies are meant to exist.
    #[serde(deserialize_with = "null_default")]
    pub dismissed: bool,
    /// Oldest first.
    pub files: Option<Vec<DuplicateFile>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicateTotals {
    #[serde(deserialize_with = "null_default")]
    pub groups: i64,
    #[serde(deserialize_with = "null_default")]
    pub redundant: i64,
    #[serde(deserialize_with = "null_default")]
    pub reclaimable: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicateThresholds {
    #[serde(deserialize_with = "null_default")]
    pub redundant: i64,
    #[serde(deserialize_with = "null_default")]
    pub bytes: i64,
}

/// The answer to `GET /api/duplicates`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicateScan {
    pub groups: Option<Vec<DuplicateGroup>>,
    #[serde(deserialize_with = "null_default")]
    pub total_groups: i64,
    #[serde(deserialize_with = "null_default")]
    pub total_files: i64,
    #[serde(deserialize_with = "null_default")]
    pub reclaimable: i64,
    #[serde(deserialize_with = "null_default")]
    pub dismissed_groups: i64,
    pub active: Option<DuplicateTotals>,
    #[serde(deserialize_with = "null_default")]
    pub truncated: bool,
    /// Live files the scan could not consider, because they carry no checksum.
    #[serde(deserialize_with = "null_default")]
    pub unchecksummed: i64,
    #[serde(deserialize_with = "null_default")]
    pub due: bool,
    pub thresholds: Option<DuplicateThresholds>,
}

/// The answer to `POST /api/duplicates`, whichever verb was sent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicateDecision {
    #[serde(deserialize_with = "null_default")]
    pub kept: i64,
    #[serde(deserialize_with = "null_default")]
    pub restored: i64,
}

/// The answer from `GET` and `PUT /api/theme`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeInfo {
    pub theme: Option<String>,
}

/// What the server's storage is holding, as the administration screen draws it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskReport {
    pub driver: Option<String>,
    #[serde(rename = "local", deserialize_with = "null_default")]
    pub is_local: bool,
    #[serde(deserialize_with = "null_default")]
    pub live_files: i64,
    #[serde(deserialize_with = "null_default")]
    pub live_bytes: i64,
    #[serde(deserialize_with = "null_default")]
    pub trashed_files: i64,
    #[serde(deserialize_with = "null_default")]
    pub trashed_bytes: i64,
    pub disk: Option<DiskSpace>,
    #[serde(deserialize_with = "null_default")]
    pub min_free_bytes: i64,
    pub kinds: Option<Vec<DiskKind>>,
    #[serde(deserialize_with = "null_default")]
    pub kind_bytes: i64,
    pub accounts: Option<Vec<DiskAccount>>,
    pub biggest: Option<Vec<DiskFile>>,
    pub cold: Option<Vec<DiskFile>>,
    pub cold_totals: Option<DiskCold>,
    pub trash: Option<DiskTrash>,
    #[serde(deserialize_with = "null_default")]
    pub trash_retention_days: i64,
    #[serde(deserialize_with = "null_default")]
    pub trash_due_soon_days: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskSpace {
    #[serde(deserialize_with = "null_default")]
    pub total_bytes: i64,
    #[serde(deserialize_with = "null_default")]
    pub free_bytes: i64,
    #[serde(deserialize_with = "null_default")]
    pub used_bytes: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskKind {
    pub kind: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub files: i64,
    #[serde(deserialize_with = "null_default")]
    pub bytes: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskAccount {
    pub id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub tag: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub orphaned: bool,
    #[serde(deserialize_with = "null_default")]
    pub rest: bool,
    #[serde(deserialize_with = "null_default")]
    pub accounts: i64,
    #[serde(deserialize_with = "null_default")]
    pub live_files: i64,
    #[serde(deserialize_with = "null_default")]
    pub live_bytes: i64,
    #[serde(deserialize_with = "null_default")]
    pub trashed_files: i64,
    #[serde(deserialize_with = "null_default")]
    pub trashed_bytes: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskFile {
    pub id: Option<String>,
    pub code: Option<String>,
    pub filename: Option<String>,
    pub ext: Option<String>,
    pub mime_type: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub size: i64,
    pub owner_tag: Option<String>,
    pub owner_name: Option<String>,
    #[serde(deserialize_with = "lenient_time")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "null_default")]
    pub views: i64,
    /// NULL IS NOT PROOF THE FILE WAS NEVER READ.
    #[serde(deserialize_with = "lenient_time")]
    pub last_accessed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskCold {
    #[serde(deserialize_with = "null_default")]
    pub files: i64,
    #[serde(deserialize_with = "null_default")]
    pub bytes: i64,
    #[serde(deserialize_with = "null_default")]
    pub days: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskTrash {
    #[serde(deserialize_with = "null_default")]
    pub files: i64,
    #[serde(deserialize_with = "null_default")]
    pub bytes: i64,
    #[serde(deserialize_with = "lenient_time")]
    pub oldest: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "null_default")]
    pub due_soon_files: i64,
    #[serde(deserialize_with = "null_default")]
    pub due_soon_bytes: i64,
    #[serde(deserialize_with = "null_default")]
    pub overdue_files: i64,
    #[serde(deserialize_with = "null_default")]
    pub overdue_bytes: i64,
}

/// Custom name, expiry, password and namespace for one short link.
///
/// Held for one request and never written to disk: the password is a credential
/// somebody typed for a single link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortLinkOptions {
    pub custom_code: Option<String>,
    pub expires_in_days: Option<i64>,
    pub password: Option<String>,
    /// Put the account's tag in front of the code. True is the server's own default.
    pub use_user_code: bool,
}

impl Default for ShortLinkOptions {
    fn default() -> Self {
        ShortLinkOptions {
            custom_code: None,
            expires_in_days: None,
            password: None,
            use_user_code: true,
        }
    }
}

impl ShortLinkOptions {
    /// Whether any of this differs from just sending the URL on its own.
    pub fn has_any(&self) -> bool {
        self.custom_code.as_deref().is_some_and(|c| !c.is_empty())
            || self.expires_in_days.is_some()
            || self.password.as_deref().is_some_and(|p| !p.is_empty())
            || !self.use_user_code
    }
}

/// The answer to `POST /api/shortlinks`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortLink {
    /// The code on its own, without the host.
    pub code: Option<String>,
    /// The absolute short link.
    pub url: Option<String>,
}

/// The answer to `POST /upload`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UploadResponse {
    /// Viewer page for the upload.
    pub url: Option<String>,
    /// The file itself, for embedding.
    pub raw_url: Option<String>,
    /// Whichever of the two this account asked to be given.
    pub copy_url: Option<String>,
    #[serde(rename = "deletionUrl")]
    pub deletion_url: Option<String>,
    /// Folder the upload actually landed in, or null for none.
    pub folder: Option<String>,
    /// Present when the requested folder could not be honoured.
    pub folder_warning: Option<String>,
}

impl UploadResponse {
    /// `copy_url`, not `url`, is what belongs on the clipboard.
    pub fn link(&self) -> Option<&str> {
        self.copy_url
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.url.as_deref().filter(|s| !s.is_empty()))
    }
}

/// What one call to `/api/client-settings` answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CloudDocument {
    pub version: i64,
    pub settings: serde_json::Map<String, serde_json::Value>,
    pub limit_bytes: i64,
    /// The write was refused because somebody else wrote first (409).
    pub conflict: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_sends_only_what_was_named() {
        let edit = UploadEdit::new()
            .move_to(None)
            .set_published(true)
            .set_max_views(Some(3))
            .set_max_views(None);
        let names: Vec<&str> = edit.fields().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["folderId", "published", "maxViews"]);
        assert_eq!(edit.fields()[0].1, serde_json::Value::Null);
        assert_eq!(edit.fields()[2].1, serde_json::Value::Null);
        assert!(UploadEdit::new().is_empty());
    }

    #[test]
    fn listings_survive_nulls_and_missing_fields() {
        let page: UploadPage = serde_json::from_str(
            r#"{"page":2,"perPage":null,"items":[{"id":"x","size":null,"createdAt":"2026-10-18T02:00:00.000Z","width":null,"expiresAt":"garbage"}]}"#,
        )
        .unwrap();
        assert_eq!(page.page, 2);
        assert_eq!(page.per_page, 0);
        let item = &page.items.unwrap()[0];
        assert_eq!(item.size, 0);
        assert!(item.created_at.is_some());
        assert_eq!(item.expires_at, None);
    }

    #[test]
    fn the_copy_link_wins_over_the_page() {
        let response = UploadResponse {
            url: Some("https://h/page".into()),
            copy_url: Some("https://h/raw.png".into()),
            ..Default::default()
        };
        assert_eq!(response.link(), Some("https://h/raw.png"));
    }
}

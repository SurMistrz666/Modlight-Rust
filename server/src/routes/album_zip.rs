//! Port of `app/a/[userCode]/[slug]/zip/route.ts`: a streaming ZIP of a
//! PUBLIC folder (`GET /a/{userCode}/{slug}/zip`).
//!
//! Every published item of the folder is streamed from the storage driver into
//! the archive ([`crate::archiver`], archiver 8's zip layout at zlib level 6) and
//! on to the client, without buffering files in memory. A storage error part
//! way through fails the download loudly (the connection is cut), as archiver's
//! `error`/`warning` events did, rather than producing a silently partial zip.
//!
//! `getPublicFolder` already filters out password-protected, expired and
//! burned-through items. Security fix 1 adds the rest: an item that still has
//! view budget left is the full content of a file with a view cap, so putting
//! it into the archive spends one view (counted as a download, like
//! `/download`); an item whose budget is refused at that moment is left out.
//! The folder's owner and administrators spend nothing, as everywhere else.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use bytes::Bytes;
use futures_util::StreamExt;
use uuid::Uuid;

use crate::archiver::{ZipWriter, entry_name};
use crate::auth_helpers::Sessions;
use crate::data::folders::{PublicFolderItem, get_public_folder};
use crate::db::models::{Columns, Upload};
use crate::error::AppResult;
use crate::media::serve::{consume_view, plain_text, put_headers, viewer_relation};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/a/{userCode}/{slug}/zip", get(zip))
}

/// zlib level 6 keeps already-compressed media small enough without pegging
/// the CPU at max level.
const ZLIB_LEVEL: u32 = 6;

async fn zip(
    State(state): State<AppState>,
    Path((user_code, slug)): Path<(String, String)>,
    Sessions(session): Sessions,
    req: Request,
) -> AppResult<Response> {
    let Some(data) = get_public_folder(&state.db, &user_code, &slug).await? else {
        return Ok(plain_text(StatusCode::NOT_FOUND, "Not found"));
    };
    let mut items = data.items;

    // Security fix 1: capped items pay a view to travel in the archive.
    let ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();
    let sql = format!(
        r#"select {} from {} where "uploads"."id" = any($1) and "uploads"."max_views" is not null"#,
        Upload::COLUMNS,
        Upload::TABLE
    );
    let capped: Vec<Upload> = sqlx::query_as(&sql).bind(&ids).fetch_all(&state.db).await?;
    if !capped.is_empty() {
        let privileged = viewer_relation(&state.db, req.headers(), &session, &data.owner.id).await?.privileged;
        if !privileged {
            let mut refused = Vec::new();
            for media in &capped {
                if !consume_view(&state.db, media, true).await? {
                    refused.push(media.id);
                }
            }
            items.retain(|i| !refused.contains(&i.id));
        }
    }

    let filename = {
        let f = sanitize_filename(&data.folder.slug);
        if f.is_empty() { "folder".to_string() } else { f }
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(4);
    let storage = state.storage.clone();
    tokio::spawn(async move {
        if let Err(err) = build_archive(&storage, &items, &tx).await {
            // The client sees the connection cut rather than a corrupt or
            // partial archive that looks complete.
            let _ = tx.send(Err(std::io::Error::other(err.to_string()))).await;
        }
    });
    let body = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });

    let mut res = Response::new(Body::from_stream(body));
    put_headers(
        &mut res,
        [
            ("Content-Type", "application/zip".to_string()),
            ("Content-Disposition", format!("attachment; filename=\"{filename}.zip\"")),
            ("Cache-Control", "no-store".to_string()),
        ],
    );
    Ok(res)
}

/// `buildArchive(archive, items, storage)`: every item in folder order, then
/// the central directory. An empty folder still yields a valid (empty) zip.
async fn build_archive(
    storage: &crate::storage::Storage,
    items: &[PublicFolderItem],
    tx: &tokio::sync::mpsc::Sender<std::io::Result<Bytes>>,
) -> anyhow::Result<()> {
    let send = |bytes: Bytes| async move {
        if bytes.is_empty() {
            return Ok(());
        }
        // The client went away: stop producing more bytes.
        tx.send(Ok(bytes)).await.map_err(|_| anyhow::anyhow!("client went away"))
    };
    let driver = storage.get()?;
    let mut used: Vec<String> = Vec::new();
    // Every entry is dated when it is appended, which upstream did for all of
    // them in one loop before the first byte flowed.
    let appended = chrono::Utc::now();
    let mut zip = ZipWriter::new(ZLIB_LEVEL);
    for item in items {
        let name = entry_name(&unique_name(&mut used, &item.filename, &item.code));
        if name.is_empty() {
            anyhow::bail!("entry name must be a non-empty string value");
        }
        let mut stream = driver.read_stream(&item.storage_key, None).await?;
        send(zip.begin_entry(&name, appended)).await?;
        while let Some(chunk) = stream.next().await {
            send(zip.write(&chunk?)?).await?;
        }
        send(zip.end_entry()?).await?;
    }
    send(zip.finish()).await?;
    Ok(())
}

/// `uniqueName(used, filename, code)`: on a collision the upload's short
/// `code` (which is unique) is prefixed.
fn unique_name(used: &mut Vec<String>, filename: &str, code: &str) -> String {
    let sanitized = sanitize_entry_name(filename);
    let base = if !sanitized.is_empty() {
        sanitized
    } else if !code.is_empty() {
        code.to_string()
    } else {
        "file".to_string()
    };
    if !used.contains(&base) {
        used.push(base.clone());
        return base;
    }
    let mut candidate = format!("{code}-{base}");
    let mut n = 1;
    while used.contains(&candidate) {
        candidate = format!("{code}-{n}-{base}");
        n += 1;
    }
    used.push(candidate.clone());
    candidate
}

/// `sanitizeEntryName(name)`: path separators become `_`, control characters
/// go, leading dots go, then the JavaScript trim.
fn sanitize_entry_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_sep = false;
    for c in name.chars() {
        if c == '/' || c == '\\' {
            if !in_sep {
                out.push('_');
                in_sep = true;
            }
            continue;
        }
        in_sep = false;
        if c <= '\u{1f}' || c == '\u{7f}' {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_start_matches('.');
    crate::config::js_trim(out).to_string()
}

/// `sanitizeFilename(name)`: the Content-Disposition filename, with no quotes,
/// slashes or control characters.
fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_sep = false;
    for c in name.chars() {
        if c == '/' || c == '\\' {
            if !in_sep {
                out.push('-');
                in_sep = true;
            }
            continue;
        }
        in_sep = false;
        if c == '"' || c <= '\u{1f}' || c == '\u{7f}' {
            continue;
        }
        out.push(c);
    }
    crate::config::js_trim(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_are_flat_and_unique() {
        let mut used = Vec::new();
        assert_eq!(unique_name(&mut used, "a.png", "c1"), "a.png");
        assert_eq!(unique_name(&mut used, "a.png", "c2"), "c2-a.png");
        assert_eq!(unique_name(&mut used, "a.png", "c2"), "c2-1-a.png");
        assert_eq!(unique_name(&mut used, "../../etc/passwd", "c3"), "_.._etc_passwd");
        assert_eq!(unique_name(&mut used, "...", "c4"), "c4");
        assert_eq!(unique_name(&mut used, "", ""), "file");
        assert_eq!(sanitize_entry_name("  a\u{0}b\tc\u{7f}.txt  "), "abc.txt");
        assert_eq!(sanitize_entry_name(". hidden"), "hidden");
        assert_eq!(sanitize_entry_name("x\\\\/y"), "x_y");
    }

    #[test]
    fn the_download_name_is_header_safe() {
        assert_eq!(sanitize_filename("my-album"), "my-album");
        assert_eq!(sanitize_filename("a\"b/c\\\\d\u{1}"), "ab-c-d");
    }
}

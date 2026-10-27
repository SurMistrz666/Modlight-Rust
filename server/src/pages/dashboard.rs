//! Loader of `app/(app)/dashboard/page.tsx`: the file library (CONTRACT 6.11).
//!
//! A bare `/dashboard` (no query key at all) of an account that has folders
//! redirects to the folder explorer; any query, even `?all=1`, lists files.
//! The query is parsed exactly as upstream parses it, then `kindCounts`,
//! `listUploads`, `folderPath` and `getUserPrefs` feed the props.

use chrono::Timelike;
use serde::Serialize;
use uuid::Uuid;

use crate::config::js_number;
use crate::data::folders::{Crumb, folder_path, list_user_folders};
use crate::data::upload_query::{FILE_KINDS, SORT_KEYS};
use crate::data::uploads::{FileItem, FolderFilter, ListParams, file_item, kind_counts, list_uploads};
use crate::data::user_prefs::get_user_prefs;
use crate::db::types::JsDate;
use crate::js_date::{parse_js_date, to_local};
use crate::media::urls::user_base_url;
use crate::pages::meta::PageMeta;
use crate::pages::{PageCtx, PageData, PageError, PageResult, SearchParams};

/// Minimum-size presets of the toolbar.
fn size_preset(raw: &str) -> Option<i64> {
    match raw {
        "10mb" => Some(10 * 1024 * 1024),
        "100mb" => Some(100 * 1024 * 1024),
        "1gb" => Some(1024 * 1024 * 1024),
        _ => None,
    }
}

const PER_PAGE_OPTIONS: [f64; 3] = [30.0, 60.0, 120.0];
const DEFAULT_PER_PAGE: i64 = 30;

/// `/^\d{4}-\d{2}-\d{2}$/`
fn is_day(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// `parseDay(value, endOfDay)`: a `YYYY-MM-DD` is LOCAL midnight (end of the
/// local day with `end_of_day`); anything else goes to `new Date(value)`.
pub fn parse_day(value: Option<&str>, end_of_day: bool) -> Option<JsDate> {
    let value = value.filter(|v| !v.is_empty())?;
    let day = is_day(value);
    let d = if day { parse_js_date(&format!("{value}T00:00:00"))? } else { parse_js_date(value)? };
    if end_of_day && day {
        // d.setHours(23, 59, 59, 999), local.
        let local = to_local(d);
        let naive =
            local.naive_local().with_hour(23)?.with_minute(59)?.with_second(59)?.with_nanosecond(999_000_000)?;
        return crate::js_date::local_to_utc(naive);
    }
    Some(d)
}

/// `Math.max(1, Number(first(sp.page)) || 1)`
pub fn page_param(raw: Option<&str>) -> f64 {
    let n = raw.map(js_number).unwrap_or(f64::NAN);
    let n = if n.is_nan() || n == 0.0 { 1.0 } else { n };
    n.max(1.0)
}

/// `PER_PAGE_OPTIONS.includes(Number(first(sp.per))) ? n : 30`
pub fn per_page_param(raw: Option<&str>) -> i64 {
    let n = raw.map(js_number).unwrap_or(f64::NAN);
    if PER_PAGE_OPTIONS.contains(&n) { n as i64 } else { DEFAULT_PER_PAGE }
}

/// `first(sp.search)?.trim() || undefined`
pub fn search_param(sp: &SearchParams) -> Option<String> {
    sp.get("search").map(crate::config::js_trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn sort_param(sp: &SearchParams) -> String {
    sp.get("sort").filter(|s| SORT_KEYS.contains(s)).unwrap_or("newest").to_string()
}

pub fn kind_param(sp: &SearchParams) -> Option<String> {
    sp.get("kind").filter(|k| !k.is_empty() && FILE_KINDS.contains(k)).map(str::to_string)
}

pub fn view_param(sp: &SearchParams) -> &'static str {
    if sp.get("view") == Some("list") { "list" } else { "grid" }
}

/// `{ id, name, slug, isPublic }` of the open folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CurrentFolder {
    id: Uuid,
    name: String,
    slug: String,
    is_public: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ResultTotals {
    total: i64,
    page: i64,
    #[serde(rename = "perPage")]
    per_page: i64,
}

/// `DashboardPageProps`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    search: Option<String>,
    sort: String,
    view: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_before: Option<String>,
    /// `null` for "none", the raw string otherwise; absent when undefined.
    #[serde(skip_serializing_if = "Option::is_none")]
    folder_id: Option<Option<String>>,
    include_subfolders: bool,
    filtering: bool,
    counts: indexmap::IndexMap<&'static str, i64>,
    result: ResultTotals,
    items: Vec<FileItem>,
    breadcrumb: Vec<Crumb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_folder: Option<CurrentFolder>,
    link_base: String,
    user_code: String,
    copy_raw: bool,
    /// Not upstream: the account's file vault (docs/ENCRYPTION.md); absent for
    /// an account without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    vault: Option<crate::vault::status::LibraryVault>,
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let sp = &ctx.search;
    let session = ctx.require_user().await?;
    let user = &session.user;
    let db = &ctx.state.db;

    let all_folders = list_user_folders(db, &user.id).await?;
    if sp.0.is_empty() && !all_folders.is_empty() {
        return Err(PageError::redirect("/dashboard/folders"));
    }

    let page = page_param(sp.get("page"));
    let per_page = per_page_param(sp.get("per"));
    let search = search_param(sp);
    let sort = sort_param(sp);
    let view = view_param(sp);
    // "none" = loose files; another value = that folder id; absent = everything.
    let folder_id: Option<Option<String>> = match sp.get("folder") {
        Some("none") => Some(None),
        Some(f) if !f.is_empty() => Some(Some(f.to_string())),
        _ => None,
    };
    let include_subfolders = sp.get("sub") == Some("1");
    let kind = kind_param(sp);
    let visibility = sp.get("visibility").filter(|v| *v == "public" || *v == "private").map(str::to_string);
    let raw_size = sp.get("size").map(str::to_string);
    let min_size = raw_size.as_deref().filter(|s| !s.is_empty()).and_then(size_preset);
    let raw_after = sp.get("after").map(str::to_string);
    let raw_before = sp.get("before").map(str::to_string);
    let after = parse_day(raw_after.as_deref(), false);
    let before = parse_day(raw_before.as_deref(), true);

    let folder_filter = match &folder_id {
        None => FolderFilter::Any,
        Some(None) => FolderFilter::Unfiled,
        Some(Some(id)) => FolderFilter::Folder(id.clone()),
    };
    let open_folder = folder_id.clone().flatten();
    let params = ListParams {
        user_id: user.id.clone(),
        page: Some(page),
        per_page: Some(per_page),
        search: search.clone(),
        sort: Some(sort.clone()),
        folder: folder_filter,
        include_subfolders,
        kind: kind.clone(),
        visibility: visibility.clone(),
        min_size: min_size.map(|n| n as f64),
        after,
        before,
        ..Default::default()
    };
    let (counts, result, breadcrumb, prefs) = tokio::try_join!(
        kind_counts(db, &user.id),
        list_uploads(db, &params),
        async {
            match &open_folder {
                Some(id) => folder_path(db, &user.id, id).await,
                None => Ok(Vec::new()),
            }
        },
        get_user_prefs(db, &user.id),
    )?;

    let link_base = user_base_url(&ctx.state.config, prefs.as_ref().and_then(|p| p.preferred_domain.as_deref()));
    let current_folder = open_folder.as_ref().and_then(|id| {
        all_folders.iter().find(|f| f.id.to_string() == *id).map(|f| CurrentFolder {
            id: f.id,
            name: f.name.clone(),
            slug: f.slug.clone(),
            is_public: f.is_public,
        })
    });
    let names: std::collections::HashMap<Uuid, &str> = all_folders.iter().map(|f| (f.id, f.name.as_str())).collect();
    let items: Vec<FileItem> = result
        .items
        .iter()
        .map(|u| file_item(u, u.folder_id.and_then(|f| names.get(&f).map(|n| n.to_string()))))
        .collect();
    let filtering = search.is_some()
        || kind.is_some()
        || visibility.is_some()
        || min_size.is_some()
        || after.is_some()
        || before.is_some();

    let props = DashboardProps {
        search,
        sort,
        view,
        kind,
        visibility,
        raw_size,
        raw_after,
        raw_before,
        folder_id,
        include_subfolders,
        filtering,
        counts,
        result: ResultTotals { total: result.total, page: result.page, per_page: result.per_page },
        items,
        breadcrumb,
        current_folder,
        link_base,
        user_code: user.user_code.clone().unwrap_or_default(),
        copy_raw: user.copy_raw.unwrap_or(false),
        vault: crate::vault::status::library_vault(&ctx.state, &ctx.headers, &user.id).await?,
    };
    PageData::new(&props, PageMeta::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_parsing_matches_upstream() {
        assert_eq!(page_param(None), 1.0);
        assert_eq!(page_param(Some("")), 1.0);
        assert_eq!(page_param(Some("0")), 1.0);
        assert_eq!(page_param(Some("-3")), 1.0);
        assert_eq!(page_param(Some(" 2 ")), 2.0);
        assert_eq!(page_param(Some("2.7")), 2.7);
        assert!(page_param(Some("Infinity")).is_infinite());
        assert_eq!(per_page_param(Some(" 60 ")), 60);
        assert_eq!(per_page_param(Some("6e1")), 60);
        assert_eq!(per_page_param(Some("0x3c")), 60);
        assert_eq!(per_page_param(Some("50")), 30);
        assert_eq!(per_page_param(None), 30);
        assert!(is_day("2026-01-31"));
        assert!(!is_day("2026-1-31"));
    }
}

//! Port of `lib/media/convert-plan.ts`: deciding WHAT an in-place conversion
//! would do, before anything is done.
//!
//! Every judgement here is a pure function over a database row, so the whole
//! decision surface - which files a bulk target applies to, which are already
//! in that format, what a conversion does to the quota, which columns are
//! rewritten - is a unit test rather than something discovered on production
//! data.
//!
//! The separation matters for a second reason. `convert_in_place` is the only
//! module that WRITES, and it is deliberately kept small enough to read in one
//! sitting; the arithmetic and the taxonomy live here so that file can be about
//! ordering and failure recovery and nothing else.

use serde::Serialize;
use serde::ser::SerializeMap;

use crate::db::models::Upload;
use crate::db::types::JsDate;
use crate::media::av_targets::{AvSourceSize, AvTarget, av_option_for, av_output};
use crate::media::convert_targets::{ConvertTarget, convert_option_for};
use crate::media::download_menu::{DownloadMenuOptions, DownloadOption, Engine, download_options_for};
use crate::media::mime::effective_type;
use crate::rpc::tagged::Undefined;

pub use crate::media::download_menu::Engine as ConvertEngine;
pub use crate::media::paths::{swap_filename_ext, swap_key_ext};

/// `copy` leaves the source untouched and produces a second file with its own
/// code; `replace` rewrites the bytes behind the file that already exists, so
/// its link keeps working and nothing new appears in the dashboard.
///
/// `Other` is any other value a caller sends: upstream's type said
/// `"replace" | "copy"` but nothing checked it at runtime, and every branch
/// tests one of the two names exactly. Such a mode therefore behaves as a
/// replace everywhere (`mode === "copy" ? copy : replace`), except the two
/// places that test `=== "replace"`: the paste guard in `planConversion` does
/// not fire, and the audit entry is written as `convert.copy`. Kept, because
/// only the owner or an admin reaches it (see SKELETON, media processing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConvertMode {
    Replace,
    Copy,
    Other,
}

impl ConvertMode {
    /// How upstream reads a value it was handed.
    pub fn from_js(v: &str) -> ConvertMode {
        match v {
            "replace" => ConvertMode::Replace,
            "copy" => ConvertMode::Copy,
            _ => ConvertMode::Other,
        }
    }
}

/// A file that will not be converted, and it is not an error.
///
/// `already-in-format` is the one the reported bug turns on: select five JPEGs
/// and three PNGs, ask for PNG, and three of them have nothing to do. The
/// target tables never OFFER a file the format it already is, so without this
/// check those three would be reported as "cannot be converted", which is both
/// false and alarming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConvertSkipReason {
    AlreadyInFormat,
    UnsupportedTarget,
    Paste,
    Trashed,
    Expired,
}

impl ConvertSkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ConvertSkipReason::AlreadyInFormat => "already-in-format",
            ConvertSkipReason::UnsupportedTarget => "unsupported-target",
            ConvertSkipReason::Paste => "paste",
            ConvertSkipReason::Trashed => "trashed",
            ConvertSkipReason::Expired => "expired",
        }
    }
}

/// A conversion that was attempted and did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConvertFailure {
    NotFound,
    Forbidden,
    UnsupportedTarget,
    TooLarge,
    Quota,
    NoSpace,
    Busy,
    Timeout,
    Aborted,
    Conflict,
    Storage,
    Encoder,
}

impl ConvertFailure {
    pub fn as_str(self) -> &'static str {
        match self {
            ConvertFailure::NotFound => "not-found",
            ConvertFailure::Forbidden => "forbidden",
            ConvertFailure::UnsupportedTarget => "unsupported-target",
            ConvertFailure::TooLarge => "too-large",
            ConvertFailure::Quota => "quota",
            ConvertFailure::NoSpace => "no-space",
            ConvertFailure::Busy => "busy",
            ConvertFailure::Timeout => "timeout",
            ConvertFailure::Aborted => "aborted",
            ConvertFailure::Conflict => "conflict",
            ConvertFailure::Storage => "storage",
            ConvertFailure::Encoder => "encoder",
        }
    }
}

/// `ConvertOutcome`
#[derive(Debug, Clone, PartialEq)]
pub enum ConvertOutcome {
    Converted {
        upload_id: String,
        code: String,
        ext: String,
        mime_type: String,
        size: i64,
        previous_size: i64,
        previous_ext: String,
        /// Something true about the result nobody asked for ("clipped", "audio-dropped").
        note: Option<String>,
    },
    Created {
        upload_id: String,
        source_id: String,
        code: String,
        ext: String,
        mime_type: String,
        size: i64,
        note: Option<String>,
    },
    Skipped {
        upload_id: String,
        reason: ConvertSkipReason,
    },
    Failed {
        upload_id: String,
        reason: ConvertFailure,
    },
}

impl ConvertOutcome {
    pub fn status(&self) -> &'static str {
        match self {
            ConvertOutcome::Converted { .. } => "converted",
            ConvertOutcome::Created { .. } => "created",
            ConvertOutcome::Skipped { .. } => "skipped",
            ConvertOutcome::Failed { .. } => "failed",
        }
    }

    pub fn upload_id(&self) -> &str {
        match self {
            ConvertOutcome::Converted { upload_id, .. }
            | ConvertOutcome::Created { upload_id, .. }
            | ConvertOutcome::Skipped { upload_id, .. }
            | ConvertOutcome::Failed { upload_id, .. } => upload_id,
        }
    }
}

/// The object literals upstream returns, key for key: `note` is always
/// present on the two successes and `undefined` when there is none.
impl Serialize for ConvertOutcome {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        map.serialize_entry("status", self.status())?;
        let note = |map: &mut S::SerializeMap, note: &Option<String>| match note {
            Some(n) => map.serialize_entry("note", n),
            None => map.serialize_entry("note", &Undefined),
        };
        match self {
            ConvertOutcome::Converted {
                upload_id,
                code,
                ext,
                mime_type,
                size,
                previous_size,
                previous_ext,
                note: n,
            } => {
                map.serialize_entry("uploadId", upload_id)?;
                map.serialize_entry("code", code)?;
                map.serialize_entry("ext", ext)?;
                map.serialize_entry("mimeType", mime_type)?;
                map.serialize_entry("size", size)?;
                map.serialize_entry("previousSize", previous_size)?;
                map.serialize_entry("previousExt", previous_ext)?;
                note(&mut map, n)?;
            }
            ConvertOutcome::Created { upload_id, source_id, code, ext, mime_type, size, note: n } => {
                map.serialize_entry("uploadId", upload_id)?;
                map.serialize_entry("sourceId", source_id)?;
                map.serialize_entry("code", code)?;
                map.serialize_entry("ext", ext)?;
                map.serialize_entry("mimeType", mime_type)?;
                map.serialize_entry("size", size)?;
                note(&mut map, n)?;
            }
            ConvertOutcome::Skipped { upload_id, reason } => {
                map.serialize_entry("uploadId", upload_id)?;
                map.serialize_entry("reason", reason.as_str())?;
            }
            ConvertOutcome::Failed { upload_id, reason } => {
                map.serialize_entry("uploadId", upload_id)?;
                map.serialize_entry("reason", reason.as_str())?;
            }
        }
        map.end()
    }
}

/// The report a bulk run hands back. Counts first, because that is the sentence.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConvertReport {
    pub to: String,
    pub mode: String,
    pub converted: usize,
    pub skipped: usize,
    pub failed: usize,
    pub results: Vec<ConvertOutcome>,
}

/// The columns any of this needs. Declared structurally upstream so a caller
/// may hand over a narrow `select()` of a thousand rows instead of pulling
/// password hashes and blurhashes into memory.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConvertCandidate {
    pub id: String,
    pub ext: Option<String>,
    pub mime_type: Option<String>,
    pub frames: Option<f64>,
    /// Picture size, for the one target that is offered only to files too big
    /// for a hardware decoder. Null on every row uploaded before the columns
    /// existed, and that reads as "do not offer it" rather than as "small".
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub size: f64,
    pub kind: String,
    pub deleted_at: Option<JsDate>,
    pub expires_at: Option<JsDate>,
}

impl ConvertCandidate {
    fn av_size(&self) -> AvSourceSize {
        AvSourceSize { width: self.width, height: self.height }
    }
}

impl From<&Upload> for ConvertCandidate {
    fn from(u: &Upload) -> Self {
        ConvertCandidate {
            id: u.id.to_string(),
            ext: Some(u.ext.clone()),
            mime_type: Some(u.mime_type.clone()),
            frames: u.frames.map(f64::from),
            width: u.width.map(f64::from),
            height: u.height.map(f64::from),
            size: u.size as f64,
            kind: u.kind.clone(),
            deleted_at: u.deleted_at,
            expires_at: u.expires_at,
        }
    }
}

// ---------------------------------------------------------------------------
// What a target produces
// ---------------------------------------------------------------------------

/// MIME and extension the libvips engine writes. Mirrors `convert`.
pub fn image_output(to: ConvertTarget) -> (&'static str, &'static str) {
    match to {
        ConvertTarget::Pdf => ("application/pdf", "pdf"),
        ConvertTarget::Png => ("image/png", "png"),
        ConvertTarget::Jpg => ("image/jpeg", "jpg"),
        ConvertTarget::Webp => ("image/webp", "webp"),
        ConvertTarget::Avif => ("image/avif", "avif"),
    }
}

/// `outputFor(to)`: what comes out of a target, without running it, as
/// `(mime, ext)`. Both engines can be asked for `webp`, and they write the
/// same MIME and the same extension, so one table answers for both.
pub fn output_for(to: &str) -> Option<(&'static str, &'static str)> {
    ConvertTarget::parse(Some(to)).map(image_output).or_else(|| AvTarget::parse(Some(to)).map(av_output))
}

// ---------------------------------------------------------------------------
// "It already is that"
// ---------------------------------------------------------------------------

/// Extensions that name the same format under two spellings. Kept
/// deliberately short: every entry is a claim that converting between the two
/// would be a no-op.
fn ext_alias(e: &str) -> Option<&'static str> {
    match e {
        "jpeg" | "jpe" => Some("jpg"),
        "apng" => Some("png"),
        _ => None,
    }
}

/// Types that ARE one of our targets. The row's stored MIME is the stronger
/// signal - a file named ".txt" holding a PNG is a PNG - so it is consulted
/// first, with the extension as the fallback for the many rows that carry
/// application/octet-stream.
fn mime_format(m: &str) -> Option<&'static str> {
    Some(match m {
        "application/pdf" => "pdf",
        "image/png" | "image/apng" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/gif" => "gif",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" | "audio/vorbis" => "ogg",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "video/webm" => "webm",
        _ => return None,
    })
}

fn normalise_ext(ext: Option<&str>) -> String {
    let e = ext.unwrap_or("");
    let e = e.strip_prefix('.').unwrap_or(e).to_lowercase();
    ext_alias(&e).map(str::to_string).unwrap_or(e)
}

/// `currentFormatOf(mime, ext)`: the target name this file already answers
/// to, or "" when it is none of them.
pub fn current_format_of(mime: Option<&str>, ext: Option<&str>) -> String {
    if let Some(f) = mime_format(&effective_type(mime, ext)) {
        return f.to_string();
    }
    let e = normalise_ext(ext);
    if output_for(&e).is_some() { e } else { String::new() }
}

/// `isAlreadyInFormat(mime, ext, to)`: would converting to `to` produce the
/// format this file already is?
pub fn is_already_in_format(mime: Option<&str>, ext: Option<&str>, to: &str) -> bool {
    let target = normalise_ext(Some(to));
    if target.is_empty() {
        return false;
    }
    current_format_of(mime, ext) == target || normalise_ext(ext) == target
}

// ---------------------------------------------------------------------------
// Which engine, and is it offered at all
// ---------------------------------------------------------------------------

/// `conversionEngineFor(mime, ext, frames, to, size)`: which engine would
/// answer `to` for this file, or `None` when nothing would.
///
/// The order is not incidental: it is the same order the /convert route
/// dispatches in (ffmpeg first), because both engines answer to `webp` and
/// they mean different things by it - all 43 frames of an AVIF sequence to
/// ffmpeg, the cover frame to libvips.
pub fn conversion_engine_for(
    mime: Option<&str>,
    ext: Option<&str>,
    frames: Option<f64>,
    to: Option<&str>,
    size: Option<&AvSourceSize>,
) -> Option<Engine> {
    if av_option_for(mime, ext, frames, to, size).is_some() {
        return Some(Engine::Av);
    }
    if convert_option_for(mime, ext, frames, to).is_some() {
        return Some(Engine::Image);
    }
    None
}

/// `convertTargetsForRow(row)`: everything this file can be converted INTO,
/// in menu order. The very list the "Download as..." menu renders
/// (`canEncode: true`, because an in-place conversion is only ever run by the
/// owner or an admin), so the convert submenu cannot offer a target the writer
/// would then refuse.
pub fn convert_targets_for_row(row: &ConvertCandidate) -> Vec<DownloadOption> {
    download_options_for(
        row.mime_type.as_deref(),
        row.ext.as_deref(),
        row.frames,
        DownloadMenuOptions { can_encode: true, size: row.av_size() },
    )
}

// ---------------------------------------------------------------------------
// The per-file decision
// ---------------------------------------------------------------------------

/// `ConvertPlan`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertPlan {
    Ok { engine: Engine, ext: &'static str, mime: &'static str },
    Skip(ConvertSkipReason),
}

/// `planConversion(row, to, mode, now)`: may this row be converted to `to`,
/// and by which engine?
///
/// Cheapest and most specific first, because the ORDER is what the user reads
/// in the report. A trashed file is not "unsupported", and a file that is
/// already a PNG is not "unsupported" either.
pub fn plan_conversion(row: &ConvertCandidate, to: &str, mode: ConvertMode, now: JsDate) -> ConvertPlan {
    if row.deleted_at.is_some() {
        return ConvertPlan::Skip(ConvertSkipReason::Trashed);
    }
    // A paste's bytes ARE its text: replacing them with a PDF leaves a row that
    // still claims kind='paste' and renders as source code. A copy is fine, and
    // lands as an ordinary file.
    if row.kind == "paste" && mode == ConvertMode::Replace {
        return ConvertPlan::Skip(ConvertSkipReason::Paste);
    }
    if row.expires_at.is_some_and(|e| e.millis() <= now.millis()) {
        return ConvertPlan::Skip(ConvertSkipReason::Expired);
    }
    if is_already_in_format(row.mime_type.as_deref(), row.ext.as_deref(), to) {
        return ConvertPlan::Skip(ConvertSkipReason::AlreadyInFormat);
    }
    let size = row.av_size();
    let engine = conversion_engine_for(row.mime_type.as_deref(), row.ext.as_deref(), row.frames, Some(to), Some(&size));
    match (engine, output_for(to)) {
        (Some(engine), Some((mime, ext))) => ConvertPlan::Ok { engine, ext, mime },
        _ => ConvertPlan::Skip(ConvertSkipReason::UnsupportedTarget),
    }
}

/// `BulkPlan`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkPlan {
    pub to: String,
    pub mode: ConvertMode,
    pub convert: Vec<(String, Engine)>,
    pub skipped: Vec<(String, ConvertSkipReason)>,
}

/// `planBulkConversion(rows, to, mode, now)`: split a selection into "these
/// will change" and "these will not, and why".
pub fn plan_bulk_conversion(rows: &[ConvertCandidate], to: &str, mode: ConvertMode, now: JsDate) -> BulkPlan {
    let mut convert = Vec::new();
    let mut skipped = Vec::new();
    for row in rows {
        match plan_conversion(row, to, mode, now) {
            ConvertPlan::Ok { engine, .. } => convert.push((row.id.clone(), engine)),
            ConvertPlan::Skip(reason) => skipped.push((row.id.clone(), reason)),
        }
    }
    BulkPlan { to: to.to_string(), mode, convert, skipped }
}

/// `TargetSummary`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSummary {
    pub to: String,
    /// The engine the FIRST file offering this target would use, for
    /// labelling only.
    pub engine: Engine,
    /// Files the conversion would actually run on.
    pub eligible: u32,
    /// Files already in that format - nothing to do, and not a problem.
    pub already_in_format: u32,
    /// Files whose format cannot go there, plus anything trashed or expired.
    pub unsupported: u32,
}

/// `bulkTargetSummary(rows, mode, now)`: which targets to offer for a MIXED
/// selection, and how many files each covers. The union rather than the
/// intersection; ordered by reach, then by the order the single-file menu
/// uses, so the entry that does the most work is the one under the cursor.
pub fn bulk_target_summary(rows: &[ConvertCandidate], mode: ConvertMode, now: JsDate) -> Vec<TargetSummary> {
    let mut seen: Vec<TargetSummary> = Vec::new();
    for row in rows {
        for option in convert_targets_for_row(row) {
            if !seen.iter().any(|s| s.to == option.to) {
                seen.push(TargetSummary {
                    to: option.to.to_string(),
                    engine: option.engine,
                    eligible: 0,
                    already_in_format: 0,
                    unsupported: 0,
                });
            }
        }
    }
    for row in rows {
        for entry in seen.iter_mut() {
            match plan_conversion(row, &entry.to, mode, now) {
                ConvertPlan::Ok { .. } => entry.eligible += 1,
                ConvertPlan::Skip(ConvertSkipReason::AlreadyInFormat) => entry.already_in_format += 1,
                ConvertPlan::Skip(_) => entry.unsupported += 1,
            }
        }
    }
    // `seen` is in first-offered order, so a stable sort on reach alone is
    // `b.eligible - a.eligible || order.indexOf(a.to) - order.indexOf(b.to)`.
    let mut out: Vec<TargetSummary> = seen.into_iter().filter(|e| e.eligible > 0).collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.eligible));
    out
}

// ---------------------------------------------------------------------------
// Quota
// ---------------------------------------------------------------------------

/// `quotaDeltaFor(mode, oldSize, newSize)`: bytes a conversion adds to the
/// owner's usage. A copy is a second file, so it costs its whole size. A
/// replace hands the old bytes back, so it costs the DIFFERENCE.
pub fn quota_delta_for(mode: ConvertMode, old_size: i64, new_size: i64) -> i64 {
    match mode {
        ConvertMode::Copy => new_size,
        ConvertMode::Replace | ConvertMode::Other => new_size - old_size,
    }
}

/// `quotaAllowsDelta(owner, delta)`: `maxQuota` null or negative is
/// unlimited. A delta of zero or less is ALWAYS allowed: refusing to shrink a
/// file because the account is already over its limit would trap the owner in
/// the state the limit exists to get them out of.
pub fn quota_allows_delta(used_quota: Option<i64>, max_quota: Option<i64>, delta: i64) -> bool {
    if delta <= 0 {
        return true;
    }
    let max = max_quota.unwrap_or(-1);
    if max < 0 {
        return true;
    }
    used_quota.unwrap_or(0) + delta <= max
}

// ---------------------------------------------------------------------------
// What the row becomes
// ---------------------------------------------------------------------------

/// `ConvertedBytes`: everything an in-place conversion learns about the new
/// bytes. Playing time and frame rate of the RESULT are listed for the same
/// reason `width` is: the patch below writes them unconditionally, so a stale
/// value cannot survive a conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertedBytes {
    pub storage_key: String,
    pub size: i64,
    pub checksum: String,
    pub mime: String,
    pub ext: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub frames: Option<i32>,
    pub duration_seconds: Option<f64>,
    pub fps: Option<f64>,
    pub blurhash: Option<String>,
}

/// The columns of `uploads` before the conversion that the patch reads.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchSource<'a> {
    pub ext: Option<&'a str>,
    pub filename: &'a str,
    pub original_ext: Option<&'a str>,
}

/// The exact column patch an in-place conversion applies
/// (`conversionRowPatch`).
#[derive(Debug, Clone, PartialEq)]
pub struct ConversionRowPatch {
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    pub storage_key: String,
    pub checksum: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub frames: Option<i32>,
    pub duration_seconds: Option<f64>,
    pub fps: Option<f64>,
    pub blurhash: Option<String>,
    pub thumb_key: Option<String>,
    pub filename: String,
    pub converted_at: JsDate,
    pub original_ext: Option<String>,
    pub updated_at: JsDate,
}

/// `conversionRowPatch(before, bytes, now)`.
///
/// Its most important property is what it does NOT contain. `code` stays, so
/// every link already shared keeps resolving - the extension in a URL is
/// decorative and the code belongs to this row forever. `delete_token` stays,
/// so the DeletionURL in an old ShareX config keeps working. The counters, the
/// folder, the published flag and the whole access gate (password, expiry,
/// view cap) stay, because they describe the LINK and the link has not changed.
///
/// `thumbKey` is cleared and `blurhash` recomputed for the opposite reason:
/// those describe the BYTES, and the bytes are new.
pub fn conversion_row_patch(before: &PatchSource<'_>, bytes: &ConvertedBytes, now: JsDate) -> ConversionRowPatch {
    ConversionRowPatch {
        ext: bytes.ext.clone(),
        mime_type: bytes.mime.clone(),
        size: bytes.size,
        storage_key: bytes.storage_key.clone(),
        checksum: bytes.checksum.clone(),
        width: bytes.width,
        height: bytes.height,
        frames: bytes.frames,
        duration_seconds: bytes.duration_seconds,
        fps: bytes.fps,
        blurhash: bytes.blurhash.clone(),
        // A thumbnail of the old bytes is a picture of a file that no longer exists.
        thumb_key: None,
        filename: swap_filename_ext(before.filename, &bytes.ext),
        converted_at: now,
        // Only the FIRST conversion records where the file came from;
        // converting a PNG to WebP and then to AVIF still started as a PNG.
        original_ext: before.original_ext.or(before.ext).map(str::to_string),
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/convert-plan.test.ts
    use super::*;
    use crate::media::paths::{code_from_media_code, suffix_key};
    use crate::media::urls::{public_url, raw_url, storage_key_for};

    fn now() -> JsDate {
        JsDate::parse_iso("2026-08-11T12:00:00.000Z").unwrap()
    }

    fn file() -> ConvertCandidate {
        ConvertCandidate {
            id: "id-1".into(),
            ext: Some("png".into()),
            mime_type: Some("image/png".into()),
            frames: Some(1.0),
            size: 1000.0,
            kind: "file".into(),
            ..Default::default()
        }
    }

    fn bytes(ext: &str, mime: &str) -> ConvertedBytes {
        ConvertedBytes {
            storage_key: format!("alicja/bomep7.{ext}"),
            size: 2700,
            checksum: "c".repeat(64),
            mime: mime.into(),
            ext: ext.into(),
            width: Some(480),
            height: Some(270),
            frames: Some(72),
            duration_seconds: None,
            fps: None,
            blurhash: None,
        }
    }

    const ROW: PatchSource<'static> =
        PatchSource { ext: Some("mp4"), filename: "wakacje.2024.mp4", original_ext: None };

    #[test]
    fn an_old_link_keeps_working_after_an_in_place_conversion() {
        let before = public_url("alicja", "bomep7", "mp4", "");
        let after = public_url("alicja", "bomep7", "gif", "");
        assert_ne!(before, after);
        let segment = |u: &str| u.rsplit('/').next().unwrap().to_string();
        assert_eq!(code_from_media_code(&segment(&before)).as_deref(), Some("bomep7"));
        assert_eq!(code_from_media_code(&segment(&after)).as_deref(), Some("bomep7"));
        assert_eq!(code_from_media_code("bomep7").as_deref(), Some("bomep7"));
        assert!(raw_url("alicja", "bomep7", "mp4", "").contains("/bomep7/raw.mp4"));
        assert!(raw_url("alicja", "bomep7", "gif", "").contains("/bomep7/raw.gif"));
    }

    #[test]
    fn rewrites_exactly_what_the_bytes_changed() {
        let mut b = bytes("gif", "image/gif");
        b.blurhash = Some("LEHV6nWB".into());
        let patch = conversion_row_patch(&ROW, &b, now());
        // The struct has no field for any column a link is made of (code,
        // delete token, owner, counters, gate, folder, timestamps but these).
        assert_eq!(patch.ext, "gif");
        assert_eq!(patch.mime_type, "image/gif");
        assert_eq!(patch.filename, "wakacje.2024.gif");
        assert_eq!(patch.thumb_key, None);
        assert_eq!(patch.original_ext.as_deref(), Some("mp4"));
        assert_eq!(patch.converted_at, now());
        assert_eq!(patch.duration_seconds, None);
        assert_eq!(patch.fps, None);

        let mut copy = bytes("mkv", "video/x-matroska");
        copy.duration_seconds = Some(13.434);
        copy.fps = Some(30.0);
        let patch = conversion_row_patch(&ROW, &copy, now());
        assert_eq!(patch.duration_seconds, Some(13.434));
        assert_eq!(patch.fps, Some(30.0));

        let second = conversion_row_patch(
            &PatchSource { ext: Some("gif"), filename: "a.gif", original_ext: Some("mp4") },
            &bytes("webp", "image/webp"),
            now(),
        );
        assert_eq!(second.original_ext.as_deref(), Some("mp4"));
    }

    #[test]
    fn storage_keys_and_filenames() {
        assert_eq!(swap_key_ext("ToqOse/bomep7.mp4", "gif"), "ToqOse/bomep7.gif");
        assert_eq!(swap_key_ext("old-tag/bomep7.mp4", "gif"), "old-tag/bomep7.gif");
        assert_eq!(swap_key_ext(&storage_key_for("ToqOse", "bomep7", "mp4"), "gif"), "ToqOse/bomep7.gif");
        assert_eq!(swap_key_ext("ToqOse/bomep7", "png"), "ToqOse/bomep7.png");
        assert_eq!(swap_key_ext("ToqOse/bomep7.png", ""), "ToqOse/bomep7");
        assert_eq!(swap_filename_ext("wakacje.2024.mp4", "gif"), "wakacje.2024.gif");
        assert_eq!(swap_filename_ext("zrzut", "png"), "zrzut.png");
        assert_eq!(swap_filename_ext(".gitignore", "txt"), ".gitignore.txt");
        assert_eq!(suffix_key("ToqOse/bomep7.gif", "a1b2"), "ToqOse/bomep7-a1b2.gif");
        assert_eq!(suffix_key("ToqOse/bomep7", "a1b2"), "ToqOse/bomep7-a1b2");
    }

    #[test]
    fn what_a_file_already_is() {
        assert_eq!(current_format_of(Some("image/png"), Some("png")), "png");
        assert_eq!(current_format_of(Some("image/jpeg"), Some("jpg")), "jpg");
        assert_eq!(current_format_of(Some("video/quicktime"), Some("mov")), "mov");
        assert_eq!(current_format_of(Some("audio/mpeg"), Some("mp3")), "mp3");
        assert_eq!(current_format_of(Some("application/octet-stream"), Some("avif")), "avif");
        assert_eq!(current_format_of(Some("application/octet-stream"), Some("webp")), "webp");
        assert!(is_already_in_format(Some("image/jpeg"), Some("jpeg"), "jpg"));
        assert!(is_already_in_format(Some("application/octet-stream"), Some("jpe"), "jpg"));
        assert!(is_already_in_format(Some("image/apng"), Some("apng"), "png"));
        assert!(!is_already_in_format(Some("image/png"), Some("png"), "webp"));
        assert!(!is_already_in_format(Some("video/mp4"), Some("mp4"), "mkv"));
    }

    #[test]
    fn which_engine_answers() {
        let e = |m: &str, x: &str, f: Option<f64>, to: &str| conversion_engine_for(Some(m), Some(x), f, Some(to), None);
        assert_eq!(e("video/mp4", "mp4", None, "mkv"), Some(Engine::Av));
        assert_eq!(e("audio/mpeg", "mp3", None, "ogg"), Some(Engine::Av));
        assert_eq!(e("image/avif", "avif", None, "webp"), Some(Engine::Av));
        assert_eq!(e("image/png", "png", Some(1.0), "webp"), Some(Engine::Image));
        assert_eq!(e("image/jpeg", "jpg", Some(1.0), "pdf"), Some(Engine::Image));
        assert_eq!(e("application/zip", "zip", None, "png"), None);
        assert_eq!(e("image/png", "png", Some(1.0), "mkv"), None);
        assert_eq!(output_for("gif"), Some(("image/gif", "gif")));
        assert_eq!(output_for("pdf"), Some(("application/pdf", "pdf")));
        assert_eq!(output_for("nonsense"), None);
    }

    #[test]
    fn what_a_single_conversion_decides() {
        assert_eq!(
            plan_conversion(&file(), "webp", ConvertMode::Replace, now()),
            ConvertPlan::Ok { engine: Engine::Image, ext: "webp", mime: "image/webp" }
        );
        assert_eq!(
            plan_conversion(&file(), "png", ConvertMode::Replace, now()),
            ConvertPlan::Skip(ConvertSkipReason::AlreadyInFormat)
        );
        let trashed = ConvertCandidate { deleted_at: JsDate::parse_iso("2026-01-01T00:00:00.000Z"), ..file() };
        assert!(matches!(plan_conversion(&trashed, "png", ConvertMode::Replace, now()), ConvertPlan::Skip(_)));
        assert_eq!(
            plan_conversion(&trashed, "webp", ConvertMode::Replace, now()),
            ConvertPlan::Skip(ConvertSkipReason::Trashed)
        );
        let expired = ConvertCandidate { expires_at: JsDate::parse_iso("2026-08-01T00:00:00.000Z"), ..file() };
        assert_eq!(
            plan_conversion(&expired, "webp", ConvertMode::Replace, now()),
            ConvertPlan::Skip(ConvertSkipReason::Expired)
        );
        let paste = ConvertCandidate {
            kind: "paste".into(),
            ext: Some("ts".into()),
            mime_type: Some("text/plain".into()),
            frames: None,
            ..file()
        };
        assert_eq!(
            plan_conversion(&paste, "pdf", ConvertMode::Replace, now()),
            ConvertPlan::Skip(ConvertSkipReason::Paste)
        );
        let zip = ConvertCandidate { ext: Some("zip".into()), mime_type: Some("application/zip".into()), ..file() };
        assert_eq!(
            plan_conversion(&zip, "png", ConvertMode::Replace, now()),
            ConvertPlan::Skip(ConvertSkipReason::UnsupportedTarget)
        );
    }

    fn named(id: String, ext: &str, mime: &str) -> ConvertCandidate {
        ConvertCandidate { id, ext: Some(ext.into()), mime_type: Some(mime.into()), ..file() }
    }

    fn selection() -> (Vec<ConvertCandidate>, Vec<ConvertCandidate>) {
        let jpegs = (0..5).map(|i| named(format!("jpg-{i}"), "jpg", "image/jpeg")).collect();
        let pngs = (0..3).map(|i| named(format!("png-{i}"), "png", "image/png")).collect();
        (jpegs, pngs)
    }

    #[test]
    fn a_mixed_selection() {
        let (jpegs, pngs) = selection();
        let all: Vec<_> = jpegs.iter().chain(&pngs).cloned().collect();
        let plan = plan_bulk_conversion(&all, "png", ConvertMode::Replace, now());
        assert_eq!(
            plan.convert.iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
            jpegs.iter().map(|f| f.id.clone()).collect::<Vec<_>>()
        );
        assert_eq!(plan.skipped.len(), 3);
        assert!(plan.skipped.iter().all(|s| s.1 == ConvertSkipReason::AlreadyInFormat));

        let summary = bulk_target_summary(&all, ConvertMode::Replace, now());
        let by = |to: &str| summary.iter().find(|s| s.to == to).unwrap().clone();
        assert_eq!(by("webp").eligible, 8);
        assert_eq!(by("avif").eligible, 8);
        assert_eq!(by("png").eligible, 5);
        assert_eq!(by("png").already_in_format, 3);
        assert_eq!(by("jpg").eligible, 3);
        assert_eq!(by("jpg").already_in_format, 5);
        assert_eq!(summary[0].eligible, 8);
        let mut sorted: Vec<u32> = summary.iter().map(|s| s.eligible).collect();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(summary.iter().map(|s| s.eligible).collect::<Vec<_>>(), sorted);
        assert!(summary.iter().all(|s| s.eligible > 0));
        assert!(!summary.iter().any(|s| s.to == "mkv"));

        let mut mixed = pngs.clone();
        mixed.push(ConvertCandidate { frames: None, ..named("vid".into(), "mp4", "video/mp4") });
        let summary = bulk_target_summary(&mixed, ConvertMode::Replace, now());
        let by = |to: &str| summary.iter().find(|s| s.to == to).unwrap().clone();
        assert_eq!(by("mkv").eligible, 1);
        assert_eq!(by("mkv").unsupported, 3);
        assert_eq!(by("webp").eligible, 3);

        let mut with_trash = jpegs.clone();
        with_trash.push(ConvertCandidate {
            id: "gone".into(),
            ext: Some("jpg".into()),
            deleted_at: Some(now()),
            ..file()
        });
        let plan = plan_bulk_conversion(&with_trash, "png", ConvertMode::Replace, now());
        assert_eq!(plan.convert.len(), 5);
        assert_eq!(plan.skipped, vec![("gone".to_string(), ConvertSkipReason::Trashed)]);
    }

    #[test]
    fn what_a_conversion_costs_the_quota() {
        assert_eq!(quota_delta_for(ConvertMode::Copy, 1000, 300), 300);
        assert_eq!(quota_delta_for(ConvertMode::Copy, 1000, 4000), 4000);
        assert_eq!(quota_delta_for(ConvertMode::Replace, 1000, 300), -700);
        assert_eq!(quota_delta_for(ConvertMode::Replace, 1000, 4000), 3000);
        assert_eq!(quota_delta_for(ConvertMode::Replace, 1000, 1000), 0);
        assert!(quota_allows_delta(Some(900), Some(1000), 100));
        assert!(!quota_allows_delta(Some(900), Some(1000), 101));
        assert!(quota_allows_delta(Some(5000), Some(1000), -1));
        assert!(quota_allows_delta(Some(5000), Some(1000), 0));
        assert!(!quota_allows_delta(Some(5000), Some(1000), 1));
        assert!(quota_allows_delta(Some(10i64.pow(12)), Some(-1), 10i64.pow(9)));
        assert!(quota_allows_delta(Some(10i64.pow(12)), None, 10i64.pow(9)));
    }

    #[test]
    fn outcomes_serialise_like_the_upstream_literals() {
        let v = crate::rpc::tagged::to_tagged_value(&ConvertOutcome::Converted {
            upload_id: "u".into(),
            code: "c".into(),
            ext: "webp".into(),
            mime_type: "image/webp".into(),
            size: 10,
            previous_size: 20,
            previous_ext: "png".into(),
            note: None,
        })
        .unwrap();
        assert_eq!(
            v.to_string(),
            r#"{"status":"converted","uploadId":"u","code":"c","ext":"webp","mimeType":"image/webp","size":10,"previousSize":20,"previousExt":"png","note":{"$undefined":1}}"#
        );
        let v = serde_json::to_string(&ConvertOutcome::Skipped {
            upload_id: "u".into(),
            reason: ConvertSkipReason::AlreadyInFormat,
        })
        .unwrap();
        assert_eq!(v, r#"{"status":"skipped","uploadId":"u","reason":"already-in-format"}"#);
    }
}

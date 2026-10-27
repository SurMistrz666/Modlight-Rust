//! Port of `app/[userCode]/[mediaCode]/[action]/route.ts`: everything a public
//! file URL can be asked for besides its preview page.
//!
//! | action | what it is |
//! |---|---|
//! | `raw`, `raw.<ext>` | the bytes (Range, cache headers), a thumbnail with `w`/`h`, a moving tile with `motion=1` |
//! | `download` | the bytes as an attachment, counted as a download |
//! | `render` | an uploaded HTML/SVG page or a .docx, rendered in a sandbox, admitted by a ticket |
//! | `timeline.vtt`, `timeline.jpg` | the scrub-bar preview of a video (track and sprite sheet) |
//! | `convert` | "Download as…" (`to=`, `check=1`, `t=`, `d=`) |
//!
//! Anything else is 404. The view counter follows upstream's rules for files
//! without a view cap; for capped files security fix 1 applies (see
//! [`crate::media::serve::ViewBudget`]): a request upstream exempted from the
//! counter (a seek, an embedded subresource, a thumbnail request falling back to
//! the original) needs the view grant of a counted view, or it is counted.
//!
//! The work that turns a file into something else (posters, motion tiles,
//! timelines, conversions, document rendering) is called through
//! [`crate::media::part_a`] under its upstream names.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use bytes::Bytes;
use uuid::Uuid;

use crate::auth_helpers::{RequestSession, Sessions};
use crate::db::models::{Columns, Upload};
use crate::error::AppResult;
use crate::http::cookies::{encode_uri_component, parse_cookies_next};
use crate::http::set_cookie::ResponseCookies;
use crate::media::abort::{AbortController, AbortSignal};
use crate::media::av_targets::{AvSourceSize, av_option_for};
use crate::media::bots::is_preview_bot;
use crate::media::convert_targets::{ConvertTarget, Motion, convert_option_for};
use crate::media::derived_cache::{content_hash, derived_key, may_cache_derived};
use crate::media::ffmpeg::{AvAbortedError, AvTimeoutError};
use crate::media::image::{ThumbnailFit, ThumbnailOptions, make_thumbnail};
use crate::media::mime::{MediaKind, media_kind};
use crate::media::motion_preview::{MOTION_DEFAULT_SIZE, wants_motion};
use crate::media::part_a::{
    self, AvConvertRequest, DocxResult, MAX_DOCX_BYTES, MotionOptions, MotionRefusal, MotionResult,
    TIMELINE_SHEET_ACTION, TIMELINE_TRACK_ACTION, TimelineRefusal, TimelineSheet, TimelineTrack,
};
use crate::media::paste_type::row_mime_type;
use crate::media::paths::code_from_media_code;
use crate::media::probe::probe_image;
use crate::media::render_ticket::{
    MAX_RENDER_BYTES, RenderAdmission, SandboxShape, is_generated, render_admission, render_content_type, render_csp,
    sandbox_shape,
};
use crate::media::serve::{
    Lookup, ViewBudget, cache_headers_for, consume_view, first_header, has_view_grant, is_privileged_viewer,
    joined_header, lookup_media, plain_text, put_headers, stream_media, view_budget, view_grant_cookie,
    viewer_relation,
};
use crate::media::transform_cost::{
    CONVERT_TIMEOUT_MS, CONVERT_WEIGHT, MAX_THUMBNAIL_PIXELS, MAX_TRANSFORM_BYTES, MOTION_TIMEOUT_MS, MOTION_WEIGHT,
    THUMBNAIL_TIMEOUT_MS, THUMBNAIL_WEIGHT, TIMELINE_TIMEOUT_MS, TIMELINE_TRACK_TIMEOUT_MS, TIMELINE_TRACK_WEIGHT,
    TIMELINE_WEIGHT,
};
use crate::media::transform_queue::{AbortedError, BusyError, RunOptions, TimeoutError, image_transforms};
use crate::password::{MediaUnlockSubject, is_media_unlocked};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/{userCode}/{mediaCode}/{action}", get(handle))
}

/// Thumbnails are only rendered at these sizes. Arbitrary numbers from the query
/// string would let anyone mint unlimited distinct renders (and cache entries)
/// out of one file. 1200x630 is the Open Graph card size used by the preview page.
const THUMB_SIZES: [u32; 7] = [64, 128, 256, 480, 630, 800, 1200];

/// Deadline for building a document preview. Bounds the RESPONSE, not the
/// work; the real bound is the zip guard, which refuses an archive before the
/// converter ever sees it.
const DOCX_TIMEOUT_MS: u64 = 20_000;

/// `snapThumbSize(raw)`: round up to the next allowed step rather than
/// rejecting, so existing links keep working while the set of renders stays
/// bounded.
fn snap_thumb_size(raw: Option<&str>) -> Option<u32> {
    let raw = raw.filter(|r| !r.is_empty())?;
    let n = crate::config::js_number(raw);
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    Some(THUMB_SIZES.into_iter().find(|s| f64::from(*s) >= n).unwrap_or(THUMB_SIZES[THUMB_SIZES.len() - 1]))
}

/// `isInitialRequest(rangeHeader)`: a media player issues a Range request per
/// seek; counting each one would burn a one-time link on the first scrub. Only
/// the opening request counts.
fn is_initial_request(range: Option<&str>) -> bool {
    match range {
        None | Some("") => true,
        Some(r) => crate::config::js_trim(r).starts_with("bytes=0-"),
    }
}

/// `seconds(raw)`: a non-negative number of seconds from the query string, or
/// None. NaN, Infinity and negatives become "the caller did not say", and the
/// cap keeps `?t=1e300` from becoming a 300-character argument.
fn seconds(raw: Option<&str>) -> Option<f64> {
    let raw = raw.filter(|r| !r.is_empty())?;
    let n = crate::config::js_number(raw);
    if !n.is_finite() || !(0.0..=86_400.0).contains(&n) {
        return None;
    }
    Some(n)
}

/// The query string as `URLSearchParams`.
#[derive(Debug, Clone, Default)]
struct Query(Vec<(String, String)>);

impl Query {
    fn parse(q: Option<&str>) -> Query {
        Query(form_urlencoded::parse(q.unwrap_or("").as_bytes()).into_owned().collect())
    }
    fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }
}

/// `isPreviewFetch(req, url)`: a preview rather than someone viewing the file.
///
/// Only ever consulted for /raw and /download. It is NOT a general exemption:
/// a conversion is always a deliberate download. (Security fix 1: for a file
/// with a view cap, being a preview only waives the counter together with the
/// view grant, see `ViewBudget`.)
fn is_preview_fetch(query: &Query, sec_fetch_dest: Option<&str>) -> bool {
    // `motion` joins `w`/`h` here: a moving tile is asked for by a grid exactly
    // like a still one.
    if query.has("w") || query.has("h") {
        return true;
    }
    if wants_motion(&query.0) {
        return true;
    }
    matches!(sec_fetch_dest, Some("image") | Some("video"))
}

fn not_found() -> Response {
    plain_text(StatusCode::NOT_FOUND, "Not found")
}

fn no_store() -> (&'static str, String) {
    ("Cache-Control", "no-store".to_string())
}

fn status(code: u16) -> StatusCode {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// The request as the handler reads it.
struct Ask {
    headers: HeaderMap,
    cookies: ResponseCookies,
    query: Query,
    raw_path: String,
    raw_query: Option<String>,
    signal: AbortSignal,
}

impl Ask {
    /// Where the vault's unlock form sends the browser back to: this URL.
    fn next_path(&self) -> String {
        match self.raw_query.as_deref().filter(|q| !q.is_empty()) {
            Some(q) => format!("{}?{q}", self.raw_path),
            None => self.raw_path.clone(),
        }
    }
}

/// Also reached from `routes::api::downloads` for a `/downloads/...` path that
/// names no build (an account tagged `downloads`).
pub(crate) async fn handle(
    State(state): State<AppState>,
    Path((user_code, media_code, action)): Path<(String, String, String)>,
    Sessions(session): Sessions,
    req: Request,
) -> AppResult<Response> {
    let (parts, _body) = req.into_parts();
    // `req.signal`: a client that hangs up drops this future, which aborts the
    // signal handed to the limiter and the engines.
    let controller = AbortController::new();
    let guard = controller.abort_on_drop();
    let ask = Ask {
        cookies: parts.extensions.get::<ResponseCookies>().cloned().unwrap_or_default(),
        query: Query::parse(parts.uri.query()),
        raw_path: parts.uri.path().to_string(),
        raw_query: parts.uri.query().map(str::to_string),
        headers: parts.headers,
        signal: controller.signal(),
    };
    let res = dispatch(&state, &session, &user_code, &media_code, &action, &ask).await;
    guard.disarm();
    res
}

async fn dispatch(
    state: &AppState,
    session: &RequestSession,
    user_code: &str,
    media_code: &str,
    action: &str,
    ask: &Ask,
) -> AppResult<Response> {
    // Handled before anything else: the sandboxed preview is authorized by its own
    // ticket, looks the file up by id rather than by public code, and must NOT
    // spend a view (the page that issued the ticket already counted one).
    if action == "render" {
        return render_sandboxed(state, session, user_code, media_code, ask).await;
    }

    let (found, spent) = match lookup_media(&state.db, user_code, media_code).await? {
        Lookup::Live(found) => (found, false),
        Lookup::Spent(found) => (found, true),
        Lookup::Missing => return Ok(not_found()),
    };
    let media = &found.media;
    let secret = &state.config.auth.secret;
    let media_id = media.id.to_string();
    let grant = media.max_views.is_some() && has_view_grant(secret, &ask.headers, &media_id);
    // Security fix 1: a budget spent EXACTLY is still open to the holder of the
    // grant of the view that spent it, for the requests that belong to that
    // view (the bytes, ranged or not, and their derived previews). Everything
    // else answers what upstream answered: not found.
    let grant_only = spent;
    if spent && !(grant && matches!(action_kind(action), ActionKind::Raw | ActionKind::Download | ActionKind::Timeline))
    {
        return Ok(not_found());
    }

    // The owner (and admins) see everything and are exempt from the counters -
    // checking your own upload should not spend its view budget.
    let privileged = is_privileged_viewer(&state.db, &ask.headers, session, &found.owner_id).await?;

    // Unpublished media is visible only to its owner or an admin.
    if !media.published && !privileged {
        return Ok(not_found());
    }

    // Not upstream: the owner's file vault (docs/ENCRYPTION.md). Nobody is
    // exempt, the owner and administrators included; from here on `state`
    // reads this file's plaintext only for a request that holds a grant.
    let Some(keyed) =
        crate::vault::access::open_files(state, &ask.headers, &found.owner_id, found.owner_vault.as_deref(), &[media])
            .await?
    else {
        return Ok(crate::vault::gate::locked_response(state, &ask.headers, &found.owner_user_code, &ask.next_path()));
    };
    let state = &keyed;

    // Same gate as the preview page: the page is what the visitor unlocks, but
    // /raw is what the <img> and the player actually fetch.
    if media.password_hash.as_deref().is_some_and(|h| !h.is_empty()) && !privileged && !unlocked(state, ask, media) {
        return Ok(plain_text(StatusCode::UNAUTHORIZED, "Password required"));
    }

    let range = joined_header(&ask.headers, "range");
    let dest = joined_header(&ask.headers, "sec-fetch-dest");
    // The bot check: an image advertises its og:image as this very URL, so an
    // unfurl lands here looking exactly like a reader. ...but the exemption stops
    // where a BUDGET starts: User-Agent is chosen by the caller, so a capped file
    // counts every request instead.
    let bot_exempt = media.max_views.is_none() && is_preview_bot(first_header(&ask.headers, "user-agent").as_deref());
    let should_count = !privileged
        && is_initial_request(range.as_deref())
        && !is_preview_fetch(&ask.query, dest.as_deref())
        && !bot_exempt;
    let gate = Gate { state, media, privileged, should_count, grant, grant_only, cookies: &ask.cookies };

    match action_kind(action) {
        ActionKind::Download => {
            if !gate.admit_content(true).await? {
                return Ok(not_found());
            }
            Ok(stream_media(state, media, range.as_deref(), true).await)
        }
        // The scrub-bar preview, in two halves. Neither spends a view: both are
        // fetched by the player while the page is loading or while a pointer is
        // moving over the bar.
        ActionKind::Timeline if action == TIMELINE_TRACK_ACTION => Ok(timeline_track_action(state, media, ask).await),
        ActionKind::Timeline => {
            let res = timeline_sheet_action(state, media, ask).await;
            // Upstream counts no timeline request; for a capped file the sheet
            // (frames of the video) needs the grant or a view (security fix 1).
            let sheet_gate = Gate { should_count: false, ..gate };
            if res.status() == StatusCode::OK && !sheet_gate.admit_content(false).await? {
                return Ok(not_found());
            }
            Ok(res)
        }
        ActionKind::Raw => raw_action(&gate, ask, range.as_deref()).await,
        // On-the-fly conversion / "download as…". The set of targets depends
        // entirely on the file: media::download_menu is what the UI renders.
        ActionKind::Convert => convert_action(state, media, privileged, ask).await,
        ActionKind::Other => Ok(not_found()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    Raw,
    Download,
    Timeline,
    Convert,
    Other,
}

fn action_kind(action: &str) -> ActionKind {
    if action == "download" {
        ActionKind::Download
    } else if action == TIMELINE_TRACK_ACTION || action == TIMELINE_SHEET_ACTION {
        ActionKind::Timeline
    } else if action == "raw" || action.starts_with("raw.") {
        ActionKind::Raw
    } else if action == "convert" {
        ActionKind::Convert
    } else {
        ActionKind::Other
    }
}

fn unlocked(state: &AppState, ask: &Ask, media: &Upload) -> bool {
    let jar = parse_cookies_next(&ask.headers);
    is_media_unlocked(
        &state.config.auth.secret,
        &MediaUnlockSubject {
            media_id: media.id.to_string(),
            owner_id: media.owner_id.clone(),
            password_version: media.password_version.map(i64::from),
        },
        |name| jar.get(name).cloned(),
    )
}

/// Who pays for a response that carries the file's content.
#[derive(Clone, Copy)]
struct Gate<'a> {
    state: &'a AppState,
    media: &'a Upload,
    privileged: bool,
    /// What upstream's `shouldCount` says.
    should_count: bool,
    /// The request carries the view grant of this (capped) file.
    grant: bool,
    /// The budget is spent: only a grant may serve, nothing may be counted.
    grant_only: bool,
    /// Where a grant minted by this request goes (`Set-Cookie`).
    cookies: &'a ResponseCookies,
}

impl Gate<'_> {
    /// Count the view upstream counts, BEFORE anything else happens (`/raw`
    /// counts the opening request even when it ends up drawing a thumbnail).
    async fn count_upfront(&self, download: bool) -> anyhow::Result<bool> {
        if !self.should_count {
            return Ok(true);
        }
        self.consume(download).await
    }

    /// Security fix 1 at the moment the file's content is about to go out: its
    /// stored bytes, or a picture made of them (a resized still, a moving tile,
    /// a timeline sprite sheet).
    async fn admit_content(&self, download: bool) -> anyhow::Result<bool> {
        match view_budget(self.media, self.privileged, self.should_count, self.grant) {
            ViewBudget::Free | ViewBudget::Granted => Ok(true),
            ViewBudget::Consume => self.consume(download).await,
        }
    }

    async fn consume(&self, download: bool) -> anyhow::Result<bool> {
        if self.grant_only {
            return Ok(false);
        }
        let ok = consume_view(&self.state.db, self.media, download).await?;
        if ok && self.media.max_views.is_some() {
            // The view just counted covers the requests that belong to it.
            let cookie = view_grant_cookie(
                &self.state.config.auth.secret,
                &self.media.id.to_string(),
                self.state.config.is_prod,
            );
            self.cookies.push(cookie);
        }
        Ok(ok)
    }
}

async fn raw_action(gate: &Gate<'_>, ask: &Ask, range: Option<&str>) -> AppResult<Response> {
    let state = gate.state;
    let media = gate.media;
    // The opening request is counted first, exactly where upstream counts it.
    let counted_upfront = gate.should_count;
    if !gate.count_upfront(false).await? {
        return Ok(not_found());
    }

    let width = snap_thumb_size(ask.query.get("w"));
    let height = snap_thumb_size(ask.query.get("h"));

    // mediaKind, not an `image/` prefix test: a row stored as octet-stream by a
    // client that did not know the format is still an image. Video joins images
    // here; the size ceiling stays on images only (a video poster is a seek).
    let kind = media_kind(Some(&media.mime_type), Some(&media.ext));
    let thumbnailable =
        kind == MediaKind::Video || (kind == MediaKind::Image && media.size as u64 <= MAX_TRANSFORM_BYTES);
    let inside = ask.query.get("fit") == Some("inside");

    // A tile that MOVES: never falls back to a still or to the original.
    if wants_motion(&ask.query.0) {
        let opts = MotionOptions {
            // A bare `?motion=1` still gets a bounded size rather than the file.
            width: width.or(if height.is_some() { None } else { Some(MOTION_DEFAULT_SIZE) }),
            height,
            inside,
        };
        let res = motion_action(state, media, opts, ask).await;
        // A moving tile is the file's pictures (security fix 1).
        if res.status() == StatusCode::OK && !counted_upfront && !gate.admit_content(false).await? {
            return Ok(not_found());
        }
        return Ok(res);
    }

    if (width.is_some() || height.is_some()) && thumbnailable {
        // Through the SAME limiter as /convert: it reads the whole file, decodes
        // it with libvips, and anybody with a public link can ask for one.
        let fit = if inside { ThumbnailFit::Inside } else { ThumbnailFit::Cover };
        let job_state = state.clone();
        let job_media = media.clone();
        let run =
            image_transforms()
                .run(
                    move |signal| async move {
                        Ok(render_thumbnail(&job_state, &job_media, width, height, fit, signal).await)
                    },
                    RunOptions {
                        weight: Some(THUMBNAIL_WEIGHT),
                        timeout_ms: Some(THUMBNAIL_TIMEOUT_MS),
                        signal: Some(ask.signal.clone()),
                    },
                )
                .await;
        let thumb = match run {
            Ok(thumb) => thumb,
            // Backpressure, honestly: falling through to the original would answer
            // a request for a 4 kB tile with a 30 MB file.
            Err(err) => match transform_refusal(&err) {
                Some(refusal) => return Ok(refusal),
                None => None,
            },
        };
        if let Some(thumb) = thumb {
            // A resized still is the picture, at up to 1200 px; for anything
            // smaller than that it is the whole of it (security fix 1).
            if !counted_upfront && !gate.admit_content(false).await? {
                return Ok(not_found());
            }
            let mut res = Response::new(Body::from(thumb));
            put_headers(
                &mut res,
                [("Content-Type", "image/webp".to_string()), ("X-Content-Type-Options", "nosniff".to_string())]
                    .into_iter()
                    .chain(cache_headers_for(media)),
            );
            return Ok(res);
        }
        // For a VIDEO falling through would hand back the video: a client drawing
        // a grid of tiles would start downloading gigabytes.
        if kind == MediaKind::Video {
            let mut res = plain_text(status(415), "No poster for this file");
            put_headers(&mut res, [no_store()]);
            return Ok(res);
        }
        // fall through to the untransformed file
    }

    // The stored bytes. A request upstream counted has paid already; one it
    // exempted pays now unless it carries the view grant (security fix 1).
    if !counted_upfront && !gate.admit_content(false).await? {
        return Ok(not_found());
    }
    Ok(stream_media(state, media, range, false).await)
}

/// `renderThumbnail(media, opts, signal)`: best-effort; None means "serve the
/// original instead". Runs inside the shared limiter.
async fn render_thumbnail(
    state: &AppState,
    media: &Upload,
    width: Option<u32>,
    height: Option<u32>,
    fit: ThumbnailFit,
    signal: AbortSignal,
) -> Option<Vec<u8>> {
    let opts = ThumbnailOptions { width, height, fit, timeout_ms: Some(THUMBNAIL_TIMEOUT_MS), ..Default::default() };
    // A video has no still to resize, so one is taken first (a seek, cached).
    if media_kind(Some(&media.mime_type), Some(&media.ext)) == MediaKind::Video {
        let frame = part_a::video_poster(state, media, signal).await.ok()??;
        return make_thumbnail(frame, opts).await.ok();
    }
    let storage = state.storage.get().ok()?;
    let buf = storage.read_buffer(&media.storage_key).await.ok()?;
    if signal.aborted() {
        return None;
    }
    // probeImage rather than a bare metadata read: it also carries the `avis`
    // brand workaround. The declared size is the guard - a 40 kB PNG can claim
    // 100k x 100k.
    let probe = probe_image(buf).await?;
    if probe.frame_pixels == 0 || probe.frame_pixels > MAX_THUMBNAIL_PIXELS {
        return None;
    }
    // The file has just been opened, so its frame count is free here; a row that
    // does not carry one can never be offered a moving tile. Not for a cover-only
    // read, where 1 is what libvips could reach and not what the file holds.
    if !probe.cover_only {
        remember(state, media, None, Some(probe.pages as i32));
    }
    make_thumbnail(probe.input, opts).await.ok()
}

/// `remember(media, facts)`: record a fact we only learn by opening the file,
/// and only where the row has none. Never awaited: a row that stays unfilled
/// simply gets asked again next time.
fn remember(state: &AppState, media: &Upload, checksum: Option<&str>, frames: Option<i32>) {
    let checksum = checksum.filter(|c| !c.is_empty() && media.checksum.is_none()).map(str::to_string);
    let frames = frames.filter(|f| *f != 0 && media.frames.is_none());
    let (db, id) = (state.db.clone(), media.id);
    let query = match (checksum, frames) {
        (None, None) => return,
        (Some(c), Some(f)) => {
            sqlx::query(r#"update "uploads" set "checksum" = $1, "frames" = $2 where "uploads"."id" = $3"#)
                .bind(c)
                .bind(f)
        }
        (Some(c), None) => sqlx::query(r#"update "uploads" set "checksum" = $1 where "uploads"."id" = $2"#).bind(c),
        (None, Some(f)) => sqlx::query(r#"update "uploads" set "frames" = $1 where "uploads"."id" = $2"#).bind(f),
    };
    let query = query.bind(id);
    tokio::spawn(async move {
        let _ = query.execute(&db).await;
    });
}

/// Turn a limiter (or engine) refusal into a response, or None when the error
/// is a real failure the caller should report as 500.
fn transform_refusal(err: &anyhow::Error) -> Option<Response> {
    let with = |code: u16, body: &str, retry: &str, reason: &str| {
        let mut res = plain_text(status(code), body);
        put_headers(
            &mut res,
            [
                ("Retry-After", retry.to_string()),
                ("X-Convert-Refused", reason.to_string()),
                ("Cache-Control", "no-store".to_string()),
            ],
        );
        res
    };
    let gone = || {
        let mut res = Response::new(Body::empty());
        *res.status_mut() = status(499);
        put_headers(&mut res, [no_store()]);
        res
    };
    // ffprobe's own timeout and "the caller went away" mean the same things as
    // the queue's own two errors, so they get the same two answers.
    if err.downcast_ref::<AvTimeoutError>().is_some() {
        return Some(with(503, "Reading the file took too long", "10", "timeout"));
    }
    if err.downcast_ref::<AvAbortedError>().is_some() {
        return Some(gone());
    }
    if err.downcast_ref::<BusyError>().is_some() {
        return Some(with(503, "Too many transforms in progress", "5", "busy"));
    }
    if err.downcast_ref::<TimeoutError>().is_some() {
        return Some(with(503, "Transform took too long", "30", "timeout"));
    }
    if err.downcast_ref::<AbortedError>().is_some() {
        // Nobody is listening; the status only has to be cheap and non-cacheable.
        return Some(gone());
    }
    None
}

// ── Motion ──────────────────────────────────────────────────────────────────

/// `motionAction(req, media, opts)`: through the SAME limiter as the still
/// thumbnail and /convert. When there is no preview to make, the answer is
/// never the original file and never a still.
async fn motion_action(state: &AppState, media: &Upload, opts: MotionOptions, ask: &Ask) -> Response {
    let (job_state, job_media) = (state.clone(), media.clone());
    let run = image_transforms()
        .run(
            move |signal| async move { part_a::motion_preview(&job_state, &job_media, opts, signal).await },
            RunOptions {
                weight: Some(MOTION_WEIGHT),
                timeout_ms: Some(MOTION_TIMEOUT_MS),
                signal: Some(ask.signal.clone()),
            },
        )
        .await;
    let result = match run {
        Ok(result) => result,
        Err(err) => return transform_refusal(&err).unwrap_or_else(|| motion_refusal(MotionRefusal::Unreadable)),
    };
    match result {
        MotionResult::Refused(reason) => motion_refusal(reason),
        MotionResult::Ok { buffer, frames } => {
            let mut res = Response::new(Body::from(buffer));
            put_headers(
                &mut res,
                [
                    ("Content-Type", "image/webp".to_string()),
                    ("X-Content-Type-Options", "nosniff".to_string()),
                    // Counted out of the bytes we are about to send, so a client can
                    // tell a moving tile from a still one without decoding it.
                    ("X-Motion-Frames", frames.to_string()),
                ]
                .into_iter()
                .chain(cache_headers_for(media)),
            );
            res
        }
    }
}

/// `motionRefusal(reason)`: deliberately NOT in the `X-Convert-Refused`
/// vocabulary; `no-store` because a row gains and loses the ability to produce
/// a preview.
fn motion_refusal(reason: MotionRefusal) -> Response {
    let text = match reason {
        MotionRefusal::NotAnimated => "This file does not move",
        MotionRefusal::TooLarge => "File too large for a motion preview",
        MotionRefusal::TooManyPixels => "Too many frames to preview",
        MotionRefusal::UnknownDuration => "This file does not say how long it is",
        MotionRefusal::Unreadable => "This file cannot be previewed",
        MotionRefusal::NoEngine => "Motion previews are not available on this server",
    };
    let mut res = Response::new(Body::from(text));
    *res.status_mut() = status(if reason == MotionRefusal::TooLarge { 413 } else { 415 });
    put_headers(
        &mut res,
        [
            ("Content-Type", "text/plain; charset=utf-8".to_string()),
            ("X-Motion-Refused", reason.as_str().to_string()),
            no_store(),
        ],
    );
    res
}

// ── Timeline ────────────────────────────────────────────────────────────────

/// `timelineTrackAction`: the metadata track, at a THUMBNAIL's weight and with
/// a short deadline, so the cheap half never queues behind the expensive one.
async fn timeline_track_action(state: &AppState, media: &Upload, ask: &Ask) -> Response {
    let (job_state, job_media) = (state.clone(), media.clone());
    let run = image_transforms()
        .run(
            move |signal| async move { part_a::timeline_track(&job_state, &job_media, signal).await },
            RunOptions {
                weight: Some(TIMELINE_TRACK_WEIGHT),
                timeout_ms: Some(TIMELINE_TRACK_TIMEOUT_MS),
                signal: Some(ask.signal.clone()),
            },
        )
        .await;
    match run {
        Err(err) => transform_refusal(&err).unwrap_or_else(|| timeline_refusal(TimelineRefusal::Unreadable)),
        Ok(TimelineTrack::Refused(reason)) => timeline_refusal(reason),
        Ok(TimelineTrack::Ok { vtt }) => {
            let mut res = Response::new(Body::from(vtt));
            put_headers(
                &mut res,
                [
                    ("Content-Type", "text/vtt; charset=utf-8".to_string()),
                    ("X-Content-Type-Options", "nosniff".to_string()),
                ]
                .into_iter()
                .chain(derived_cache_headers(media)),
            );
            res
        }
    }
}

/// `timelineSheetAction`: a conversion's weight and deadline. Never falls
/// through to the file.
async fn timeline_sheet_action(state: &AppState, media: &Upload, ask: &Ask) -> Response {
    let (job_state, job_media) = (state.clone(), media.clone());
    let run = image_transforms()
        .run(
            move |signal| async move { part_a::timeline_sheet(&job_state, &job_media, signal).await },
            RunOptions {
                weight: Some(TIMELINE_WEIGHT),
                timeout_ms: Some(TIMELINE_TIMEOUT_MS),
                signal: Some(ask.signal.clone()),
            },
        )
        .await;
    match run {
        Err(err) => transform_refusal(&err).unwrap_or_else(|| timeline_refusal(TimelineRefusal::Unreadable)),
        Ok(TimelineSheet::Refused(reason)) => timeline_refusal(reason),
        Ok(TimelineSheet::Ok { buffer }) => {
            let mut res = Response::new(Body::from(buffer));
            put_headers(
                &mut res,
                [("Content-Type", "image/jpeg".to_string()), ("X-Content-Type-Options", "nosniff".to_string())]
                    .into_iter()
                    .chain(derived_cache_headers(media)),
            );
            res
        }
    }
}

/// `derivedCacheHeaders(media)`: a day, revalidated, for an artefact whose
/// SHAPE is decided by our code; the restricted case is untouched.
fn derived_cache_headers(media: &Upload) -> indexmap::IndexMap<&'static str, String> {
    let mut headers = cache_headers_for(media);
    if headers.get("Cache-Control").is_some_and(|v| v.starts_with("public")) {
        headers.insert("Cache-Control", "public, max-age=86400, must-revalidate".to_string());
    }
    headers
}

/// `timelineRefusal(reason)`
fn timeline_refusal(reason: TimelineRefusal) -> Response {
    let text = match reason {
        TimelineRefusal::NotVideo => "This file has no timeline to preview",
        TimelineRefusal::TooLarge => "File too large for a timeline preview",
        TimelineRefusal::TooShort => "This file is too short for a timeline preview",
        TimelineRefusal::TooManyPixels => "Frames too large to preview",
        TimelineRefusal::UnknownDuration => "This file does not say how long it is",
        TimelineRefusal::Unreadable => "This file cannot be previewed",
        TimelineRefusal::NoEngine => "Timeline previews are not available on this server",
    };
    let mut res = Response::new(Body::from(text));
    *res.status_mut() = status(if reason == TimelineRefusal::TooLarge { 413 } else { 415 });
    put_headers(
        &mut res,
        [
            ("Content-Type", "text/plain; charset=utf-8".to_string()),
            ("X-Timeline-Refused", reason.as_str().to_string()),
            no_store(),
        ],
    );
    res
}

// ── Convert ─────────────────────────────────────────────────────────────────

/// MIME + extension a target produces, without running it.
fn target_output(to: ConvertTarget) -> (&'static str, &'static str) {
    match to {
        ConvertTarget::Pdf => ("application/pdf", "pdf"),
        ConvertTarget::Png => ("image/png", "png"),
        ConvertTarget::Jpg => ("image/jpeg", "jpg"),
        ConvertTarget::Webp => ("image/webp", "webp"),
        ConvertTarget::Avif => ("image/avif", "avif"),
    }
}

/// `media.filename.replace(/\.[^.]+$/, "") || media.code`
fn base_name(media: &Upload) -> String {
    let name = &media.filename;
    let stripped = match name.rfind('.') {
        Some(dot) if dot + 1 < name.len() => &name[..dot],
        _ => name.as_str(),
    };
    if stripped.is_empty() { media.code.clone() } else { stripped.to_string() }
}

fn converted_response(buffer: Vec<u8>, mime: &str, ext: &str, base: &str) -> Response {
    let mut res = Response::new(Body::from(buffer));
    put_headers(
        &mut res,
        [
            ("Content-Type", mime.to_string()),
            (
                "Content-Disposition",
                format!("attachment; filename*=UTF-8''{}", encode_uri_component(&format!("{base}.{ext}"))),
            ),
            ("X-Content-Type-Options", "nosniff".to_string()),
            // The derived bytes are cached on OUR disk; the response still must not
            // enter a shared cache, because the file behind it can be unpublished
            // or deleted at any moment.
            no_store(),
        ],
    );
    res
}

/// `imageRefusal(status, reason, extra)`: in the same vocabulary the audio/video
/// engine uses; the token is what messages/{en,pl}/files.json translates.
fn image_refusal(code: u16, reason: &str, extra: &[(&'static str, String)]) -> Response {
    let text = match reason {
        "wrong-target" => "This file cannot be converted to that format",
        "too-large" => "File too large to convert",
        "unreadable" => "This format cannot be converted",
        "too-many-pixels" => "Image too large to convert to this format",
        other => other,
    };
    let mut res = Response::new(Body::from(text.to_string()));
    *res.status_mut() = status(code);
    put_headers(
        &mut res,
        [("Content-Type", "text/plain; charset=utf-8".to_string()), ("X-Convert-Refused", reason.to_string())]
            .into_iter()
            .chain(extra.iter().cloned())
            .chain([no_store()]),
    );
    res
}

/// "Yes, this would work." The answer to `?check=1`; carries no bytes.
fn convert_ok() -> Response {
    let mut res = Response::new(Body::empty());
    *res.status_mut() = StatusCode::NO_CONTENT;
    put_headers(&mut res, [no_store()]);
    res
}

fn conversion_failed() -> Response {
    plain_text(StatusCode::INTERNAL_SERVER_ERROR, "Conversion failed")
}

/// `convertAction(req, media, privileged)`: "Download as…" - the one endpoint
/// on this route that spends real CPU on demand for anybody holding a link.
async fn convert_action(state: &AppState, media: &Upload, privileged: bool, ask: &Ask) -> AppResult<Response> {
    let to = ask.query.get("to");
    // "Would this work?" - asked by the menu before it starts a download. Every
    // gate below runs for a check; the work does not, and no view is spent.
    let check = ask.query.get("check") == Some("1");
    let size = AvSourceSize { width: media.width.map(f64::from), height: media.height.map(f64::from) };
    let frames = media.frames.map(f64::from);

    // ffmpeg first, because an ANIMATION is claimed by both engines and they share
    // target names.
    if av_option_for(Some(&media.mime_type), Some(&media.ext), frames, to, Some(&size)).is_some() {
        let db = state.db.clone();
        let admit_media = media.clone();
        let handled = part_a::av_convert(
            state,
            AvConvertRequest {
                media,
                privileged,
                to: to.map(str::to_string),
                offset_seconds: seconds(ask.query.get("t")),
                clip_seconds: seconds(ask.query.get("d")),
                signal: ask.signal.clone(),
                dry_run: check,
                // No `isPreviewFetch` exemption: a conversion is always a
                // deliberate download.
                admit: if privileged {
                    None
                } else {
                    Some(Box::new(move || {
                        Box::pin(async move { consume_view(&db, &admit_media, true).await.unwrap_or(false) })
                    }))
                },
            },
        )
        .await;
        match handled {
            Ok(Some(res)) => return Ok(res),
            Ok(None) => {}
            Err(err) => return Ok(transform_refusal(&err).unwrap_or_else(conversion_failed)),
        }
    }

    // The admission test IS the menu.
    let Some(option) = convert_option_for(Some(&media.mime_type), Some(&media.ext), frames, to) else {
        return Ok(image_refusal(415, "wrong-target", &[]));
    };
    if media.size as u64 > MAX_TRANSFORM_BYTES {
        return Ok(image_refusal(413, "too-large", &[("limit", MAX_TRANSFORM_BYTES.to_string())]));
    }

    // A check that the ROW can answer, with no read and no decode.
    if check
        && let (Some(w), Some(h), Some(frames)) =
            (media.width.filter(|w| *w != 0), media.height.filter(|h| *h != 0), media.frames)
    {
        let frame_pixels = w as u64 * h as u64;
        let budget =
            part_a::conversion_budget(frames > 1, frame_pixels, frame_pixels * frames.max(0) as u64, option.to);
        return Ok(if budget.ok { convert_ok() } else { image_refusal(413, "too-many-pixels", &[]) });
    }

    // A conversion is a download, and a view-capped file pays for it. Skipped for
    // a check, which must not be able to burn a one-time link.
    if !check && !privileged && !consume_view(&state.db, media, true).await? {
        return Ok(not_found());
    }

    let variant = format!("{}{}", option.to.as_str(), if option.motion == Some(Motion::Keeps) { "-anim" } else { "" });
    let cacheable = may_cache_derived(media.into());
    let base = base_name(media);
    let (out_mime, out_ext) = target_output(option.to);
    let media_id = media.id.to_string();

    // A hash we already have means a cache hit costs nothing at all.
    if cacheable && let Some(sum) = media.checksum.as_deref() {
        let key = derived_key(&media_id, sum, &variant, out_ext);
        if let Some(hit) = part_a::read_derived(state, &key).await {
            return Ok(if check { convert_ok() } else { converted_response(hit.to_vec(), out_mime, out_ext, &base) });
        }
    }

    let (job_state, job_media) = (state.clone(), media.clone());
    let to = option.to;
    let run = image_transforms()
        .run(
            move |signal| async move {
                let state = job_state;
                let media = job_media;
                let media_id = media.id.to_string();
                let src: Bytes = state.storage.get()?.read_buffer(&media.storage_key).await?;
                let hash = media.checksum.clone().unwrap_or_else(|| content_hash(&src));
                if cacheable && media.checksum.is_none() {
                    let key = derived_key(&media_id, &hash, &variant, out_ext);
                    if let Some(hit) = part_a::read_derived(&state, &key).await {
                        remember(&state, &media, Some(&hash), None);
                        return Ok(if check {
                            convert_ok()
                        } else {
                            converted_response(hit.to_vec(), out_mime, out_ext, &base)
                        });
                    }
                }
                // Can libvips open this at all, BEFORE committing to the work?
                let Some(probe) = probe_image(src).await else {
                    return Ok(image_refusal(415, "unreadable", &[]));
                };
                remember(&state, &media, Some(&hash), if probe.cover_only { None } else { Some(probe.pages as i32) });
                let budget = part_a::conversion_budget(probe.animated, probe.frame_pixels, probe.total_pixels, to);
                if !budget.ok {
                    return Ok(image_refusal(413, "too-many-pixels", &[]));
                }
                if check {
                    return Ok(convert_ok());
                }
                let converted =
                    part_a::convert_image_buffer(probe.input, to, budget.animated, CONVERT_TIMEOUT_MS, signal).await?;
                if cacheable {
                    let key = derived_key(&media_id, &hash, &variant, &converted.ext);
                    part_a::write_derived(&state, &key, Bytes::from(converted.buffer.clone())).await;
                }
                Ok(converted_response(converted.buffer, &converted.mime, &converted.ext, &base))
            },
            RunOptions {
                weight: Some(CONVERT_WEIGHT),
                timeout_ms: Some(CONVERT_TIMEOUT_MS),
                signal: Some(ask.signal.clone()),
            },
        )
        .await;
    Ok(match run {
        Ok(res) => res,
        Err(err) => transform_refusal(&err).unwrap_or_else(conversion_failed),
    })
}

// ── Render ──────────────────────────────────────────────────────────────────

fn not_renderable() -> Response {
    let mut res = plain_text(StatusCode::NOT_FOUND, "Not found");
    put_headers(&mut res, [no_store()]);
    res
}

/// `renderSandboxed(req, userCode, mediaCode)`: serve an uploaded HTML/SVG
/// document for real, in isolation. Every step is a gate, cheapest first:
/// a frame load only; a valid ticket minted by the preview page; every page
/// gate re-checked against the row the ticket names; the MIME allowlist and
/// the size ceiling; a response CSP that puts the document in an opaque origin.
async fn render_sandboxed(
    state: &AppState,
    session: &RequestSession,
    user_code: &str,
    media_code: &str,
    ask: &Ask,
) -> AppResult<Response> {
    // The sandboxing CSP comes from the next.config rule, matched against the RAW
    // path, while the router hands this handler the DECODED segment: refusing
    // anything not spelled literally keeps the two in step.
    if !ask.raw_path.ends_with("/render") {
        return Ok(not_renderable());
    }
    let secret = &state.config.auth.secret;
    let admission =
        render_admission(secret, joined_header(&ask.headers, "sec-fetch-dest").as_deref(), ask.query.get("t"));
    let media_id = match admission {
        RenderAdmission::Allow { media_id } => media_id,
        // A human who followed the link gets the preview page, where the toggle
        // is - only for a positively identified top-level load.
        RenderAdmission::NotAFrame => {
            // Both segments come from the (decoded) path, so they are re-encoded:
            // a header value must not be able to carry a line break out of a URL.
            let back = format!("/{}/{}", encode_uri_component(user_code), encode_uri_component(media_code));
            let mut res = Response::new(Body::empty());
            *res.status_mut() = StatusCode::FOUND;
            if let Ok(v) = HeaderValue::from_str(&back) {
                res.headers_mut().insert(header::LOCATION, v);
            }
            put_headers(&mut res, [no_store()]);
            return Ok(res);
        }
        RenderAdmission::UnknownDest | RenderAdmission::BadTicket => {
            let mut res = plain_text(StatusCode::FORBIDDEN, "Preview link is invalid or expired");
            put_headers(&mut res, [no_store()]);
            return Ok(res);
        }
    };

    // `eq(uploads.id, ...)` with a string that is not a uuid makes Postgres
    // throw; a forged ticket cannot get here (the HMAC), but the TypeScript
    // server would answer such an id with a 500 too.
    let id: Uuid = media_id.parse().map_err(|e| anyhow::anyhow!("invalid input syntax for type uuid: {e}"))?;
    let sql = format!(
        r#"select {} from {} where ("uploads"."id" = $1 and "uploads"."deleted_at" is null) limit 1"#,
        Upload::COLUMNS,
        Upload::TABLE
    );
    let Some(media) = sqlx::query_as::<_, Upload>(&sql).bind(id).fetch_optional(&state.db).await? else {
        return Ok(not_renderable());
    };
    // The ticket names the file; the URL must agree.
    if Some(media.code.as_str()) != code_from_media_code(media_code).as_deref() {
        return Ok(not_renderable());
    }
    // ...and the tag must belong to the owner, current or retired.
    if !tag_belongs_to_owner(state, user_code, media.owner_id.as_deref()).await? {
        return Ok(not_renderable());
    }
    let privileged = match media.owner_id.as_deref() {
        Some(owner) => viewer_relation(&state.db, &ask.headers, session, owner).await?.privileged,
        None => false,
    };
    if !media.published && !privileged {
        return Ok(not_renderable());
    }
    if media.expires_at.is_some_and(|e| e.millis() <= crate::db::types::JsDate::now().millis()) {
        return Ok(not_renderable());
    }
    // Strictly greater, not >=: the preview page that issued this ticket already
    // spent one view, so on the last permitted view the counter has ALREADY
    // reached the cap by the time the toggle can be clicked.
    if media.max_views.is_some_and(|max| media.views > max) {
        return Ok(not_renderable());
    }
    // Password: the ticket is not a substitute for the unlock cookie.
    if media.password_hash.as_deref().is_some_and(|h| !h.is_empty()) && !privileged && !unlocked(state, ask, &media) {
        return Ok(plain_text(StatusCode::UNAUTHORIZED, "Password required"));
    }
    // Not upstream: nor for the file vault's grant (docs/ENCRYPTION.md).
    let owner_vault = match media.owner_id.as_deref() {
        Some(owner) => crate::vault::store::load(&state.db, owner).await?.map(|v| v.state),
        None => None,
    };
    let keyed = match media.owner_id.as_deref() {
        Some(owner) => {
            crate::vault::access::open_files(state, &ask.headers, owner, owner_vault.as_deref(), &[&media]).await?
        }
        None if media.is_encrypted() => None,
        None => Some(state.clone()),
    };
    let Some(keyed) = keyed else {
        let mut res = plain_text(StatusCode::UNAUTHORIZED, "Files are encrypted: unlock required");
        put_headers(&mut res, [no_store()]);
        return Ok(res);
    };
    let state = &keyed;

    // `rowMimeType`, because this handler looks the row up by id: a paste
    // declares its type through its language.
    let mime = row_mime_type(Some(&media.kind), media.language.as_deref(), &media.mime_type);
    let Some(shape) = sandbox_shape(Some(&mime), Some(&media.ext)) else {
        return Ok(not_renderable());
    };
    let limit = if shape == SandboxShape::Docx { MAX_DOCX_BYTES } else { MAX_RENDER_BYTES };
    if media.size > limit {
        return Ok(plain_text(StatusCode::PAYLOAD_TOO_LARGE, "Too large to preview"));
    }

    // A document is CONVERTED rather than served, then travels through exactly
    // the same sandbox as an uploaded page.
    let body = if is_generated(shape) {
        match render_document(state, &media).await {
            Ok(html) => Body::from(html),
            Err(res) => return Ok(*res),
        }
    } else {
        let Ok(storage) = state.storage.get() else {
            return Ok(not_renderable());
        };
        match storage.read_stream(&media.storage_key, None).await {
            Ok(stream) => Body::from_stream(stream),
            // Row without bytes behind it: a 404 is the honest answer, not a 500.
            Err(_) => return Ok(not_renderable()),
        }
    };
    let mut res = Response::new(body);
    put_headers(
        &mut res,
        [
            ("Content-Type", render_content_type(shape).to_string()),
            ("Content-Security-Policy", render_csp(shape)),
            ("X-Content-Type-Options", "nosniff".to_string()),
            ("Content-Disposition", "inline".to_string()),
            ("Referrer-Policy", "no-referrer".to_string()),
            ("Cache-Control", "private, no-store".to_string()),
            // The response differs by how it was requested and by who is asking.
            ("Vary", "Sec-Fetch-Dest, Cookie".to_string()),
        ],
    );
    Ok(res)
}

/// `renderDocument(media)`: a .docx converted to HTML for the sandbox, through
/// the shared limiter at thumbnail weight.
async fn render_document(state: &AppState, media: &Upload) -> Result<String, Box<Response>> {
    let (job_state, key) = (state.clone(), media.storage_key.clone());
    let run = image_transforms()
        .run(
            move |_signal| async move {
                let buf = job_state.storage.get()?.read_buffer(&key).await?;
                part_a::docx_to_html(buf).await
            },
            RunOptions { weight: Some(THUMBNAIL_WEIGHT), timeout_ms: Some(DOCX_TIMEOUT_MS), signal: None },
        )
        .await;
    match run {
        Err(err) => Err(Box::new(
            transform_refusal(&err).unwrap_or_else(|| plain_text(StatusCode::INTERNAL_SERVER_ERROR, "Preview failed")),
        )),
        Ok(DocxResult::Refused { reason }) => {
            let code = if reason == "too-large" || reason == "too-big" { 413 } else { 415 };
            let mut res = plain_text(status(code), &format!("Cannot preview this document ({reason})"));
            put_headers(&mut res, [("X-Preview-Refused", reason), no_store()]);
            Err(Box::new(res))
        }
        Ok(DocxResult::Ok { html }) => Ok(html),
    }
}

/// `tagBelongsToOwner(tag, ownerId)`: the owner's current tag or one they
/// retired. An upload with no owner is addressed by nobody's tag.
async fn tag_belongs_to_owner(state: &AppState, tag: &str, owner_id: Option<&str>) -> anyhow::Result<bool> {
    let Some(owner_id) = owner_id else {
        return Ok(false);
    };
    let owner: Option<Option<String>> =
        sqlx::query_scalar(r#"select "user_code" from "user" where "user"."id" = $1 limit 1"#)
            .bind(owner_id)
            .fetch_optional(&state.db)
            .await?;
    let Some(current) = owner else {
        return Ok(false);
    };
    if current.as_deref() == Some(tag) {
        return Ok(true);
    }
    let alias: Option<String> = sqlx::query_scalar(
        r#"select "user_id" from "user_code_aliases" where "user_code_aliases"."code" = $1 limit 1"#,
    )
    .bind(tag)
    .fetch_optional(&state.db)
    .await?;
    Ok(alias.as_deref() == Some(owner_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_sizes_snap_up_to_a_bounded_set() {
        assert_eq!(snap_thumb_size(None), None);
        assert_eq!(snap_thumb_size(Some("")), None);
        assert_eq!(snap_thumb_size(Some("0")), None);
        assert_eq!(snap_thumb_size(Some("-5")), None);
        assert_eq!(snap_thumb_size(Some("abc")), None);
        assert_eq!(snap_thumb_size(Some("1")), Some(64));
        assert_eq!(snap_thumb_size(Some("64")), Some(64));
        assert_eq!(snap_thumb_size(Some("65")), Some(128));
        assert_eq!(snap_thumb_size(Some("700")), Some(800));
        assert_eq!(snap_thumb_size(Some("99999")), Some(1200));
        assert_eq!(snap_thumb_size(Some("0x40")), Some(64));
        assert_eq!(snap_thumb_size(Some(" 200 ")), Some(256));
    }

    #[test]
    fn only_the_opening_request_is_initial() {
        assert!(is_initial_request(None));
        assert!(is_initial_request(Some("")));
        assert!(is_initial_request(Some("bytes=0-")));
        assert!(is_initial_request(Some(" bytes=0-4095")));
        assert!(!is_initial_request(Some("bytes=1-")));
        assert!(!is_initial_request(Some("bytes=-500")));
    }

    #[test]
    fn previews_are_named_by_parameters_or_destination() {
        let q = |s: &str| Query::parse(Some(s));
        assert!(is_preview_fetch(&q("w=1"), None));
        assert!(is_preview_fetch(&q("h="), None));
        assert!(is_preview_fetch(&q("motion=1"), None));
        assert!(!is_preview_fetch(&q("motion=yes"), None));
        assert!(is_preview_fetch(&q(""), Some("image")));
        assert!(is_preview_fetch(&q(""), Some("video")));
        assert!(!is_preview_fetch(&q(""), Some("document")));
        assert!(!is_preview_fetch(&q(""), Some("iframe")));
    }

    #[test]
    fn seconds_are_narrow() {
        assert_eq!(seconds(Some("1.5")), Some(1.5));
        assert_eq!(seconds(Some("86400")), Some(86_400.0));
        assert_eq!(seconds(Some("86401")), None);
        assert_eq!(seconds(Some("-1")), None);
        assert_eq!(seconds(Some("1e300")), None);
        assert_eq!(seconds(Some("NaN")), None);
        assert_eq!(seconds(Some("")), None);
    }

    #[test]
    fn actions_are_named_exactly() {
        assert_eq!(action_kind("raw"), ActionKind::Raw);
        assert_eq!(action_kind("raw.png"), ActionKind::Raw);
        assert_eq!(action_kind("raw."), ActionKind::Raw);
        assert_eq!(action_kind("download"), ActionKind::Download);
        assert_eq!(action_kind("timeline.vtt"), ActionKind::Timeline);
        assert_eq!(action_kind("timeline.jpg"), ActionKind::Timeline);
        assert_eq!(action_kind("convert"), ActionKind::Convert);
        assert_eq!(action_kind("Raw"), ActionKind::Other);
        assert_eq!(action_kind("poster"), ActionKind::Other);
    }
}

//! QA mode (PORT.md section 4): fixtures that stand in for a server, and headless
//! renders of every phase-1 screen for `--qa-shot` and the snapshot tests. Nothing here
//! opens a window or talks to a network.

use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone, Utc};
use egui::{Vec2, vec2};
use image::{Rgba, RgbaImage};
use modlight_api::{AccountInfo, DuplicateScan, FolderList, Quota, Upload, UploadPage};
use modlight_core::account::{AccountStore, StoredAccount};
use modlight_core::config::AppConfig;
use modlight_core::culture::{Culture, set_thread_culture, set_thread_utc};
use modlight_core::l10n::{Language, set_thread_language};
use modlight_core::theme::ThemeOptions;
use serde_json::json;

use crate::app::{MAIN_SIZE, Mode, ModlightApp};
use crate::state::{AppState, Env, Tab};

/// Every screen `--qa-shot` and the snapshot tests can draw.
pub const SCREENS: &[&str] = &[
    "my-space",
    "my-space-panel",
    "my-space-folder",
    "my-space-trash",
    "my-space-details",
    "my-space-menu",
    "my-space-empty",
    "my-space-error",
    "my-space-loading",
    "my-space-pager",
    "signed-out",
    "toasts",
    "tab-upload",
    "tab-create",
    "tab-tools",
    "sign-in",
    "sign-in-signed",
    "prompt",
    "confirm",
    "folder-delete",
    "link-settings",
    "duplicates",
    "notification",
    "settings-profile",
    "settings-theme",
    "settings-general",
    "tab-upload-busy",
    "paste",
    "paste-languages",
    "text-upload",
    "url-upload",
    "shorten-url",
    "clipboard-upload",
    "clipboard-upload-text",
    "drop-window",
    "before-upload",
    "after-upload",
    "large-file",
    "multi-upload",
    "file-exists",
    "message-box",
    "notification-upload",
    "tab-create-problem",
    "capture-menu",
    "region-capture",
    "region-capture-hover",
    "region-capture-annotate",
    "region-capture-light",
    "recording-toolbar",
    "recording-toolbar-compact",
    "after-capture",
    "after-capture-recording",
    "auto-capture",
    "quick-task-menu",
    "quick-task-editor",
    "scrolling-capture",
    "my-space-wide",
    // Phase 3, media (`qa_media::SCREENS`).
    "tab-download",
    "tab-download-busy",
    "tab-download-missing",
    "download-youtube",
    "components",
    "settings-components",
    "update-available",
    "update-download",
    // Phase 4, the image editor (`qa_editor::SCREENS`).
    "image-editor",
    "image-editor-start",
    "image-editor-annotate",
    // Phase 3, settings (`qa_settings::SCREENS`).
    "settings-capture-region",
    "settings-capture",
    "settings-image",
    "settings-image-effects",
    "settings-thumbnail",
    "settings-file-naming",
    "settings-recorder",
    "settings-ocr",
    "settings-clipboard-upload",
    "settings-notifications",
    "settings-tools",
    "settings-actions",
    "settings-watch-folders",
    "settings-task-advanced",
    "settings-action-editor",
    "settings-watch-folder-editor",
    "settings-notification-buttons",
    "settings-integration",
    "settings-paths",
    "settings-backup",
    "settings-main-window",
    "settings-copy-formats",
    "settings-copy-format-editor",
    "settings-upload-limits",
    "settings-history",
    "settings-print",
    "settings-proxy",
    "settings-advanced",
    "settings-hotkeys",
    "settings-hotkeys-capturing",
    "settings-hotkeys-reset",
    "settings-hotkeys-disabled",
    "hotkey-task-settings",
    "hotkey-task-folder",
    "hotkey-task-capture",
    "tab-server",
    "tab-server-problem",
    "tab-server-loading",
    "about",
    "debug-log",
    "debug-log-upload",
    "first-run",
    "editor-selector",
    "ffmpeg-options",
    "ffmpeg-options-video",
    "ffmpeg-options-audio",
    "ffmpeg-options-advanced",
    // Phase 5, tools (`tools::qa::SCREENS`).
];

/// How a screen is drawn: skin, light or dark, language, size, scale.
#[derive(Clone, Debug)]
pub struct Look {
    pub skin: String,
    pub dark: bool,
    pub language: Language,
    pub size: Option<Vec2>,
    pub scale: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            skin: "default".into(),
            dark: true,
            language: Language::English,
            size: None,
            scale: 1.0,
        }
    }
}

/// The clock every fixture is frozen at.
pub fn frozen_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap()
}

fn culture_for(language: Language) -> Culture {
    match language {
        Language::English => Culture::english(),
        Language::Polish => Culture::polish(),
    }
}

/// A state with no server behind it: fixture account, settings in memory.
pub fn state(look: &Look, signed_in: bool) -> AppState {
    set_thread_language(Some(look.language));
    set_thread_culture(Some(culture_for(look.language)));
    set_thread_utc(true);
    let env = Env {
        profile: std::env::temp_dir().join("modlight-qa-shot"),
        qa: true,
        frozen_now: Some(frozen_now()),
        machine: "QA Mac".into(),
        system_language: match look.language {
            Language::English => "en".into(),
            Language::Polish => "pl".into(),
        },
        system_locale: match look.language {
            Language::English => "en_US".into(),
            Language::Polish => "pl_PL".into(),
        },
        reduce_motion: true,
        system_dark: Some(look.dark),
    };
    let mut config = AppConfig::in_memory();
    let options = ThemeOptions {
        skin: look.skin.clone(),
        skin_confirmed: look.skin.clone(),
        theme: if look.dark {
            "Dark".into()
        } else {
            "Light".into()
        },
        use_system_theme: false,
        ..ThemeOptions::default()
    };
    config.set_theme_options(&options);
    let account = if signed_in {
        AccountStore::in_memory(
            StoredAccount {
                host: "https://files.example.com".into(),
                tag: "alex".into(),
                name: "Alex Rivera".into(),
                email: "alex@example.com".into(),
                avatar: String::new(),
                signed_in_at: Some(frozen_now() - Duration::days(3)),
            },
            "qa-token-not-a-secret",
        )
    } else {
        AccountStore::in_memory(
            StoredAccount {
                host: "https://files.example.com".into(),
                ..StoredAccount::default()
            },
            "",
        )
    };
    let mut state = AppState::new(env, config, account);
    state.options = options;
    state.refresh_theme(None);
    state
}

fn upload(
    id: &str,
    name: &str,
    kind: &str,
    mime: &str,
    size: i64,
    folder: Option<&str>,
    minutes: i64,
) -> Upload {
    let ext = name.rsplit('.').next().unwrap_or("");
    let code = format!("q{id}");
    let thumb = matches!(kind, "image" | "video")
        .then(|| format!("https://files.example.com/t/{code}.webp"));
    let motion = (kind == "video" || ext == "gif")
        .then(|| format!("https://files.example.com/m/{code}.webp"));
    serde_json::from_value(json!({
        "id": id,
        "code": code,
        "filename": name,
        "ext": ext,
        "mimeType": mime,
        "kind": kind,
        "size": size,
        "width": if kind == "image" { json!(1600) } else { json!(null) },
        "height": if kind == "image" { json!(900) } else { json!(null) },
        "published": false,
        "views": 0,
        "downloads": 0,
        "createdAt": (frozen_now() - Duration::minutes(minutes)).to_rfc3339(),
        "folderId": folder,
        "url": format!("https://files.example.com/alex/{code}"),
        "rawUrl": format!("https://files.example.com/alex/{code}.{ext}"),
        "thumbUrl": thumb,
        "motionThumbUrl": motion,
        "copyUrl": format!("https://files.example.com/alex/{code}"),
    }))
    .expect("a fixture row")
}

fn folders() -> FolderList {
    serde_json::from_value(json!({
        "rootValue": "root",
        "unfiled": { "fileCount": 0, "size": 0 },
        "folders": [
            { "id": "f-design", "name": "Design", "slug": "design", "parentId": null, "isPublic": false, "fileCount": 4, "size": 489_984, "sortOrder": 0 },
            { "id": "f-docs", "name": "Documents", "slug": "documents", "parentId": null, "isPublic": false, "fileCount": 2, "size": 60_928, "sortOrder": 1 },
            { "id": "f-rec", "name": "Recordings", "slug": "recordings", "parentId": null, "isPublic": true, "fileCount": 1, "size": 5_452_595, "sortOrder": 2 },
            { "id": "f-shots", "name": "Screenshots", "slug": "screenshots", "parentId": null, "isPublic": false, "fileCount": 2, "size": 51_302, "sortOrder": 3 },
            { "id": "f-logos", "name": "Logos", "slug": "logos", "parentId": "f-design", "isPublic": false, "fileCount": 0, "size": 0, "sortOrder": 0 }
        ]
    }))
    .expect("fixture folders")
}

/// The library of the upstream screenshot: nine files over four folders.
fn files() -> Vec<Upload> {
    vec![
        upload(
            "1",
            "image-pipeline.py",
            "document",
            "text/x-python",
            1_126,
            Some("f-docs"),
            22,
        ),
        upload(
            "2",
            "quarterly-report.pdf",
            "document",
            "application/pdf",
            59_802,
            Some("f-docs"),
            22,
        ),
        upload(
            "3",
            "fractal-zoom.mp4",
            "video",
            "video/mp4",
            5_452_595,
            Some("f-rec"),
            22,
        ),
        upload(
            "4",
            "weekly-uploads-chart.png",
            "image",
            "image/png",
            20_787,
            Some("f-shots"),
            22,
        ),
        upload(
            "5",
            "deploy-terminal.png",
            "image",
            "image/png",
            30_515,
            Some("f-shots"),
            22,
        ),
        upload(
            "6",
            "loader-animation.gif",
            "image",
            "image/gif",
            60_006,
            Some("f-design"),
            22,
        ),
        upload(
            "7",
            "brand-palette.png",
            "image",
            "image/png",
            10_444,
            Some("f-design"),
            22,
        ),
        upload(
            "8",
            "mountain-sunset.jpg",
            "image",
            "image/jpeg",
            52_224,
            Some("f-design"),
            22,
        ),
        upload(
            "9",
            "aurora-wallpaper.png",
            "image",
            "image/png",
            367_308,
            Some("f-design"),
            23,
        ),
    ]
}

/// A picture for each fixture image, drawn here so nothing is copied from anywhere.
fn picture(id: &str) -> Option<RgbaImage> {
    let (w, h) = (288u32, 204u32);
    let mut image = RgbaImage::new(w, h);
    let lerp = |a: u8, b: u8, t: f32| (a as f32 + (b as f32 - a as f32) * t) as u8;
    match id {
        "3" => {
            for (x, y, p) in image.enumerate_pixels_mut() {
                let fx = x as f32 / w as f32 - 0.5;
                let fy = y as f32 / h as f32 - 0.6;
                let r = (fx * fx + fy * fy).sqrt();
                let a = fy.atan2(fx);
                *p = Rgba([
                    (128.0 + 127.0 * (a * 3.0).sin()) as u8,
                    (128.0 + 127.0 * (r * 18.0).cos()) as u8,
                    (128.0 + 127.0 * (a * 2.0 + r * 9.0).sin()) as u8,
                    255,
                ]);
            }
        }
        "4" => {
            for p in image.pixels_mut() {
                *p = Rgba([16, 18, 34, 255]);
            }
            let bars = [0.25, 0.4, 0.55, 0.6, 0.5, 0.75, 0.85, 0.95];
            for (i, height) in bars.iter().enumerate() {
                let x0 = 8 + i as u32 * 35;
                let top = h - (height * (h - 20) as f32) as u32;
                let color = if i == 7 {
                    Rgba([110, 220, 140, 255])
                } else {
                    Rgba([110, 110, 240, 255])
                };
                for x in x0..(x0 + 24).min(w) {
                    for y in top..h {
                        image.put_pixel(x, y, color);
                    }
                }
            }
        }
        "5" => {
            for p in image.pixels_mut() {
                *p = Rgba([14, 16, 24, 255]);
            }
            for line in 0..9u32 {
                let len = 60 + (line * 37) % 160;
                for x in 6..(6 + len).min(w) {
                    for y in (10 + line * 14)..(12 + line * 14) {
                        image.put_pixel(x, y, Rgba([170, 175, 190, 255]));
                    }
                }
            }
        }
        "6" => {
            for p in image.pixels_mut() {
                *p = Rgba([18, 18, 40, 255]);
            }
            for i in 0..10 {
                let a = i as f32 / 10.0 * std::f32::consts::TAU;
                let cx = 144.0 + 70.0 * a.cos();
                let cy = 100.0 + 70.0 * a.sin();
                let radius = 4.0 + i as f32 * 1.6;
                for (x, y, p) in image.enumerate_pixels_mut() {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    if d < radius {
                        *p = Rgba([lerp(60, 200, i as f32 / 10.0), 80, 240, 255]);
                    }
                }
            }
        }
        "7" => {
            let colors = [
                [245, 166, 94],
                [232, 85, 98],
                [250, 250, 250],
                [30, 34, 48],
                [250, 250, 250],
            ];
            for (x, _, p) in image.enumerate_pixels_mut() {
                let band = (x * colors.len() as u32 / w) as usize;
                let c = colors[band];
                *p = Rgba([c[0], c[1], c[2], 255]);
            }
        }
        "8" => {
            for (x, y, p) in image.enumerate_pixels_mut() {
                let t = y as f32 / h as f32;
                let ridge =
                    120.0 + 20.0 * ((x as f32) / 30.0).sin() + 10.0 * ((x as f32) / 11.0).cos();
                *p = if (y as f32) > ridge {
                    Rgba([lerp(120, 70, t), lerp(80, 50, t), lerp(130, 90, t), 255])
                } else {
                    Rgba([lerp(240, 200, t), lerp(170, 120, t), lerp(140, 150, t), 255])
                };
                let d = ((x as f32 - 96.0).powi(2) + (y as f32 - 50.0).powi(2)).sqrt();
                if d < 22.0 {
                    *p = Rgba([250, 230, 170, 255]);
                }
            }
        }
        "9" => {
            for (x, y, p) in image.enumerate_pixels_mut() {
                let t = y as f32 / h as f32;
                let s = x as f32 / w as f32;
                *p = Rgba([
                    lerp(90, 40, t),
                    lerp(60, 20, t + s * 0.2),
                    lerp(220, 90, t),
                    255,
                ]);
            }
        }
        _ => return None,
    }
    Some(image)
}

fn into_color_image(image: &RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

/// Fill My space as a loaded listing would, pictures included.
pub fn load_library(state: &mut AppState, ctx: &egui::Context, rows: Vec<Upload>, total: i64) {
    state.space.has_account = state.account.is_signed_in();
    state.space.identity = state
        .credential()
        .map(|c| format!("{}\n{}", c.host, c.token))
        .unwrap_or_default();
    state.space.apply_folders(folders());
    let page = UploadPage {
        page: 1,
        per_page: state.space.page_size,
        total,
        root_value: Some("root".into()),
        items: Some(rows),
    };
    state.space.apply_page(page, 1);
    for entry in &state.space.files {
        if let Some(image) = picture(&entry.file.id) {
            let texture = ctx.load_texture(
                format!("qa-thumb-{}", entry.file.id),
                into_color_image(&image),
                egui::TextureOptions::LINEAR,
            );
            state.media.thumbs.insert(entry.file.id.clone(), texture);
        }
    }
    state.space.restored_last_folder = true;
    state.space.restored_page_size = true;
}

fn account_info() -> AccountInfo {
    AccountInfo {
        tag: Some("alex".into()),
        name: Some("Alex Rivera".into()),
        email: Some("alex@example.com".into()),
        avatar: None,
        host: Some("https://files.example.com".into()),
        quota: Some(Quota {
            used: 6_081_741,
            max: Some(10 * 1024 * 1024 * 1024),
            remaining: None,
            unlimited: false,
        }),
        default_folder_id: None,
        hide_uploads: false,
        copy_raw: false,
        role: Some("admin".into()),
    }
}

/// The storage chip as `/api/me` would set it.
fn fill_strip(state: &mut AppState) {
    let info = account_info();
    state.admin.record("https://files.example.com", &info);
    if let Some(credential) = state.credential() {
        state
            .strip
            .show(format!("{}\n{}", credential.host, credential.token), &info);
    }
}

fn duplicates_fixture() -> DuplicateScan {
    serde_json::from_value(json!({
        "groups": [
            {
                "checksum": "a1", "copies": 3, "size": 52224, "bytes": 156672, "reclaimable": 104448, "dismissed": false,
                "files": [
                    { "id": "8", "code": "q8", "filename": "mountain-sunset.jpg", "ext": "jpg", "size": 52224, "createdAt": "2026-09-01T10:00:00Z", "folderName": "Design" },
                    { "id": "d2", "code": "qd2", "filename": "mountain-sunset (1).jpg", "ext": "jpg", "size": 52224, "createdAt": "2026-09-14T08:30:00Z", "folderName": null },
                    { "id": "d3", "code": "qd3", "filename": "sunset-copy.jpg", "ext": "jpg", "size": 52224, "createdAt": "2026-10-02T19:12:00Z", "folderName": "Screenshots" }
                ]
            },
            {
                "checksum": "b2", "copies": 2, "size": 59802, "bytes": 119604, "reclaimable": 59802, "dismissed": false,
                "files": [
                    { "id": "2", "code": "q2", "filename": "quarterly-report.pdf", "ext": "pdf", "size": 59802, "createdAt": "2026-08-20T09:00:00Z", "folderName": "Documents" },
                    { "id": "d5", "code": "qd5", "filename": "quarterly-report-final.pdf", "ext": "pdf", "size": 59802, "createdAt": "2026-08-21T09:00:00Z", "folderName": "Documents" }
                ]
            }
        ],
        "totalGroups": 2, "totalFiles": 5, "reclaimable": 164250, "dismissedGroups": 0,
        "active": { "groups": 2, "redundant": 3, "reclaimable": 164250 },
        "truncated": false, "unchecksummed": 1, "due": true,
        "thresholds": { "redundant": 3, "bytes": 104857600 }
    }))
    .expect("fixture duplicates")
}

/// Set the state up for `screen`. Returns the window size the screen wants.
pub fn prepare(screen: &str, state: &mut AppState, ctx: &egui::Context) -> Vec2 {
    let signed = state.account.is_signed_in();
    if signed {
        fill_strip(state);
    }
    if let Some(size) = crate::qa_uploads::prepare(screen, state, picture("8")) {
        return size;
    }
    if let Some(size) = crate::qa_capture::prepare(screen, state) {
        return size;
    }
    if let Some(size) = crate::qa_media::prepare(screen, state) {
        return size;
    }
    if let Some(size) = crate::qa_settings::prepare(screen, state) {
        return size;
    }
    if let Some(size) = crate::qa_editor::prepare(screen, state) {
        return size;
    }
    if let Some(size) = crate::tools::qa::prepare(screen, state, ctx) {
        return size;
    }
    match screen {
        "my-space" | "toasts" | "my-space-wide" => {
            load_library(state, ctx, files(), 9);
            if screen == "toasts" {
                use modlight_core::l10n::{Arg, format, s, t, uploaders};
                state.time = 0.0;
                state.toast_upload(
                    &format(t(s::MainWindow_ToastUploadedTo), &[Arg::from("Design")]),
                    "mountain-sunset.jpg",
                    "https://files.example.com/alex/q8",
                    false,
                );
                state.notice(
                    modlight_core::icons::FOLDER_INPUT,
                    &format(
                        t(s::MySpace_MovedToast),
                        &[Arg::from(2usize), Arg::from("Screenshots")],
                    ),
                    "",
                    false,
                );
                state.toast_upload(
                    t(s::MainWindow_ToastFailed),
                    "huge-video.mov",
                    t(uploaders::Modlight_File_too_large),
                    true,
                );
            }
        }
        "my-space-panel" => {
            load_library(state, ctx, files(), 9);
            state.library_panel = true;
            state.panel_moved_at = f64::NEG_INFINITY;
        }
        "my-space-folder" => {
            state.space.apply_folders(folders());
            state.space.current_folder = Some("f-design".into());
            let rows: Vec<Upload> = files()
                .into_iter()
                .filter(|f| f.folder_id.as_deref() == Some("f-design"))
                .collect();
            load_library(state, ctx, rows, 4);
            state.space.current_folder = Some("f-design".into());
            state.space.select_file("8");
        }
        "my-space-trash" => {
            state.space.trashed = true;
            let mut rows = files();
            rows.truncate(3);
            for row in &mut rows {
                row.url = None;
                row.copy_url = None;
            }
            load_library(state, ctx, rows, 3);
            state.space.trashed = true;
        }
        "my-space-details" => {
            load_library(state, ctx, files(), 9);
            state.space.select_file("8");
            state.details.open_on("8", 0.0, true);
            if let Some(texture) = state.media.thumbs.get("8").cloned() {
                state.media.preview = Some(("8".into(), texture));
            }
        }
        "my-space-menu" => {
            load_library(state, ctx, files(), 9);
            state.space.select_file("4");
        }
        "my-space-empty" => {
            state.space.has_account = true;
            state.space.loaded_once = true;
            state.space.restored_last_folder = true;
        }
        "my-space-error" => {
            state.space.has_account = true;
            state.space.loaded_once = true;
            state.space.set_error(
                true,
                &modlight_core::messages::unreachable(
                    "https://files.example.com",
                    "Connection refused (os error 61)",
                ),
            );
        }
        "my-space-loading" => {
            state.space.has_account = true;
            state.space.is_busy = true;
        }
        "my-space-pager" => {
            let mut rows = Vec::new();
            for i in 0..120 {
                let kinds = [
                    ("document", "application/pdf", "pdf"),
                    ("archive", "application/zip", "zip"),
                    ("audio", "audio/mpeg", "mp3"),
                    ("document", "text/plain", "txt"),
                ];
                let (kind, mime, ext) = kinds[i % kinds.len()];
                rows.push(upload(
                    &format!("p{i}"),
                    &format!("file-{:03}.{ext}", i + 121),
                    kind,
                    mime,
                    1000 + i as i64 * 731,
                    None,
                    60 + i as i64,
                ));
            }
            load_library(state, ctx, rows, 1250);
            state.space.page = 2;
            let (page, size, total) = (2, state.space.page_size, 1250);
            state.space.pager.update(page, size, total);
        }
        "tab-upload" => state.tab = Tab::Upload,
        "tab-create" => state.tab = Tab::Create,
        "tab-tools" => state.tab = Tab::Tools,
        "sign-in" => {
            let mut window = crate::signin::SignInWindow::new("https://files.example.com");
            window.show_manual_token = true;
            window.initialized = true;
            state.windows.sign_in = Some(window);
        }
        "sign-in-signed" => {
            let mut window = crate::signin::SignInWindow::new("https://files.example.com");
            window.initialized = true;
            window.show_account(&account_info(), "https://files.example.com");
            window.set_status(
                &modlight_core::messages::signed_in_as("Alex Rivera"),
                crate::signin::Tone::Success,
            );
            state.windows.sign_in = Some(window);
        }
        "prompt" => {
            state.windows.prompt = Some(crate::dialogs::prompt::Prompt::new(
                modlight_core::l10n::t(modlight_core::l10n::s::MySpace_NewSubfolder),
                &modlight_core::l10n::format(
                    modlight_core::l10n::t(modlight_core::l10n::s::MySpace_NewFolderInside),
                    &[modlight_core::l10n::Arg::from("Design")],
                ),
                modlight_core::l10n::t(modlight_core::l10n::s::MySpace_FolderNameLabel),
                modlight_core::l10n::t(modlight_core::l10n::s::MySpace_Create),
                "Logos",
                "",
                120,
                crate::dialogs::prompt::PromptPurpose::NewFolder {
                    parent: Some("f-design".into()),
                },
            ));
        }
        "confirm" => {
            let question = modlight_core::l10n::format(
                modlight_core::l10n::t(modlight_core::l10n::s::MySpace_DeleteConfirm),
                &[modlight_core::l10n::Arg::from("mountain-sunset.jpg")],
            );
            state.windows.confirm = Some(crate::dialogs::confirm::Confirm::new(
                modlight_core::l10n::t(modlight_core::l10n::s::MySpace_DeleteTitle),
                &question,
                crate::dialogs::confirm::ConfirmAction::Trash(vec!["8".into()]),
            ));
        }
        "folder-delete" => {
            state.windows.folder_delete = Some(crate::dialogs::folder_delete::FolderDelete::new(
                "f-design", "Design", 4, 1, 0.0,
            ));
        }
        "link-settings" => {
            load_library(state, ctx, files(), 9);
            if let Some(file) = state.space.file("8") {
                state.windows.link_settings =
                    Some(crate::dialogs::link_settings::LinkSettings::new(file));
            }
        }
        "duplicates" => {
            let mut window = crate::dialogs::duplicates::Duplicates::default();
            window.apply_scan(duplicates_fixture(), false);
            state.windows.duplicates = Some(window);
        }
        "notification" => {
            let settings = state.config.toast();
            state.windows.notification = Some(crate::notification::Notification::tip(
                modlight_core::l10n::t(modlight_core::l10n::s::ShareXIsMinimizedToTheSystemTray),
                "Modlight",
                Some(8000.0),
                &settings,
                0.0,
            ));
        }
        "settings-profile" | "settings-theme" | "settings-general" => {
            let section = match screen {
                "settings-theme" => "application.theme",
                "settings-general" => "application.general",
                _ => "account.profile",
            };
            let mut window = crate::settings::SettingsWindow::new(section, &state.options.skin);
            window.account.info = Some(account_info());
            window.account.host = "https://files.example.com".into();
            state.windows.settings = Some(window);
        }
        _ => {}
    }
    match screen {
        "sign-in" | "sign-in-signed" => vec2(crate::signin::WIDTH, 420.0),
        "prompt" => vec2(crate::dialogs::prompt::WIDTH, 260.0),
        "confirm" => vec2(crate::dialogs::confirm::WIDTH, 160.0),
        "folder-delete" => vec2(crate::dialogs::folder_delete::WIDTH, 360.0),
        "link-settings" => vec2(crate::dialogs::link_settings::WIDTH, 520.0),
        "duplicates" => vec2(
            crate::dialogs::duplicates::WIDTH,
            crate::dialogs::duplicates::HEIGHT,
        ),
        "notification" => vec2(428.0, 160.0),
        // The size of upstream's library screenshot, where every card is whole.
        "my-space-wide" => vec2(1486.0, 943.0),
        "settings-profile" | "settings-theme" | "settings-general" => {
            vec2(crate::settings::WIDTH, crate::settings::HEIGHT)
        }
        _ => MAIN_SIZE,
    }
}

/// Whether the screen is a whole main window (as opposed to one dialog).
pub fn is_main_window(screen: &str) -> bool {
    !crate::qa_uploads::is_dialog(screen)
        && !crate::qa_capture::is_dialog(screen)
        && !crate::qa_media::is_dialog(screen)
        && !crate::qa_settings::is_dialog(screen)
        && !crate::qa_editor::is_dialog(screen)
        && !crate::tools::qa::is_dialog(screen)
        && !matches!(
            screen,
            "sign-in"
                | "sign-in-signed"
                | "prompt"
                | "confirm"
                | "folder-delete"
                | "link-settings"
                | "duplicates"
                | "notification"
                | "settings-profile"
                | "settings-theme"
                | "settings-general"
        )
}

/// The app of one QA screen: the main window, or a dialog drawn alone.
pub struct ShotApp {
    pub app: ModlightApp,
    pub screen: String,
    /// The height the dialog asked for, to size the next render.
    pub wanted: Option<f32>,
}

impl eframe::App for ShotApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        set_thread_language(Some(self.app.state.language));
        if is_main_window(&self.screen) {
            self.app.frame(ui);
            crate::qa_capture::draw_over_main(&self.screen, &ctx, &self.app.state);
            if self.screen == "my-space-menu" {
                // Popups do not open headlessly: the card's menu is drawn where it opens.
                let entries = crate::myspace::menus::file(&self.app.state, "4");
                let at = egui::pos2(586.0, 488.0);
                egui::Area::new(egui::Id::new("qa-menu"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(at)
                    .show(&ctx, |ui| {
                        crate::menus::panel(ui, &self.app.state.colors, at, &entries);
                    });
            }
            return;
        }
        let state = &mut self.app.state;
        state.time = ctx.input(|i| i.time);
        if let Some(wanted) = crate::qa_uploads::draw(&self.screen, ui, state) {
            self.wanted = wanted;
            return;
        }
        if let Some(wanted) = crate::qa_capture::draw(&self.screen, ui, state) {
            self.wanted = wanted;
            return;
        }
        if let Some(wanted) = crate::qa_media::draw(&self.screen, ui, state) {
            self.wanted = wanted;
            return;
        }
        if let Some(wanted) = crate::qa_settings::draw(&self.screen, ui, state) {
            self.wanted = wanted;
            return;
        }
        if let Some(wanted) = crate::qa_editor::draw(&self.screen, ui, state) {
            self.wanted = wanted;
            return;
        }
        if let Some(wanted) = crate::tools::qa::draw(&self.screen, ui, state) {
            self.wanted = wanted;
            return;
        }
        let colors = state.colors.clone();
        let wanted = match self.screen.as_str() {
            "sign-in" | "sign-in-signed" => {
                let _ = crate::signin::ui(ui, state);
                state.windows.sign_in.as_ref().map(|w| w.content_height)
            }
            "prompt" => state.windows.prompt.as_mut().map(|w| {
                let _ = w.ui(ui, &colors);
                w.content_height
            }),
            "confirm" => state.windows.confirm.as_mut().map(|w| {
                let _ = w.ui(ui, &colors);
                w.content_height
            }),
            "folder-delete" => state.windows.folder_delete.as_mut().map(|w| {
                let _ = w.ui(ui, &colors, 0.0);
                w.content_height
            }),
            "link-settings" => {
                let now = state.env.now();
                state.windows.link_settings.as_mut().map(|w| {
                    let _ = w.ui(ui, &colors, now);
                    w.content_height
                })
            }
            "duplicates" => {
                let now = state.env.now();
                if let Some(w) = state.windows.duplicates.as_mut() {
                    let _ = w.ui(ui, &colors, now);
                }
                None
            }
            "notification" => state.windows.notification.as_mut().map(|n| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, colors.background_main());
                let _ = n.ui(ui, &colors, 1.0);
                n.window_size().y
            }),
            _ => {
                let _ = crate::settings::ui(ui, state);
                None
            }
        };
        self.wanted = wanted;
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Draw `screen` headlessly and return the picture.
pub fn render(screen: &str, look: &Look) -> Result<RgbaImage, String> {
    if !SCREENS.contains(&screen) {
        return Err(format!(
            "unknown screen '{screen}'; known: {}",
            SCREENS.join(", ")
        ));
    }
    set_thread_language(Some(look.language));
    set_thread_culture(Some(culture_for(look.language)));
    set_thread_utc(true);
    // Random name codes (`%ra{10}`) draw the same characters in every render.
    modlight_core::names::helpers::set_thread_seed(Some(20261020));
    let signed_in = !matches!(screen, "signed-out" | "sign-in");
    let mut size = MAIN_SIZE;
    let mut harness = egui_kittest::Harness::builder()
        .with_size(look.size.unwrap_or(MAIN_SIZE))
        .with_pixels_per_point(look.scale)
        .with_theme(if look.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        })
        .with_max_steps(64)
        .wgpu()
        .build_eframe(|cc| {
            let state = state(look, signed_in);
            let mut app = ModlightApp::new(cc, state, Mode::Shot);
            size = prepare(screen, &mut app.state, &cc.egui_ctx);
            ShotApp {
                app,
                screen: screen.to_string(),
                wanted: None,
            }
        });
    let size = look.size.unwrap_or(size);
    harness.set_size(size);
    harness.run_steps(3);
    if let Some(wanted) = harness.state().wanted {
        let height = wanted.max(60.0).round();
        if (height - size.y).abs() > 0.5 {
            harness.set_size(vec2(size.x, height));
            harness.run_steps(3);
        }
    }
    harness.render()
}

/// Where `--qa-shot` writes when given a bare name.
pub fn default_output(screen: &str) -> PathBuf {
    PathBuf::from(format!("{screen}.png"))
}

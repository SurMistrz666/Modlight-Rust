//! QA fixtures for phase 3's media screens: the Download tab (empty, busy, tools
//! missing, the YouTube panel), the components window and settings page, and the
//! app-update windows. Nothing here runs a process, opens a web view or touches a
//! network.

use std::path::PathBuf;

use egui::{Ui, Vec2, vec2};
use modlight_core::components::install::{InstallProgress, Stage};
use modlight_core::download::item::State;
use modlight_core::download::run::{FormatResult, Outcome, Progress};
use modlight_core::download::tool::ToolPaths;
use modlight_core::download::{
    AudioFormat, Destination, DownloadFormat, DownloadLink, MediaKind, Quality, settings,
};
use modlight_core::l10n::{s, t};
use serde_json::json;

use crate::state::{AppState, Tab};

/// The screens of this module, in `qa::SCREENS` order.
pub const SCREENS: &[&str] = &[
    "tab-download",
    "tab-download-busy",
    "tab-download-missing",
    "download-youtube",
    "components",
    "settings-components",
    "update-available",
    "update-download",
];

const TOOLS: &str = "/Users/alex/Library/Application Support/Modlight/Tools";

/// A tool's file name in `Tools` (`.exe` on Windows).
fn program(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn tools(yt_dlp: bool, ffmpeg: bool) -> ToolPaths {
    let folder = PathBuf::from(crate::qa::fixture_path(TOOLS));
    ToolPaths {
        yt_dlp: yt_dlp.then(|| folder.join(program("yt-dlp"))),
        ffmpeg: ffmpeg.then(|| folder.join(program("ffmpeg"))),
        quickjs: None,
    }
}

fn manifest() -> modlight_core::update::ReleaseManifest {
    // Windows' release is upstream's installer, when the update check takes one.
    #[cfg(windows)]
    if let Ok(manifest) = modlight_core::update::parse_manifest(
        r#"{"name":"Modlight App","version":"0.8.0","url":"https://files.example.com/r/Modlight-0.8.0-setup.exe","filename":"Modlight-0.8.0-setup.exe","size":41200000,"sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824","notes":"Download tab and screen recording on Windows. Faster uploads of large files."}"#,
    ) {
        return manifest;
    }
    modlight_core::update::parse_manifest(
        r#"{"name":"Modlight App","version":"0.8.0","url":"https://files.example.com/r/Modlight-0.8.0.dmg","filename":"Modlight-0.8.0.dmg","size":41200000,"sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824","notes":"Download tab and screen recording on the Mac. Faster uploads of large files."}"#,
    )
    .expect("the fixture manifest")
}

fn link(url: &str) -> DownloadLink {
    DownloadLink::parse(url).expect("a fixture address")
}

/// Rows in every state the queue shows.
fn busy_queue(state: &mut AppState) {
    let movies = PathBuf::from(crate::qa::fixture_path("/Users/alex/Movies"));
    let queue = crate::download_tab::qa_queue(&mut state.downloads);
    let video = |q| DownloadFormat::new(MediaKind::Video, q, AudioFormat::M4a);
    let audio = DownloadFormat::new(MediaKind::Audio, Quality::Best, AudioFormat::Mp3);

    // Oldest first: they end up newest on top.
    let canceled = queue.enqueue(
        link("https://vimeo.com/76979871"),
        movies.clone(),
        false,
        Destination::Disk,
        video(Quality::Best),
        &crate::download_tab::video_label(Quality::Best),
    );
    let failed_update = queue.enqueue(
        link("https://www.facebook.com/watch/?v=1234567890"),
        movies.clone(),
        false,
        Destination::Disk,
        video(Quality::Best),
        &crate::download_tab::video_label(Quality::Best),
    );
    let failed_sign_in = queue.enqueue(
        link("https://www.youtube.com/watch?v=Tq92D6wQ1mg"),
        movies.clone(),
        false,
        Destination::Disk,
        video(Quality::H1080),
        &crate::download_tab::video_label(Quality::H1080),
    );
    let done = queue.enqueue(
        link("https://www.tiktok.com/@nasa/video/7404291234294299"),
        movies.clone(),
        false,
        Destination::Both,
        video(Quality::H1080),
        &crate::download_tab::video_label(Quality::H1080),
    );
    let paused = queue.enqueue(
        link("https://www.youtube.com/watch?v=aqz-KE-bpKQ"),
        movies.clone(),
        false,
        Destination::Disk,
        video(Quality::H2160),
        &crate::download_tab::video_label(Quality::H2160),
    );
    let merging = queue.enqueue(
        link("https://soundcloud.com/artist/track"),
        movies.clone(),
        false,
        Destination::Disk,
        audio,
        crate::download_tab::audio_label(AudioFormat::Mp3),
    );
    let running = queue.enqueue(
        link("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
        movies.clone(),
        false,
        Destination::Disk,
        video(Quality::H1080),
        &crate::download_tab::video_label(Quality::H1080),
    );
    let _waiting = queue.enqueue(
        link("https://x.com/nasa/status/1234567890123456789"),
        movies.clone(),
        false,
        Destination::Modlight,
        video(Quality::H720),
        &crate::download_tab::video_label(Quality::H720),
    );

    if let Some(item) = queue.get_mut(running) {
        item.start();
        item.named("Rick Astley - Never Gonna Give You Up (Official Video)");
        item.chose(&FormatResult {
            resolution: 1080,
            note: String::new(),
        });
        item.apply(
            &Progress {
                percent: 42.0,
                downloaded: 52_400_000,
                total: 124_800_000,
                speed: 6_300_000.0,
                eta: 11,
                is_working: false,
            },
            false,
        );
    }
    if let Some(item) = queue.get_mut(merging) {
        item.start();
        item.named("Night Drive (Extended Mix)");
        item.apply(
            &Progress {
                percent: 100.0,
                downloaded: 9_800_000,
                total: 9_800_000,
                speed: 0.0,
                eta: -1,
                is_working: true,
            },
            false,
        );
    }
    if let Some(item) = queue.get_mut(paused) {
        item.start();
        item.named("Big Buck Bunny 4K");
        item.chose(&FormatResult {
            resolution: 1440,
            note: modlight_core::l10n::format(
                t(s::Download_NoteQualityLower),
                &[2160.into(), 1440.into()],
            ),
        });
        item.apply(
            &Progress {
                percent: 18.0,
                downloaded: 240_000_000,
                total: 1_330_000_000,
                speed: 4_000_000.0,
                eta: 280,
                is_working: false,
            },
            false,
        );
        item.pause();
        item.finish(
            &Outcome {
                canceled: true,
                ..Outcome::default()
            },
            false,
        );
    }
    if let Some(item) = queue.get_mut(done) {
        item.start();
        item.chose(&FormatResult {
            resolution: 1080,
            note: String::new(),
        });
        // A finished row names its file; the fixture has none on disk.
        item.state = State::Completed;
        item.title = "Mars sunset [7404291234294299].mp4".into();
        item.detail = modlight_core::l10n::format(
            t(s::Download_SavedAndSent),
            &[crate::qa::fixture_path("/Users/alex/Movies").into()],
        );
        item.percent = 100.0;
        item.size_text = "18.4 MB".into();
        item.speed_text.clear();
        item.eta_text.clear();
        item.file_path = Some(movies.join("Mars sunset [7404291234294299].mp4"));
    }
    if let Some(item) = queue.get_mut(failed_sign_in) {
        item.start();
        item.finish(
            &Outcome {
                error: t(s::Download_ErrorSignIn).to_string(),
                needs_sign_in: true,
                ..Outcome::default()
            },
            false,
        );
    }
    if let Some(item) = queue.get_mut(failed_update) {
        item.start();
        let failure = modlight_core::download::error::describe(
            modlight_core::download::error::ErrorKind::SelfReportedBug,
            "ERROR: [facebook] 1234567890: Cannot parse data; please report this issue on https://github.com/yt-dlp/yt-dlp/issues",
            false,
            false,
            &modlight_core::download::error::VersionFacts {
                standing: modlight_core::download::error::Standing::Behind,
                installed: "2026.08.19".into(),
                latest: "2026.10.02".into(),
            },
        );
        item.finish(
            &Outcome {
                error: failure.message,
                needs_update: failure.needs_update,
                ..Outcome::default()
            },
            false,
        );
    }
    if let Some(item) = queue.get_mut(canceled) {
        item.cancel();
    }
}

/// Set up `screen`; `None` when it is not one of these. Returns the window size.
pub fn prepare(screen: &str, state: &mut AppState) -> Option<Vec2> {
    if !SCREENS.contains(&screen) {
        return None;
    }
    state.components.display_profile = Some(PathBuf::from(crate::qa::fixture_path(
        "/Users/alex/Library/Application Support/Modlight",
    )));
    let size = match screen {
        "tab-download" => {
            state.tab = Tab::Download;
            crate::download_tab::qa_fill(&mut state.downloads, tools(true, true), false);
            crate::app::MAIN_SIZE
        }
        "tab-download-busy" => {
            state.tab = Tab::Download;
            crate::download_tab::qa_fill(&mut state.downloads, tools(true, true), true);
            settings::set_folder(
                &mut state.config,
                &crate::qa::fixture_path("/Users/alex/Movies"),
            );
            settings::set_video_quality(&mut state.config, Quality::H1080);
            busy_queue(state);
            crate::app::MAIN_SIZE
        }
        "tab-download-missing" => {
            state.tab = Tab::Download;
            crate::download_tab::qa_fill(&mut state.downloads, tools(false, false), false);
            settings::set_media_kind(&mut state.config, MediaKind::Audio);
            settings::set_audio_format(&mut state.config, AudioFormat::M4a);
            settings::set_destination(&mut state.config, Destination::Both);
            settings::set_folder(
                &mut state.config,
                &crate::qa::fixture_path("/Users/alex/Music/Downloads"),
            );
            state.downloads.url_text = "not a link".into();
            state.downloads.url_error = t(s::Download_BadUrl).to_string();
            crate::app::MAIN_SIZE
        }
        "download-youtube" => {
            state.tab = Tab::Download;
            crate::download_tab::qa_fill(&mut state.downloads, tools(true, true), true);
            crate::download_tab::qa_open_panel(&mut state.downloads, t(s::Download_SignInDone));
            crate::app::MAIN_SIZE
        }
        "components" | "settings-components" => {
            state.components.started = true;
            state.components.qa_installed = Some(vec![
                modlight_core::components::YT_DLP,
                modlight_core::components::EXIFTOOL,
            ]);
            state.config.set(
                "ExternalComponentsLastUpdateCheck",
                json!("2026-10-18T09:30:00Z"),
            );
            if let Some(model) =
                modlight_core::components::find(modlight_core::components::BACKGROUND_MODEL)
            {
                modlight_core::components::settings::set_wanted(&mut state.config, model, false);
            }
            for row in &mut state.components.rows {
                match row.id {
                    modlight_core::components::FFMPEG => {
                        crate::components_window::qa_progress(
                            row,
                            InstallProgress {
                                stage: Stage::Downloading,
                                received: 11_400_000,
                                total: 27_185_234,
                                bytes_per_second: 3_200_000.0,
                            },
                        );
                    }
                    modlight_core::components::YT_DLP => {
                        row.installed_version = "2026.08.19".into();
                        row.version_asked = true;
                    }
                    modlight_core::components::EXIFTOOL => {
                        row.installed_version = "13.59".into();
                        row.version_asked = true;
                    }
                    _ => {}
                }
            }
            if let Some(yt) = modlight_core::components::find(modlight_core::components::YT_DLP) {
                modlight_core::components::updates::remember_latest(yt, "2026.10.02");
            }
            if screen == "components" {
                let feature = t(s::Components_FeatureDownload);
                state.components.window = Some(crate::components_window::ComponentsWindow {
                    required: Some(modlight_core::components::FFMPEG.into()),
                    headline: modlight_core::l10n::format(
                        t(s::Components_RequiredHeadline),
                        &[feature.into(), t(s::Components_FFmpeg).into()],
                    ),
                    then: Vec::new(),
                });
                vec2(
                    crate::components_window::WIDTH,
                    crate::components_window::HEIGHT,
                )
            } else {
                let mut window = crate::settings::SettingsWindow::new(
                    "application.components",
                    &state.options.skin,
                );
                window.account.host = "https://files.example.com".into();
                state.windows.settings = Some(window);
                vec2(crate::settings::WIDTH, crate::settings::HEIGHT)
            }
        }
        "update-available" => {
            state.updates.message = Some(manifest());
            vec2(crate::update_window::MESSAGE_WIDTH, 300.0)
        }
        "update-download" => {
            state.updates.downloader = Some(crate::update_window::qa_downloader(manifest(), false));
            vec2(
                crate::update_window::DOWNLOADER_WIDTH,
                crate::update_window::DOWNLOADER_HEIGHT,
            )
        }
        _ => return None,
    };
    Some(size)
}

/// Whether `screen` is a window of this module drawn alone (not the main window).
pub fn is_dialog(screen: &str) -> bool {
    matches!(
        screen,
        "components" | "settings-components" | "update-available" | "update-download"
    )
}

/// Draw a dialog screen alone into `ui`; `Some(height)` when the dialog sizes itself.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    let height = match screen {
        "components" => {
            let _ = crate::components_window::window_ui(ui, state);
            None
        }
        "settings-components" => {
            let _ = crate::settings::ui(ui, state);
            None
        }
        "update-available" => {
            let _ = crate::update_window::message_ui(ui, state);
            Some(state.updates.message_height)
        }
        "update-download" => {
            let _ = crate::update_window::downloader_ui(ui, state);
            None
        }
        _ => return None,
    };
    Some(height)
}

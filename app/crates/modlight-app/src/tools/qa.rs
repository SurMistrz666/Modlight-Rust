//! QA fixtures for phase 5's tool windows: each screen opens one window with fixture
//! data and draws it headlessly for `--qa-shot` and the snapshot tests. Nothing here
//! reads the screen, the clipboard, a model or a network.

use std::sync::Arc;

use egui::{Ui, Vec2, vec2};
use modlight_core::capture::geometry::IRect;
use modlight_tools::color::Color;
use modlight_tools::settings::{ColorPickerOptions, ScreenColorPickerOptions};

use crate::state::AppState;

/// The screens of this module, in `qa::SCREENS` order.
pub const SCREENS: &[&str] = &[
    "color-picker",
    "color-picker-standard",
    "screen-color-picker",
    "qr-code",
    "qr-code-empty",
    "image-viewer",
    "pin-startup",
    "pin-startup-error",
    "pin-options",
    "pinned-image",
    "image-combiner",
    "image-combiner-empty",
    "video-converter",
    "video-converter-encoding",
    "video-converter-empty",
    "metadata",
    "metadata-strip",
    "metadata-empty",
    "input-box",
    "output-box",
    "error-window",
    "ocr",
    "ocr-services",
    "ocr-error",
    "analyze-image",
    "analyze-image-empty",
    "analyze-image-error",
    "analyze-image-options",
    "analyze-image-options-gemini",
    "background-remover",
    "background-remover-source",
    "background-remover-empty",
    "history",
    "history-advanced",
    "history-empty",
    "history-edit",
    "history-stats",
    "history-delete",
    "image-history",
    "image-history-settings",
    "image-history-import",
    "print",
    "print-settings",
];

/// ExifTool's `-G -t` output for the fixture photo.
const METADATA_SAMPLE: &str = "ExifTool\tExifTool Version Number\t13.59\nFile\tFile Name\tmountain-sunset.jpg\nFile\tDirectory\t/Users/alex/Pictures\nFile\tFile Size\t52 kB\nFile\tFile Type\tJPEG\nFile\tMIME Type\timage/jpeg\nFile\tImage Width\t1600\nFile\tImage Height\t900\nEXIF\tMake\tApple\nEXIF\tCamera Model Name\tiPhone 15 Pro\nEXIF\tOrientation\tHorizontal (normal)\nEXIF\tDate/Time Original\t2026:09:14 19:42:07\nEXIF\tExposure Time\t1/120\nEXIF\tF Number\t1.8\nEXIF\tISO\t64\nXMP\tCreator Tool\tModlight https://files.example.com\nComposite\tImage Size\t1600x900\nComposite\tMegapixels\t1.4\n";

/// How the fixture files are shown (their real folder is a temporary one).
pub fn fixture_names() -> Vec<String> {
    [
        "mountain-sunset.png",
        "aurora-wallpaper.png",
        "brand-palette.png",
    ]
    .iter()
    .map(|n| crate::qa::fixture_path(&format!("/Users/alex/Pictures/{n}")))
    .collect()
}

/// Fixture pictures on disk (a temporary folder), made once.
pub fn fixture_files() -> Vec<std::path::PathBuf> {
    let folder = std::env::temp_dir().join("modlight-qa-tools");
    let _ = std::fs::create_dir_all(&folder);
    let mut out = Vec::new();
    for (name, w, h) in [
        ("mountain-sunset.png", 640u32, 400u32),
        ("aurora-wallpaper.png", 480, 400),
        ("brand-palette.png", 320, 200),
    ] {
        let path = folder.join(name);
        if !path.is_file() {
            let _ = modlight_tools::images::save(&sunset(w, h), &path);
        }
        out.push(path);
    }
    out
}

/// A picture to show: a sunset like the fixture library's, drawn here.
pub fn sunset(w: u32, h: u32) -> image::RgbaImage {
    let mut image = image::RgbaImage::new(w, h);
    let lerp = |a: f32, b: f32, t: f32| (a + (b - a) * t) as u8;
    for (x, y, p) in image.enumerate_pixels_mut() {
        let t = y as f32 / h as f32;
        let ridge = h as f32 * 0.6 + 30.0 * ((x as f32) / 50.0).sin();
        *p = if (y as f32) > ridge {
            image::Rgba([
                lerp(120.0, 70.0, t),
                lerp(80.0, 50.0, t),
                lerp(130.0, 90.0, t),
                255,
            ])
        } else {
            image::Rgba([
                lerp(240.0, 200.0, t),
                lerp(170.0, 120.0, t),
                lerp(140.0, 150.0, t),
                255,
            ])
        };
        let d = ((x as f32 - w as f32 * 0.3).powi(2) + (y as f32 - h as f32 * 0.25).powi(2)).sqrt();
        if d < h as f32 * 0.1 {
            *p = image::Rgba([250, 230, 170, 255]);
        }
    }
    image
}

/// A few recent colours.
fn recent() -> Vec<Color> {
    vec![
        Color::rgb(18, 140, 200),
        Color::rgb(255, 106, 0),
        Color::rgb(46, 204, 113),
        Color::rgb(155, 89, 182),
        Color::rgb(241, 196, 15),
        Color::rgb(52, 73, 94),
        Color::rgb(231, 76, 60),
        Color::WHITE,
        Color::BLACK,
    ]
}

/// Whether `screen` is one of these (drawn as a window of its own).
pub fn is_dialog(screen: &str) -> bool {
    SCREENS.contains(&screen) || super::image_effects::SCREENS.contains(&screen)
}

/// Set the state up for `screen`; the size the window wants.
pub fn prepare(screen: &str, state: &mut AppState, ctx: &egui::Context) -> Option<Vec2> {
    let _ = ctx;
    if !is_dialog(screen) {
        return None;
    }
    if let Some(size) = super::image_effects::qa_prepare(screen, state) {
        return Some(size);
    }
    use super::color_picker::{ColorPickerWindow, Purpose};
    match screen {
        "color-picker" | "color-picker-standard" => {
            let options = ColorPickerOptions {
                recent_colors_selected: screen == "color-picker",
            };
            state.tools.open_keyed(|key| {
                let mut window = ColorPickerWindow::new(
                    key,
                    Purpose::Tool,
                    Color::rgb(18, 140, 200),
                    options,
                    ScreenColorPickerOptions::default(),
                    recent(),
                );
                window.previous = Color::RED;
                window.has_previous = true;
                Box::new(window)
            });
            Some(vec2(super::color_picker::WIDTH, 526.0))
        }
        "screen-color-picker" => {
            let source = state.captures.source.clone();
            let screen_rect = IRect::new(0, 0, 800, 500);
            let image = source
                .capture_rect(source.screen_bounds(), false)
                .unwrap_or_else(|_| image::RgbaImage::new(800, 500));
            let mut screens = source.screens();
            screens.truncate(1);
            if let Some(first) = screens.first_mut() {
                first.bounds = screen_rect;
            }
            let desktop = super::screen_picker::Desktop {
                image: Arc::new(image),
                bounds: source.screen_bounds(),
                scale: 1.0,
                screens,
                pointer: (163, 123),
            };
            let options = ScreenColorPickerOptions {
                info_text: "#$HEX$nX: $x Y: $y".into(),
                ..ScreenColorPickerOptions::default()
            };
            state.tools.screen_picker = Some(super::screen_picker::ScreenPicker::new(
                super::screen_picker::PickTarget::ColorPicker(0),
                options,
                desktop,
            ));
            Some(vec2(800.0, 500.0))
        }
        "qr-code" | "qr-code-empty" => {
            let text =
                (screen == "qr-code").then(|| "https://files.example.com/alex/q8".to_string());
            state.tools.open_keyed(|key| {
                Box::new(super::qr::QrWindow::new(
                    key,
                    super::qr::QrOptions {
                        initial_text: text,
                        ..Default::default()
                    },
                ))
            });
            Some(vec2(940.0, 610.0))
        }
        "image-viewer" => {
            let folder = std::env::temp_dir().join("modlight-qa-image-viewer");
            let _ = std::fs::create_dir_all(&folder);
            for (name, w, h) in [
                ("aurora.png", 800, 500),
                ("mountain-sunset.png", 1200, 760),
                ("zebra.png", 600, 600),
            ] {
                let path = folder.join(name);
                if !path.is_file() {
                    let _ = modlight_tools::images::save(&sunset(w, h), &path);
                }
            }
            let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1040.0, 700.0));
            let mut window = super::image_viewer::ImageViewerWindow::new_on(screen);
            window.load_file(&folder.join("mountain-sunset.png"));
            state.tools.open_keyed(|key| {
                window.key = key;
                Box::new(window)
            });
            Some(vec2(1040.0, 700.0))
        }
        "pin-startup" | "pin-startup-error" => {
            let settings = state.default_task_settings();
            let error = screen == "pin-startup-error";
            state.tools.open_keyed(|key| {
                let mut window = super::pin::PinStartupWindow::new(key, settings);
                if error {
                    window.error = modlight_core::l10n::t(
                        modlight_core::l10n::tools::PinToScreenStartupViewModel_Clipboard_no_image,
                    )
                    .to_string();
                }
                Box::new(window)
            });
            Some(vec2(560.0, 230.0))
        }
        "pin-options" => {
            state.open_pin_options(None);
            Some(vec2(760.0, 560.0))
        }
        "pinned-image" => {
            let settings = state.default_task_settings();
            state.pin_picture(Arc::new(sunset(420, 260)), &settings);
            let pin = state.tools.pins.last_mut().expect("pinned");
            pin.show_toolbar = true;
            Some(pin.size)
        }
        "image-combiner" | "image-combiner-empty" => {
            let settings = state.default_task_settings();
            let files = if screen == "image-combiner" {
                fixture_files()
            } else {
                Vec::new()
            };
            let key = state.tools.open_keyed(|key| {
                Box::new(super::combiner::CombinerWindow::new(key, settings, files))
            });
            if let Some(window) = state
                .tools
                .window_mut::<super::combiner::CombinerWindow>(key)
            {
                window.options.space = 10;
                window.options.wrap_after = 2;
                window.selected = vec![1];
                window.current = Some(1);
                window.preview_now(ctx);
                window.qa_names = fixture_names();
            }
            Some(vec2(980.0, 650.0))
        }
        "video-converter" | "video-converter-encoding" | "video-converter-empty" => {
            let settings = state.default_task_settings();
            let input = (screen != "video-converter-empty")
                .then(|| crate::qa::fixture_path("/Users/alex/Movies/screen-recording.mov"));
            let key = state.tools.open_keyed(|key| {
                Box::new(super::video::VideoWindow::new(
                    key,
                    settings,
                    Some(std::path::PathBuf::from(crate::qa::fixture_path(
                        "/opt/homebrew/bin/ffmpeg",
                    ))),
                    input,
                ))
            });
            if screen == "video-converter-encoding"
                && let Some(window) = state.tools.window_mut::<super::video::VideoWindow>(key)
            {
                window
                    .conversion
                    .set_codec(modlight_tools::settings::ConverterVideoCodecs::Vp9);
                window.conversion.use_bitrate = true;
                window.encoding = true;
                window.progress = 42.0;
                window.status = modlight_core::l10n::t(
                    modlight_core::l10n::tools::VideoConverterViewModel_Converting,
                )
                .to_string();
            }
            Some(vec2(850.0, 475.0))
        }
        "metadata" | "metadata-strip" | "metadata-empty" => {
            let file = (screen != "metadata-empty")
                .then(|| fixture_files()[0].to_string_lossy().into_owned());
            let key = state
                .tools
                .open_keyed(|key| Box::new(super::metadata::MetadataWindow::new(key, file, None)));
            if let Some(window) = state
                .tools
                .window_mut::<super::metadata::MetadataWindow>(key)
            {
                window.started = true;
                if screen != "metadata-empty" {
                    window.qa_display = Some(crate::qa::fixture_path(
                        "/Users/alex/Pictures/mountain-sunset.jpg",
                    ));
                    // ExifTool on Windows writes the drive and forward slashes.
                    let sample = if cfg!(windows) {
                        METADATA_SAMPLE
                            .replace("\t/Users/alex/Pictures\n", "\tC:/Users/alex/Pictures\n")
                    } else {
                        METADATA_SAMPLE.to_string()
                    };
                    window.loaded(Ok(modlight_tools::metadata::parse(&sample)));
                }
                window.confirming_strip = screen == "metadata-strip";
            }
            Some(vec2(900.0, 650.0))
        }
        "input-box" => {
            state.open_input_box(
                "Image URL",
                Some("https://files.example.com/alex/q8.png"),
                None,
                None,
            );
            Some(vec2(520.0, 120.0))
        }
        "output-box" => {
            let text = "ffmpeg version 8.1 Copyright (c) 2000-2026 the FFmpeg developers\n  built with Apple clang version 17.0.0\nInput #0, avfoundation, from 'Capture screen 0':\n  Duration: N/A, start: 0.000000, bitrate: N/A\n  Stream #0:0: Video: rawvideo (UYVY / 0x59565955), uyvy422, 2880x1800, 30 fps\n[libx264 @ 0x7fa1] height not divisible by 2 (2880x1799)\nError initializing output stream 0:0 -- Error while opening encoder for output stream #0:0\nConversion failed!";
            state.open_output_box(
                text,
                modlight_core::l10n::t(modlight_core::l10n::helpers::FFmpegCLIManager_Error_title),
                true,
            );
            Some(vec2(900.0, 650.0))
        }
        "error-window" => {
            let details = "called `Option::unwrap()` on a `None` value\n   at crates/modlight-app/src/uploads.rs:812 (thread modlight-worker)\n\nModlight 0.7.7 Dev";
            state.open_error_window("called `Option::unwrap()` on a `None` value", details, true);
            Some(vec2(760.0, 560.0))
        }
        "ocr" | "ocr-services" | "ocr-error" => {
            let settings = state.default_task_settings();
            let options = modlight_tools::settings::OcrOptions::default();
            let languages = state.ocr_languages();
            let text = "Quarterly report 2026\nRevenue grew 18% over the third quarter, led by uploads from\nthe desktop app and the new sharing links.\n\nNext review: 14 November";
            let key = state.tools.open_keyed(|key| {
                Box::new(super::ocr::OcrWindow::new(
                    key,
                    settings,
                    options,
                    languages,
                    Arc::new(sunset(200, 100)),
                    super::ocr::Recognizer::Fixed(String::new()),
                    None,
                ))
            });
            if let Some(window) = state.tools.window_mut::<super::ocr::OcrWindow>(key) {
                window.qa_started();
                if screen == "ocr-error" {
                    window.error = modlight_core::l10n::format(
                        modlight_core::l10n::t(
                            modlight_core::l10n::tools::OCRHelper_Language_unavailable,
                        ),
                        &[modlight_core::l10n::Arg::from("Klingon")],
                    );
                } else {
                    window.result = text.to_string();
                }
                if screen == "ocr-services" {
                    window.editing_services = true;
                    window.editing = Some(1);
                }
            }
            Some(vec2(940.0, 600.0))
        }
        "analyze-image" | "analyze-image-empty" | "analyze-image-error" => {
            use super::analyze::{AnalyzeWindow, Network};
            let settings = state.default_task_settings();
            let mut options = modlight_tools::settings::AiOptions::default();
            if screen == "analyze-image" {
                options.openai_api_key = Some("sk-qa".into());
            }
            let file = (screen != "analyze-image-empty").then(|| fixture_files()[0].clone());
            let key = state.tools.open_keyed(|key| {
                Box::new(AnalyzeWindow::new(
                    key,
                    settings,
                    options,
                    file,
                    Network::Offline(Err(String::new())),
                ))
            });
            if let Some(window) = state.tools.window_mut::<AnalyzeWindow>(key) {
                window.qa_started();
                window.qa_display = Some("mountain-sunset.png".into());
                match screen {
                    "analyze-image" => {
                        window.preset = Some(1);
                        window.options.input =
                            modlight_tools::analyze::preset_prompts()[1].to_string();
                        window.result = "A wide landscape at sunset: a pale sun low in a peach sky above a ridge of violet hills.\n\nThe colours run from warm orange near the horizon to soft pink higher up; the hills are in shadow, with no people, buildings or text in the picture.".into();
                        window.elapsed = modlight_core::l10n::format(
                            modlight_core::l10n::t(
                                modlight_core::l10n::tools::AnalyzeImageViewModel_Time_ms,
                            ),
                            &[modlight_core::l10n::Arg::Int(2384)],
                        );
                    }
                    "analyze-image-error" => {
                        window.error = modlight_core::l10n::t(
                            modlight_core::l10n::tools::AnalyzeImageViewModel_Configure_API_key,
                        )
                        .to_string();
                    }
                    _ => {}
                }
            }
            Some(vec2(1000.0, 650.0))
        }
        "analyze-image-options" | "analyze-image-options-gemini" => {
            use super::analyze::{AnalyzeOptionsWindow, Network};
            let mut options = modlight_tools::settings::AiOptions {
                openai_api_key: Some("sk-qa-0123456789".into()),
                ..Default::default()
            };
            if screen == "analyze-image-options-gemini" {
                options.provider = modlight_tools::settings::AiProvider::Gemini;
            }
            let key = state.tools.open_keyed(|key| {
                Box::new(AnalyzeOptionsWindow::new(
                    key,
                    0,
                    super::jobs::OptionsHome::Defaults,
                    &options,
                    Network::Offline(Err(String::new())),
                ))
            });
            if screen == "analyze-image-options"
                && let Some(window) = state.tools.window_mut::<AnalyzeOptionsWindow>(key)
            {
                window.status = modlight_core::l10n::t(
                    modlight_core::l10n::tools::AnalyzeImageService_Connection_OK,
                )
                .to_string();
                window.status_success = true;
            }
            Some(vec2(650.0, 590.0))
        }
        "background-remover" | "background-remover-source" | "background-remover-empty" => {
            use super::background::{BackgroundWindow, Notice};
            let settings = state.default_task_settings();
            let models = if screen == "background-remover-empty" {
                Vec::new()
            } else {
                vec![modlight_tools::background::ModelFile {
                    path: crate::qa::fixture_path(
                        "/Users/alex/Library/Application Support/Modlight/Models/bria-rmbg-2.0.onnx",
                    )
                    .into(),
                    file_name: "bria-rmbg-2.0.onnx".into(),
                    size: 1_024_331_469,
                }]
            };
            let folder = state.models_folder();
            let key = state
                .tools
                .open_keyed(|key| Box::new(BackgroundWindow::new(key, &settings, folder, models)));
            if screen != "background-remover-empty"
                && let Some(window) = state.tools.window_mut::<BackgroundWindow>(key)
            {
                let picture = sunset(640, 400);
                window.set_source(picture.clone(), fixture_files()[0].clone());
                window.qa_display = Some(crate::qa::fixture_path(
                    "/Users/alex/Pictures/mountain-sunset.png",
                ));
                if screen == "background-remover" {
                    // The "subject": a soft ellipse around the sun and the ridge.
                    let (w, h) = picture.dimensions();
                    let mask: Vec<u8> = (0..h)
                        .flat_map(|y| (0..w).map(move |x| (x, y)))
                        .map(|(x, y)| {
                            let dx = (x as f32 - w as f32 * 0.45) / (w as f32 * 0.32);
                            let dy = (y as f32 - h as f32 * 0.5) / (h as f32 * 0.36);
                            let d = (dx * dx + dy * dy).sqrt();
                            ((1.15 - d) * 4.0).clamp(0.0, 1.0).mul_add(255.0, 0.0) as u8
                        })
                        .collect();
                    window.set_result(modlight_core::background_removal::apply_alpha_mask(
                        &picture, &mask,
                    ));
                    window.slider = 0.5;
                    window.notice = Some(Notice {
                        message: modlight_core::l10n::format(
                            modlight_core::l10n::t(
                                modlight_core::l10n::tools::BackgroundRemoverViewModel_Background_removed_ms,
                            ),
                            &[modlight_core::l10n::Arg::Int(1834)],
                        ),
                        icon: modlight_core::icons::ERASER,
                        until: f64::MAX,
                    });
                }
            }
            Some(vec2(1080.0, 720.0))
        }
        "history" | "history-advanced" | "history-empty" | "history-edit" | "history-stats"
        | "history-delete" => {
            use super::history::{Act, HistoryHost, Modal};
            use super::history_list::HistoryWindow;
            let settings = modlight_tools::settings::HistorySettings::default();
            let key = state
                .tools
                .open_keyed(|key| Box::new(HistoryWindow::new(key, None, settings)));
            let items = if screen == "history-empty" {
                Vec::new()
            } else {
                history_items()
            };
            super::with_window::<HistoryWindow, _>(state, key, |window, state| {
                window.qa_load(state, items);
                match screen {
                    "history-advanced" => {
                        window.advanced = true;
                        // The day the reference pictures were drawn, not this machine's
                        // today: every day draws the same fields.
                        let day = super::history_list::short_date(
                            chrono::NaiveDate::from_ymd_opt(2026, 10, 20).expect("a date"),
                        );
                        window.adv.from = day.clone();
                        window.adv.to = day;
                        window.adv.type_on = true;
                        window.adv.kind = window.types.iter().position(|t| t == "Image");
                        window.refilter(state);
                    }
                    "history-edit" => {
                        window.selected = vec![4];
                        super::history::run(window, state, Act::EditItem);
                    }
                    "history-stats" => {
                        window.data.modal = Some(Modal::Stats(modlight_tools::history::stats(
                            &window.data.items,
                        )));
                    }
                    "history-delete" => {
                        window.selected = vec![12, 11, 10];
                        super::history::run(window, state, Act::DeleteFile);
                    }
                    _ => {}
                }
            });
            Some(vec2(1300.0, 700.0))
        }
        "image-history" | "image-history-settings" | "image-history-import" => {
            use super::image_history::ImageHistoryWindow;
            let settings = modlight_tools::settings::ImageHistorySettings::default();
            let key = state
                .tools
                .open_keyed(|key| Box::new(ImageHistoryWindow::new(key, None, settings)));
            super::with_window::<ImageHistoryWindow, _>(state, key, |window, state| {
                window.qa_load(state, history_items());
                window.selected = vec![12];
                match screen {
                    "image-history-settings" => window.qa_open_settings(),
                    "image-history-import" => {
                        super::history::open_import(window, true);
                        if let Some(super::history::Modal::Import(form)) =
                            window.data.modal.as_mut()
                        {
                            form.folder =
                                crate::qa::fixture_path("/Users/alex/Pictures/Screenshots");
                            form.status = modlight_core::l10n::format(
                                modlight_core::l10n::t(modlight_core::l10n::history::ImageHistoryWindow_SuccessfullyImportedFilesFormat),
                                &[modlight_core::l10n::Arg::Int(1284)],
                            );
                        }
                    }
                    _ => {}
                }
            });
            Some(vec2(1100.0, 720.0))
        }
        "print" | "print-settings" => {
            let mut settings = modlight_tools::print::PrintSettings::default();
            if screen == "print-settings" {
                settings.auto_scale_image = false;
                settings.show_print_dialog = false;
                settings.margin = 20;
            }
            let image = Arc::new(sunset(640, 400));
            let preview_only = screen == "print-settings";
            state.tools.open_keyed(|key| {
                Box::new(super::print::PrintWindow::new(
                    key,
                    image,
                    settings,
                    preview_only,
                ))
            });
            Some(vec2(470.0, 330.0))
        }
        _ => None,
    }
}

/// History rows for the history windows' QA screens, oldest first (ids 1 to 12). The
/// three newest pictures are the fixture files; the rest point at files that are not
/// there.
pub fn history_items() -> Vec<modlight_core::history::HistoryItem> {
    use modlight_core::history::{HistoryItem, HistoryTags};
    use modlight_tools::history::at;
    let files = fixture_files();
    type Row<'a> = (&'a str, &'a str, &'a str, (i32, u32, u32, u32, u32));
    let rows: [Row; 12] = [
        ("Q3-report.pdf", "File", "Finder", (2025, 11, 3, 9, 12)),
        (
            "meeting-notes.txt",
            "Text",
            "TextEdit",
            (2025, 12, 1, 14, 40),
        ),
        (
            "onboarding.mp4",
            "File",
            "QuickTime Player",
            (2026, 1, 15, 10, 5),
        ),
        ("logo-draft.png", "Image", "Figma", (2026, 2, 2, 16, 22)),
        (
            "dashboard-dark.png",
            "Image",
            "Safari",
            (2026, 3, 9, 11, 48),
        ),
        ("snippet.rs", "Text", "Xcode", (2026, 4, 21, 18, 3)),
        ("bug-repro.gif", "Image", "Simulator", (2026, 5, 30, 13, 27)),
        ("invoice-0420.pdf", "File", "Preview", (2026, 6, 12, 8, 55)),
        ("roadmap.png", "Image", "Keynote", (2026, 8, 18, 15, 10)),
        ("brand-palette.png", "Image", "Figma", (2026, 9, 14, 19, 42)),
        (
            "aurora-wallpaper.png",
            "Image",
            "Photos",
            (2026, 10, 2, 7, 30),
        ),
        (
            "mountain-sunset.png",
            "Image",
            "Photos",
            (2026, 10, 19, 21, 4),
        ),
    ];
    rows.iter()
        .enumerate()
        .map(|(index, (name, kind, process, (y, mo, d, h, mi)))| {
            let id = index as i64 + 1;
            let file_path = match *name {
                "mountain-sunset.png" => files[0].to_string_lossy().into_owned(),
                "aurora-wallpaper.png" => files[1].to_string_lossy().into_owned(),
                "brand-palette.png" => files[2].to_string_lossy().into_owned(),
                other => crate::qa::fixture_path(&format!("/Users/alex/Pictures/Modlight/{other}")),
            };
            let mut tags = HistoryTags::new();
            tags.insert("WindowTitle", Some(format!("{name} - {process}")));
            tags.insert("ProcessName", Some(process.to_string()));
            let mut item = HistoryItem {
                id,
                file_name: name.to_string(),
                file_path,
                date_time: Some(at(*y, *mo, *d, *h, *mi)),
                kind: kind.to_string(),
                host: "Modlight".into(),
                url: format!(
                    "https://files.example.com/alex/{id:02}x{}",
                    &name[name.rfind('.').unwrap_or(0)..]
                ),
                tags: Some(tags),
                ..HistoryItem::default()
            };
            if *kind == "Image" {
                item.thumbnail_url = format!("https://files.example.com/alex/{id:02}x-thumb.jpg");
            }
            if matches!(id, 4 | 10) {
                item.set_favorite(true);
            }
            if id == 5 {
                item.set_tag("release");
                item.shortened_url = "https://mdl.ink/k3Q9".into();
                item.deletion_url = "https://files.example.com/delete/05x".into();
            }
            item
        })
        .collect()
}

/// Draw `screen`'s window; `Some(height)` for windows whose height follows the content.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    if !is_dialog(screen) {
        return None;
    }
    if screen == "pinned-image" {
        let colors = state.colors.clone();
        let now = state.time;
        if let Some(pin) = state.tools.pins.last_mut() {
            // The desktop behind a transparent window.
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(58, 74, 120));
            let _ = pin.ui(ui, &colors, now);
        }
        return Some(None);
    }
    if screen == "screen-color-picker" {
        let colors = state.colors.clone();
        if let Some(picker) = state.tools.screen_picker.as_mut()
            && let Some(first) = picker.desktop.screens.first().cloned()
        {
            picker.ui(ui, &colors, &first);
        }
        return Some(None);
    }
    let mut windows = std::mem::take(&mut state.tools.windows);
    let mut wanted = None;
    if let Some(hosted) = windows.first_mut() {
        let _ = hosted.window.ui(ui, state);
        if hosted.window.spec().fit_height {
            wanted = hosted.window.content_height();
        }
    }
    windows.append(&mut state.tools.windows);
    state.tools.windows = windows;
    Some(wanted)
}

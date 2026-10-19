//! Modlight App for macOS.
//!
//! ```text
//! Modlight                                  the app (settings in ~/Library/Application Support/Modlight)
//! Modlight --qa-profile <dir>               QA: settings and token in <dir>, no tray, no Keychain
//! Modlight --qa-shot <screen> <out.png>     QA: draw one screen headlessly and exit
//!          [--qa-lang en|pl] [--qa-skin <id>] [--qa-light] [--qa-size WxH] [--qa-scale N]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use egui::{ViewportBuilder, vec2};
use modlight_app::app::{MAIN_MIN_SIZE, MAIN_SIZE, Mode, ModlightApp};
use modlight_app::qa;
use modlight_app::state::{AppState, Env};
use modlight_core::account::{AccountStore, CredentialStore, FileCredentialStore};
use modlight_core::config::AppConfig;
use modlight_core::l10n::Language;

struct Args {
    qa_profile: Option<PathBuf>,
    shot: Option<(String, PathBuf)>,
    look: qa::Look,
}

fn usage() -> String {
    format!(
        "usage: Modlight [--qa-profile <dir>] [--qa-shot <screen> <out.png> [--qa-lang en|pl] [--qa-skin <id>] [--qa-light] [--qa-size WxH] [--qa-scale N]]\nscreens: {}",
        qa::SCREENS.join(", ")
    )
}

fn parse() -> Result<Args, String> {
    let mut args = Args {
        qa_profile: None,
        shot: None,
        look: qa::Look::default(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--qa-profile" => args.qa_profile = Some(it.next().ok_or_else(usage)?.into()),
            "--qa-shot" => {
                let screen = it.next().ok_or_else(usage)?;
                let out = it
                    .next()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| qa::default_output(&screen));
                args.shot = Some((screen, out));
            }
            "--qa-lang" => {
                args.look.language = match it.next().ok_or_else(usage)?.as_str() {
                    "pl" => Language::Polish,
                    "en" => Language::English,
                    other => return Err(format!("unknown language '{other}'\n{}", usage())),
                }
            }
            "--qa-skin" => args.look.skin = it.next().ok_or_else(usage)?,
            "--qa-light" => args.look.dark = false,
            "--qa-dark" => args.look.dark = true,
            "--qa-size" => {
                let text = it.next().ok_or_else(usage)?;
                let (w, h) = text.split_once('x').ok_or_else(usage)?;
                let w: f32 = w.parse().map_err(|_| usage())?;
                let h: f32 = h.parse().map_err(|_| usage())?;
                args.look.size = Some(vec2(w, h));
            }
            "--qa-scale" => {
                args.look.scale = it.next().ok_or_else(usage)?.parse().map_err(|_| usage())?
            }
            "-h" | "--help" => return Err(usage()),
            // macOS passes a process serial number to apps opened from Finder.
            other if other.starts_with("-psn_") => {}
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }
    Ok(args)
}

fn profile_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(modlight_core::APP_NAME)
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    if let Some((screen, out)) = &args.shot {
        return match qa::render(screen, &args.look)
            .and_then(|image| image.save(out).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                println!("{}", out.display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("qa-shot failed: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let qa_mode = args.qa_profile.is_some();
    let profile = args.qa_profile.clone().unwrap_or_else(profile_dir);
    if let Err(error) = std::fs::create_dir_all(&profile) {
        eprintln!(
            "the settings folder {} cannot be created: {error}",
            profile.display()
        );
        return ExitCode::FAILURE;
    }
    let config = AppConfig::load(&profile);
    let credentials: Box<dyn CredentialStore> = if qa_mode {
        Box::new(FileCredentialStore::new(profile.join("credentials")))
    } else {
        Box::new(modlight_macos::KeychainStore::for_app())
    };
    let account = AccountStore::load(&profile, credentials);
    let env = Env {
        profile,
        qa: qa_mode,
        frozen_now: None,
        machine: modlight_macos::system::machine_name(),
        system_language: modlight_macos::system::preferred_language(),
        system_locale: modlight_macos::system::system_locale(),
        reduce_motion: modlight_macos::system::reduce_motion(),
        system_dark: None,
    };
    let bounds = config.main_window();
    let state = AppState::new(env, config, account);

    let mut viewport = ViewportBuilder::default()
        .with_title(modlight_core::title())
        .with_inner_size(MAIN_SIZE)
        .with_min_inner_size(MAIN_MIN_SIZE)
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    if bounds.remember_size
        && let Some((w, h)) = bounds.size
    {
        viewport = viewport.with_inner_size(vec2(w as f32, h as f32).max(MAIN_MIN_SIZE));
    }
    if bounds.remember_position
        && let Some((x, y)) = bounds.position
    {
        viewport = viewport.with_position(egui::pos2(x as f32, y as f32));
    }
    if bounds.maximized {
        viewport = viewport.with_maximized(true);
    }
    if let Some(icon) = modlight_app::app::app_icon() {
        viewport = viewport.with_icon(egui::IconData {
            width: icon.width(),
            height: icon.height(),
            rgba: icon.into_raw(),
        });
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let mode = if qa_mode { Mode::Qa } else { Mode::Normal };
    match eframe::run_native(
        modlight_core::APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(ModlightApp::new(cc, state, mode)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Modlight could not start: {error}");
            ExitCode::FAILURE
        }
    }
}

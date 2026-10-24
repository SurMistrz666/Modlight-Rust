//! Modlight App for macOS and Windows.
//!
//! ```text
//! Modlight                                  the app (settings in ~/Library/Application Support/Modlight;
//!                                           on Windows in Documents\Modlight, or where PersonalPath.cfg says)
//! Modlight [-silent] [-multi] [<file or folder>...]
//!                                           Windows: start in the tray (Start with Windows passes
//!                                           -silent); another instance beside a running one;
//!                                           upload the files (Explorer's menu, Send to)
//! Modlight --qa-profile <dir>               QA: settings and token in <dir>, no tray, no Keychain
//! Modlight --qa-shot <screen> <out.png>     QA: draw one screen headlessly and exit
//!          [--qa-lang en|pl] [--qa-skin <id>] [--qa-light] [--qa-size WxH] [--qa-scale N]
//! ```
//!
//! On Windows this is a GUI program: no console window opens with it. Started from a
//! console, `--qa-shot`, `--help` and argument errors write into that console.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use egui::{ViewportBuilder, vec2};
use modlight_app::app::{MAIN_MIN_SIZE, MAIN_SIZE, Mode, ModlightApp};
use modlight_app::state::{AppState, Env};
use modlight_app::{platform, qa};
use modlight_core::account::{AccountStore, CredentialStore, FileCredentialStore};
use modlight_core::config::AppConfig;
use modlight_core::l10n::Language;

struct Args {
    qa_profile: Option<PathBuf>,
    shot: Option<(String, PathBuf)>,
    look: qa::Look,
    /// Windows: files and folders to upload (`UseCommandLineArgs`, one upload each).
    #[cfg(windows)]
    files: Vec<PathBuf>,
    /// Windows: `-silent`, start in the tray without showing the window.
    #[cfg(windows)]
    silent: bool,
    /// Windows: `-multi`, run beside a Modlight already running (`MultiInstance`).
    #[cfg(windows)]
    multi: bool,
    /// Windows: upstream's other commands, which this port does not take (logged).
    #[cfg(windows)]
    ignored: Vec<String>,
}

fn usage() -> String {
    format!(
        "usage: Modlight [--qa-profile <dir>] [--qa-shot <screen> <out.png> [--qa-lang en|pl] [--qa-skin <id>] [--qa-light] [--qa-size WxH] [--qa-scale N]]{}\nscreens: {}",
        if cfg!(windows) {
            " [-silent] [-multi] [<file or folder>...]"
        } else {
            ""
        },
        qa::SCREENS.join(", ")
    )
}

fn parse() -> Result<Args, String> {
    let mut args = Args {
        qa_profile: None,
        shot: None,
        look: qa::Look::default(),
        #[cfg(windows)]
        files: Vec::new(),
        #[cfg(windows)]
        silent: false,
        #[cfg(windows)]
        multi: false,
        #[cfg(windows)]
        ignored: Vec::new(),
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
            // Upstream's command line (`CLIManager`, `Program.cs`): a word with one dash is
            // a command, anything else a file or folder. `-silent` (`-s`) and `-multi`
            // (`-m`) are taken; `-portable` (`-p`), `-sandbox`, `-NoHotkeys` and the task
            // commands are accepted and left aside (logged): the personal folder is always
            // the installed one's, and the hotkey warning is a notice in the window.
            #[cfg(windows)]
            other if other.len() > 1 && other.starts_with('-') && !other.starts_with("--") => {
                let name = &other[1..];
                let is =
                    |a: &str, b: &str| name.eq_ignore_ascii_case(a) || name.eq_ignore_ascii_case(b);
                if is("silent", "s") {
                    args.silent = true;
                } else if is("multi", "m") {
                    args.multi = true;
                } else {
                    args.ignored.push(other.to_string());
                }
            }
            #[cfg(windows)]
            other if !other.starts_with('-') => args.files.push(PathBuf::from(other)),
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }
    Ok(args)
}

/// The personal folder (`Program.PersonalFolder`).
fn profile_dir() -> PathBuf {
    #[cfg(windows)]
    {
        modlight_app::personal_folder::resolve()
    }
    #[cfg(not(windows))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(modlight_core::APP_NAME)
    }
}

/// Say why the app cannot start: on the console, and on Windows, where nobody sees a
/// console, in a message box too (upstream's `MessageBox.Show`).
fn fail(message: &str, qa_mode: bool) {
    eprintln!("{message}");
    #[cfg(windows)]
    if !qa_mode {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(modlight_core::APP_NAME)
            .set_description(message)
            .show();
    }
    #[cfg(not(windows))]
    let _ = qa_mode;
}

/// `SingleInstanceManager`: when this user already runs Modlight, this start's files go
/// to it, or it comes forward, and this start ends. A `-silent` start with nothing to
/// hand over only asks whether one runs: a start with Windows must not raise a window.
#[cfg(windows)]
fn handed_to_running_instance(args: &Args) -> bool {
    if !args.files.is_empty() {
        platform::integration::forward_to_running_instance(&args.files)
    } else if args.silent {
        platform::integration::forward_to_running_instance(&[])
    } else {
        platform::integration::activate_running_instance()
    }
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            #[cfg(windows)]
            modlight_app::win32::attach_parent_console();
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    if let Some((screen, out)) = &args.shot {
        #[cfg(windows)]
        modlight_app::win32::attach_parent_console();
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
    #[cfg(windows)]
    if !qa_mode && !args.multi && handed_to_running_instance(&args) {
        return ExitCode::SUCCESS;
    }
    let profile = args.qa_profile.clone().unwrap_or_else(profile_dir);
    if let Err(error) = std::fs::create_dir_all(&profile) {
        fail(
            &format!(
                "the settings folder {} cannot be created: {error}",
                profile.display()
            ),
            qa_mode,
        );
        return ExitCode::FAILURE;
    }
    // `DebugHelper.Logger`: the session in memory, the month's file under Logs.
    modlight_core::debug_log::init(Some(&profile), true);
    log::info!(
        "{} started ({}, {})",
        modlight_core::title(),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    #[cfg(windows)]
    for command in &args.ignored {
        log::info!("Command line: {command} is not supported here and was ignored");
    }
    let first_launch = !profile
        .join(modlight_core::config::APPLICATION_CONFIG_FILE)
        .exists();
    let config = AppConfig::load(&profile);
    // `CleanupManager`: old settings backups and log files, when asked for.
    modlight_core::debug_log::cleanup(&profile, &config);
    let credentials: Box<dyn CredentialStore> = if qa_mode {
        Box::new(FileCredentialStore::new(profile.join("credentials")))
    } else {
        Box::new(platform::SecretStore::for_app())
    };
    let account = AccountStore::load(&profile, credentials);
    let env = Env {
        profile,
        qa: qa_mode,
        frozen_now: None,
        machine: platform::system::machine_name(),
        system_language: platform::system::preferred_language(),
        system_locale: platform::system::system_locale(),
        reduce_motion: platform::system::reduce_motion(),
        system_dark: None,
    };
    let bounds = config.main_window();
    // `MainForm`: `-silent` or "Minimize to tray on start" keep the window hidden, when
    // there is a tray icon to bring it back from.
    #[cfg(windows)]
    let start_hidden = !qa_mode && (args.silent || config.silent_run()) && config.show_tray();
    #[cfg(not(windows))]
    let start_hidden = false;
    let mut state = AppState::new(env, config, account);
    state.first_launch = first_launch && !qa_mode;
    // This start's own files wait for the window like those of later starts.
    #[cfg(windows)]
    if !args.files.is_empty() {
        platform::integration::receive_files(args.files.clone());
    }

    let mut viewport = ViewportBuilder::default()
        .with_title(modlight_core::title())
        .with_inner_size(MAIN_SIZE)
        .with_min_inner_size(MAIN_MIN_SIZE);
    // The window draws its own title band (`ExtendClientAreaToDecorationsHint`): on a Mac
    // under the traffic lights of a full-size content view; on Windows without the system
    // frame, with its own caption buttons and resize edges (`shell::caption_buttons`).
    #[cfg(not(windows))]
    {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    #[cfg(windows)]
    {
        viewport = viewport.with_decorations(false);
    }
    // `RestoreWindowBounds`: the remembered size and place, on a screen that exists.
    let (remembered_size, remembered) = modlight_core::config::restore_main_window(
        &bounds,
        (MAIN_SIZE.x, MAIN_SIZE.y),
        (MAIN_MIN_SIZE.x, MAIN_MIN_SIZE.y),
        &platform::system::visible_frames(),
    );
    let remembered_size = remembered_size.map(|(w, h)| vec2(w, h));
    if let Some(size) = remembered_size {
        viewport = viewport.with_inner_size(size);
    }
    if let Some((x, y)) = remembered {
        viewport = viewport.with_position(egui::pos2(x, y));
    }
    // `WindowStartupLocation="CenterScreen"`.
    #[cfg(windows)]
    if remembered.is_none()
        && let Some((x, y, w, h)) = platform::system::visible_frame()
    {
        let size = remembered_size.unwrap_or(MAIN_SIZE);
        viewport = viewport.with_position(egui::pos2(
            (x + (w - size.x) / 2.0).max(x),
            (y + (h - size.y) / 2.0).max(y),
        ));
    }
    if bounds.maximized {
        viewport = viewport.with_maximized(true);
    }
    // On macOS the window icon becomes the running app's Dock icon (and without one
    // eframe shows its own), so it gets the same Apple grid as the bundle's .icns
    // from scripts/build-app.sh: artwork in 824 of 1024 px, transparent margin.
    // Elsewhere the full-bleed picture is right.
    if let Some(icon) = modlight_app::app::app_icon() {
        #[cfg(target_os = "macos")]
        let icon = {
            let size = icon.width().max(icon.height());
            let inner = (size * 824 + 512) / 1024;
            let artwork =
                image::imageops::resize(&icon, inner, inner, image::imageops::FilterType::Lanczos3);
            let mut padded = image::RgbaImage::new(size, size);
            let offset = i64::from((size - inner) / 2);
            image::imageops::overlay(&mut padded, &artwork, offset, offset);
            padded
        };
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
        Box::new(move |cc| {
            let mut app = ModlightApp::new(cc, state, mode);
            app.start_hidden = start_hidden;
            Ok(Box::new(app))
        }),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            fail(&format!("Modlight could not start: {error}"), qa_mode);
            ExitCode::FAILURE
        }
    }
}

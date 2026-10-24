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

use std::ffi::OsString;
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
    parse_from(std::env::args_os().skip(1))
}

/// The value after one of the port's own options: plain text.
fn option_value(it: &mut impl Iterator<Item = OsString>) -> Result<String, String> {
    it.next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)
}

/// The words after the program's name. Each is taken as the system gives it, not as
/// Unicode (`env::args` would panic on a file name with a lone surrogate, which Windows
/// allows and Explorer passes on).
fn parse_from(mut it: impl Iterator<Item = OsString>) -> Result<Args, String> {
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
    // Windows: upstream's words, read once the port's own options are out of the way.
    #[cfg(windows)]
    let mut upstream: Vec<OsString> = Vec::new();
    while let Some(word) = it.next() {
        // The port's own options are plain text; a word that is not Unicode can only be
        // a file of upstream's command line.
        let arg = match word.into_string() {
            Ok(arg) => arg,
            #[cfg(windows)]
            Err(word) => {
                upstream.push(word);
                continue;
            }
            #[cfg(not(windows))]
            Err(word) => {
                return Err(format!(
                    "unknown argument '{}'\n{}",
                    word.to_string_lossy(),
                    usage()
                ));
            }
        };
        match arg.as_str() {
            "--qa-profile" => args.qa_profile = Some(it.next().ok_or_else(usage)?.into()),
            "--qa-shot" => {
                let screen = option_value(&mut it)?;
                let out = it
                    .next()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| qa::default_output(&screen));
                args.shot = Some((screen, out));
            }
            "--qa-lang" => {
                args.look.language = match option_value(&mut it)?.as_str() {
                    "pl" => Language::Polish,
                    "en" => Language::English,
                    other => return Err(format!("unknown language '{other}'\n{}", usage())),
                }
            }
            "--qa-skin" => args.look.skin = option_value(&mut it)?,
            "--qa-light" => args.look.dark = false,
            "--qa-dark" => args.look.dark = true,
            "--qa-size" => {
                let text = option_value(&mut it)?;
                let (w, h) = text.split_once('x').ok_or_else(usage)?;
                let w: f32 = w.parse().map_err(|_| usage())?;
                let h: f32 = h.parse().map_err(|_| usage())?;
                args.look.size = Some(vec2(w, h));
            }
            "--qa-scale" => {
                args.look.scale = option_value(&mut it)?.parse().map_err(|_| usage())?
            }
            "-h" | "--help" => return Err(usage()),
            // macOS passes a process serial number to apps opened from Finder.
            other if other.starts_with("-psn_") => {}
            // Upstream's command line (`CLIManager`, `Program.cs`), read below.
            #[cfg(windows)]
            other if !other.starts_with('-') || (other.len() > 1 && !other.starts_with("--")) => {
                upstream.push(other.into())
            }
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }
    // `-silent` (`-s`) and `-multi` (`-m`) are taken; `-portable` (`-p`), `-sandbox`,
    // `-NoHotkeys` and the task commands (with their parameters) are accepted and left
    // aside (logged): the personal folder is always the installed one's, and the hotkey
    // warning is a notice in the window.
    #[cfg(windows)]
    {
        let line = upstream_command_line(&upstream);
        for (name, parameter) in line.commands {
            let is =
                |a: &str, b: &str| name.eq_ignore_ascii_case(a) || name.eq_ignore_ascii_case(b);
            if is("silent", "s") {
                args.silent = true;
            } else if is("multi", "m") {
                args.multi = true;
            } else {
                args.ignored.push(match parameter {
                    Some(parameter) => format!("-{name} \"{parameter}\""),
                    None => format!("-{name}"),
                });
            }
        }
        args.files = line.files;
    }
    Ok(args)
}

/// What upstream makes of its command line (`CLIManager.ParseCommands` and
/// `UseCommandLineArgs`): a word with one dash is a command, and the word right after a
/// command is that command's parameter (`-ImageEffect <file.sxie>`, which upstream's own
/// `.sxie` association passes, `-task <name>`, `-workflow <name>`); every other word is
/// a file or folder to upload.
#[cfg(any(windows, test))]
#[derive(Debug, Default, PartialEq)]
struct CommandLine {
    /// Each command without its dash, with its parameter.
    commands: Vec<(String, Option<String>)>,
    files: Vec<PathBuf>,
}

#[cfg(any(windows, test))]
fn upstream_command_line(words: &[OsString]) -> CommandLine {
    let mut line = CommandLine::default();
    // `lastCommand`: the command still waiting for its parameter.
    let mut waiting: Option<usize> = None;
    for word in words {
        let text = word.to_string_lossy();
        if let Some(name) = text.strip_prefix('-') {
            line.commands.push((name.to_string(), None));
            waiting = Some(line.commands.len() - 1);
        } else if let Some(command) = waiting.take() {
            line.commands[command].1 = Some(text.into_owned());
        } else {
            // A file keeps its name exactly as the system gave it.
            line.files.push(PathBuf::from(word));
        }
    }
    line
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
    // `CleanupManager`: old log files, when asked for.
    if config.get("AutoCleanupLogFiles").and_then(|v| v.as_bool()) == Some(true) {
        let keep = config
            .get("CleanupKeepFileCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(10)
            .max(0) as usize;
        modlight_core::debug_log::cleanup_folder(
            &profile.join(modlight_core::debug_log::LOGS_FOLDER),
            "Modlight-Log-",
            ".txt",
            keep,
        );
    }
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
    let remembered_size = bounds
        .size
        .filter(|_| bounds.remember_size)
        .map(|(w, h)| vec2(w as f32, h as f32).max(MAIN_MIN_SIZE));
    if let Some(size) = remembered_size {
        viewport = viewport.with_inner_size(size);
    }
    let remembered = bounds.position.filter(|_| bounds.remember_position);
    if let Some((x, y)) = remembered {
        viewport = viewport.with_position(egui::pos2(x as f32, y as f32));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line(words: &[&str]) -> CommandLine {
        let words: Vec<OsString> = words.iter().map(OsString::from).collect();
        upstream_command_line(&words)
    }

    fn command(name: &str, parameter: Option<&str>) -> (String, Option<String>) {
        (name.to_string(), parameter.map(str::to_string))
    }

    /// A file name the system allows but Unicode does not: Windows takes a lone
    /// surrogate in a name, and Explorer hands such a file over as it is.
    fn not_unicode() -> OsString {
        #[cfg(windows)]
        let name = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&['C' as u16, ':' as u16, '\\' as u16, 0xD800, 'x' as u16])
        };
        #[cfg(unix)]
        let name = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(b"/tmp/\xffx".to_vec())
        };
        assert!(name.to_str().is_none());
        name
    }

    #[test]
    fn a_word_that_is_not_unicode_does_not_stop_the_start() {
        // `env::args` panicked on such a word, before any window or message.
        let result = parse_from(vec![not_unicode()].into_iter());
        #[cfg(windows)]
        assert_eq!(result.unwrap().files, vec![PathBuf::from(not_unicode())]);
        #[cfg(not(windows))]
        assert!(result.is_err());
        // Upstream's reading keeps the name as it is.
        let line = upstream_command_line(&[not_unicode()]);
        assert_eq!(line.files, vec![PathBuf::from(not_unicode())]);
        // The port's own options still read their values.
        let args = parse_from(
            ["--qa-lang", "pl", "--qa-size", "800x600"]
                .into_iter()
                .map(OsString::from),
        )
        .unwrap();
        assert_eq!(args.look.language, Language::Polish);
        assert_eq!(args.look.size, Some(vec2(800.0, 600.0)));
        assert!(parse_from([OsString::from("--qa-lang")].into_iter()).is_err());
    }

    #[test]
    fn the_word_after_a_command_is_its_parameter_not_a_file() {
        // Upstream's `.sxie` association (`"<exe>" -ImageEffect "%1"`): the package is
        // the command's parameter, never a file to upload.
        let effect = line(&["-ImageEffect", r"C:\Effects\glow.sxie"]);
        assert!(effect.files.is_empty(), "{effect:?}");
        assert_eq!(
            effect.commands,
            vec![command("ImageEffect", Some(r"C:\Effects\glow.sxie"))]
        );
        // `-task <name> <file>`: the name goes with the command, the file is uploaded.
        let task = line(&["-task", "Upload", r"C:\a.png"]);
        assert_eq!(task.commands, vec![command("task", Some("Upload"))]);
        assert_eq!(task.files, vec![PathBuf::from(r"C:\a.png")]);
        // `-silent <word>`: upstream reads the word as silent's parameter, not a file.
        let silent = line(&["-silent", r"C:\a.png"]);
        assert_eq!(silent.commands, vec![command("silent", Some(r"C:\a.png"))]);
        assert!(silent.files.is_empty());
        // Two commands in a row: the first has no parameter.
        let flags = line(&["-silent", "-multi"]);
        assert_eq!(
            flags.commands,
            vec![command("silent", None), command("multi", None)]
        );
    }

    #[test]
    fn free_standing_words_are_files() {
        // Explorer's "Upload with Modlight" (`"<exe>" "%1"`) and Send to (every file).
        let send_to = line(&[r"C:\a.png", r"D:\b c\d.txt", r"\\server\share\e.jpg"]);
        assert!(send_to.commands.is_empty());
        assert_eq!(
            send_to.files,
            vec![
                PathBuf::from(r"C:\a.png"),
                PathBuf::from(r"D:\b c\d.txt"),
                PathBuf::from(r"\\server\share\e.jpg"),
            ]
        );
        // A file before a command, and one after the command's parameter.
        let mixed = line(&["a.png", "-workflow", "Capture", "b.png"]);
        assert_eq!(mixed.commands, vec![command("workflow", Some("Capture"))]);
        assert_eq!(
            mixed.files,
            vec![PathBuf::from("a.png"), PathBuf::from("b.png")]
        );
        assert_eq!(line(&[]), CommandLine::default());
    }
}

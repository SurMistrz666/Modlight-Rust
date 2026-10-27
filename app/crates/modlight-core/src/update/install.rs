//! Putting a verified build in place of the running one.
//!
//! [`preflight`] says, before anything is downloaded, whether this copy can replace itself
//! at all; [`prepare`] unpacks the verified zip into a folder of its own (`install-*` in
//! [`super::download_folder`]), checks what came out and writes the helper; [`start`]
//! starts the helper detached, after which the app quits; [`cleanup`], on the next start,
//! puts what the update left in the log and away.
//!
//! - **macOS**: the running copy must be `<somewhere>/X.app` (not translocated, its folder
//!   writable) signed by a developer team; the zip must hold `Modlight.app` and nothing
//!   that leads outside it ([`check_mac_zip`]), is unpacked with `ditto` (which keeps the
//!   signature's extended attributes), and must be intact and signed by the same
//!   developer ([`super::codesign`]). The helper puts it where the running bundle is,
//!   moves the old one to the Trash and opens the new one ([`super::helper::MACOS_SCRIPT`]).
//! - **Windows**: the running copy must not be a cargo build and its folder must be
//!   writable; the zip (`build-app.ps1`'s) must hold `Modlight.exe` and nothing that leads
//!   outside the folder ([`unpack_files`]). The helper replaces the program's files and
//!   starts it again ([`super::helper::WINDOWS_SCRIPT`]).
//!
//! Nothing outside the running copy's own place and the update's folder is touched.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::components::archive::safe_relative;
use crate::l10n::{Arg, format, port, t};

#[cfg(target_os = "macos")]
use super::codesign;
use super::codesign::Signing;
use super::helper;

/// The bundle inside the macOS zip (`ditto -c -k --keepParent Modlight.app`).
pub const MAC_BUNDLE: &str = "Modlight.app";
/// The program inside the Windows zip.
pub const WINDOWS_EXE: &str = "Modlight.exe";
/// More than an update's unpacked size could ever be (a Modlight build is about 150 MB).
pub const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;
/// More entries than a Modlight zip has.
pub const MAX_ENTRIES: usize = 20_000;
/// The helper's output, in the update's folder.
pub const LOG_NAME: &str = "update.log";
/// What the staging folders of [`prepare`] are called.
const STAGE_PREFIX: &str = "install-";

/// Why an update is not installed. Every one of them sends the person to the server's
/// download page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallProblem {
    /// macOS: not running from an app bundle (`cargo run`, a bare binary).
    NotAppBundle,
    /// macOS: App Translocation runs the bundle from a read-only copy.
    Translocated,
    /// The folder the app runs from (or the bundle) cannot be written.
    CannotWrite(PathBuf),
    /// macOS: the running app is ad hoc signed or signed by no team.
    Unsigned,
    /// macOS: the new bundle is not the same developer's, or not intact.
    SignerDiffers,
    /// Windows: a development build in a cargo `target` folder.
    DevelopmentBuild(PathBuf),
    /// The zip is not an update for this system, or could not be unpacked.
    NotAnUpdate(String),
    /// The helper could not be written or started.
    StartFailed(String),
    /// A system this updater does not install on.
    NotSupported,
}

impl InstallProblem {
    /// What the person reads.
    pub fn text(&self) -> String {
        match self {
            InstallProblem::NotAppBundle | InstallProblem::NotSupported => {
                t(port::UpdateNotAppBundle).to_string()
            }
            InstallProblem::Translocated => t(port::UpdateTranslocated).to_string(),
            InstallProblem::CannotWrite(folder) => format(
                t(port::UpdateCannotWrite),
                &[Arg::from(folder.display().to_string())],
            ),
            InstallProblem::Unsigned => t(port::UpdateUnsigned).to_string(),
            InstallProblem::SignerDiffers => t(port::UpdateSignerDiffers).to_string(),
            InstallProblem::DevelopmentBuild(folder) => format(
                t(port::UpdateDevelopmentBuild),
                &[Arg::from(folder.display().to_string())],
            ),
            InstallProblem::NotAnUpdate(detail) => {
                format(t(port::UpdateNotAnUpdate), &[Arg::from(detail.as_str())])
            }
            InstallProblem::StartFailed(detail) => {
                format(t(port::UpdateStartFailed), &[Arg::from(detail.as_str())])
            }
        }
    }
}

/// Where the running copy is and who signed it: what an update replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// macOS: the `.app` bundle; Windows: the program's folder.
    pub location: PathBuf,
    /// The program's file name (`Modlight`, `Modlight.exe`).
    pub exe: String,
    /// macOS: the running bundle's signature.
    pub signing: Option<Signing>,
}

/// Whether this copy can replace itself, and where it is. Cheap enough to ask before
/// anything is downloaded (on a Mac two runs of `codesign --display`); blocking.
pub fn preflight() -> Result<Installed, InstallProblem> {
    let exe = std::env::current_exe().map_err(|_| InstallProblem::NotAppBundle)?;
    preflight_for(&exe)
}

/// [`preflight`] for a given program path.
pub fn preflight_for(exe: &Path) -> Result<Installed, InstallProblem> {
    #[cfg(target_os = "macos")]
    {
        preflight_mac(exe)
    }
    #[cfg(windows)]
    {
        preflight_windows(exe)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = exe;
        Err(InstallProblem::NotSupported)
    }
}

/// `.../X.app/Contents/MacOS/<program>` gives `.../X.app`, when that bundle has its
/// `Info.plist`.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|n| n == name);
    let is_app = bundle
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("app"));
    (named(macos, "MacOS")
        && named(contents, "Contents")
        && is_app
        && contents.join("Info.plist").is_file())
    .then(|| bundle.to_path_buf())
}

/// Whether macOS runs this bundle from App Translocation's read-only copy (a quarantined
/// app opened where it was downloaded).
pub fn is_translocated(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str() == "AppTranslocation")
}

#[cfg(unix)]
fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(text) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a NUL-terminated path; `access` only reads it.
    unsafe { libc::access(text.as_ptr(), libc::W_OK) == 0 }
}

/// Windows says whether a folder takes a new file only by trying: a probe file, at once
/// deleted.
#[cfg(windows)]
fn writable(folder: &Path) -> bool {
    let probe = folder.join(format!(
        ".modlight-update-{:08x}.tmp",
        rand::random::<u32>()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
fn preflight_mac(exe: &Path) -> Result<Installed, InstallProblem> {
    let exe = exe
        .canonicalize()
        .map_err(|_| InstallProblem::NotAppBundle)?;
    let bundle = bundle_of(&exe).ok_or(InstallProblem::NotAppBundle)?;
    if is_translocated(&bundle) {
        return Err(InstallProblem::Translocated);
    }
    let folder = bundle
        .parent()
        .ok_or(InstallProblem::NotAppBundle)?
        .to_path_buf();
    if !writable(&folder) || !writable(&bundle) {
        return Err(InstallProblem::CannotWrite(folder));
    }
    let signing = codesign::read(&bundle).map_err(|error| {
        log::info!("Update: this copy is not signed ({error})");
        InstallProblem::Unsigned
    })?;
    // Compared with itself: whether there is a developer to compare a new build with.
    if codesign::same_developer(&signing, &signing).is_err() {
        return Err(InstallProblem::Unsigned);
    }
    Ok(Installed {
        location: bundle,
        exe: exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        signing: Some(signing),
    })
}

/// Whether `exe` runs from a cargo target folder (`modlight-windows`' rule for the Startup
/// shortcut): a folder named `target` that cargo marked (`CACHEDIR.TAG`), or that has
/// `debug` or `release` below it on the way to the program.
pub fn is_development_build(exe: &Path) -> bool {
    exe.ancestors().skip(1).any(|folder| {
        folder
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("target"))
            && (folder.join("CACHEDIR.TAG").is_file()
                || exe.strip_prefix(folder).is_ok_and(|rest| {
                    rest.components().any(|part| {
                        let part = part.as_os_str();
                        part.eq_ignore_ascii_case("debug") || part.eq_ignore_ascii_case("release")
                    })
                }))
    })
}

#[cfg(windows)]
fn preflight_windows(exe: &Path) -> Result<Installed, InstallProblem> {
    let exe = std::path::absolute(exe).map_err(|e| InstallProblem::StartFailed(e.to_string()))?;
    let folder = exe
        .parent()
        .ok_or(InstallProblem::NotAppBundle)?
        .to_path_buf();
    if is_development_build(&exe) {
        return Err(InstallProblem::DevelopmentBuild(folder));
    }
    if !writable(&folder) {
        return Err(InstallProblem::CannotWrite(folder));
    }
    Ok(Installed {
        location: folder,
        exe: exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| WINDOWS_EXE.to_string()),
        signing: None,
    })
}

/// What [`prepare`] made: the helper, ready to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// The update's own folder (the new version, the helper, its log).
    pub stage: PathBuf,
    /// Where the helper writes what it does.
    pub log: PathBuf,
}

/// A new staging folder in the update folder.
fn new_stage() -> Result<PathBuf, InstallProblem> {
    let stage =
        super::download_folder().join(format!("{STAGE_PREFIX}{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&stage).map_err(|e| InstallProblem::StartFailed(e.to_string()))?;
    Ok(stage)
}

/// Unpack and check the verified zip, and write the helper that will put it in place of
/// `installed`. Blocking (ditto and codesign on a Mac). On any problem the staging folder
/// is removed again.
pub fn prepare(zip: &Path, installed: &Installed) -> Result<Prepared, InstallProblem> {
    let stage = new_stage()?;
    let result = prepare_in(zip, installed, &stage);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

fn prepare_in(zip: &Path, installed: &Installed, stage: &Path) -> Result<Prepared, InstallProblem> {
    #[cfg(target_os = "macos")]
    {
        prepare_mac(zip, installed, stage, std::process::id(), true)
    }
    #[cfg(windows)]
    {
        prepare_windows(zip, installed, stage, std::process::id(), true)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (zip, installed, stage);
        Err(InstallProblem::NotSupported)
    }
}

/// macOS: `zip` unpacked into `<stage>/new/Modlight.app`, checked against the running
/// bundle's signature, and the helper written (`relaunch` false: the tests' run).
#[cfg(target_os = "macos")]
pub fn prepare_mac(
    zip: &Path,
    installed: &Installed,
    stage: &Path,
    pid: u32,
    relaunch: bool,
) -> Result<Prepared, InstallProblem> {
    let running = installed.signing.as_ref().ok_or(InstallProblem::Unsigned)?;
    let new_bundle = unpack_mac(zip, stage)?;
    check_signature(&new_bundle, running)?;
    mac_helper(stage, installed, &new_bundle, pid, relaunch)
}

/// The macOS zip unpacked into `<stage>/new`: the new `Modlight.app` there.
#[cfg(target_os = "macos")]
pub fn unpack_mac(zip: &Path, stage: &Path) -> Result<PathBuf, InstallProblem> {
    check_mac_zip(zip, MAX_UNPACKED).map_err(InstallProblem::NotAnUpdate)?;
    let unpacked = stage.join("new");
    std::fs::create_dir_all(&unpacked).map_err(|e| InstallProblem::NotAnUpdate(e.to_string()))?;
    // ditto, not the zip crate: it puts back the extended attributes `ditto -c` stored
    // beside each file, and a sealed bundle must come out exactly as it went in.
    let output = std::process::Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(zip)
        .arg(&unpacked)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| InstallProblem::NotAnUpdate(format!("/usr/bin/ditto: {e}")))?;
    if !output.status.success() {
        return Err(InstallProblem::NotAnUpdate(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let new_bundle = unpacked.join(MAC_BUNDLE);
    let is_folder = std::fs::symlink_metadata(&new_bundle).is_ok_and(|m| m.is_dir());
    if !is_folder || !new_bundle.join("Contents/MacOS").is_dir() {
        return Err(InstallProblem::NotAnUpdate(format!(
            "{MAC_BUNDLE} is not in the archive"
        )));
    }
    Ok(new_bundle)
}

/// Whether the unpacked `new_bundle` may replace the app `running` describes: the same
/// developer ([`codesign::same_developer`]), intact and satisfying the running app's
/// designated requirement ([`codesign::verify`]).
#[cfg(target_os = "macos")]
pub fn check_signature(new_bundle: &Path, running: &Signing) -> Result<(), InstallProblem> {
    let signing = codesign::read(new_bundle).map_err(|error| {
        log::warn!("Update: the new version is not signed ({error})");
        InstallProblem::SignerDiffers
    })?;
    codesign::same_developer(running, &signing).map_err(|problem| match problem {
        codesign::SignatureProblem::Unsigned => InstallProblem::Unsigned,
        codesign::SignatureProblem::Differs => {
            log::warn!(
                "Update: the new version is signed by {:?} ({:?}), this copy by {:?} ({:?})",
                signing.team,
                signing.identifier,
                running.team,
                running.identifier
            );
            InstallProblem::SignerDiffers
        }
    })?;
    codesign::verify(new_bundle, &running.requirement).map_err(|error| {
        log::warn!("Update: the new version does not satisfy this copy's requirement ({error})");
        InstallProblem::SignerDiffers
    })
}

/// The macOS helper written into `stage`, to put `new_bundle` where `installed` is.
#[cfg(target_os = "macos")]
pub fn mac_helper(
    stage: &Path,
    installed: &Installed,
    new_bundle: &Path,
    pid: u32,
    relaunch: bool,
) -> Result<Prepared, InstallProblem> {
    let script = stage.join("finish-update.sh");
    write_script(&script, &helper::macos_script(), 0o700)?;
    let aside_folder = stage.join("old");
    std::fs::create_dir_all(&aside_folder)
        .map_err(|e| InstallProblem::StartFailed(e.to_string()))?;
    let aside = aside_folder.join(installed.location.file_name().unwrap_or_default());
    let run = helper::MacosRun {
        script: &script,
        pid,
        old: &installed.location,
        new: new_bundle,
        aside: &aside,
        open: relaunch,
        trash: relaunch,
    };
    Ok(Prepared {
        program: run.program(),
        args: run.args(),
        stage: stage.to_path_buf(),
        log: stage.join(LOG_NAME),
    })
}

/// Windows: `zip` unpacked into `<stage>\files` and the helper written.
#[cfg(windows)]
pub fn prepare_windows(
    zip: &Path,
    installed: &Installed,
    stage: &Path,
    pid: u32,
    restart: bool,
) -> Result<Prepared, InstallProblem> {
    let files = stage.join("files");
    unpack_files(zip, &files, WINDOWS_EXE, MAX_UNPACKED).map_err(InstallProblem::NotAnUpdate)?;
    let script = stage.join("finish-update.ps1");
    write_script(&script, &helper::windows_script(), 0o600)?;
    let root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let backup = stage.join("backup");
    // The new version's program, whatever this copy's file is called.
    let run = helper::WindowsRun {
        powershell: &powershell,
        script: &script,
        pid,
        source: &files,
        target: &installed.location,
        backup: &backup,
        exe: WINDOWS_EXE,
        start: restart,
    };
    Ok(Prepared {
        program: run.program(),
        args: run.args(),
        stage: stage.to_path_buf(),
        log: stage.join(LOG_NAME),
    })
}

#[allow(unused_variables)]
fn write_script(path: &Path, text: &str, mode: u32) -> Result<(), InstallProblem> {
    std::fs::write(path, text).map_err(|e| InstallProblem::StartFailed(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| InstallProblem::StartFailed(e.to_string()))?;
    }
    Ok(())
}

/// Start the helper detached, its output going to its log. The app quits right after; the
/// helper waits for it.
pub fn start(prepared: &Prepared) -> Result<(), InstallProblem> {
    let failed = |e: std::io::Error| InstallProblem::StartFailed(e.to_string());
    let log = std::fs::File::create(&prepared.log).map_err(failed)?;
    let mut command = std::process::Command::new(&prepared.program);
    command
        .args(&prepared.args)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(failed)?)
        .stderr(log);
    #[cfg(unix)]
    {
        // A group of its own: nothing sent to the app's group reaches it.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let flags = crate::components::process::CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
        // Out of any job the app was started in, so the job's end does not end the
        // helper; a job that forbids that still gets the helper, inside it.
        command.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB);
        if command.spawn().is_ok() {
            log::info!("Update: the helper started");
            return Ok(());
        }
        command.creation_flags(flags);
    }
    command.spawn().map_err(failed)?;
    log::info!("Update: the helper started");
    Ok(())
}

/// What earlier updates left in the update folder: each staging folder's helper log is
/// written to the debug log, an old bundle the helper set aside but could not move to the
/// Trash goes there now (`trash`), then the folder is removed; earlier downloads go too.
/// Run once at start, before any new update. Blocking.
pub fn cleanup(trash: &dyn Fn(&Path) -> Result<(), String>) {
    cleanup_in(&super::download_folder(), trash);
}

/// [`cleanup`] in a given folder.
pub fn cleanup_in(root: &Path, trash: &dyn Fn(&Path) -> Result<(), String>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    'stages: for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && (name.ends_with(".zip") || name.ends_with(".part")) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if !kind.is_dir() || !name.starts_with(STAGE_PREFIX) {
            continue;
        }
        if let Ok(file) = std::fs::File::open(path.join(LOG_NAME)) {
            let mut text = String::new();
            let _ = file.take(64 * 1024).read_to_string(&mut text);
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                log::info!("Update helper: {line}");
            }
        }
        if let Ok(set_aside) = std::fs::read_dir(path.join("old")) {
            for old in set_aside.flatten() {
                match trash(&old.path()) {
                    Ok(()) => log::info!(
                        "Update: the previous version is in the Trash ({})",
                        old.path().display()
                    ),
                    Err(error) => {
                        // Never deleted: kept where it is for the next start.
                        log::warn!(
                            "Update: the previous version stays at {} ({error})",
                            old.path().display()
                        );
                        continue 'stages;
                    }
                }
            }
        }
        if let Err(error) = std::fs::remove_dir_all(&path) {
            log::info!("Update: {} stays for now ({error})", path.display());
        }
    }
}

/// The macOS zip's entries, read before `ditto` unpacks it: all of them under
/// `Modlight.app/` (or `__MACOSX/`, where other zip tools put extended attributes), none
/// that leaves the folder (absolute, `..`, a backslash), links only downwards
/// ([`link_stays_inside`]) and nothing stored through a link, at most `max` bytes in all.
pub fn check_mac_zip(zip: &Path, max: u64) -> Result<(), String> {
    let file = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!("{} entries", archive.len()));
    }
    let mut total: u64 = 0;
    let mut names = Vec::new();
    let mut links = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let relative =
            safe_relative(&name).ok_or_else(|| format!("a name that leaves the folder: {name}"))?;
        let top = first_part(&relative);
        if top != MAC_BUNDLE && top != "__MACOSX" {
            return Err(format!("{name} is not part of {MAC_BUNDLE}"));
        }
        total = total.saturating_add(entry.size());
        if total > max {
            return Err(format!("more than {max} bytes"));
        }
        if entry.is_symlink() {
            let mut target = String::new();
            (&mut entry)
                .take(4096)
                .read_to_string(&mut target)
                .map_err(|e| e.to_string())?;
            if !link_stays_inside(&target) {
                return Err(format!("{name} links outside {MAC_BUNDLE}: {target}"));
            }
            links.push(relative.clone());
        }
        names.push(relative);
    }
    if !names.iter().any(|n| first_part(n) == MAC_BUNDLE) {
        return Err(format!("{MAC_BUNDLE} is not in the archive"));
    }
    for name in &names {
        if let Some(link) = links
            .iter()
            .find(|link| name != *link && name.starts_with(link))
        {
            return Err(format!(
                "{} is stored through the link {}",
                name.display(),
                link.display()
            ));
        }
    }
    Ok(())
}

fn first_part(path: &Path) -> String {
    path.components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether a link pointing at `target` stays below the folder it sits in: a relative
/// path without `..` (a framework's `Versions/Current -> A`). Going up is refused outright:
/// through another link, `..` can lead anywhere whatever the names say.
fn link_stays_inside(target: &str) -> bool {
    !target.is_empty()
        && !target.contains('\\')
        && !target.contains('\0')
        && Path::new(target)
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// Unpack a Windows update zip into `destination` (which must not exist yet or be empty):
/// files and folders only (a link is an error), names with `\` read as `/` (Windows
/// PowerShell 5.1's `Compress-Archive` writes them so), none that leaves the folder; the
/// zip must have `program` at its top, or inside one folder that holds everything, which
/// is then left out. At most `max` bytes in all, counted as they are written.
pub fn unpack_files(zip: &Path, destination: &Path, program: &str, max: u64) -> Result<(), String> {
    let file = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!("{} entries", archive.len()));
    }
    // First pass: the names, the declared sizes, and where the program is.
    let mut entries: Vec<(usize, PathBuf, bool)> = Vec::new();
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if entry.is_symlink() {
            return Err(format!("{name} is a link"));
        }
        let relative =
            safe_relative(&name).ok_or_else(|| format!("a name that leaves the folder: {name}"))?;
        total = total.saturating_add(entry.size());
        if total > max {
            return Err(format!("more than {max} bytes"));
        }
        entries.push((index, relative, entry.is_dir()));
    }
    let is_program = |path: &Path| {
        path.to_str()
            .is_some_and(|text| text.eq_ignore_ascii_case(program))
    };
    let strip: Option<PathBuf> = if entries
        .iter()
        .any(|(_, path, dir)| !dir && is_program(path))
    {
        None
    } else {
        // One folder holding everything, with the program in it.
        let top = entries
            .first()
            .map(|(_, path, _)| PathBuf::from(first_part(path)));
        match top {
            Some(top)
                if entries.iter().all(|(_, path, _)| path.starts_with(&top))
                    && entries.iter().any(|(_, path, dir)| {
                        !dir && path.strip_prefix(&top).is_ok_and(is_program)
                    }) =>
            {
                Some(top)
            }
            _ => return Err(format!("{program} is not in the archive")),
        }
    };
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let mut written: u64 = 0;
    for (index, relative, is_dir) in entries {
        let inner = match &strip {
            Some(top) => relative
                .strip_prefix(top)
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            None => relative,
        };
        if inner.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(&inner);
        if is_dir {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let declared = entry.size();
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        // A zip can lie about its sizes: what is written is counted, not what it claims.
        let allowed = declared.min(max.saturating_sub(written));
        let copied = std::io::copy(&mut (&mut entry).take(allowed + 1), &mut out)
            .map_err(|e| e.to_string())?;
        if copied > allowed {
            return Err(format!("{} is larger than it says", inner.display()));
        }
        out.flush().map_err(|e| e.to_string())?;
        written += copied;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    /// A zip with these entries: name, bytes, and a link target for a symbolic link.
    pub(crate) fn zip_with(entries: &[(&str, &[u8], Option<&str>)]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut writer = zip::ZipWriter::new(file.reopen().unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes, link) in entries {
            if let Some(target) = link {
                writer.add_symlink(*name, *target, options).unwrap();
            } else if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
        }
        writer.finish().unwrap();
        file
    }

    /// A small app bundle (`/bin/echo` as its program) with a marker file, unsigned.
    #[cfg(target_os = "macos")]
    pub(crate) fn fake_bundle(folder: &Path, name: &str, marker: &str) -> PathBuf {
        let bundle = folder.join(name);
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
        std::fs::copy("/bin/echo", bundle.join("Contents/MacOS/Modlight")).unwrap();
        std::fs::write(bundle.join("Contents/Resources/data.txt"), marker).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Modlight</string>
<key>CFBundleIdentifier</key><string>org.modlight.Modlight</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
"#,
        )
        .unwrap();
        bundle
    }

    #[test]
    fn a_mac_zip_holds_modlight_app_and_nothing_that_leads_out_of_it() {
        let good = zip_with(&[
            ("Modlight.app/", b"", None),
            ("Modlight.app/Contents/Info.plist", b"<plist/>", None),
            ("Modlight.app/Contents/._Info.plist", b"xattrs", None),
            (
                "Modlight.app/Contents/MacOS/Modlight",
                b"\xcf\xfa\xed\xfe",
                None,
            ),
            (
                "Modlight.app/Contents/Frameworks/X.framework/Versions/Current",
                b"",
                Some("A"),
            ),
            ("__MACOSX/Modlight.app/._Contents", b"xattrs", None),
        ]);
        assert_eq!(check_mac_zip(good.path(), MAX_UNPACKED), Ok(()));
        for (entries, why) in [
            (
                vec![
                    ("Modlight.app/Contents/Info.plist", &b"x"[..], None),
                    ("../evil", b"x", None),
                ],
                "a name with ..",
            ),
            (
                vec![
                    ("Modlight.app/x", &b"x"[..], None),
                    ("/etc/evil", b"x", None),
                ],
                "an absolute name",
            ),
            (
                vec![
                    ("Modlight.app/x", &b"x"[..], None),
                    ("Other.app/x", b"x", None),
                ],
                "something besides the bundle",
            ),
            (
                vec![("Modlight.app\\Contents\\x", &b"x"[..], None)],
                "a backslash",
            ),
            (vec![("Readme.txt", &b"x"[..], None)], "no bundle"),
            (
                vec![("Modlight.app/Contents/l", &b""[..], Some("/etc"))],
                "an absolute link",
            ),
            (
                vec![(
                    "Modlight.app/Contents/l",
                    &b""[..],
                    Some("../../../Library"),
                )],
                "a link out of the bundle",
            ),
            (
                vec![("Modlight.app/l", &b""[..], Some(".."))],
                "a link to the bundle's folder",
            ),
            (
                vec![(
                    "Modlight.app/Contents/l",
                    &b""[..],
                    Some("../Contents/MacOS"),
                )],
                "a link that goes up, even to come back",
            ),
            (
                vec![
                    ("Modlight.app/Contents/l", &b""[..], Some("MacOS")),
                    ("Modlight.app/Contents/l/evil", b"x", None),
                ],
                "a file stored through a link",
            ),
        ] {
            let zip = zip_with(&entries);
            assert!(check_mac_zip(zip.path(), MAX_UNPACKED).is_err(), "{why}");
        }
        let big = zip_with(&[("Modlight.app/Contents/MacOS/Modlight", &[0u8; 4096], None)]);
        assert!(check_mac_zip(big.path(), 1000).is_err());
        let not_a_zip = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(not_a_zip.path(), b"PK? no").unwrap();
        assert!(check_mac_zip(not_a_zip.path(), MAX_UNPACKED).is_err());
    }

    #[test]
    fn a_windows_zip_unpacks_into_its_folder_and_nowhere_else() {
        // build-app.ps1's zip: the folder's files at the top.
        let zip = zip_with(&[
            ("Modlight.exe", b"MZ new", None),
            ("DirectML.dll", b"dll", None),
            ("Licenses/", b"", None),
            ("Licenses/ffmpeg.txt", b"licence", None),
        ]);
        let out = tempfile::tempdir().unwrap();
        let files = out.path().join("files");
        unpack_files(zip.path(), &files, WINDOWS_EXE, MAX_UNPACKED).unwrap();
        assert_eq!(
            std::fs::read(files.join("Modlight.exe")).unwrap(),
            b"MZ new"
        );
        assert_eq!(
            std::fs::read(files.join("Licenses/ffmpeg.txt")).unwrap(),
            b"licence"
        );
        // Windows PowerShell 5.1's backslashes, and one folder around everything.
        let zip = zip_with(&[
            ("Modlight\\modlight.EXE", b"MZ", None),
            ("Modlight\\Licenses\\a.txt", b"a", None),
        ]);
        let files = out.path().join("wrapped");
        unpack_files(zip.path(), &files, WINDOWS_EXE, MAX_UNPACKED).unwrap();
        assert!(files.join("modlight.EXE").is_file());
        assert!(files.join("Licenses").join("a.txt").is_file());
        assert!(!files.join("Modlight").exists());
        // Refused: no program, a way out, a link, too much.
        for (entries, why) in [
            (vec![("Readme.txt", &b"x"[..], None)], "no program"),
            (
                vec![
                    ("Modlight.exe", &b"MZ"[..], None),
                    ("../evil.dll", b"x", None),
                ],
                "a name with ..",
            ),
            (
                vec![
                    ("Modlight.exe", &b"MZ"[..], None),
                    ("..\\evil.dll", b"x", None),
                ],
                "a backslashed ..",
            ),
            (
                vec![
                    ("Modlight.exe", &b"MZ"[..], None),
                    ("/evil.dll", b"x", None),
                ],
                "an absolute name",
            ),
            (
                vec![
                    ("Modlight.exe", &b"MZ"[..], None),
                    ("link", b"", Some("C:/Windows")),
                ],
                "a link",
            ),
            (
                vec![
                    ("a/Modlight.exe", &b"MZ"[..], None),
                    ("b/x.dll", b"x", None),
                ],
                "the program inside one of several folders",
            ),
        ] {
            let zip = zip_with(&entries);
            let target = out.path().join(why.replace(' ', "-"));
            assert!(
                unpack_files(zip.path(), &target, WINDOWS_EXE, MAX_UNPACKED).is_err(),
                "{why}"
            );
            assert!(!out.path().join("evil.dll").exists(), "{why}");
        }
        let big = zip_with(&[("Modlight.exe", &[0u8; 4096], None)]);
        assert!(unpack_files(big.path(), &out.path().join("big"), WINDOWS_EXE, 1000).is_err());
    }

    #[cfg(target_os = "macos")]
    fn sign_ad_hoc(bundle: &Path) {
        let output = std::process::Command::new(codesign::CODESIGN)
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(bundle)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[cfg(target_os = "macos")]
    fn marker(bundle: &Path) -> String {
        std::fs::read_to_string(bundle.join("Contents/Resources/data.txt")).unwrap()
    }

    /// A stand-in for the app: a process that ends after `seconds`, reaped as it ends (the
    /// real app's parent, launchd, reaps it; a zombie would still answer `kill -0`).
    #[cfg(target_os = "macos")]
    fn app_for(seconds: &str) -> (u32, std::thread::JoinHandle<()>) {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg(seconds)
            .spawn()
            .unwrap();
        let pid = child.id();
        (pid, std::thread::spawn(move || drop(child.wait())))
    }

    /// The whole macOS swap on throwaway bundles: zipped as `build-app.sh` zips them,
    /// unpacked with ditto (the seal survives), and put in place by the real helper, which
    /// first waits for the "app" to end. Nothing is opened, nothing goes to the Trash.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_helper_swaps_the_bundles_once_the_app_has_quit() {
        let root = tempfile::tempdir().unwrap();
        let applications = root.path().join("Applications");
        let old = fake_bundle(&applications, "Modlight.app", "old");
        let new = fake_bundle(&root.path().join("build"), "Modlight.app", "new");
        sign_ad_hoc(&new);
        let zip = root.path().join("Modlight-0.7.9-9f8e7d6c-macos-arm64.zip");
        let zipped = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&new)
            .arg(&zip)
            .output()
            .unwrap();
        assert!(zipped.status.success(), "{zipped:?}");
        assert_eq!(check_mac_zip(&zip, MAX_UNPACKED), Ok(()));

        let stage = root.path().join("install-0000000000000001");
        std::fs::create_dir_all(&stage).unwrap();
        let unpacked = unpack_mac(&zip, &stage).unwrap();
        assert_eq!(unpacked, stage.join("new/Modlight.app"));
        let signing = codesign::read(&unpacked).unwrap();
        assert_eq!(codesign::verify(&unpacked, &signing.requirement), Ok(()));
        // An ad hoc signed copy never takes a new version, not even an identical one.
        assert_eq!(
            check_signature(&unpacked, &signing),
            Err(InstallProblem::Unsigned)
        );
        let mut team = signing.clone();
        team.team = Some("TEAM123456".into());
        team.ad_hoc = false;
        team.requirement = "identifier \"org.modlight.Modlight\" and anchor apple generic".into();
        assert_eq!(
            check_signature(&unpacked, &team),
            Err(InstallProblem::SignerDiffers)
        );

        let installed = Installed {
            location: old.clone(),
            exe: "Modlight".into(),
            signing: None,
        };
        let (pid, app) = app_for("1");
        let prepared = mac_helper(&stage, &installed, &unpacked, pid, false).unwrap();
        assert_eq!(prepared.program, "/bin/sh");
        let started = std::time::Instant::now();
        let run = std::process::Command::new(&prepared.program)
            .args(&prepared.args)
            .output()
            .unwrap();
        app.join().unwrap();
        let log = String::from_utf8_lossy(&run.stdout).to_string();
        assert!(run.status.success(), "{log}");
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(500),
            "it waited for the app"
        );
        assert_eq!(marker(&old), "new");
        assert_eq!(marker(&stage.join("old/Modlight.app")), "old");
        assert!(!unpacked.exists());
        assert_eq!(codesign::verify(&old, &signing.requirement), Ok(()));
        for line in ["Modlight has quit", "the new version is in place", "done"] {
            assert!(log.contains(line), "{line}: {log}");
        }

        // Again: the new version is gone from the stage and the old is set aside there,
        // so nothing changes.
        let again = std::process::Command::new(&prepared.program)
            .args(&prepared.args)
            .output()
            .unwrap();
        assert!(!again.status.success());
        assert_eq!(marker(&old), "new");
    }

    /// The new bundle cannot be moved in (a folder that may not be written cannot change
    /// parents): the old one goes back where it was.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_helper_puts_the_old_version_back_when_the_new_cannot_go_in() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let old = fake_bundle(&root.path().join("Applications"), "Modlight.app", "old");
        let stage = root.path().join("install-0000000000000002");
        let new = fake_bundle(&stage.join("new"), "Modlight.app", "new");
        let installed = Installed {
            location: old.clone(),
            exe: "Modlight".into(),
            signing: None,
        };
        let prepared = mac_helper(&stage, &installed, &new, u32::MAX >> 1, false).unwrap();
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o555)).unwrap();
        let run = std::process::Command::new(&prepared.program)
            .args(&prepared.args)
            .output()
            .unwrap();
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).unwrap();
        let log = String::from_utf8_lossy(&run.stdout).to_string();
        assert!(!run.status.success(), "{log}");
        assert!(log.contains("the old version is back"), "{log}");
        assert_eq!(marker(&old), "old");
        assert_eq!(marker(&new), "new");
    }

    /// An app that does not quit: the helper gives up and changes nothing.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_helper_changes_nothing_while_the_app_still_runs() {
        let root = tempfile::tempdir().unwrap();
        let old = fake_bundle(&root.path().join("Applications"), "Modlight.app", "old");
        let stage = root.path().join("install-0000000000000003");
        let new = fake_bundle(&stage.join("new"), "Modlight.app", "new");
        let script = stage.join("finish-update.sh");
        std::fs::write(&script, helper::macos_script_waiting(1)).unwrap();
        let mut app = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let run = helper::MacosRun {
            script: &script,
            pid: app.id(),
            old: &old,
            new: &new,
            aside: &stage.join("old-Modlight.app"),
            open: false,
            trash: false,
        };
        let output = std::process::Command::new(run.program())
            .args(run.args())
            .output()
            .unwrap();
        let _ = app.kill();
        let _ = app.wait();
        let log = String::from_utf8_lossy(&output.stdout).to_string();
        assert!(!output.status.success());
        assert!(log.contains("did not quit within 1 s"), "{log}");
        assert_eq!(marker(&old), "old");
        assert_eq!(marker(&new), "new");
    }

    #[test]
    fn problems_read_as_sentences() {
        crate::l10n::set_thread_language(Some(crate::l10n::Language::English));
        assert!(
            InstallProblem::CannotWrite(PathBuf::from("/Applications"))
                .text()
                .contains("/Applications")
        );
        assert!(
            InstallProblem::NotAnUpdate("Modlight.exe is not in the archive".into())
                .text()
                .ends_with(": Modlight.exe is not in the archive")
        );
        for problem in [
            InstallProblem::NotAppBundle,
            InstallProblem::Translocated,
            InstallProblem::Unsigned,
            InstallProblem::SignerDiffers,
            InstallProblem::DevelopmentBuild(PathBuf::from("target")),
            InstallProblem::StartFailed("x".into()),
            InstallProblem::NotSupported,
        ] {
            assert!(!problem.text().is_empty(), "{problem:?}");
        }
        crate::l10n::set_thread_language(None);
    }

    #[test]
    fn a_bundle_is_found_from_its_program() {
        let folder = tempfile::tempdir().unwrap();
        let bundle = folder.path().join("Modlight 2.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::write(bundle.join("Contents/Info.plist"), "<plist/>").unwrap();
        let exe = bundle.join("Contents/MacOS/Modlight");
        assert_eq!(bundle_of(&exe), Some(bundle.clone()));
        assert_eq!(
            bundle_of(&folder.path().join("target/debug/Modlight")),
            None
        );
        assert_eq!(bundle_of(Path::new("/Modlight")), None);
        // Without its Info.plist a folder named .app is not a bundle.
        std::fs::remove_file(bundle.join("Contents/Info.plist")).unwrap();
        assert_eq!(bundle_of(&exe), None);
        assert!(is_translocated(Path::new(
            "/private/var/folders/x/T/AppTranslocation/1234/d/Modlight.app"
        )));
        assert!(!is_translocated(Path::new("/Applications/Modlight.app")));
    }

    #[test]
    fn a_cargo_build_is_not_an_installed_copy() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        assert!(is_development_build(
            &root.join("app/target/release/Modlight.exe")
        ));
        assert!(is_development_build(&root.join(
            "x/target/x86_64-pc-windows-msvc/debug/deps/modlight.exe"
        )));
        std::fs::create_dir_all(root.join("target/dist")).unwrap();
        std::fs::write(root.join("target/CACHEDIR.TAG"), "Signature").unwrap();
        assert!(is_development_build(&root.join("target/dist/Modlight.exe")));
        assert!(!is_development_build(
            &root.join("Apps/Modlight/Modlight.exe")
        ));
        assert!(!is_development_build(
            &root.join("target-practice/Modlight.exe")
        ));
    }

    #[test]
    fn cleanup_logs_trashes_and_removes_what_an_update_left() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("install-0123456789abcdef");
        std::fs::create_dir_all(stage.join("old/Modlight.app/Contents")).unwrap();
        std::fs::write(stage.join(LOG_NAME), "2026-10-27T14:03:05Z done\n").unwrap();
        std::fs::write(root.path().join("Modlight-0.7.9-x-macos-arm64.zip"), "zip").unwrap();
        std::fs::write(
            root.path().join("Modlight-0.8.0-x-macos-arm64.zip.part"),
            "z",
        )
        .unwrap();
        std::fs::write(root.path().join("notes.txt"), "kept").unwrap();
        std::fs::create_dir_all(root.path().join("other")).unwrap();
        let trashed = std::sync::Mutex::new(Vec::new());
        // The Trash refuses at first: the old bundle and its folder stay.
        cleanup_in(root.path(), &|_| Err("no Trash here".into()));
        assert!(stage.join("old/Modlight.app").is_dir());
        cleanup_in(root.path(), &|path| {
            trashed.lock().unwrap().push(path.to_path_buf());
            std::fs::rename(path, root.path().join("trashed.app")).map_err(|e| e.to_string())
        });
        assert_eq!(
            trashed.into_inner().unwrap(),
            vec![stage.join("old/Modlight.app")]
        );
        assert!(!stage.exists());
        assert!(
            root.path().join("trashed.app").is_dir(),
            "moved, never deleted"
        );
        assert!(root.path().join("notes.txt").is_file());
        assert!(root.path().join("other").is_dir());
        assert!(
            !root
                .path()
                .join("Modlight-0.7.9-x-macos-arm64.zip")
                .exists()
        );
        assert!(
            !root
                .path()
                .join("Modlight-0.8.0-x-macos-arm64.zip.part")
                .exists()
        );
    }
}

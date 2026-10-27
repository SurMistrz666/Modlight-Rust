//! The helpers that finish an update once the app has quit: a running program cannot
//! replace itself, so the app writes one of these next to the verified new version,
//! starts it detached and quits. The helper waits for the app's process to end, swaps the
//! old version for the new one (and back, when that fails half way), and starts the app.
//!
//! The scripts are fixed texts; everything that varies (process id, paths) comes as
//! arguments, never spliced into the text, so no path can be read as code. Both texts are
//! compiled on every system and their shape is tested everywhere; the macOS one is also
//! run by the tests (with a stand-in for the Trash and without opening anything).

use std::ffi::OsString;
use std::path::Path;

/// How long a helper waits for the app to quit before it gives up, changing nothing.
pub const QUIT_TIMEOUT_SECONDS: u32 = 120;

/// The macOS helper, run with `/bin/sh`. Arguments:
/// 1. the app's process id;
/// 2. the bundle to replace (absolute, the one the app runs from);
/// 3. the verified new bundle (absolute, in the update's folder);
/// 4. where the old bundle is set aside first (absolute, in the update's folder);
/// 5. `open` to open the new version when done (anything else: do not);
/// 6. `trash` to move the old version to the Trash when done (anything else: leave it set
///    aside, for the tests; the app moves a leftover one to the Trash when it starts).
///
/// It writes what it does on standard output, which the app sends to the update's log.
/// Nothing is deleted: a half-copied new bundle (another volume) is set aside too.
pub const MACOS_SCRIPT: &str = r#"#!/bin/sh
# Modlight's update helper (written by Modlight next to the verified new version).
# Waits for Modlight to quit, puts the new Modlight.app where the old one was, moves the
# old one to the Trash and opens the new one. Touches nothing but those bundles.
set -u
pid=$1
old=$2
new=$3
aside=$4
relaunch=$5
trash=$6

say() {
    printf '%s %s\n' "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

reopen() {
    if [ "$relaunch" = open ] && [ -d "$old" ]; then
        /usr/bin/open "$old" || say "could not open $old"
    fi
}

# 1. Wait until Modlight has quit (checked five times a second).
waited=0
while /bin/kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge __QUIT_TICKS__ ]; then
        say "Modlight did not quit within __QUIT_SECONDS__ s; nothing was changed"
        exit 1
    fi
    /bin/sleep 0.2
    waited=$((waited + 1))
done
say "Modlight has quit"

case "$old" in
    /*.app) ;;
    *) say "not an app bundle: $old; nothing was changed"; exit 1 ;;
esac
if [ ! -d "$old/Contents" ] || [ ! -d "$new/Contents" ] || [ -e "$aside" ]; then
    say "the old or the new bundle is not where it was; nothing was changed"
    reopen
    exit 1
fi

# 2. The old bundle out of the way, the new one in its place.
if ! /bin/mv "$old" "$aside"; then
    say "could not move $old; nothing was changed"
    reopen
    exit 1
fi
if ! /bin/mv "$new" "$old"; then
    say "could not put the new version in place; putting the old one back"
    if [ -e "$old" ]; then
        /bin/mv "$old" "$aside.incomplete" || say "could not set the incomplete copy aside"
    fi
    if /bin/mv "$aside" "$old"; then
        say "the old version is back"
    else
        say "could not put the old version back; it is at $aside"
    fi
    reopen
    exit 1
fi
say "the new version is in place"

# 3. The old version to the Trash (NSFileManager through JavaScript for Automation, which
#    asks for no permission; a plain move into the Trash folder if that fails).
if [ "$trash" = trash ]; then
    trashed=$(/usr/bin/osascript -l JavaScript -e '
function run(argv) {
    ObjC.import("Foundation");
    var url = $.NSURL.fileURLWithPath(argv[0]);
    return $.NSFileManager.defaultManager.trashItemAtURLResultingItemURLError(url, null, null) ? "trashed" : "failed";
}' "$aside" 2>/dev/null)
    if [ "$trashed" = trashed ]; then
        say "the old version is in the Trash"
    else
        name=$(/usr/bin/basename "$old" .app)
        stamp=$(/bin/date +%Y-%m-%d\ %H.%M.%S)
        if [ -d "$HOME/.Trash" ] && /bin/mv "$aside" "$HOME/.Trash/$name $stamp.app" 2>/dev/null; then
            say "the old version is in the Trash"
        else
            say "the old version stays at $aside until Modlight starts"
        fi
    fi
fi

# 4. The new version.
reopen
say "done"
exit 0
"#;

/// The Windows helper, run with Windows PowerShell 5.1 (`powershell.exe -File`; written
/// for PowerShell 7 too). It replaces only the files the new version has (a file of the
/// folder that the zip does not have, such as `PersonalPath.cfg`, stays), keeps each
/// replaced one in `Backup` until all are in place, and puts them back when one cannot be
/// written. Its output goes to the update's log.
pub const WINDOWS_SCRIPT: &str = r#"# Modlight's update helper (written by Modlight next to the verified new version).
# Waits for Modlight to quit, replaces the program's files with the new version's and
# starts Modlight again. Touches nothing but the program's folder and the update's folder.
param(
    [Parameter(Mandatory = $true)][int]$ProcessId,
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$Backup,
    [Parameter(Mandatory = $true)][string]$Exe,
    [switch]$NoStart
)

$ErrorActionPreference = 'Stop'

function Say([string]$Text) {
    $stamp = [DateTime]::UtcNow.ToString("yyyy'-'MM'-'dd'T'HH':'mm':'ss'Z'", [Globalization.CultureInfo]::InvariantCulture)
    Write-Output "$stamp $Text"
}

# A file the antivirus or the indexer still holds is tried again for a few seconds.
function Retry([scriptblock]$Action) {
    for ($attempt = 1; ; $attempt++) {
        try {
            & $Action
            return
        } catch {
            if ($attempt -ge 20) { throw }
            Start-Sleep -Milliseconds 250
        }
    }
}

function Start-Modlight {
    $program = Join-Path $Target $Exe
    if (-not $NoStart -and (Test-Path -LiteralPath $program -PathType Leaf)) {
        Start-Process -FilePath $program -WorkingDirectory $Target
    }
}

# 1. Wait until Modlight has quit.
$process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
if ($process) {
    if (-not $process.WaitForExit(__QUIT_MILLISECONDS__)) {
        Say 'Modlight did not quit within __QUIT_SECONDS__ s; nothing was changed'
        exit 1
    }
}
Say 'Modlight has quit'

if (-not (Test-Path -LiteralPath (Join-Path $Source $Exe) -PathType Leaf) -or
    -not (Test-Path -LiteralPath $Target -PathType Container)) {
    Say 'the new version or the program folder is not where it was; nothing was changed'
    Start-Modlight
    exit 1
}
# The folders spelled as Get-ChildItem spells its files, so a file's path minus the
# folder is its place in the folder.
$Source = (Get-Item -LiteralPath $Source).FullName.TrimEnd('\', '/')
$Target = (Get-Item -LiteralPath $Target).FullName.TrimEnd('\', '/')
$Backup = [IO.Path]::GetFullPath($Backup).TrimEnd('\', '/')

# 2. Each new file in place; the one it replaces kept in Backup until all are in.
$done = New-Object System.Collections.ArrayList
try {
    foreach ($file in Get-ChildItem -LiteralPath $Source -Recurse -File -Force) {
        $relative = $file.FullName.Substring($Source.Length).TrimStart('\', '/')
        $destination = [IO.Path]::Combine($Target, $relative)
        $kept = [IO.Path]::Combine($Backup, $relative)
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
        $replaced = [IO.File]::Exists($destination)
        if ($replaced) {
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($kept))
            Retry { [IO.File]::Move($destination, $kept) }
        }
        [void]$done.Add(@($destination, $kept, $replaced))
        Retry { [IO.File]::Copy($file.FullName, $destination, $true) }
    }
} catch {
    Say "could not put the new version in place ($($_.Exception.Message)); putting the old files back"
    for ($i = $done.Count - 1; $i -ge 0; $i--) {
        $destination, $kept, $replaced = $done[$i]
        try {
            if ([IO.File]::Exists($destination)) {
                [IO.File]::Delete($destination)
            }
            if ($replaced) {
                [IO.File]::Move($kept, $destination)
            }
        } catch {
            Say "could not put back $destination ($($_.Exception.Message)); the old file is $kept"
        }
    }
    Start-Modlight
    exit 1
}
Say 'the new version is in place'

# 3. The new version.
Start-Modlight
Say 'done'
exit 0
"#;

/// The macOS script as written to disk: the timeouts filled in.
pub fn macos_script() -> String {
    macos_script_waiting(QUIT_TIMEOUT_SECONDS)
}

/// The macOS script waiting at most `seconds` for the app to quit (the tests' short wait).
pub fn macos_script_waiting(seconds: u32) -> String {
    MACOS_SCRIPT
        .replace("__QUIT_TICKS__", &(seconds * 5).to_string())
        .replace("__QUIT_SECONDS__", &seconds.to_string())
}

/// The Windows script as written to disk: the timeouts filled in, Windows line ends, and a
/// byte order mark so that Windows PowerShell 5.1 reads it as UTF-8 whatever the system's
/// code page is.
pub fn windows_script() -> String {
    let text = WINDOWS_SCRIPT
        .replace(
            "__QUIT_MILLISECONDS__",
            &(QUIT_TIMEOUT_SECONDS * 1000).to_string(),
        )
        .replace("__QUIT_SECONDS__", &QUIT_TIMEOUT_SECONDS.to_string());
    format!("\u{feff}{}", text.replace('\n', "\r\n"))
}

/// What the macOS helper is started with: `/bin/sh <script> <arguments>`.
pub struct MacosRun<'a> {
    pub script: &'a Path,
    pub pid: u32,
    pub old: &'a Path,
    pub new: &'a Path,
    pub aside: &'a Path,
    pub open: bool,
    pub trash: bool,
}

impl MacosRun<'_> {
    pub fn program(&self) -> OsString {
        "/bin/sh".into()
    }

    pub fn args(&self) -> Vec<OsString> {
        vec![
            self.script.into(),
            self.pid.to_string().into(),
            self.old.into(),
            self.new.into(),
            self.aside.into(),
            if self.open { "open" } else { "no-open" }.into(),
            if self.trash { "trash" } else { "keep" }.into(),
        ]
    }
}

/// What the Windows helper is started with: Windows PowerShell with the script and its
/// named parameters.
pub struct WindowsRun<'a> {
    /// `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`.
    pub powershell: &'a Path,
    pub script: &'a Path,
    pub pid: u32,
    pub source: &'a Path,
    pub target: &'a Path,
    pub backup: &'a Path,
    pub exe: &'a str,
    pub start: bool,
}

impl WindowsRun<'_> {
    pub fn program(&self) -> OsString {
        self.powershell.into()
    }

    pub fn args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-WindowStyle".into(),
            "Hidden".into(),
            "-File".into(),
            self.script.into(),
            "-ProcessId".into(),
            self.pid.to_string().into(),
            "-Source".into(),
            self.source.into(),
            "-Target".into(),
            self.target.into(),
            "-Backup".into(),
            self.backup.into(),
            "-Exe".into(),
            self.exe.into(),
        ];
        if !self.start {
            args.push("-NoStart".into());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_take_everything_that_varies_as_arguments() {
        let mac = macos_script();
        assert!(mac.starts_with("#!/bin/sh\n"));
        assert!(!mac.contains("__"), "every placeholder is filled in");
        assert!(mac.contains("-ge 600 ]"), "120 s at five checks a second");
        // Only fixed tools by their absolute paths.
        for tool in [
            "/bin/mv",
            "/bin/kill -0",
            "/usr/bin/open",
            "/usr/bin/osascript",
        ] {
            assert!(mac.contains(tool), "{tool}");
        }
        // It never deletes: no rm, no rmdir.
        assert!(!mac.contains("rm "));
        assert!(!mac.contains("rmdir"));
        // The arguments are read once, quoted wherever they are used.
        assert!(mac.contains("pid=$1\nold=$2\nnew=$3\naside=$4\nrelaunch=$5\ntrash=$6\n"));
        assert!(!mac.contains(" $old ") && !mac.contains(" $new "));

        let windows = windows_script();
        assert!(
            windows.starts_with('\u{feff}'),
            "UTF-8 for Windows PowerShell 5.1"
        );
        assert!(windows.contains("\r\n") && !windows.replace("\r\n", "").contains('\n'));
        assert!(!windows.contains("__"));
        assert!(windows.contains("WaitForExit(120000)"));
        for parameter in [
            "$ProcessId",
            "$Source",
            "$Target",
            "$Backup",
            "$Exe",
            "$NoStart",
        ] {
            assert!(windows.contains(parameter), "{parameter}");
        }
        // Files are moved and copied by .NET, which reads no wildcards into a path; the
        // old file goes back over the new one when something fails; nothing is removed
        // but a new file that is being undone.
        assert!(!windows.contains(" -Path $"));
        assert!(!windows.contains("Remove-Item"));
        assert!(windows.contains("[IO.File]::Move($destination, $kept)"));
        assert!(windows.contains("[IO.File]::Move($kept, $destination)"));
        assert!(windows.contains("[IO.File]::Copy($file.FullName, $destination, $true)"));
    }

    #[test]
    fn the_helpers_are_started_with_their_arguments_in_order() {
        let run = MacosRun {
            script: Path::new("/tmp/u/finish-update.sh"),
            pid: 4242,
            old: Path::new("/Applications/Modlight.app"),
            new: Path::new("/tmp/u/new/Modlight.app"),
            aside: Path::new("/tmp/u/old/Modlight.app"),
            open: true,
            trash: true,
        };
        assert_eq!(run.program(), "/bin/sh");
        assert_eq!(
            run.args(),
            [
                "/tmp/u/finish-update.sh",
                "4242",
                "/Applications/Modlight.app",
                "/tmp/u/new/Modlight.app",
                "/tmp/u/old/Modlight.app",
                "open",
                "trash"
            ]
            .map(OsString::from)
        );
        let run = WindowsRun {
            powershell: Path::new("C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"),
            script: Path::new("C:/Temp/u/finish-update.ps1"),
            pid: 4242,
            source: Path::new("C:/Temp/u/files"),
            target: Path::new("D:/Apps/Modlight"),
            backup: Path::new("C:/Temp/u/backup"),
            exe: "Modlight.exe",
            start: false,
        };
        let args: Vec<String> = run
            .args()
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args[..9],
            [
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-File",
                "C:/Temp/u/finish-update.ps1"
            ]
        );
        let pairs: Vec<(&str, &str)> = args[9..19]
            .chunks(2)
            .map(|pair| (pair[0].as_str(), pair[1].as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("-ProcessId", "4242"),
                ("-Source", "C:/Temp/u/files"),
                ("-Target", "D:/Apps/Modlight"),
                ("-Backup", "C:/Temp/u/backup"),
                ("-Exe", "Modlight.exe")
            ]
        );
        assert_eq!(args.last().map(String::as_str), Some("-NoStart"));
    }
}

//! `ShareX.HelpersLib/Helpers/FileHelpers.cs`, the parts the task flow uses, with the
//! `System.IO.Path`, `File.Exists` and `Environment` behaviour they rely on, as .NET has
//! it on the system the app runs on.
//!
//! On Windows that is upstream's own platform: `\` and `/` both separate folders, a root
//! is a drive, a UNC share or a device path, the invalid characters of a file name are
//! `"<>|:*?\/` and the control characters, special folders are the shell's known folders
//! and environment variables are expanded by the system, names compared without case.
//! On macOS it is .NET on Unix: `/` is the only separator, NUL and `/` are the invalid
//! characters of a file name (NUL of a path), and the special folders map to their macOS
//! counterparts. Upstream adds nothing of its own beyond trimming the white space around.

use std::sync::RwLock;

/// `FileHelpers.ImageFileExtensions`.
pub const IMAGE_FILE_EXTENSIONS: &[&str] =
    &["jpg", "jpeg", "png", "gif", "bmp", "ico", "tif", "tiff"];

/// `FileHelpers.TextFileExtensions`: what `%rf{path}` accepts and what is uploaded as text.
pub const TEXT_FILE_EXTENSIONS: &[&str] = &[
    "txt", "log", "nfo", "c", "cpp", "cc", "cxx", "h", "hpp", "hxx", "cs", "vb", "html", "htm",
    "xhtml", "xht", "xml", "css", "js", "php", "bat", "java", "lua", "py", "pl", "cfg", "ini",
    "dart", "go", "gohtml",
];

/// `FileHelpers.VideoFileExtensions` (upstream lists "m4v" twice).
pub const VIDEO_FILE_EXTENSIONS: &[&str] = &[
    "mp4", "webm", "mkv", "avi", "vob", "ogv", "ogg", "mov", "qt", "wmv", "m4p", "m4v", "mpg",
    "mp2", "mpeg", "mpe", "mpv", "m2v", "m4v", "flv", "f4v",
];

/// `Path.GetInvalidFileNameChars()` on Unix.
#[cfg(not(windows))]
pub const INVALID_FILE_NAME_CHARS: &[char] = &['\0', '/'];

/// `Path.GetInvalidFileNameChars()` on Windows, in .NET's order: `"<>|`, NUL and the
/// other control characters, then `:*?\/`.
#[cfg(windows)]
pub const INVALID_FILE_NAME_CHARS: &[char] = &[
    '"', '<', '>', '|', '\0', '\u{1}', '\u{2}', '\u{3}', '\u{4}', '\u{5}', '\u{6}', '\u{7}',
    '\u{8}', '\u{9}', '\u{a}', '\u{b}', '\u{c}', '\u{d}', '\u{e}', '\u{f}', '\u{10}', '\u{11}',
    '\u{12}', '\u{13}', '\u{14}', '\u{15}', '\u{16}', '\u{17}', '\u{18}', '\u{19}', '\u{1a}',
    '\u{1b}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ':', '*', '?', '\\', '/',
];

/// `Path.DirectorySeparatorChar` and `Path.AltDirectorySeparatorChar` (both `/` on Unix).
fn is_directory_separator(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// Extensions read as part of a double extension (`archive.tar.gz` is "tar.gz").
const KNOWN_SECOND_EXTENSIONS: [&str; 1] = ["tar"];

/// `GetFileNameExtension(filePath, includeDot)`: the text after the last dot of the
/// whole path, with "tar.gz"-style double extensions kept together. Like upstream, a dot
/// in a folder name counts when the file has none ("v1.2/readme" gives "2/readme").
pub fn get_file_name_extension(file_path: &str, include_dot: bool) -> String {
    file_name_extension(file_path, include_dot, true)
}

/// `GetFileNameExtension(filePath, includeDot, checkSecondExtension: false)`.
pub fn get_single_file_name_extension(file_path: &str, include_dot: bool) -> String {
    file_name_extension(file_path, include_dot, false)
}

fn file_name_extension(file_path: &str, include_dot: bool, check_second: bool) -> String {
    let Some(pos) = file_path.rfind('.') else {
        return String::new();
    };
    let mut extension = file_path[pos + 1..].to_string();
    if check_second {
        let second = file_name_extension(&file_path[..pos], false, false);
        if KNOWN_SECOND_EXTENSIONS
            .iter()
            .any(|known| second.eq_ignore_ascii_case(known))
        {
            extension = format!("{second}.{extension}");
        }
    }
    if include_dot {
        extension.insert(0, '.');
    }
    extension
}

/// `ChangeFileNameExtension`: everything from the last dot replaced by the new
/// extension (whose own last dot and what precedes it are dropped); an empty extension
/// only removes the old one.
pub fn change_file_name_extension(file_name: &str, extension: &str) -> String {
    if file_name.is_empty() {
        return String::new();
    }
    let stem = file_name
        .rfind('.')
        .map_or(file_name, |pos| &file_name[..pos]);
    if extension.is_empty() {
        return stem.to_string();
    }
    let extension = extension
        .rfind('.')
        .map_or(extension, |pos| &extension[pos + 1..]);
    format!("{stem}.{extension}")
}

/// `AppendExtension`: one dot between the name and the extension, whatever dots they
/// already end and start with.
pub fn append_extension(file_path: &str, extension: &str) -> String {
    format!(
        "{}.{}",
        file_path.trim_end_matches('.'),
        extension.trim_start_matches('.')
    )
}

/// `CheckExtension`: whether the extension (double extensions included) is one of
/// `extensions`, ignoring case.
pub fn check_extension(file_path: &str, extensions: &[&str]) -> bool {
    let extension = get_file_name_extension(file_path, false);
    !extension.is_empty()
        && extensions
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
}

/// `IsImageFile`.
pub fn is_image_file(file_path: &str) -> bool {
    check_extension(file_path, IMAGE_FILE_EXTENSIONS)
}

/// `IsTextFile`.
pub fn is_text_file(file_path: &str) -> bool {
    check_extension(file_path, TEXT_FILE_EXTENSIONS)
}

/// `IsVideoFile`.
pub fn is_video_file(file_path: &str) -> bool {
    check_extension(file_path, VIDEO_FILE_EXTENSIONS)
}

/// `SanitizeFileName(fileName)`: trimmed, without the characters a name cannot hold.
pub fn sanitize_file_name(file_name: &str) -> String {
    sanitize_file_name_with(file_name, "")
}

/// `SanitizeFileName(fileName, replaceWith)`.
pub fn sanitize_file_name_with(file_name: &str, replace_with: &str) -> String {
    sanitize(file_name, replace_with, INVALID_FILE_NAME_CHARS)
}

/// `SanitizePath(path)`: trimmed, without the characters a path cannot hold; folder
/// separators stay.
pub fn sanitize_path(path: &str) -> String {
    sanitize_path_with(path, "")
}

/// `SanitizePath(path, replaceWith)`: the file name's invalid characters except the
/// folder separators. A Windows drive root (`C:\`) at the start is kept out of the
/// cleaning like upstream does (its colon would go otherwise); on macOS there is none.
pub fn sanitize_path_with(path: &str, replace_with: &str) -> String {
    let root = get_path_root(path);
    let rest = &path[root.len()..];
    let invalid: Vec<char> = INVALID_FILE_NAME_CHARS
        .iter()
        .copied()
        .filter(|c| !is_directory_separator(*c))
        .collect();
    format!("{root}{}", sanitize(rest, replace_with, &invalid))
}

fn sanitize(text: &str, replace_with: &str, invalid: &[char]) -> String {
    let mut text = text.trim().to_string();
    for c in invalid {
        text = text.replace(*c, replace_with);
    }
    text
}

/// `GetPathRoot`: the Windows drive root ("C:\") the path starts with, or nothing.
pub fn get_path_root(path: &str) -> String {
    match path.find(":\\") {
        Some(separator) if separator > 0 => path[..separator + 2].to_string(),
        _ => String::new(),
    }
}

/// `GetFileSize`: the size in bytes, -1 when there is no such file (or it is a folder).
pub fn get_file_size(file_path: &str) -> i64 {
    // `FileInfo` reads a dangling symbolic link's own size.
    let metadata = std::fs::metadata(file_path).or_else(|_| std::fs::symlink_metadata(file_path));
    match metadata {
        Ok(metadata) if !metadata.is_dir() => i64::try_from(metadata.len()).unwrap_or(i64::MAX),
        _ => -1,
    }
}

/// `GetUniqueFilePath`: the path itself when nothing is there yet, else the first free
/// "name (n).ext". Numbering starts at 2 like upstream ("shot.png" becomes
/// "shot (2).png"), and a name that already ends in " (n)" counts on from n.
///
/// Upstream's `\d` also accepts non-ASCII digits and `int.Parse` then throws on them,
/// as it does on a counter past `int.MaxValue`; such a name is treated as having no
/// counter here.
pub fn get_unique_file_path(file_path: &str) -> String {
    if !file_exists(file_path) {
        return file_path.to_string();
    }
    let folder = path::get_directory_name(file_path).unwrap_or_default();
    let mut file_name = path::get_file_name_without_extension(file_path).to_string();
    let extension = path::get_extension(file_path);
    let mut number: i32 = 1;
    if let Some((name, counter)) = split_counter(&file_name) {
        file_name = name;
        number = counter;
    }
    loop {
        // C# `int` arithmetic: past int.MaxValue the counter wraps.
        number = number.wrapping_add(1);
        let candidate = path::combine(&folder, &format!("{file_name} ({number}){extension}"));
        if !file_exists(&candidate) {
            return candidate;
        }
    }
}

/// `^(.+) \((\d+)\)$` on a file name: the name before " (n)" and n. `$` also matches
/// before a final new line, and `.` does not match one.
fn split_counter(name: &str) -> Option<(String, i32)> {
    let body = name.strip_suffix('\n').unwrap_or(name);
    let body = body.strip_suffix(')')?;
    let digit_count = body.bytes().rev().take_while(u8::is_ascii_digit).count();
    let (head, digits) = body.split_at(body.len() - digit_count);
    if digits.is_empty() {
        return None;
    }
    let head = head.strip_suffix(" (")?;
    if head.is_empty() || head.contains('\n') {
        return None;
    }
    let counter = digits.parse().ok()?;
    Some((head.to_string(), counter))
}

/// `File.Exists`: something that is not a folder, never for an empty path or one ending
/// in a separator. A relative path is taken from the working folder.
///
/// On Unix a dangling symbolic link counts and a link to a folder does not. On Windows
/// the entry's own attributes decide (`GetFileAttributesEx` does not follow a link): a
/// file link counts even when dangling, a folder link or junction never does.
pub fn file_exists(file_path: &str) -> bool {
    if file_path.is_empty() || file_path.contains('\0') {
        return false;
    }
    let full = path::get_full_path(file_path);
    if full.ends_with(is_directory_separator) {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        std::fs::symlink_metadata(&full)
            .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY == 0)
    }
    #[cfg(not(windows))]
    match std::fs::symlink_metadata(&full) {
        Err(_) => false,
        Ok(metadata) if metadata.is_dir() => false,
        Ok(metadata) if metadata.file_type().is_symlink() => {
            !std::fs::metadata(&full).is_ok_and(|target| target.is_dir())
        }
        Ok(_) => true,
    }
}

/// What a special folder name expands to on macOS.
#[cfg(not(windows))]
#[derive(Clone, Copy)]
enum Folder {
    /// A folder in the home folder ("" is the home folder itself).
    Home(&'static str),
    /// A folder of the system.
    Fixed(&'static str),
    /// No macOS counterpart: the name expands to nothing, like .NET's
    /// `Environment.GetFolderPath` answers on macOS for such folders.
    Nothing,
}

/// `Environment.SpecialFolder`, in the enum's order, with the macOS folder each name
/// stands for in `%Name%`. The Windows folders with a macOS counterpart map to it
/// (documents, pictures, music, movies, desktop, home, Application Support, caches,
/// fonts, /Applications); the rest (Start menu, Windows, x86 program files...) expand
/// to nothing. `MyDocuments` and `Personal` are the same enum value; .NET prints it as
/// one of the two names only, so upstream expands just that one; both work here.
#[cfg(not(windows))]
const SPECIAL_FOLDERS: [(&str, Folder); 47] = [
    ("Desktop", Folder::Home("Desktop")),
    ("Programs", Folder::Nothing),
    ("MyDocuments", Folder::Home("Documents")),
    ("Personal", Folder::Home("Documents")),
    ("Favorites", Folder::Home("Library/Favorites")),
    ("Startup", Folder::Nothing),
    ("Recent", Folder::Nothing),
    ("SendTo", Folder::Nothing),
    ("StartMenu", Folder::Nothing),
    ("MyMusic", Folder::Home("Music")),
    ("MyVideos", Folder::Home("Movies")),
    ("DesktopDirectory", Folder::Home("Desktop")),
    ("MyComputer", Folder::Nothing),
    ("NetworkShortcuts", Folder::Nothing),
    ("Fonts", Folder::Home("Library/Fonts")),
    ("Templates", Folder::Nothing),
    ("CommonStartMenu", Folder::Nothing),
    ("CommonPrograms", Folder::Nothing),
    ("CommonStartup", Folder::Nothing),
    ("CommonDesktopDirectory", Folder::Nothing),
    (
        "ApplicationData",
        Folder::Home("Library/Application Support"),
    ),
    ("PrinterShortcuts", Folder::Nothing),
    (
        "LocalApplicationData",
        Folder::Home("Library/Application Support"),
    ),
    ("InternetCache", Folder::Home("Library/Caches")),
    ("Cookies", Folder::Home("Library/Cookies")),
    ("History", Folder::Nothing),
    (
        "CommonApplicationData",
        Folder::Fixed("/Library/Application Support"),
    ),
    ("Windows", Folder::Nothing),
    ("System", Folder::Fixed("/System")),
    ("ProgramFiles", Folder::Fixed("/Applications")),
    ("MyPictures", Folder::Home("Pictures")),
    ("UserProfile", Folder::Home("")),
    ("SystemX86", Folder::Nothing),
    ("ProgramFilesX86", Folder::Nothing),
    ("CommonProgramFiles", Folder::Nothing),
    ("CommonProgramFilesX86", Folder::Nothing),
    ("CommonTemplates", Folder::Nothing),
    ("CommonDocuments", Folder::Fixed("/Users/Shared")),
    ("CommonAdminTools", Folder::Nothing),
    ("AdminTools", Folder::Nothing),
    ("CommonMusic", Folder::Nothing),
    ("CommonPictures", Folder::Nothing),
    ("CommonVideos", Folder::Nothing),
    ("Resources", Folder::Nothing),
    ("LocalizedResources", Folder::Nothing),
    ("CommonOemLinks", Folder::Nothing),
    ("CDBurning", Folder::Nothing),
];

/// What a special folder name expands to on Windows: the call .NET's
/// `Environment.GetFolderPath` makes for it.
#[cfg(windows)]
#[derive(Clone, Copy)]
enum Folder {
    /// A known folder (`SHGetKnownFolderPath`), with the environment variable .NET reads
    /// when the shell has none (Nano Server).
    Known(windows_sys::core::GUID, Option<&'static str>),
    /// `Environment.SystemDirectory`.
    System,
}

/// `Environment.SpecialFolder`, in the enum's order, with the known folder .NET asks
/// the shell for on Windows (`Environment.Windows.cs`). A virtual folder (`MyComputer`,
/// and `PrinterShortcuts`, which .NET maps to the printers folder) and a folder this
/// Windows does not have expand to nothing, as they do upstream.
#[cfg(windows)]
const SPECIAL_FOLDERS: [(&str, Folder); 47] = {
    use windows_sys::Win32::UI::Shell::*;
    [
        ("Desktop", Folder::Known(FOLDERID_Desktop, None)),
        ("Programs", Folder::Known(FOLDERID_Programs, None)),
        ("MyDocuments", Folder::Known(FOLDERID_Documents, None)),
        ("Personal", Folder::Known(FOLDERID_Documents, None)),
        ("Favorites", Folder::Known(FOLDERID_Favorites, None)),
        ("Startup", Folder::Known(FOLDERID_Startup, None)),
        ("Recent", Folder::Known(FOLDERID_Recent, None)),
        ("SendTo", Folder::Known(FOLDERID_SendTo, None)),
        ("StartMenu", Folder::Known(FOLDERID_StartMenu, None)),
        ("MyMusic", Folder::Known(FOLDERID_Music, None)),
        ("MyVideos", Folder::Known(FOLDERID_Videos, None)),
        ("DesktopDirectory", Folder::Known(FOLDERID_Desktop, None)),
        ("MyComputer", Folder::Known(FOLDERID_ComputerFolder, None)),
        ("NetworkShortcuts", Folder::Known(FOLDERID_NetHood, None)),
        ("Fonts", Folder::Known(FOLDERID_Fonts, None)),
        ("Templates", Folder::Known(FOLDERID_Templates, None)),
        (
            "CommonStartMenu",
            Folder::Known(FOLDERID_CommonStartMenu, None),
        ),
        (
            "CommonPrograms",
            Folder::Known(FOLDERID_CommonPrograms, None),
        ),
        ("CommonStartup", Folder::Known(FOLDERID_CommonStartup, None)),
        (
            "CommonDesktopDirectory",
            Folder::Known(FOLDERID_PublicDesktop, None),
        ),
        (
            "ApplicationData",
            Folder::Known(FOLDERID_RoamingAppData, Some("APPDATA")),
        ),
        (
            "PrinterShortcuts",
            Folder::Known(FOLDERID_PrintersFolder, None),
        ),
        (
            "LocalApplicationData",
            Folder::Known(FOLDERID_LocalAppData, Some("LOCALAPPDATA")),
        ),
        ("InternetCache", Folder::Known(FOLDERID_InternetCache, None)),
        ("Cookies", Folder::Known(FOLDERID_Cookies, None)),
        ("History", Folder::Known(FOLDERID_History, None)),
        (
            "CommonApplicationData",
            Folder::Known(FOLDERID_ProgramData, Some("ProgramData")),
        ),
        ("Windows", Folder::Known(FOLDERID_Windows, Some("windir"))),
        ("System", Folder::System),
        (
            "ProgramFiles",
            Folder::Known(FOLDERID_ProgramFiles, Some("ProgramFiles")),
        ),
        ("MyPictures", Folder::Known(FOLDERID_Pictures, None)),
        (
            "UserProfile",
            Folder::Known(FOLDERID_Profile, Some("USERPROFILE")),
        ),
        ("SystemX86", Folder::Known(FOLDERID_SystemX86, None)),
        (
            "ProgramFilesX86",
            Folder::Known(FOLDERID_ProgramFilesX86, Some("ProgramFiles(x86)")),
        ),
        (
            "CommonProgramFiles",
            Folder::Known(FOLDERID_ProgramFilesCommon, Some("CommonProgramFiles")),
        ),
        (
            "CommonProgramFilesX86",
            Folder::Known(
                FOLDERID_ProgramFilesCommonX86,
                Some("CommonProgramFiles(x86)"),
            ),
        ),
        (
            "CommonTemplates",
            Folder::Known(FOLDERID_CommonTemplates, None),
        ),
        (
            "CommonDocuments",
            Folder::Known(FOLDERID_PublicDocuments, None),
        ),
        (
            "CommonAdminTools",
            Folder::Known(FOLDERID_CommonAdminTools, None),
        ),
        ("AdminTools", Folder::Known(FOLDERID_AdminTools, None)),
        ("CommonMusic", Folder::Known(FOLDERID_PublicMusic, None)),
        (
            "CommonPictures",
            Folder::Known(FOLDERID_PublicPictures, None),
        ),
        ("CommonVideos", Folder::Known(FOLDERID_PublicVideos, None)),
        ("Resources", Folder::Known(FOLDERID_ResourceDir, None)),
        (
            "LocalizedResources",
            Folder::Known(FOLDERID_LocalizedResourcesDir, None),
        ),
        (
            "CommonOemLinks",
            Folder::Known(FOLDERID_CommonOEMLinks, None),
        ),
        ("CDBurning", Folder::Known(FOLDERID_CDBurning, None)),
    ]
};

/// The home folder: `$HOME`, or "/" like .NET falls back to.
#[cfg(not(windows))]
fn home_directory() -> String {
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .unwrap_or_else(|| "/".to_string())
}

/// `Environment.GetFolderPath(SpecialFolder)` by the folder's name (case ignored), with
/// the meanings of `SPECIAL_FOLDERS`; "" for a folder this system does not have,
/// `None` for a name that is not a special folder.
pub fn special_folder_path(name: &str) -> Option<String> {
    let (_, folder) = SPECIAL_FOLDERS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))?;
    Some(folder_path(*folder))
}

#[cfg(not(windows))]
fn folder_path(folder: Folder) -> String {
    match folder {
        Folder::Home("") => home_directory(),
        Folder::Home(relative) => path::combine(&home_directory(), relative),
        Folder::Fixed(path) => path.to_string(),
        Folder::Nothing => String::new(),
    }
}

#[cfg(windows)]
fn folder_path(folder: Folder) -> String {
    use crate::folders::known;
    match folder {
        Folder::System => known::system_directory(),
        Folder::Known(id, fallback) => known::path(&id)
            .or_else(|| fallback.and_then(|name| std::env::var(name).ok()))
            .unwrap_or_default(),
    }
}

static CUSTOM_SPECIAL_FOLDERS: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

/// `HelpersOptions.ShareXSpecialFolders`: the app's own `%Name%` folders (upstream has
/// one, "ShareXImageEffects"), used where upstream passes `supportCustomSpecialFolders`.
pub fn set_custom_special_folders(folders: Vec<(String, String)>) {
    if let Ok(mut slot) = CUSTOM_SPECIAL_FOLDERS.write() {
        *slot = folders;
    }
}

fn custom_special_folders() -> Vec<(String, String)> {
    CUSTOM_SPECIAL_FOLDERS
        .read()
        .map(|folders| folders.clone())
        .unwrap_or_default()
}

/// `ExpandFolderVariables`: `%Name%` of the app's own folders (when asked) and of the
/// special folders, case ignored, then environment variables (`%USERPROFILE%`,
/// `%HOME%`). A special folder is looked up only when its name is in the path, which
/// gives what upstream's asking for every one gives.
pub fn expand_folder_variables(path: &str, support_custom_special_folders: bool) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut path = path.to_string();
    if support_custom_special_folders {
        for (name, folder) in custom_special_folders() {
            path = replace_ignore_case(&path, &format!("%{name}%"), &folder);
        }
    }
    for (name, folder) in SPECIAL_FOLDERS {
        let token = format!("%{name}%");
        if find_ignore_case(&path, &token).is_some() {
            path = replace_ignore_case(&path, &token, &folder_path(folder));
        }
    }
    expand_environment_variables(&path)
}

/// `Environment.ExpandEnvironmentVariables` as .NET does it on Unix: `%NAME%` becomes
/// the variable's value when it is set (names are case sensitive), anything else stays
/// as written, and the closing `%` of an unknown name may open the next one.
#[cfg(not(windows))]
pub fn expand_environment_variables(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    while last < text.len() {
        // .NET searches from the unit after `last`; here from the character after it.
        let next = last + text[last..].chars().next().map_or(1, char::len_utf8);
        let Some(pos) = text[next..].find('%').map(|i| next + i) else {
            break;
        };
        if text.as_bytes()[last] == b'%' {
            let key = &text[last + 1..pos];
            if let Some(value) = environment_variable(key) {
                result.push_str(&value);
                last = pos + 1;
                continue;
            }
        }
        result.push_str(&text[last..pos]);
        last = pos;
    }
    result.push_str(&text[last..]);
    result
}

#[cfg(not(windows))]
fn environment_variable(key: &str) -> Option<String> {
    if key.is_empty() || key.contains(['=', '\0']) {
        return None;
    }
    std::env::var_os(key).map(|value| value.to_string_lossy().into_owned())
}

/// `Environment.ExpandEnvironmentVariables` as .NET does it on Windows: the system's
/// `ExpandEnvironmentStringsW`, so `%NAME%` is looked up without case and an unknown
/// name stays as written. When the system refuses the text it comes back unchanged
/// (upstream catches .NET's exception and keeps the path as it was).
#[cfg(windows)]
pub fn expand_environment_variables(text: &str) -> String {
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    if text.is_empty() {
        return String::new();
    }
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut buffer = vec![0u16; wide.len().max(128)];
    loop {
        // SAFETY: `wide` ends in NUL and the buffer's length is passed with it.
        let length = unsafe {
            ExpandEnvironmentStringsW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32)
        } as usize;
        if length == 0 {
            return text.to_string();
        }
        if length <= buffer.len() {
            // The length counts the terminator.
            return String::from_utf16_lossy(&buffer[..length - 1]);
        }
        buffer.resize(length, 0);
    }
}

/// `string.Replace(old, new, StringComparison.OrdinalIgnoreCase)`.
fn replace_ignore_case(text: &str, old: &str, new: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((start, end)) = find_ignore_case(rest, old) {
        out.push_str(&rest[..start]);
        out.push_str(new);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The first match of `needle` in `haystack` as a byte range, comparing characters by
/// their simple upper case the way `OrdinalIgnoreCase` does.
fn find_ignore_case(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    for (start, _) in haystack.char_indices() {
        let mut candidate = haystack[start..].char_indices();
        let mut end = start;
        let mut matched = true;
        for wanted in needle.chars() {
            match candidate.next() {
                Some((offset, c)) if ordinal_upper(c) == ordinal_upper(wanted) => {
                    end = start + offset + c.len_utf8();
                }
                _ => {
                    matched = false;
                    break;
                }
            }
        }
        if matched {
            return Some((start, end));
        }
    }
    None
}

/// The simple upper case .NET's ordinal comparisons use; they leave the dotless i and
/// the long s alone so they never equal ASCII letters.
fn ordinal_upper(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    if c == '\u{131}' || c == '\u{17F}' {
        return c;
    }
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// `AppDomain.CurrentDomain.BaseDirectory`: the folder of the running program (inside
/// a macOS bundle, `Contents/MacOS/`; on Windows the exe's folder), ending in a
/// separator.
pub fn base_directory() -> String {
    let root = if cfg!(windows) { "\\" } else { "/" };
    let folder = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|parent| parent.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| root.to_string());
    if folder.ends_with(is_directory_separator) {
        folder
    } else {
        format!("{folder}{}", std::path::MAIN_SEPARATOR)
    }
}

/// `GetAbsolutePath`: folder variables expanded, a relative path taken from the
/// program's folder (not the working folder), `.` and `..` resolved without looking at
/// the disk.
pub fn get_absolute_path(path: &str) -> String {
    let path = expand_folder_variables(path, false);
    let path = if path::is_path_rooted(&path) {
        path
    } else {
        path::combine(&base_directory(), &path)
    };
    path::get_full_path(&path)
}

/// `System.IO.Path` as .NET implements it on Windows: `\` and `/` both separate (`\` is
/// the one written), and a path's root is a drive (`C:`, `C:\`), a leading separator, a
/// UNC share (`\\server\share\`) or a device path (`\\?\`, `\\.\`), as
/// `PathInternal.GetRootLength` reads them.
#[cfg(windows)]
pub mod path {
    /// `PathInternal.IsDirectorySeparator`.
    fn is_separator(c: u8) -> bool {
        c == b'\\' || c == b'/'
    }

    /// `PathInternal.IsValidDriveChar`.
    fn is_drive_letter(c: u8) -> bool {
        c.is_ascii_alphabetic()
    }

    /// `PathInternal.IsExtended`: `\\?\` (or `\??\`), never normalised.
    fn is_extended(path: &[u8]) -> bool {
        path.len() >= 4
            && path[0] == b'\\'
            && (path[1] == b'\\' || path[1] == b'?')
            && path[2] == b'?'
            && path[3] == b'\\'
    }

    /// `PathInternal.IsDevice`: `\\?\`, `\\.\` and the extended form.
    fn is_device(path: &[u8]) -> bool {
        is_extended(path)
            || (path.len() >= 4
                && is_separator(path[0])
                && is_separator(path[1])
                && (path[2] == b'.' || path[2] == b'?')
                && is_separator(path[3]))
    }

    /// `PathInternal.IsDeviceUNC`: `\\?\UNC\`.
    fn is_device_unc(path: &[u8]) -> bool {
        path.len() >= 8
            && is_device(path)
            && is_separator(path[7])
            && path[4] == b'U'
            && path[5] == b'N'
            && path[6] == b'C'
    }

    /// `PathInternal.GetRootLength`: how much of the path is its root.
    pub(crate) fn root_length(path: &str) -> usize {
        let bytes = path.as_bytes();
        let length = bytes.len();
        let device = is_device(bytes);
        let device_unc = device && is_device_unc(bytes);
        if (!device || device_unc) && length > 0 && is_separator(bytes[0]) {
            if device_unc || (length > 1 && is_separator(bytes[1])) {
                // UNC: past the server and the share.
                let mut i = if device_unc { 8 } else { 2 };
                let mut separators = 2;
                while i < length {
                    if is_separator(bytes[i]) {
                        separators -= 1;
                        if separators == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                i
            } else {
                // The current drive's root ("\foo").
                1
            }
        } else if device {
            // A device ("\\?\C:\", "\\.\pipe\"): up to and with the first separator after
            // at least one other character.
            let mut i = 4;
            while i < length && !is_separator(bytes[i]) {
                i += 1;
            }
            if i < length && i > 4 && is_separator(bytes[i]) {
                i += 1;
            }
            i
        } else if length >= 2 && bytes[1] == b':' && is_drive_letter(bytes[0]) {
            if length > 2 && is_separator(bytes[2]) {
                3
            } else {
                2
            }
        } else {
            0
        }
    }

    /// `Path.IsPathRooted`: a leading separator or a drive ("C:", relative to that
    /// drive's folder, counts too).
    pub fn is_path_rooted(path: &str) -> bool {
        let bytes = path.as_bytes();
        (!bytes.is_empty() && is_separator(bytes[0]))
            || (bytes.len() >= 2 && bytes[1] == b':' && is_drive_letter(bytes[0]))
    }

    /// `Path.GetFileName`: what follows the last separator, never part of the root
    /// ("C:foo" is "foo").
    pub fn get_file_name(path: &str) -> &str {
        let root = root_length(path);
        match path.bytes().rposition(is_separator) {
            Some(i) if i >= root => &path[i + 1..],
            _ => &path[root..],
        }
    }

    /// `Path.GetFileNameWithoutExtension`: the file name up to its last dot.
    pub fn get_file_name_without_extension(path: &str) -> &str {
        let name = get_file_name(path);
        name.rfind('.').map_or(name, |pos| &name[..pos])
    }

    /// `Path.GetExtension`: from the last dot after the last separator, "" when there
    /// is none or the path ends with the dot.
    pub fn get_extension(path: &str) -> &str {
        let bytes = path.as_bytes();
        for i in (0..bytes.len()).rev() {
            if bytes[i] == b'.' {
                return if i + 1 < bytes.len() { &path[i..] } else { "" };
            }
            if is_separator(bytes[i]) {
                break;
            }
        }
        ""
    }

    /// `Path.GetDirectoryName`: the path up to its last separator (separators after the
    /// root folded into one `\`), `None` for "", blanks and a bare root.
    pub fn get_directory_name(path: &str) -> Option<String> {
        if path.bytes().all(|b| b == b' ') {
            return None;
        }
        let bytes = path.as_bytes();
        let root = root_length(path);
        let mut end = bytes.len();
        if end <= root {
            return None;
        }
        while end > root {
            end -= 1;
            if is_separator(bytes[end]) {
                break;
            }
        }
        while end > root && is_separator(bytes[end - 1]) {
            end -= 1;
        }
        Some(normalize_separators(&path[..end]))
    }

    /// `PathInternal.NormalizeDirectorySeparators`: every separator a `\`, runs of them
    /// one, except the two a UNC path starts with.
    fn normalize_separators(path: &str) -> String {
        let mut out = String::with_capacity(path.len());
        let mut chars = path.chars().peekable();
        if let Some(&first) = chars.peek()
            && (first == '\\' || first == '/')
        {
            chars.next();
            out.push('\\');
        }
        while let Some(c) = chars.next() {
            if c == '\\' || c == '/' {
                if chars
                    .peek()
                    .is_some_and(|next| *next == '\\' || *next == '/')
                {
                    continue;
                }
                out.push('\\');
            } else {
                out.push(c);
            }
        }
        out
    }

    /// `Path.Combine(first, second)`: `second` alone when it is rooted, a `\` between
    /// the two unless one already has a separator there.
    pub fn combine(first: &str, second: &str) -> String {
        if first.is_empty() {
            return second.to_string();
        }
        if second.is_empty() {
            return first.to_string();
        }
        if is_path_rooted(second) {
            return second.to_string();
        }
        let separated = first.ends_with(['\\', '/']) || second.starts_with(['\\', '/']);
        if separated {
            format!("{first}{second}")
        } else {
            format!("{first}\\{second}")
        }
    }

    /// `Path.GetFullPath`: the system's `GetFullPathNameW` (through
    /// `std::path::absolute`): a relative path taken from the working folder or its
    /// drive's, `/` turned into `\`, `.` and `..` removed and trailing dots and blanks
    /// of the last name dropped, without touching the disk. A `\\?\` path is returned
    /// as it is. An empty path is the working folder (.NET throws).
    pub fn get_full_path(path: &str) -> String {
        if path.is_empty() {
            return std::env::current_dir()
                .map(|folder| folder.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if is_extended(path.as_bytes()) {
            return path.to_string();
        }
        std::path::absolute(path)
            .map(|full| full.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_string())
    }
}

/// `System.IO.Path` as .NET implements it on Unix, where `/` is the only separator.
#[cfg(not(windows))]
pub mod path {
    /// `Path.IsPathRooted`.
    pub fn is_path_rooted(path: &str) -> bool {
        path.starts_with('/')
    }

    /// `Path.GetFileName`: what follows the last `/`.
    pub fn get_file_name(path: &str) -> &str {
        path.rfind('/').map_or(path, |pos| &path[pos + 1..])
    }

    /// `Path.GetFileNameWithoutExtension`: the file name up to its last dot (".bashrc"
    /// has an empty name).
    pub fn get_file_name_without_extension(path: &str) -> &str {
        let name = get_file_name(path);
        name.rfind('.').map_or(name, |pos| &name[..pos])
    }

    /// `Path.GetExtension`: from the last dot of the file name, "" when there is none or
    /// the name ends with the dot.
    pub fn get_extension(path: &str) -> &str {
        let name = get_file_name(path);
        match name.rfind('.') {
            Some(pos) if pos + 1 < name.len() => &name[pos..],
            _ => "",
        }
    }

    /// `Path.GetDirectoryName`: the path up to its last `/` (repeated separators folded),
    /// "" for a bare name, `None` for "" and "/".
    pub fn get_directory_name(path: &str) -> Option<String> {
        if path.is_empty() {
            return None;
        }
        let bytes = path.as_bytes();
        let root = usize::from(is_path_rooted(path));
        let mut end = bytes.len();
        if end <= root {
            return None;
        }
        while end > root {
            end -= 1;
            if bytes[end] == b'/' {
                break;
            }
        }
        while end > root && bytes[end - 1] == b'/' {
            end -= 1;
        }
        let mut folder = String::with_capacity(end);
        let mut previous_separator = false;
        for c in path[..end].chars() {
            if c == '/' && previous_separator {
                continue;
            }
            previous_separator = c == '/';
            folder.push(c);
        }
        Some(folder)
    }

    /// `Path.Combine(first, second)`: `second` alone when it is rooted, one separator
    /// between the two otherwise.
    pub fn combine(first: &str, second: &str) -> String {
        if first.is_empty() {
            return second.to_string();
        }
        if second.is_empty() {
            return first.to_string();
        }
        if is_path_rooted(second) {
            return second.to_string();
        }
        if first.ends_with('/') {
            format!("{first}{second}")
        } else {
            format!("{first}/{second}")
        }
    }

    /// `Path.GetFullPath`: a relative path taken from the working folder, then `.`,
    /// `..` and doubled separators removed without touching the disk (symbolic links
    /// stay as they are). A trailing separator stays. An empty path is the working
    /// folder (.NET throws).
    pub fn get_full_path(path: &str) -> String {
        let path = if is_path_rooted(path) {
            path.to_string()
        } else {
            let working = std::env::current_dir()
                .map(|folder| folder.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "/".to_string());
            combine(&working, path)
        };
        let collapsed = remove_relative_segments(&path);
        if collapsed.is_empty() {
            "/".to_string()
        } else {
            collapsed
        }
    }

    /// `PathInternal.RemoveRelativeSegments` for a rooted Unix path. The root "/" is
    /// handled inside the loop, as .NET does for a root that ends in a separator.
    fn remove_relative_segments(path: &str) -> String {
        let bytes = path.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'/' && i + 1 < bytes.len() {
                if bytes[i + 1] == b'/' {
                    i += 1;
                    continue;
                }
                if (i + 2 == bytes.len() || bytes[i + 2] == b'/') && bytes[i + 1] == b'.' {
                    i += 2;
                    continue;
                }
                if i + 2 < bytes.len()
                    && (i + 3 == bytes.len() || bytes[i + 3] == b'/')
                    && bytes[i + 1] == b'.'
                    && bytes[i + 2] == b'.'
                {
                    // Back to the previous separator; "/a/.." at the very end keeps "/".
                    match out.iter().rposition(|b| *b == b'/') {
                        Some(0) if i + 3 >= bytes.len() => out.truncate(1),
                        Some(s) => out.truncate(s),
                        None => out.clear(),
                    }
                    i += 3;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
        // Only whole ASCII segments were dropped, so the bytes are still UTF-8.
        String::from_utf8_lossy(&out).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(get_file_name_extension("shot.PNG", false), "PNG");
        assert_eq!(get_file_name_extension("shot.png", true), ".png");
        assert_eq!(get_file_name_extension("archive.tar.gz", false), "tar.gz");
        assert_eq!(get_file_name_extension("archive.TAR.gz", true), ".TAR.gz");
        assert_eq!(
            get_single_file_name_extension("archive.tar.gz", false),
            "gz"
        );
        assert_eq!(get_file_name_extension("notes.old.txt", false), "txt");
        assert_eq!(get_file_name_extension("noext", true), "");
        assert_eq!(get_file_name_extension("", true), "");
        assert_eq!(get_file_name_extension("trailing.", false), "");
        assert_eq!(get_file_name_extension("trailing.", true), ".");
        assert_eq!(get_file_name_extension("v1.2/readme", false), "2/readme");
        assert_eq!(get_file_name_extension(".bashrc", false), "bashrc");
    }

    #[test]
    fn changing_and_appending_extensions() {
        assert_eq!(change_file_name_extension("shot.png", "jpg"), "shot.jpg");
        assert_eq!(change_file_name_extension("shot.png", ".jpg"), "shot.jpg");
        assert_eq!(
            change_file_name_extension("shot.png", "a.b.webp"),
            "shot.webp"
        );
        assert_eq!(change_file_name_extension("shot", "jpg"), "shot.jpg");
        assert_eq!(change_file_name_extension("shot.png", ""), "shot");
        assert_eq!(change_file_name_extension("", "jpg"), "");
        assert_eq!(append_extension("shot", "png"), "shot.png");
        assert_eq!(append_extension("shot..", "..png"), "shot.png");
        assert_eq!(append_extension("shot.png", "png"), "shot.png.png");
    }

    #[test]
    fn file_kinds() {
        assert!(is_image_file("a.JPG"));
        assert!(is_image_file("/x/y.tiff"));
        assert!(!is_image_file("a.webp"));
        assert!(is_text_file("notes.txt"));
        assert!(is_text_file("main.go"));
        assert!(!is_text_file("txt"));
        assert!(!is_text_file("archive.tar.txt"));
        assert!(is_video_file("clip.MOV"));
        assert!(!is_video_file("song.mp3"));
        assert!(check_extension("a.tar.gz", &["tar.gz"]));
        assert!(!check_extension("a.tar.gz", &["gz"]));
        assert!(!check_extension("", &[""]));
        assert_eq!(TEXT_FILE_EXTENSIONS.len(), 30);
        assert_eq!(VIDEO_FILE_EXTENSIONS.len(), 21);
    }

    #[test]
    fn sanitizing() {
        assert_eq!(sanitize_file_name("  a/b\0c.png \t"), "abc.png");
        assert_eq!(sanitize_file_name_with("a/b", "_"), "a_b");
        #[cfg(not(windows))]
        assert_eq!(sanitize_file_name("a:b*?\"<>|"), "a:b*?\"<>|");
        assert_eq!(sanitize_path(" /Users/a/b\0c/ "), "/Users/a/bc/");
        assert_eq!(sanitize_path_with("a/\0b", "_"), "a/_b");
        assert_eq!(sanitize_path("C:\\x\0y"), "C:\\xy");
        assert_eq!(get_path_root("C:\\Users"), "C:\\");
        assert_eq!(get_path_root(":\\x"), "");
        assert_eq!(get_path_root("/Users"), "");
    }

    #[test]
    fn file_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("five.bin");
        std::fs::write(&file, b"12345").unwrap();
        assert_eq!(get_file_size(&file.to_string_lossy()), 5);
        assert_eq!(get_file_size(&dir.path().to_string_lossy()), -1);
        assert_eq!(
            get_file_size(&dir.path().join("none").to_string_lossy()),
            -1
        );
        assert_eq!(get_file_size(""), -1);
    }

    #[test]
    fn unique_paths_count_from_two() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        assert_eq!(get_unique_file_path(&at("shot.png")), at("shot.png"));
        std::fs::write(at("shot.png"), b"").unwrap();
        assert_eq!(get_unique_file_path(&at("shot.png")), at("shot (2).png"));
        std::fs::write(at("shot (2).png"), b"").unwrap();
        assert_eq!(get_unique_file_path(&at("shot.png")), at("shot (3).png"));
        // A name that already counts goes on from its number.
        std::fs::write(at("shot (7).png"), b"").unwrap();
        assert_eq!(
            get_unique_file_path(&at("shot (7).png")),
            at("shot (8).png")
        );
        // A folder in the way is not a file.
        std::fs::create_dir(at("folder.png")).unwrap();
        assert_eq!(get_unique_file_path(&at("folder.png")), at("folder.png"));
        // Without an extension, and with only an extension.
        std::fs::write(at("README"), b"").unwrap();
        assert_eq!(get_unique_file_path(&at("README")), at("README (2)"));
        std::fs::write(at(".hidden"), b"").unwrap();
        assert_eq!(get_unique_file_path(&at(".hidden")), at(" (2).hidden"));
        // The trailing dot is lost, like Path.GetExtension loses it.
        std::fs::write(at("dot."), b"").unwrap();
        assert_eq!(get_unique_file_path(&at("dot.")), at("dot (2)"));
    }

    #[test]
    fn counters_in_names() {
        assert_eq!(split_counter("a (3)"), Some(("a".to_string(), 3)));
        assert_eq!(split_counter("a (1) (12)"), Some(("a (1)".to_string(), 12)));
        assert_eq!(split_counter("a (007)"), Some(("a".to_string(), 7)));
        assert_eq!(split_counter("a (3)\n"), Some(("a".to_string(), 3)));
        // `(.+)` needs one character before " (".
        assert_eq!(split_counter("  (3)"), Some((" ".to_string(), 3)));
        assert_eq!(split_counter(" (3)"), None);
        assert_eq!(split_counter("(3)"), None);
        assert_eq!(split_counter("a(3)"), None);
        assert_eq!(split_counter("a ()"), None);
        assert_eq!(split_counter("a (x)"), None);
        assert_eq!(split_counter("a\nb (3)"), None);
        assert_eq!(split_counter("a (99999999999)"), None);
        assert_eq!(split_counter("a (٣)"), None);
    }

    #[test]
    fn file_exists_like_dotnet() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        let file = file.to_string_lossy().into_owned();
        assert!(file_exists(&file));
        assert!(!file_exists(&format!("{file}/")));
        assert!(!file_exists(&dir.path().to_string_lossy()));
        assert!(!file_exists(""));
        assert!(!file_exists("a\0b"));
        #[cfg(unix)]
        {
            let dangling = dir.path().join("dangling");
            std::os::unix::fs::symlink(dir.path().join("gone"), &dangling).unwrap();
            assert!(file_exists(&dangling.to_string_lossy()));
            let to_folder = dir.path().join("to-folder");
            std::os::unix::fs::symlink(dir.path(), &to_folder).unwrap();
            assert!(!file_exists(&to_folder.to_string_lossy()));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn special_folders_map_to_macos() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_folder_variables("%MyPictures%/Modlight", false),
            format!("{home}/Pictures/Modlight")
        );
        assert_eq!(
            expand_folder_variables("%mypictures%|%MYDOCUMENTS%|%Personal%", false),
            format!("{home}/Pictures|{home}/Documents|{home}/Documents")
        );
        assert_eq!(expand_folder_variables("%UserProfile%", false), home);
        assert_eq!(
            expand_folder_variables("%Desktop%", false),
            format!("{home}/Desktop")
        );
        assert_eq!(
            expand_folder_variables("%MyVideos%", false),
            format!("{home}/Movies")
        );
        assert_eq!(
            expand_folder_variables("%LocalApplicationData%/Modlight", false),
            format!("{home}/Library/Application Support/Modlight")
        );
        assert_eq!(
            expand_folder_variables("%ProgramFiles%", false),
            "/Applications"
        );
        // Windows-only folders expand to nothing, like .NET on macOS.
        assert_eq!(expand_folder_variables("%StartMenu%/x", false), "/x");
        assert_eq!(special_folder_path("startmenu"), Some(String::new()));
        assert_eq!(special_folder_path("NotAFolder"), None);
        assert_eq!(expand_folder_variables("", false), "");
        assert_eq!(expand_folder_variables("plain/path", false), "plain/path");
        // Every SpecialFolder name is listed once (MyDocuments and Personal share a value).
        let names: std::collections::HashSet<&str> =
            SPECIAL_FOLDERS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), 47);
    }

    #[test]
    fn custom_folders_only_when_asked() {
        set_custom_special_folders(vec![("ShareXImageEffects".to_string(), "/fx".to_string())]);
        assert_eq!(
            expand_folder_variables("%sharexImageEffects%/a.sxie", true),
            "/fx/a.sxie"
        );
        assert_eq!(
            expand_folder_variables("%ShareXImageEffects%/a.sxie", false),
            "%ShareXImageEffects%/a.sxie"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn environment_variables_like_dotnet_on_unix() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_environment_variables("%HOME%/x"),
            format!("{home}/x")
        );
        assert_eq!(
            expand_environment_variables("a%HOME%b"),
            format!("a{home}b")
        );
        // Names are case sensitive on Unix.
        assert_eq!(expand_environment_variables("%home%"), "%home%");
        assert_eq!(
            expand_environment_variables("%MODLIGHT_SURELY_UNSET%"),
            "%MODLIGHT_SURELY_UNSET%"
        );
        // The closing % of an unknown name opens the next one.
        assert_eq!(
            expand_environment_variables("%NOPE%HOME%"),
            format!("%NOPE{home}")
        );
        assert_eq!(expand_environment_variables("%%HOME%"), format!("%{home}"));
        assert_eq!(expand_environment_variables("100% sure"), "100% sure");
        assert_eq!(expand_environment_variables("%"), "%");
        assert_eq!(
            expand_environment_variables("żółw%HOME%"),
            format!("żółw{home}")
        );
        assert_eq!(expand_environment_variables(""), "");
    }

    #[test]
    fn ignore_case_matching() {
        assert_eq!(replace_ignore_case("A%x%b%X%", "%x%", "-"), "A-b-");
        assert_eq!(replace_ignore_case("Żółw", "żÓŁ", "x"), "xw");
        // OrdinalIgnoreCase leaves the dotless i and the long s alone.
        assert_eq!(replace_ignore_case("%ıx%", "%ix%", "-"), "%ıx%");
        assert_eq!(replace_ignore_case("abc", "", "-"), "abc");
    }

    #[cfg(not(windows))]
    #[test]
    fn absolute_paths() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(get_absolute_path("/a/b/../c"), "/a/c");
        assert_eq!(
            get_absolute_path("%UserProfile%/x/./y"),
            format!("{home}/x/y")
        );
        let base = base_directory();
        assert!(base.ends_with('/'));
        assert_eq!(
            get_absolute_path("ffmpeg"),
            path::get_full_path(&format!("{base}ffmpeg"))
        );
        assert_eq!(get_absolute_path(""), path::get_full_path(&base));
    }

    #[cfg(not(windows))]
    #[test]
    fn paths_like_system_io_path() {
        use path::*;
        assert_eq!(get_file_name("/a/b.txt"), "b.txt");
        assert_eq!(get_file_name("b.txt"), "b.txt");
        assert_eq!(get_file_name("/a/"), "");
        assert_eq!(get_file_name_without_extension("/a/b.tar.gz"), "b.tar");
        assert_eq!(get_file_name_without_extension("/a.d/b"), "b");
        assert_eq!(get_file_name_without_extension(".bashrc"), "");
        assert_eq!(get_extension("/a/b.png"), ".png");
        assert_eq!(get_extension("/a.d/b"), "");
        assert_eq!(get_extension("b."), "");
        assert_eq!(get_directory_name("/a/b.txt").as_deref(), Some("/a"));
        assert_eq!(get_directory_name("/a//b//c.txt").as_deref(), Some("/a/b"));
        assert_eq!(get_directory_name("/b.txt").as_deref(), Some("/"));
        assert_eq!(get_directory_name("b.txt").as_deref(), Some(""));
        assert_eq!(get_directory_name("a/").as_deref(), Some("a"));
        assert_eq!(get_directory_name("//a").as_deref(), Some("/"));
        assert_eq!(get_directory_name("/"), None);
        assert_eq!(get_directory_name(""), None);
        assert_eq!(combine("/a", "b"), "/a/b");
        assert_eq!(combine("/a/", "b"), "/a/b");
        assert_eq!(combine("/a", "/b"), "/b");
        assert_eq!(combine("", "b"), "b");
        assert_eq!(combine("/a", ""), "/a");
        assert_eq!(get_full_path("/a/b/../c"), "/a/c");
        assert_eq!(get_full_path("/a/./b//c/"), "/a/b/c/");
        assert_eq!(get_full_path("/a/.."), "/");
        assert_eq!(get_full_path("/.."), "/");
        assert_eq!(get_full_path("/../a"), "/a");
        assert_eq!(get_full_path("/a/b/.."), "/a");
        assert_eq!(get_full_path("/a/..b"), "/a/..b");
        assert_eq!(get_full_path("/ż/./ł"), "/ż/ł");
        let working = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(get_full_path("x"), format!("{working}/x"));
    }

    #[cfg(windows)]
    #[test]
    fn sanitizing_like_dotnet_on_windows() {
        assert_eq!(
            sanitize_file_name("a:b*?\"<>|\\/c\u{1}\u{1f}.png"),
            "abc.png"
        );
        assert_eq!(
            sanitize_file_name_with("12:30 <draft>", "_"),
            "12_30 _draft_"
        );
        assert_eq!(INVALID_FILE_NAME_CHARS.len(), 41);
        // Both separators stay in a path, the drive's colon too; any other colon goes.
        assert_eq!(sanitize_path("C:\\Users\\a:b/c*d\\"), "C:\\Users\\ab/cd\\");
        assert_eq!(sanitize_path_with("x\\y?z", "_"), "x\\y_z");
    }

    #[cfg(windows)]
    #[test]
    fn special_folders_are_the_known_folders() {
        use crate::folders::known;
        use windows_sys::Win32::UI::Shell::{FOLDERID_Pictures, FOLDERID_Profile};
        let profile = known::path(&FOLDERID_Profile).unwrap();
        let pictures = known::path(&FOLDERID_Pictures).unwrap();
        assert_eq!(
            expand_folder_variables("%MyPictures%\\Modlight", false),
            format!("{pictures}\\Modlight")
        );
        assert_eq!(
            expand_folder_variables("%userprofile%|%USERPROFILE%", false),
            format!("{profile}|{profile}")
        );
        let documents = known::path(&known::DOCUMENTS).unwrap();
        assert_eq!(
            expand_folder_variables("%MyDocuments%|%personal%", false),
            format!("{documents}|{documents}")
        );
        // The Windows-only folders exist here.
        let windows = special_folder_path("Windows").unwrap();
        assert!(!windows.is_empty() && std::path::Path::new(&windows).is_dir());
        let system = special_folder_path("system").unwrap();
        assert!(
            system.to_ascii_lowercase().ends_with("system32"),
            "{system}"
        );
        assert!(
            std::path::Path::new(&special_folder_path("StartMenu").unwrap()).is_dir(),
            "the Start menu"
        );
        assert!(
            std::path::Path::new(&special_folder_path("Programs").unwrap()).is_dir(),
            "the Start menu's programs"
        );
        assert!(
            std::path::Path::new(&special_folder_path("CommonApplicationData").unwrap()).is_dir(),
            "ProgramData"
        );
        #[cfg(target_arch = "x86_64")]
        assert!(
            std::path::Path::new(&special_folder_path("ProgramFilesX86").unwrap()).is_dir(),
            "Program Files (x86)"
        );
        // Virtual folders have no path, as .NET answers.
        assert_eq!(special_folder_path("MyComputer"), Some(String::new()));
        assert_eq!(special_folder_path("PrinterShortcuts"), Some(String::new()));
        assert_eq!(special_folder_path("NotAFolder"), None);
        assert_eq!(expand_folder_variables("%MyComputer%x", false), "x");
        // Every SpecialFolder name is listed once (MyDocuments and Personal share a value).
        let names: std::collections::HashSet<&str> =
            SPECIAL_FOLDERS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), 47);
    }

    #[cfg(windows)]
    #[test]
    fn environment_variables_like_dotnet_on_windows() {
        let root = std::env::var("SystemRoot").unwrap();
        assert_eq!(
            expand_environment_variables("%SystemRoot%\\x"),
            format!("{root}\\x")
        );
        // Names are compared without case on Windows.
        assert_eq!(expand_environment_variables("%systemroot%"), root);
        assert_eq!(
            expand_environment_variables("%MODLIGHT_SURELY_UNSET%"),
            "%MODLIGHT_SURELY_UNSET%"
        );
        assert_eq!(expand_environment_variables("100% sure"), "100% sure");
        assert_eq!(expand_environment_variables("%"), "%");
        assert_eq!(
            expand_environment_variables("żółw%SystemRoot%"),
            format!("żółw{root}")
        );
        assert_eq!(expand_environment_variables(""), "");
        // Longer than the first buffer.
        let long = "x".repeat(1000);
        assert_eq!(
            expand_environment_variables(&format!("{long}%SystemRoot%")),
            format!("{long}{root}")
        );
    }

    #[cfg(windows)]
    #[test]
    fn absolute_paths_on_windows() {
        let profile = std::env::var("USERPROFILE").unwrap();
        assert_eq!(get_absolute_path("C:\\a\\b\\..\\c"), "C:\\a\\c");
        assert_eq!(get_absolute_path("C:/a/./b"), "C:\\a\\b");
        assert_eq!(
            get_absolute_path("%UserProfile%\\x\\.\\y"),
            format!("{profile}\\x\\y")
        );
        let base = base_directory();
        assert!(base.ends_with('\\'), "{base}");
        assert_eq!(
            get_absolute_path("ffmpeg.exe"),
            path::get_full_path(&format!("{base}ffmpeg.exe"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn paths_like_system_io_path_on_windows() {
        use path::*;
        assert!(is_path_rooted("C:\\a"));
        assert!(is_path_rooted("C:a"));
        assert!(is_path_rooted("\\a"));
        assert!(is_path_rooted("/a"));
        assert!(is_path_rooted("\\\\server\\share"));
        assert!(!is_path_rooted("a\\b"));
        assert!(!is_path_rooted("1:\\a"));
        assert_eq!(root_length("C:\\a"), 3);
        assert_eq!(root_length("C:a"), 2);
        assert_eq!(root_length("\\a"), 1);
        assert_eq!(root_length("\\\\server\\share\\a"), 14);
        assert_eq!(root_length("\\\\?\\C:\\a"), 7);
        assert_eq!(root_length("\\\\?\\UNC\\server\\share\\a"), 20);
        assert_eq!(get_file_name("C:\\a\\b.txt"), "b.txt");
        assert_eq!(get_file_name("C:/a/b.txt"), "b.txt");
        assert_eq!(get_file_name("C:b.txt"), "b.txt");
        assert_eq!(get_file_name("C:\\"), "");
        assert_eq!(get_file_name("b.txt"), "b.txt");
        assert_eq!(get_file_name_without_extension("C:\\a\\b.tar.gz"), "b.tar");
        assert_eq!(get_file_name_without_extension("C:\\a.d\\b"), "b");
        assert_eq!(get_extension("C:\\a\\b.png"), ".png");
        assert_eq!(get_extension("C:\\a.d\\b"), "");
        assert_eq!(get_extension("C:/a.d/b"), "");
        assert_eq!(get_extension("b."), "");
        assert_eq!(get_directory_name("C:\\a\\b.txt").as_deref(), Some("C:\\a"));
        assert_eq!(
            get_directory_name("C:/a//b/c.txt").as_deref(),
            Some("C:\\a\\b")
        );
        assert_eq!(get_directory_name("C:\\b.txt").as_deref(), Some("C:\\"));
        assert_eq!(get_directory_name("C:\\"), None);
        assert_eq!(get_directory_name("C:"), None);
        assert_eq!(get_directory_name("b.txt").as_deref(), Some(""));
        assert_eq!(get_directory_name("   "), None);
        assert_eq!(get_directory_name(""), None);
        assert_eq!(
            get_directory_name("\\\\server\\share\\a.txt").as_deref(),
            Some("\\\\server\\share")
        );
        assert_eq!(get_directory_name("\\\\server\\share"), None);
        assert_eq!(combine("C:\\a", "b"), "C:\\a\\b");
        assert_eq!(combine("C:\\a\\", "b"), "C:\\a\\b");
        assert_eq!(combine("C:/a/", "b"), "C:/a/b");
        assert_eq!(combine("C:\\a", "D:\\b"), "D:\\b");
        assert_eq!(combine("C:\\a", "\\b"), "\\b");
        assert_eq!(combine("C:", "b"), "C:\\b");
        assert_eq!(combine("", "b"), "b");
        assert_eq!(combine("C:\\a", ""), "C:\\a");
        assert_eq!(get_full_path("C:\\a\\b\\..\\c"), "C:\\a\\c");
        assert_eq!(get_full_path("C:\\a\\.\\b\\\\c\\"), "C:\\a\\b\\c\\");
        assert_eq!(get_full_path("C:\\a\\.."), "C:\\");
        assert_eq!(get_full_path("C:\\.."), "C:\\");
        assert_eq!(get_full_path("C:\\a\\name. "), "C:\\a\\name");
        assert_eq!(get_full_path("\\\\?\\C:\\a\\..\\b"), "\\\\?\\C:\\a\\..\\b");
        let working = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(get_full_path("x"), format!("{working}\\x"));
        assert_eq!(get_full_path(""), working);
    }
}

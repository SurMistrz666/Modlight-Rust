//! Windows: where the personal folder is (`Program.PersonalFolder`). `Documents\Modlight`,
//! unless `PersonalPath.cfg` names another folder: the file beside the program when there
//! is one, else the one inside `Documents\Modlight` (`PersonalPathConfigFilePath`). The
//! settings' Paths page writes it (`WritePersonalPathConfig`); the change takes effect
//! at the next start.
//!
//! A Mac keeps the folder fixed (`MACOS.md`), so none of this is built there.

use std::path::{Component, Path, PathBuf};

use modlight_core::names::file_helpers::expand_folder_variables;

/// `PersonalPathConfigFileName`.
pub const CONFIG_FILE_NAME: &str = "PersonalPath.cfg";

/// The program's own folder (`AppDomain.BaseDirectory`).
fn program_folder() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// `DefaultPersonalFolder`: `Documents\Modlight`.
pub fn default_folder() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(modlight_core::APP_NAME)
}

/// `PersonalPathConfigFilePath`: beside the program when that file exists, else inside
/// the default folder.
pub fn config_file() -> PathBuf {
    if let Some(beside) = program_folder().map(|f| f.join(CONFIG_FILE_NAME))
        && beside.is_file()
    {
        return beside;
    }
    default_folder().join(CONFIG_FILE_NAME)
}

/// `ReadPersonalPathConfig`: the folder the file names, trimmed, or nothing.
pub fn read_config() -> String {
    std::fs::read_to_string(config_file())
        .map(|text| text.trim_start_matches('\u{feff}').trim().to_string())
        .unwrap_or_default()
}

/// `FileHelpers.GetAbsolutePath`: folder variables expanded, a relative path taken from
/// the program's folder, `.` and `..` resolved without looking at the disk.
pub fn absolute(path: &str) -> PathBuf {
    let expanded = PathBuf::from(expand_folder_variables(path.trim(), false));
    let full = if expanded.is_absolute() {
        expanded
    } else {
        program_folder()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(expanded)
    };
    let mut clean = PathBuf::new();
    for part in full.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    clean
}

/// `PersonalFolder`: the folder `PersonalPath.cfg` names, else `Documents\Modlight`.
pub fn resolve() -> PathBuf {
    let custom = read_config();
    if custom.is_empty() {
        default_folder()
    } else {
        absolute(&custom)
    }
}

/// `UpdatePersonalFolderPreview`: the folder a value of the text box would mean.
pub fn preview(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        default_folder().display().to_string()
    } else {
        absolute(text).display().to_string()
    }
}

/// `WritePersonalPathConfig`: store `path` (empty for the default folder). `Ok(true)` when
/// the file changed, so the change waits for a restart; `Err` with the reason when the
/// file cannot be written.
pub fn write_config(path: &str) -> Result<bool, String> {
    let path = path.trim();
    let file = config_file();
    // Nothing stored and nothing asked for: the default stays without a file.
    if path.is_empty() && !file.exists() {
        return Ok(false);
    }
    if path.eq_ignore_ascii_case(&read_config()) {
        return Ok(false);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, path).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_modlight_in_documents() {
        let folder = default_folder();
        assert_eq!(
            folder.file_name().and_then(|n| n.to_str()),
            Some(modlight_core::APP_NAME)
        );
        assert_eq!(preview(""), folder.display().to_string());
    }

    #[test]
    fn relative_folders_start_at_the_program() {
        let program = std::env::current_exe().unwrap();
        let program = program.parent().unwrap();
        assert_eq!(absolute("Data\\..\\Profile"), program.join("Profile"));
        assert_eq!(
            absolute("D:\\Shared\\Modlight"),
            PathBuf::from("D:\\Shared\\Modlight")
        );
    }
}

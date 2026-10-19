//! Finding the executables the Download tab drives (`YtDlpTool`).
//!
//! The search order is upstream's - beside the executable, then the personal folder's
//! `Tools`, then the PATH - with the macOS spelling of "the PATH": an app opened from
//! Finder inherits only `/usr/bin:/bin:/usr/sbin:/sbin`, so the places a package manager
//! puts a tool on a Mac (Homebrew on Apple silicon and on Intel, MacPorts, pipx) are
//! searched after the inherited PATH. A file must be executable to count.

use std::path::{Path, PathBuf};

/// Where somebody sent to install yt-dlp should land.
pub const INSTALL_PAGE: &str = "https://github.com/yt-dlp/yt-dlp/wiki/Installation";

/// yt-dlp resolves the "quickjs" runtime by an executable called `qjs`.
pub const QUICKJS_FILE_NAME: &str = "qjs";

/// The folders a package manager puts tools in on a Mac.
pub const PACKAGE_MANAGER_FOLDERS: [&str; 3] =
    ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];

/// What was found, as the tab states it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolPaths {
    pub yt_dlp: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    pub quickjs: Option<PathBuf>,
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// The places searched, in order, for a file name.
#[derive(Clone, Debug, Default)]
pub struct SearchPlaces {
    /// The folder the app's executable is in.
    pub beside: Option<PathBuf>,
    /// The personal folder (`<profile>`), whose `Tools` is searched.
    pub profile: PathBuf,
    /// The PATH, then the package managers' folders.
    pub path: Vec<PathBuf>,
}

impl SearchPlaces {
    /// This process's places.
    pub fn for_profile(profile: &Path) -> SearchPlaces {
        let mut path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        for folder in PACKAGE_MANAGER_FOLDERS {
            path.push(PathBuf::from(folder));
        }
        if let Some(home) = dirs_home() {
            path.push(home.join(".local/bin"));
        }
        SearchPlaces {
            beside: std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(Path::to_path_buf)),
            profile: profile.to_path_buf(),
            path,
        }
    }

    /// `Find(fileName)`.
    pub fn find(&self, file_name: &str) -> Option<PathBuf> {
        if let Some(beside) = &self.beside {
            let candidate = beside.join(file_name);
            if executable(&candidate) {
                return Some(candidate);
            }
        }
        let personal = self
            .profile
            .join(crate::components::TOOLS_FOLDER)
            .join(file_name);
        if executable(&personal) {
            return Some(personal);
        }
        self.path
            .iter()
            .filter(|dir| !dir.as_os_str().is_empty() && dir.is_absolute())
            .map(|dir| dir.join(file_name))
            .find(|candidate| executable(candidate))
    }

    /// Look for all three (`YtDlpTool.Ensure`). The bundled QuickJS first (`Tools/qjs`
    /// beside the app), then the ordinary search.
    pub fn find_tools(&self) -> ToolPaths {
        let bundled_quickjs = self
            .beside
            .as_ref()
            .map(|b| b.join("Tools").join(QUICKJS_FILE_NAME))
            .filter(|p| executable(p));
        let tools = ToolPaths {
            yt_dlp: self.find("yt-dlp"),
            ffmpeg: self.find("ffmpeg"),
            quickjs: bundled_quickjs.or_else(|| self.find(QUICKJS_FILE_NAME)),
        };
        log::info!(
            "Download: yt-dlp is {}, ffmpeg is {}, QuickJS is {}.",
            describe(&tools.yt_dlp),
            describe(&tools.ffmpeg),
            describe(&tools.quickjs)
        );
        tools
    }
}

fn describe(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "not installed".into())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_order_is_beside_then_tools_then_path() {
        let root = tempfile::tempdir().unwrap();
        let beside = root.path().join("app");
        let profile = root.path().join("profile");
        let path_dir = root.path().join("brew");
        let places = SearchPlaces {
            beside: Some(beside.clone()),
            profile: profile.clone(),
            path: vec![
                PathBuf::from(""),
                PathBuf::from("relative"),
                path_dir.clone(),
            ],
        };
        assert_eq!(places.find_tools(), ToolPaths::default());
        tool(&path_dir.join("yt-dlp"));
        assert_eq!(places.find("yt-dlp"), Some(path_dir.join("yt-dlp")));
        tool(&profile.join("Tools/yt-dlp"));
        assert_eq!(places.find("yt-dlp"), Some(profile.join("Tools/yt-dlp")));
        tool(&beside.join("yt-dlp"));
        assert_eq!(places.find("yt-dlp"), Some(beside.join("yt-dlp")));
        tool(&path_dir.join("qjs"));
        tool(&beside.join("Tools/qjs"));
        assert_eq!(places.find_tools().quickjs, Some(beside.join("Tools/qjs")));
    }

    #[test]
    fn a_file_that_cannot_run_does_not_count() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Tools")).unwrap();
        std::fs::write(root.path().join("Tools/ffmpeg"), b"x").unwrap();
        let places = SearchPlaces {
            beside: None,
            profile: root.path().to_path_buf(),
            path: Vec::new(),
        };
        assert_eq!(places.find("ffmpeg"), None);
    }
}

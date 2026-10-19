//! The YouTube sign-in's cookie jar (`NetscapeCookieFile`, `YouTubeCookies`).
//!
//! The file is a live Google login. It lives under the personal folder and nowhere else:
//! never in `ApplicationConfig.json`, never in a setting, never uploaded, never logged
//! (only the cookie count is). It is handed to yt-dlp with `--cookies` for YouTube links
//! only. On macOS the folder is created 0700 and the file written 0600, the Unix
//! spelling of the owner-only ACL upstream puts on both (yt-dlp replaces the jar through
//! a temporary file and a rename after every run, so the folder is where the rule has to
//! live).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// One cookie, in the facts the file format has room for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetscapeCookie {
    /// With the leading dot when it is a domain cookie.
    pub domain: String,
    pub path: String,
    pub name: String,
    pub value: String,
    pub secure: bool,
    /// Seconds since the epoch, or zero or less for a session cookie.
    pub expires_unix: i64,
}

/// The magic first line. Nothing may precede it, a byte order mark included.
const MAGIC: &str = "# Netscape HTTP Cookie File";

/// What yt-dlp puts in front of the domain of an httpOnly cookie. Not a comment.
const HTTP_ONLY_PREFIX: &str = "#HttpOnly_";

/// A cookie jar is a few dozen lines. Past this it is not one.
const MAX_LINES: usize = 4000;

fn clean(value: &str) -> bool {
    !value.contains(['\t', '\n', '\r'])
}

fn flag(value: bool) -> &'static str {
    if value { "TRUE" } else { "FALSE" }
}

/// The whole file as text, and how many cookies survived into it: seven TAB-separated
/// columns, the subdomain flag IS the leading dot, a session cookie has an EMPTY expiry.
pub fn build(cookies: &[NetscapeCookie]) -> (String, usize) {
    let mut out = String::new();
    out.push_str(MAGIC);
    out.push('\n');
    out.push_str(
        "# Written by Modlight. Do not edit and do not share: these lines are a live login.\n",
    );
    out.push('\n');
    let mut written = 0;
    for cookie in cookies {
        if cookie.domain.is_empty()
            || cookie.name.is_empty()
            || !clean(&cookie.domain)
            || !clean(&cookie.path)
            || !clean(&cookie.name)
            || !clean(&cookie.value)
        {
            continue;
        }
        let path = if cookie.path.is_empty() {
            "/"
        } else {
            &cookie.path
        };
        let expires = if cookie.expires_unix > 0 {
            cookie.expires_unix.to_string()
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            cookie.domain,
            flag(cookie.domain.starts_with('.')),
            path,
            flag(cookie.secure),
            expires,
            cookie.name,
            cookie.value
        ));
        written += 1;
    }
    (out, written)
}

#[cfg(unix)]
fn owner_only_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn owner_only_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Write the jar in one go, born 0600 (never widened afterwards), and say how many
/// cookies it holds. UTF-8 without a byte order mark: Python refuses the whole file
/// otherwise.
pub fn write(path: &Path, cookies: &[NetscapeCookie]) -> std::io::Result<usize> {
    let (text, written) = build(cookies);
    if let Some(folder) = path.parent() {
        owner_only_dir(folder)?;
    }
    let temporary = path.with_extension("txt.tmp");
    let _ = std::fs::remove_file(&temporary);
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    Ok(written)
}

/// The cookie NAMES in a jar on disk, never the values: what "is there a session" is
/// answered from. A line with the wrong number of columns is dropped, as yt-dlp does.
pub fn read_names(path: &Path) -> std::io::Result<Vec<String>> {
    let text = std::fs::read_to_string(path)?;
    let mut names = Vec::new();
    for raw in text.lines().take(MAX_LINES) {
        let line = if let Some(rest) = raw.strip_prefix(HTTP_ONLY_PREFIX) {
            rest
        } else if raw.is_empty() || raw.starts_with('#') {
            continue;
        } else {
            raw
        };
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() != 7 || columns[5].is_empty() {
            continue;
        }
        names.push(columns[5].to_string());
    }
    Ok(names)
}

/// Any one of these in the jar means the browser is signed in to Google.
const LOGIN_COOKIES: [&str; 4] = ["__Secure-1PSID", "__Secure-3PSID", "SID", "LOGIN_INFO"];

/// Whether these cookie names amount to a completed sign-in.
pub fn is_signed_in_jar<S: AsRef<str>>(names: &[S]) -> bool {
    names
        .iter()
        .any(|name| LOGIN_COOKIES.contains(&name.as_ref()))
}

/// Where Google's sign-in starts (`YouTubeCookies.LoginUrl`): `service=youtube` and a
/// `continue` pointing at YouTube make the flow end on youtube.com.
pub fn login_url(language: &str) -> String {
    let language = if language.len() == 2 && language.bytes().all(|b| b.is_ascii_lowercase()) {
        language
    } else {
        "en"
    };
    format!(
        "https://accounts.google.com/ServiceLogin?service=youtube&hl={language}&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin"
    )
}

/// Whether this address is the far side of the sign-in: a youtube.com host (suffix
/// match), not the /signin hop the flow passes through first.
pub fn is_signed_in_page(address: &str) -> bool {
    let Ok(url) = url::Url::parse(address) else {
        return false;
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if host != "youtube.com" && !host.ends_with(".youtube.com") {
        return false;
    }
    !url.path().to_ascii_lowercase().starts_with("/signin")
}

/// The sign-in's folder and jar in one profile (`YouTubeCookies`).
#[derive(Clone, Debug)]
pub struct YouTubeCookies {
    folder: PathBuf,
}

/// The jar's state, against its length and write time.
static STATE: Mutex<Option<(PathBuf, u64, std::time::SystemTime, bool)>> = Mutex::new(None);

impl YouTubeCookies {
    pub fn new(profile: &Path) -> YouTubeCookies {
        YouTubeCookies {
            folder: profile.join("YouTubeSignIn"),
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The jar yt-dlp is pointed at.
    pub fn file_path(&self) -> PathBuf {
        self.folder.join("cookies.txt")
    }

    /// Whether the jar on disk holds a session yt-dlp can be signed in with: read from
    /// the file (cached against its length and write time), never from the last click.
    pub fn is_signed_in(&self) -> bool {
        let path = self.file_path();
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        if meta.len() == 0 {
            return false;
        }
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if let Ok(state) = STATE.lock()
            && let Some((cached, length, stamp, signed)) = state.as_ref()
            && *cached == path
            && *length == meta.len()
            && *stamp == modified
        {
            return *signed;
        }
        let signed = read_names(&path)
            .map(|names| is_signed_in_jar(&names))
            .unwrap_or(false);
        if let Ok(mut state) = STATE.lock() {
            *state = Some((path, meta.len(), modified, signed));
        }
        signed
    }

    /// The jar to pass to yt-dlp for this link, or `None`: YouTube only.
    pub fn file_for(&self, link: &super::link::DownloadLink) -> Option<PathBuf> {
        (link.is_youtube && self.is_signed_in()).then(|| self.file_path())
    }

    /// Write the captured jar. Returns how many cookies it holds; that count is logged,
    /// the contents never are.
    pub fn save(&self, cookies: &[NetscapeCookie]) -> std::io::Result<usize> {
        let written = write(&self.file_path(), cookies)?;
        forget_state();
        log::info!("Download: captured {written} YouTube cookies.");
        Ok(written)
    }

    /// Throw the sign-in away. The macOS sign-in window keeps no browser profile on disk
    /// (a non-persistent web data store), so the jar is the whole of it.
    pub fn sign_out(&self) {
        let _ = std::fs::remove_file(self.file_path());
        forget_state();
        log::info!("Download: YouTube sign-in cleared.");
    }
}

fn forget_state() {
    if let Ok(mut state) = STATE.lock() {
        *state = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(domain: &str, name: &str, value: &str, expires: i64) -> NetscapeCookie {
        NetscapeCookie {
            domain: domain.into(),
            path: "/".into(),
            name: name.into(),
            value: value.into(),
            secure: true,
            expires_unix: expires,
        }
    }

    #[test]
    fn the_file_is_what_python_reads() {
        let (text, written) = build(&[
            cookie(".youtube.com", "SID", "abc", 1_900_000_000),
            cookie("accounts.google.com", "SESSION", "x", 0),
            cookie(".google.com", "BAD", "has\ttab", 0),
            cookie("", "NODOMAIN", "x", 0),
        ]);
        assert_eq!(written, 2);
        assert!(text.starts_with("# Netscape HTTP Cookie File\n"));
        let lines: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .collect();
        assert_eq!(
            lines,
            [
                ".youtube.com\tTRUE\t/\tTRUE\t1900000000\tSID\tabc",
                "accounts.google.com\tFALSE\t/\tTRUE\t\tSESSION\tx",
            ]
        );
    }

    #[test]
    fn the_jar_is_owner_only_and_reads_back_names_only() {
        let profile = tempfile::tempdir().unwrap();
        let jar = YouTubeCookies::new(profile.path());
        assert!(!jar.is_signed_in());
        let written = jar
            .save(&[
                cookie(".youtube.com", "__Secure-1PSID", "secret", 1_900_000_000),
                cookie(".youtube.com", "PREF", "f6=40000000", 0),
            ])
            .unwrap();
        assert_eq!(written, 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = std::fs::metadata(jar.file_path())
                .unwrap()
                .permissions()
                .mode();
            let folder = std::fs::metadata(jar.folder())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(file & 0o777, 0o600);
            assert_eq!(folder & 0o777, 0o700);
        }
        assert!(jar.is_signed_in());
        assert_eq!(
            read_names(&jar.file_path()).unwrap(),
            ["__Secure-1PSID", "PREF"]
        );
        jar.sign_out();
        assert!(!jar.is_signed_in());
        assert!(!jar.file_path().exists());
    }

    #[test]
    fn a_jar_yt_dlp_rewrote_is_still_read_right() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.txt");
        std::fs::write(
            &path,
            "# Netscape HTTP Cookie File\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\t__Secure-3PSID\tv\n.youtube.com\tTRUE\t/\tTRUE\t0\tVISITOR\tv\textra\n",
        )
        .unwrap();
        assert_eq!(read_names(&path).unwrap(), ["__Secure-3PSID"]);
        assert!(is_signed_in_jar(&read_names(&path).unwrap()));
        assert!(!is_signed_in_jar(&["PREF", "VISITOR_INFO1_LIVE"]));
    }

    #[test]
    fn the_jar_goes_only_to_youtube_links() {
        let profile = tempfile::tempdir().unwrap();
        let jar = YouTubeCookies::new(profile.path());
        jar.save(&[cookie(".youtube.com", "SID", "s", 0)]).unwrap();
        let yt = super::super::link::DownloadLink::parse("https://youtu.be/x").unwrap();
        let tiktok =
            super::super::link::DownloadLink::parse("https://tiktok.com/@a/video/1").unwrap();
        assert_eq!(jar.file_for(&yt), Some(jar.file_path()));
        assert_eq!(jar.file_for(&tiktok), None);
    }

    #[test]
    fn the_sign_in_ends_on_youtube_not_on_its_signin_hop() {
        assert!(is_signed_in_page("https://www.youtube.com/"));
        assert!(is_signed_in_page("https://m.youtube.com/feed"));
        assert!(!is_signed_in_page("https://www.youtube.com/signin?x=1"));
        assert!(!is_signed_in_page("https://youtube.com.evil.example/"));
        assert!(!is_signed_in_page(
            "https://accounts.google.com/ServiceLogin"
        ));
        assert!(login_url("pl").contains("hl=pl&"));
        assert!(login_url("p;").contains("hl=en&"));
    }
}

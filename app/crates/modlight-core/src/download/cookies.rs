//! The YouTube sign-in's cookie jar (`NetscapeCookieFile`, `YouTubeCookies`).
//!
//! The file is a live Google login. It lives under the personal folder and nowhere else:
//! never in `ApplicationConfig.json`, never in a setting, never uploaded, never logged
//! (only the cookie count is). It is handed to yt-dlp with `--cookies` for YouTube links
//! only. On Windows the folder and the file get upstream's owner-only ACL (the user as
//! owner, one entry, inherited entries dropped); on macOS the folder is created 0700 and
//! the file written 0600, the Unix spelling of it (yt-dlp replaces the jar through a
//! temporary file and a rename after every run, so the folder is where the rule has to
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
    std::fs::create_dir_all(path)?;
    #[cfg(windows)]
    if let Err(error) = acl::protect(path, true) {
        // Best effort, as upstream: refusing the sign-in over a permission the person
        // may have no way to grant would be worse.
        log::warn!("YouTube sign-in: the folder could not be made owner-only: {error}");
    }
    Ok(())
}

/// Windows: upstream's owner-only ACL (`YouTubeCookies.Prepare` and `ProtectFile`): the
/// current user as owner and the one entry, full control, inherited by what the folder
/// holds; the inherited entries are dropped (a protected DACL), so whoever could read
/// the personal folder cannot read this.
#[cfg(windows)]
pub(crate) mod acl {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
        SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE,
        OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// `FileSystemRights.FullControl`.
    const FULL_CONTROL: u32 = 0x001F_01FF;

    /// The current user's SID, as the bytes of its `TOKEN_USER`.
    fn token_user() -> std::io::Result<Vec<u64>> {
        // SAFETY: the token handle is ours and closed below; the buffer is sized by the
        // first call and aligned for TOKEN_USER (u64 words).
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut needed = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
            let ok = GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
                &mut needed,
            );
            let error = std::io::Error::last_os_error();
            CloseHandle(token);
            if ok == 0 {
                return Err(error);
            }
            Ok(buffer)
        }
    }

    /// Make `path` owned by and open to the current user alone; a folder passes the
    /// rule on to everything created in it.
    pub fn protect(path: &Path, folder: bool) -> std::io::Result<()> {
        let user = token_user()?;
        // SAFETY: `user` holds a TOKEN_USER whose SID points inside the same buffer,
        // which outlives every call below; the ACL SetEntriesInAclW allocates is freed
        // with LocalFree.
        unsafe {
            let sid = (*(user.as_ptr() as *const TOKEN_USER)).User.Sid;
            let access = EXPLICIT_ACCESS_W {
                grfAccessPermissions: FULL_CONTROL,
                grfAccessMode: SET_ACCESS,
                grfInheritance: if folder {
                    CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE
                } else {
                    NO_INHERITANCE
                },
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: sid.cast(),
                },
            };
            let mut acl: *mut ACL = std::ptr::null_mut();
            let result = SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl);
            if result != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(result as i32));
            }
            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let result = SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                sid,
                std::ptr::null_mut(),
                acl,
                std::ptr::null(),
            );
            LocalFree(acl.cast());
            if result != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(result as i32));
            }
        }
        Ok(())
    }

    /// For tests: whether the DACL is protected and how many entries it has.
    #[cfg(test)]
    pub fn describe(path: &Path) -> (bool, u32) {
        use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
        use windows_sys::Win32::Security::{
            ACL_SIZE_INFORMATION, AclSizeInformation, GetAclInformation,
            GetSecurityDescriptorControl, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
        };
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: plain queries on a path; the descriptor is freed with LocalFree.
        unsafe {
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let result = GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            );
            assert_eq!(result, ERROR_SUCCESS);
            let mut control = 0u16;
            let mut revision = 0u32;
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision);
            let mut size: ACL_SIZE_INFORMATION = std::mem::zeroed();
            GetAclInformation(
                dacl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            );
            LocalFree(descriptor);
            (control & SE_DACL_PROTECTED != 0, size.AceCount)
        }
    }
}
// Write the jar in one go, born 0600 (never widened afterwards), and say how many
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
    // `ProtectFile`: the same rule on the jar itself, whatever it inherited.
    #[cfg(windows)]
    if let Err(error) = acl::protect(path, false) {
        log::warn!("YouTube sign-in: the cookie file could not be made owner-only: {error}");
    }
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
        #[cfg(windows)]
        {
            // A protected DACL with the one entry for this user, on both.
            assert_eq!(acl::describe(jar.folder()), (true, 1));
            assert_eq!(acl::describe(&jar.file_path()), (true, 1));
            // The folder passes it on to what yt-dlp writes there.
            let written_by_ytdlp = jar.folder().join("cookies.txt.tmp-yt");
            std::fs::write(&written_by_ytdlp, b"x").unwrap();
            assert_eq!(acl::describe(&written_by_ytdlp).1, 1);
            std::fs::remove_file(&written_by_ytdlp).unwrap();
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

//! Port of `lib/admin-users.ts`: the shape of a row in Admin -> Users, the edit
//! dialog's extra fields, and the validation patterns they share.
//!
//! Lives outside the server-action module upstream so the client table can
//! `import type` it without dragging a `"use server"` boundary into the bundle.

use serde::Serialize;

/// One row of Admin -> Users.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserRow {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: Option<String>,
    pub banned: bool,
    /// ISO timestamp: server actions may only hand plain data to the client.
    pub created_at: Option<String>,
    /// `user.userCode`: the first segment of every public URL.
    pub tag: Option<String>,
    /// Resolved avatar URL, or None when this account still shows initials
    /// (a URL rather than the raw `user.image` storage key).
    pub avatar: Option<String>,
    /// Bytes accounted against the quota (trash included: it still occupies disk).
    pub used_quota: i64,
    /// Byte limit; `-1` = unlimited.
    pub max_quota: i64,
    /// Live uploads (trash excluded).
    pub files: i64,
    /// May this account mint short links on the bare domain instead of under
    /// its own tag? Off by default: the root namespace is shared with every
    /// top-level route.
    pub allow_root_short_links: bool,
    /// Has this address ever been confirmed? Drives the "Unconfirmed" badge.
    pub email_verified: bool,
    /// A mailed invitation is outstanding: the account exists but has never had
    /// a password set (distinct from "no password", also true for accounts
    /// imported without a usable hash).
    pub invite_pending: bool,
}

/// Everything the edit form needs on top of what the table shows, loaded per
/// account when the dialog opens. Note what is NOT here: the password.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserDetail {
    #[serde(flatten)]
    pub row: AdminUserRow,
    /// Login name, usable instead of the email at sign-in. May be unset.
    pub username: Option<String>,
    pub two_factor_enabled: bool,
    /// Opt-in public gallery at /u/{tag}.
    pub public_profile: bool,
    /// Generate links for NEW uploads without the tag segment.
    pub skip_tag_in_url: bool,
    /// Whether a credential (email + password) login exists at all.
    pub has_password: bool,
}

/// Login names accepted at sign-in (`/^[a-zA-Z0-9][a-zA-Z0-9._-]{1,31}$/`):
/// the shape of a tag minus the reserved-word list (a username is never a URL
/// segment, so it cannot shadow a route).
pub fn is_valid_username(v: &str) -> bool {
    let mut chars = v.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let rest: Vec<char> = chars.collect();
    first.is_ascii_alphanumeric()
        && (1..=31).contains(&rest.len())
        && rest.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `/^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/` (JavaScript `\s`): loose enough to accept
/// every address that exists, strict enough to catch typos.
pub fn is_valid_email(v: &str) -> bool {
    let not_space_at = |c: char| c != '@' && !crate::config::is_js_whitespace(c);
    let Some((local, domain)) = v.split_once('@') else {
        return false;
    };
    if local.is_empty() || !local.chars().all(not_space_at) || !domain.chars().all(not_space_at) {
        return false;
    }
    // `[^\s@]+\.[^\s@]{2,}` over the domain: some dot with at least one char
    // before it and two after it (backtracking picks any such dot).
    let chars: Vec<char> = domain.chars().collect();
    chars.iter().enumerate().any(|(i, c)| *c == '.' && i >= 1 && chars.len() - i - 1 >= 2)
}

/// Better Auth's default bounds (`emailAndPassword.minPasswordLength` = 8).
pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 128;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(is_valid_username("ab"));
        assert!(is_valid_username("a.b_c-d"));
        assert!(!is_valid_username("a"));
        assert!(!is_valid_username(".ab"));
        assert!(!is_valid_username(&format!("a{}", "b".repeat(32))));
        assert!(is_valid_username(&format!("a{}", "b".repeat(31))));
        assert!(is_valid_email("a@b.cd"));
        assert!(is_valid_email("a@b.c.de"));
        assert!(is_valid_email("a@b.cd.e"));
        assert!(!is_valid_email("a@b.c"));
        assert!(!is_valid_email("a b@c.de"));
        assert!(!is_valid_email("a@@b.cd"));
        assert!(!is_valid_email("@b.cd"));
        assert!(!is_valid_email("a@.cd"));
    }
}

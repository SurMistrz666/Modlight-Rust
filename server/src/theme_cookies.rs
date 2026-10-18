//! Port of `lib/theme-cookies.ts`: the two cookies that let the server render
//! the theme onto `<html>` before the first paint. They mirror client-side
//! state; nothing security-sensitive lives here, so a plain readable cookie is
//! the right tool.
//!
//!   ml_mode - light | dark | system   (colour mode, a viewer preference)
//!   ml_skin - a theme id              (skin, the account's own choice)

pub const MODES: &[&str] = &["light", "dark", "system"];

pub const MODE_COOKIE: &str = "ml_mode";
pub const SKIN_COOKIE: &str = "ml_skin";

/// One year: a preference, not a session.
pub const THEME_COOKIE_MAX_AGE: i64 = 60 * 60 * 24 * 365;

pub fn resolve_mode(v: Option<&str>) -> &'static str {
    match v {
        Some(v) => MODES.iter().find(|m| **m == v).copied().unwrap_or("system"),
        None => "system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_modes_fall_back_to_system() {
        assert_eq!(resolve_mode(Some("dark")), "dark");
        assert_eq!(resolve_mode(Some("DARK")), "system");
        assert_eq!(resolve_mode(None), "system");
    }
}

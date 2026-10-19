//! Where an upload goes when nobody said otherwise: the folder My space is standing in
//! (`ModlightUploadTarget`).
//!
//! One ambient answer for the whole app, because one answer per entry point is exactly
//! how dropping a file on the window, pasting, the tray and captures all ended up in the
//! account's default folder while the window plainly said "you are in CV". The rule is
//! one sentence: if My space is showing a folder, that is where new uploads go. `None`
//! means "expressed no preference", which the server answers with the account's default.

use std::sync::RwLock;

static FOLDER_ID: RwLock<Option<String>> = RwLock::new(None);

/// The folder new uploads land in, or `None` for the account's default.
pub fn current() -> Option<String> {
    FOLDER_ID.read().map(|f| f.clone()).unwrap_or(None)
}

/// Say where uploads go from now on. Empty and `None` both mean "the account decides".
pub fn set(id: Option<&str>) {
    if let Ok(mut folder) = FOLDER_ID.write() {
        *folder = id.filter(|i| !i.is_empty()).map(str::to_string);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_means_no_preference() {
        super::set(Some("f1"));
        assert_eq!(super::current().as_deref(), Some("f1"));
        super::set(Some(""));
        assert_eq!(super::current(), None);
        super::set(None);
        assert_eq!(super::current(), None);
    }
}

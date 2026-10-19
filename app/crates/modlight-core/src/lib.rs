//! Everything of Modlight App that is not a window and not a network call: texts,
//! formats, skins, icons, settings and their sync, the account, and the library's logic.

pub mod account;
pub mod config;
pub mod culture;
pub mod dotnet;
pub mod format;
pub mod history;
pub mod icons;
pub mod l10n;
pub mod library;
pub mod messages;
pub mod names;
pub mod sync;
pub mod tasks;
pub mod theme;
pub mod upload;
pub mod watch;

/// The product's name, as upstream's `Program.AppName`.
pub const APP_NAME: &str = "Modlight";

/// The version, as upstream's `VersionText`: "0.7.7 Dev" (upstream hard-codes Dev).
pub fn version_text() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    let build = parts.next().unwrap_or("0");
    let mut text = format!("{major}.{minor}");
    if build != "0" {
        text.push('.');
        text.push_str(build);
    }
    text.push_str(" Dev");
    text
}

/// `Program.Title`: "Modlight 0.7.7 Dev".
pub fn title() -> String {
    format!("{APP_NAME} {}", version_text())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_title_reads_like_upstream() {
        assert_eq!(super::title(), "Modlight 0.7.7 Dev");
    }
}

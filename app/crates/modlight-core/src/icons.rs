//! The Lucide glyphs of `lucide.ttf`, by name (`LucideIcons.cs`), generated from
//! `lucide-codepoints.json`. Draw them with the icon font.

include!(concat!(env!("OUT_DIR"), "/lucide.rs"));

/// The icon font itself, embedded.
pub const FONT: &[u8] = include_bytes!("../../../assets/upstream/icons/lucide.ttf");

#[cfg(test)]
mod tests {
    #[test]
    fn names_match_lucide_icons_cs() {
        assert_eq!(super::A_ARROW_DOWN, '\u{e585}');
        assert_eq!(super::ACTIVITY, '\u{e038}');
        assert_eq!(super::CLOUD, '\u{e088}');
        assert!(super::FONT.len() > 100_000);
    }
}

//! The emoji stickers on Windows: upstream's Segoe UI Emoji, a `COLR` font, painted in
//! colour, sequences as one glyph. A test binary of its own, because the library's unit
//! tests switch the system's fonts off for the whole process. Flags are not in the font:
//! Windows shows their letters, and so does this.
//!
//! `EMOJI_DUMP=<folder>` saves the pictures for a look.

#![cfg(windows)]

use modlight_editor::emoji::sticker;

#[test]
fn segoe_ui_emoji_paints_in_colour() {
    let dump = std::env::var("EMOJI_DUMP").ok();
    for (sequence, name) in [
        ("1f600", "grinning"),
        ("1f44d 1f3fb", "thumbs-light"),
        ("1f468 200d 1f469 200d 1f467", "family"),
        ("2764 fe0f", "heart"),
        ("1f308", "rainbow"),
    ] {
        let image = sticker(sequence, 128).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(image.dimensions(), (128, 128));
        let coloured = image
            .pixels()
            .filter(|p| p[3] > 200 && (p[0].abs_diff(p[1]) > 40 || p[1].abs_diff(p[2]) > 40))
            .count();
        assert!(coloured > 500, "{name}: {coloured} coloured pixels");
        if let Some(folder) = &dump {
            image
                .save(std::path::Path::new(folder).join(format!("{name}.png")))
                .unwrap();
        }
    }
}

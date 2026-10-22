//! The renderer's golden: one of every annotation (shapes, arrow styles and a curve,
//! dashes, freehand, outlined text, balloon, steps with a tail, the four effects,
//! spotlight, emoji) exported and compared with `tests/snapshots/export/gallery.png`.
//! Uses the system's fonts (the emoji font for the sticker): on Windows the picture may
//! be `export/windows-gallery.png` (see `common`).

mod common;

#[test]
fn every_annotation_exports_as_the_golden() {
    let image = modlight_editor::qa::gallery();
    if let Err(error) = common::check(&image, "export/gallery", 0.6, 40) {
        panic!("{error}");
    }
}

//! The renderer's golden: one of every annotation (shapes, arrow styles and a curve,
//! dashes, freehand, outlined text, balloon, steps with a tail, the four effects,
//! spotlight, emoji) exported and compared with `tests/snapshots/export/gallery.png`.
//! Uses this Mac's fonts (Apple Color Emoji for the sticker).

#[test]
fn every_annotation_exports_as_the_golden() {
    let image = modlight_editor::qa::gallery();
    let options = egui_kittest::SnapshotOptions::new()
        .threshold(0.6)
        .max_failed_pixels(40);
    egui_kittest::image_snapshot_options(&image, "export/gallery", &options);
}

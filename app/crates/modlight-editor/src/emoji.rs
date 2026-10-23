//! Emoji: the catalog (`Assets/emoji-catalog.json`, `EmojiCatalogService`) and the sticker
//! bitmaps (`WindowsEmojiBitmapRenderer`).
//!
//! Upstream draws the glyph with Direct2D and "Segoe UI Emoji". On Windows the port
//! draws the same font, a `COLR` colour font, with [`crate::colr`]; a Mac lacks it (its
//! fallback, Skia with the default typeface, draws nothing), so there the port draws
//! Apple Color Emoji's own bitmaps (the `sbix` strikes CoreText uses). Either way the
//! text is shaped with the font's ligatures so sequences (ZWJ, flags, skin tones) become
//! one glyph, then trimmed and fitted into the square exactly as upstream's renderer
//! does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use image::RgbaImage;
use rustybuzz::ttf_parser;
use serde::Deserialize;

use crate::raster::{self, Filter};

const CATALOG_JSON: &str = include_str!("../../../assets/upstream/editor/emoji-catalog.json");
/// The emoji font: upstream's on Windows, the system's own on a Mac.
pub const EMOJI_FAMILY: &str = if cfg!(windows) {
    "Segoe UI Emoji"
} else {
    "Apple Color Emoji"
};
const PREVIEW_PADDING: u32 = 6;
const STICKER_PADDING: u32 = 14;
const RAW_FONT_SCALE: f32 = 1.32;

/// `EmojiCatalogEntry`.
#[derive(Clone, Debug, Deserialize)]
pub struct EmojiEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub unicode: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(skip)]
    search_index: String,
}

impl EmojiEntry {
    /// `Glyph`.
    pub fn glyph(&self) -> String {
        to_glyph(&self.unicode)
    }

    /// `GetSearchScore`: 0 exact name, 1 name prefix, 2 keyword prefix, 3 anywhere.
    pub fn search_score(&self, search: &str) -> i32 {
        let search = search.trim().to_lowercase();
        if search.is_empty() {
            return i32::MAX;
        }
        let name = self.name.to_lowercase();
        if name == search {
            return 0;
        }
        if name.starts_with(&search) {
            return 1;
        }
        if self
            .keywords
            .iter()
            .any(|k| k.to_lowercase().starts_with(&search))
        {
            return 2;
        }
        if self.search_index.contains(&search) {
            return 3;
        }
        i32::MAX
    }
}

/// The groups in upstream's fixed order (`OrderedGroups`), then any others sorted.
pub const ORDERED_GROUPS: [&str; 9] = [
    "Smileys & Emotion",
    "People & Body",
    "Animals & Nature",
    "Food & Drink",
    "Travel & Places",
    "Activities",
    "Objects",
    "Symbols",
    "Flags",
];

pub fn catalog() -> &'static [EmojiEntry] {
    static CATALOG: OnceLock<Vec<EmojiEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut entries: Vec<EmojiEntry> = serde_json::from_str(CATALOG_JSON).unwrap_or_default();
        for e in &mut entries {
            let mut index = format!("{} {} {} {}", e.name, e.group, e.name, e.group);
            for k in &e.keywords {
                index.push(' ');
                index.push_str(k);
            }
            e.search_index = index.to_lowercase();
        }
        entries
    })
}

/// `GetGroups`.
pub fn groups() -> Vec<String> {
    let present: std::collections::BTreeSet<&str> =
        catalog().iter().map(|e| e.group.as_str()).collect();
    let mut out: Vec<String> = ORDERED_GROUPS
        .iter()
        .filter(|g| present.contains(*g))
        .map(|g| g.to_string())
        .collect();
    for g in present {
        if !out.iter().any(|o| o == g) {
            out.push(g.to_string());
        }
    }
    out
}

/// `ToGlyph`: hex code points separated by spaces or dashes.
pub fn to_glyph(sequence: &str) -> String {
    sequence
        .split([' ', '-'])
        .filter(|p| !p.trim().is_empty())
        .filter_map(|p| u32::from_str_radix(p.trim(), 16).ok())
        .filter_map(char::from_u32)
        .collect()
}

/// `GetInteractiveStickerSize`: sizes rounded to steps while dragging.
pub fn interactive_sticker_size(size: u32) -> u32 {
    let size = size.max(1);
    if size <= 64 {
        return size;
    }
    let step = if size <= 128 {
        4.0
    } else if size <= 256 {
        8.0
    } else {
        12.0
    };
    ((size as f64 / step).round() * step).max(64.0) as u32
}

type StickerCache = Mutex<HashMap<(String, u32, u32), Option<Arc<RgbaImage>>>>;

fn cache() -> &'static StickerCache {
    static CACHE: OnceLock<StickerCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `RenderStickerBitmap(sequence, size)`: the square bitmap of an emoji annotation.
pub fn sticker(sequence: &str, size: u32) -> Option<Arc<RgbaImage>> {
    render_cached(sequence, size, STICKER_PADDING)
}

/// `RenderPreviewBitmap(sequence, size)`: the picker's cells.
pub fn preview(sequence: &str, size: u32) -> Option<Arc<RgbaImage>> {
    render_cached(sequence, size, PREVIEW_PADDING)
}

fn render_cached(sequence: &str, size: u32, padding: u32) -> Option<Arc<RgbaImage>> {
    if sequence.trim().is_empty() || size == 0 {
        return None;
    }
    let key = (sequence.to_string(), size, padding);
    if let Some(found) = cache().lock().unwrap().get(&key) {
        return found.clone();
    }
    let rendered = render_square(&to_glyph(sequence), size, padding).map(Arc::new);
    let mut map = cache().lock().unwrap();
    if map.len() > 512 {
        map.clear();
    }
    map.insert(key, rendered.clone());
    rendered
}

/// The glyph image(s) of the text in the emoji font ([`EMOJI_FAMILY`]), side by side at
/// `ppem`.
pub fn raw_glyph_image(text: &str, ppem: f32) -> Option<RgbaImage> {
    let face = crate::text::find_face(EMOJI_FAMILY)?;
    face.with_face(|f| {
        let hb = rustybuzz::Face::from_face(f.clone());
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let shaped = rustybuzz::shape(&hb, &[], buffer);
        let upem = f.units_per_em() as f32;
        let scale = ppem / upem;
        if f.tables().colr.is_some() {
            return colour_glyphs(f, &shaped, scale, ppem);
        }
        let mut pieces = Vec::new();
        let mut pen = 0.0f32;
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let gid = ttf_parser::GlyphId(info.glyph_id as u16);
            if let Some(raster) = f.glyph_raster_image(gid, ppem.round().max(1.0) as u16)
                && raster.format == ttf_parser::RasterImageFormat::PNG
                && let Ok(img) = image::load_from_memory(raster.data)
            {
                let img = img.to_rgba8();
                let strike_scale = ppem / raster.pixels_per_em as f32;
                let w = (img.width() as f32 * strike_scale).round().max(1.0) as u32;
                let h = (img.height() as f32 * strike_scale).round().max(1.0) as u32;
                let img = raster::resize(&img, w, h, Filter::Linear);
                pieces.push((pen + raster.x as f32 * strike_scale, img));
            }
            pen += pos.x_advance as f32 * scale;
        }
        if pieces.is_empty() {
            return None;
        }
        let width = pieces
            .iter()
            .map(|(x, img)| x + img.width() as f32)
            .fold(pen, f32::max)
            .ceil() as u32;
        let height = pieces
            .iter()
            .map(|(_, img)| img.height())
            .max()
            .unwrap_or(1);
        let mut canvas = RgbaImage::new(width.max(1), height.max(1));
        for (x, img) in pieces {
            image::imageops::overlay(&mut canvas, &img, x.round() as i64, 0);
        }
        Some(canvas)
    })
    .flatten()
}

/// A `COLR` font's glyphs side by side on the baseline (glyphs without colour in the
/// foreground colour, black, as Direct2D draws them), with a margin a quarter of the
/// size wide around them for parts that reach past the advance; the caller trims it.
fn colour_glyphs(
    face: &ttf_parser::Face<'_>,
    shaped: &rustybuzz::GlyphBuffer,
    scale: f32,
    ppem: f32,
) -> Option<RgbaImage> {
    let margin = (ppem * 0.25).ceil();
    let advance: f32 = shaped
        .glyph_positions()
        .iter()
        .map(|pos| pos.x_advance as f32 * scale)
        .sum();
    let ascender = face.ascender() as f32 * scale;
    let descender = face.descender() as f32 * scale;
    let width = (advance + margin * 2.0).ceil().max(1.0) as u32;
    let height = (ascender - descender + margin * 2.0).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let mut pen = margin;
    let mut drawn = false;
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        let glyph = ttf_parser::GlyphId(info.glyph_id as u16);
        // Font units (y up) to pixels (y down), on this glyph's place on the baseline.
        let base = tiny_skia::Transform::from_row(
            scale,
            0.0,
            0.0,
            -scale,
            pen + pos.x_offset as f32 * scale,
            margin + ascender - pos.y_offset as f32 * scale,
        );
        if crate::colr::paint_glyph(face, glyph, base, &mut pixmap).is_some() {
            drawn = true;
        } else if let Some(path) = crate::colr::outline(face, glyph) {
            let mut paint = tiny_skia::Paint::default();
            paint.set_color_rgba8(0, 0, 0, 255);
            pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, base, None);
            drawn = true;
        }
        pen += pos.x_advance as f32 * scale;
    }
    if !drawn {
        return None;
    }
    let mut image = RgbaImage::new(width, height);
    for (pixel, out) in pixmap.pixels().iter().zip(image.pixels_mut()) {
        let color = pixel.demultiply();
        *out = image::Rgba([color.red(), color.green(), color.blue(), color.alpha()]);
    }
    Some(image)
}

/// `RenderSquareBitmap`: draw big, trim the transparent border, fit into the square.
fn render_square(glyph: &str, size: u32, padding: u32) -> Option<RgbaImage> {
    if glyph.is_empty() {
        return None;
    }
    let raw_font = (size as f32 * RAW_FONT_SCALE).max(24.0);
    let raw = raw_glyph_image(glyph, raw_font)?;
    let trimmed = trim_transparent(&raw)?;
    Some(fit_into_square(&trimmed, size, padding))
}

/// `TrimTransparentBounds`.
pub fn trim_transparent(image: &RgbaImage) -> Option<RgbaImage> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for (x, y, p) in image.enumerate_pixels() {
        if p.0[3] != 0 {
            any = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    any.then(|| {
        image::imageops::crop_imm(image, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
            .to_image()
    })
}

/// `FitIntoSquare`: uniform scale into `size - 2 * padding`, centred, CatmullRom.
pub fn fit_into_square(source: &RgbaImage, size: u32, padding: u32) -> RgbaImage {
    let available = (size as f32 - padding as f32 * 2.0).max(1.0);
    let scale = (available / source.width() as f32).min(available / source.height() as f32);
    let w = (source.width() as f32 * scale).max(1.0);
    let h = (source.height() as f32 * scale).max(1.0);
    let scaled = raster::resize(
        source,
        w.round().max(1.0) as u32,
        h.round().max(1.0) as u32,
        Filter::CatmullRom,
    );
    let mut out = RgbaImage::new(size, size);
    let left = ((size as f32 - w) / 2.0).round() as i64;
    let top = ((size as f32 - h) / 2.0).round() as i64;
    image::imageops::overlay(&mut out, &scaled, left, top);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_loads_with_upstream_groups() {
        assert!(catalog().len() > 1000);
        let groups = groups();
        assert_eq!(groups[0], "Smileys & Emotion");
        assert!(groups.iter().any(|g| g == "Flags"));
    }

    #[test]
    fn search_scores_rank_like_upstream() {
        let grinning = catalog()
            .iter()
            .find(|e| e.name == "grinning face")
            .unwrap();
        assert_eq!(grinning.search_score("grinning face"), 0);
        assert_eq!(grinning.search_score("grin"), 1);
        assert_eq!(grinning.search_score("face"), 2);
        assert_eq!(grinning.search_score("emotion"), 3);
        assert_eq!(grinning.search_score("zzz"), i32::MAX);
    }

    #[test]
    fn glyphs_and_interactive_sizes() {
        assert_eq!(to_glyph("1f600"), "\u{1F600}");
        assert_eq!(to_glyph("1f44d 1f3fb"), "\u{1F44D}\u{1F3FB}");
        assert_eq!(interactive_sticker_size(50), 50);
        assert_eq!(interactive_sticker_size(130), 128);
        assert_eq!(interactive_sticker_size(300), 300);
    }
}

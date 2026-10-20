//! The bitmaps of the effect annotations (`Core/Annotations/Effects`): blur, pixelate,
//! highlight and magnify, made the way upstream's view makes them while editing
//! (`EditorView.TryUpdateCachedEffectVisual`): the whole source image is processed once
//! per effect and strength (`CreateInteractionCacheBitmap`), and each annotation takes
//! its rectangle out of that (`UpdateEffectFromInteractionCache`). A pixelate grid is
//! therefore aligned with the image, not with the annotation.

use std::collections::VecDeque;
use std::sync::Arc;

use image::{Rgba, RgbaImage};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

use crate::color::Color;
use crate::geom::{Rect, RectI};
use crate::model::{Annotation, EffectKind, Kind};
use crate::raster::{self, Filter};

/// `BlurAnnotation.CreateBlurredSourceCache`: the source extended by its edge pixels by
/// `ceil(3 * sigma)`, blurred, cut back to the source size.
pub fn blurred_source(source: &RgbaImage, amount: f32) -> RgbaImage {
    let sigma = amount.max(0.0);
    if sigma <= 0.0 {
        return source.clone();
    }
    let pad = (sigma * 3.0).ceil() as u32;
    let extended = raster::extend_clamped(source, pad, pad, pad, pad);
    let blurred = raster::gaussian_blur(&extended, sigma);
    image::imageops::crop_imm(&blurred, pad, pad, source.width(), source.height()).to_image()
}

/// `PixelateAnnotation.CreatePixelatedSourceCache`: down with bilinear sampling, back up
/// with nearest.
pub fn pixelated_source(source: &RgbaImage, amount: f32) -> RgbaImage {
    let size = (amount.max(1.0)) as u32;
    let w = (source.width() / size).max(1);
    let h = (source.height() / size).max(1);
    let small = raster::resize(source, w, h, Filter::Linear);
    raster::resize(&small, source.width(), source.height(), Filter::Nearest)
}

/// `HighlightAnnotation.ApplyHighlightToBitmap`: every channel limited to the colour's.
pub fn highlighted_source(source: &RgbaImage, fill: &str) -> RgbaImage {
    let color = Color::parse(fill).unwrap_or(Color::rgb(255, 255, 0));
    let mut out = source.clone();
    for p in out.pixels_mut() {
        p.0[0] = p.0[0].min(color.r);
        p.0[1] = p.0[1].min(color.g);
        p.0[2] = p.0[2].min(color.b);
    }
    out
}

/// A few processed copies of the source, keyed by `GetInteractionCacheKey` and the
/// source's generation.
#[derive(Default)]
pub struct EffectCache {
    entries: VecDeque<(u64, String, Arc<RgbaImage>)>,
}

const CACHE_SIZE: usize = 4;

impl EffectCache {
    pub fn get(
        &mut self,
        generation: u64,
        key: &str,
        make: impl FnOnce() -> RgbaImage,
    ) -> Arc<RgbaImage> {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|(g, k, _)| *g == generation && k == key)
        {
            let entry = self.entries.remove(pos).unwrap();
            let image = entry.2.clone();
            self.entries.push_front(entry);
            return image;
        }
        let image = Arc::new(make());
        self.entries
            .push_front((generation, key.to_string(), image.clone()));
        while self.entries.len() > CACHE_SIZE {
            self.entries.pop_back();
        }
        image
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn cache_key(a: &Annotation, effect: EffectKind, amount: f32) -> String {
    match effect {
        EffectKind::Blur => format!("Blur:{amount}"),
        EffectKind::Pixelate => format!("Pixelate:{amount}"),
        EffectKind::Highlight => format!("Highlight:{amount}:{}", a.fill_color),
        EffectKind::Magnify => "Magnify".to_string(),
    }
}

/// Regenerates `EffectBitmap` of an effect annotation for the current source.
pub fn update_effect(
    a: &mut Annotation,
    source: &Arc<RgbaImage>,
    generation: u64,
    cache: &mut EffectCache,
) {
    let Kind::Effect(data) = &a.kind else {
        return;
    };
    let Some(effect) = data.effect else {
        return;
    };
    let amount = data.amount;
    let rect = a.bounds();
    let full_w = rect.width() as i32;
    let full_h = rect.height() as i32;
    if full_w <= 0 || full_h <= 0 {
        return;
    }
    let key = cache_key(a, effect, amount);
    let fill = a.fill_color.clone();
    let processed = cache.get(generation, &key, || match effect {
        EffectKind::Blur => blurred_source(source, amount),
        EffectKind::Pixelate => pixelated_source(source, amount),
        EffectKind::Highlight => highlighted_source(source, &fill),
        EffectKind::Magnify => (**source).clone(),
    });
    let bitmap = match effect {
        EffectKind::Magnify => magnify(&rect, a.rotation, amount, source, &processed),
        _ if a.rotation != 0.0 => from_processed_source(&rect, a.rotation, &processed),
        _ => from_aligned_cache(&rect, source, &processed),
    };
    if let Some(data) = a.effect_data_mut() {
        data.bitmap = bitmap.map(Arc::new);
    }
}

/// `UpdateEffectFromAlignedCache`.
fn from_aligned_cache(rect: &Rect, source: &RgbaImage, cached: &RgbaImage) -> Option<RgbaImage> {
    let full_w = rect.width() as i32;
    let full_h = rect.height() as i32;
    if cached.dimensions() != source.dimensions() {
        return None;
    }
    let annotation_rect = rect.truncate();
    let valid = annotation_rect.intersect(&RectI::new(
        0,
        0,
        source.width() as i32,
        source.height() as i32,
    ));
    let mut result = RgbaImage::new(full_w as u32, full_h as u32);
    if valid.width() <= 0 || valid.height() <= 0 {
        return Some(result);
    }
    let region = raster::crop(cached, valid);
    let dx = (valid.left - annotation_rect.left) as i64;
    let dy = (valid.top - annotation_rect.top) as i64;
    image::imageops::overlay(&mut result, &region, dx, dy);
    Some(result)
}

/// `UpdateEffectFromProcessedSource`: the processed image drawn under the inverse of the
/// annotation's rotation, so the visual (rotated back) shows the pixels beneath it.
fn from_processed_source(rect: &Rect, rotation: f32, processed: &RgbaImage) -> Option<RgbaImage> {
    let full_w = rect.width() as i32;
    let full_h = rect.height() as i32;
    let mut target = Pixmap::new(full_w as u32, full_h as u32)?;
    let (cx, cy) = (full_w as f32 / 2.0, full_h as f32 / 2.0);
    let transform = Transform::from_translate(cx, cy)
        .pre_rotate(-rotation)
        .pre_translate(-cx, -cy)
        .pre_translate(-rect.left, -rect.top);
    let source = raster::to_pixmap(processed);
    target.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Nearest,
            ..PixmapPaint::default()
        },
        transform,
        None,
    );
    Some(raster::from_pixmap(&target))
}

/// `MagnifyAnnotation.UpdateEffectCore`: the middle of the region, `1 / zoom` of its
/// size, drawn over the whole region with Mitchell sampling.
fn magnify(
    rect: &Rect,
    rotation: f32,
    amount: f32,
    source: &RgbaImage,
    draw_source: &RgbaImage,
) -> Option<RgbaImage> {
    let full_w = rect.width() as i32;
    let full_h = rect.height() as i32;
    let zoom = amount.max(1.0);
    if rotation != 0.0 {
        // `UpdateRotatedEffect`.
        let mut target = Pixmap::new(full_w as u32, full_h as u32)?;
        let transform = Transform::from_translate(full_w as f32 / 2.0, full_h as f32 / 2.0)
            .pre_scale(zoom, zoom)
            .pre_rotate(-rotation)
            .pre_translate(-rect.mid_x(), -rect.mid_y());
        let src = raster::to_pixmap(draw_source);
        target.draw_pixmap(
            0,
            0,
            src.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Bicubic,
                ..PixmapPaint::default()
            },
            transform,
            None,
        );
        return Some(raster::from_pixmap(&target));
    }
    let annotation_rect = rect.truncate();
    let valid = annotation_rect.intersect(&RectI::new(
        0,
        0,
        source.width() as i32,
        source.height() as i32,
    ));
    let mut result = RgbaImage::new(full_w as u32, full_h as u32);
    if valid.width() <= 0 || valid.height() <= 0 {
        return Some(result);
    }
    let capture_w = valid.width() as f32 / zoom;
    let capture_h = valid.height() as f32 / zoom;
    let center_x = valid.left as f32 + valid.width() as f32 / 2.0;
    let center_y = valid.top as f32 + valid.height() as f32 / 2.0;
    let capture_x = center_x - capture_w / 2.0;
    let capture_y = center_y - capture_h / 2.0;
    let capture = RectI::new(
        capture_x as i32,
        capture_y as i32,
        (capture_x + capture_w) as i32,
        (capture_y + capture_h) as i32,
    )
    .intersect(&RectI::new(
        0,
        0,
        source.width() as i32,
        source.height() as i32,
    ));
    if capture.width() <= 0 || capture.height() <= 0 {
        return Some(result);
    }
    let region = raster::crop(draw_source, capture);
    let scaled = raster::resize(
        &region,
        valid.width() as u32,
        valid.height() as u32,
        Filter::Mitchell,
    );
    let dx = (valid.left - annotation_rect.left) as i64;
    let dy = (valid.top - annotation_rect.top) as i64;
    image::imageops::overlay(&mut result, &scaled, dx, dy);
    Some(result)
}

/// The smart eraser's fill as a bitmap: one column or row stretched over the area, or
/// `None` for a solid colour.
pub fn smart_eraser_fill_bitmap(a: &Annotation) -> Option<RgbaImage> {
    let Kind::SmartEraser {
        fill_mode,
        edge_pixels: Some(edge),
    } = &a.kind
    else {
        return None;
    };
    if edge.is_empty() {
        return None;
    }
    use crate::model::SmartEraserFillMode::*;
    let (w, h) = match fill_mode {
        StretchVertically => (edge.len() as u32, 1),
        StretchHorizontally => (1, edge.len() as u32),
        SolidColor => return None,
    };
    let mut img = RgbaImage::new(w, h);
    for (i, packed) in edge.iter().enumerate() {
        let c = Color::unpack(*packed);
        let (x, y) = if *fill_mode == StretchVertically {
            (i as u32, 0)
        } else {
            (0, i as u32)
        };
        img.put_pixel(x, y, Rgba([c.r, c.g, c.b, c.a]));
    }
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;

    fn gradient(w: u32, h: u32) -> Arc<RgbaImage> {
        Arc::new(RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x * 5) as u8, (y * 5) as u8, 128, 255])
        }))
    }

    #[test]
    fn pixelate_grid_follows_the_image() {
        let source = gradient(40, 40);
        let mut a = Annotation::effect(EffectKind::Pixelate);
        a.start = Point::new(5.0, 5.0);
        a.end = Point::new(25.0, 25.0);
        let mut cache = EffectCache::default();
        update_effect(&mut a, &source, 1, &mut cache);
        let bitmap = a.effect_data().unwrap().bitmap.clone().unwrap();
        assert_eq!(bitmap.dimensions(), (20, 20));
        // Blocks are 10x10 from the image origin: pixel (5,5) and (14,14) of the image
        // are in different blocks, (10,10) and (19,19) in the same one.
        assert_eq!(bitmap.get_pixel(5, 5), bitmap.get_pixel(14, 14));
        assert_ne!(bitmap.get_pixel(4, 4), bitmap.get_pixel(5, 5));
    }

    #[test]
    fn region_outside_the_image_stays_transparent() {
        let source = gradient(20, 20);
        let mut a = Annotation::effect(EffectKind::Highlight);
        a.start = Point::new(10.0, 10.0);
        a.end = Point::new(30.0, 30.0);
        let mut cache = EffectCache::default();
        update_effect(&mut a, &source, 1, &mut cache);
        let bitmap = a.effect_data().unwrap().bitmap.clone().unwrap();
        assert_eq!(bitmap.get_pixel(15, 15).0[3], 0);
        assert_eq!(bitmap.get_pixel(2, 2).0[2], 0, "blue is limited by yellow");
    }

    #[test]
    fn magnify_zooms_the_centre() {
        let source = gradient(40, 40);
        let mut a = Annotation::effect(EffectKind::Magnify);
        a.start = Point::new(0.0, 0.0);
        a.end = Point::new(40.0, 40.0);
        let mut cache = EffectCache::default();
        update_effect(&mut a, &source, 1, &mut cache);
        let bitmap = a.effect_data().unwrap().bitmap.clone().unwrap();
        // 2x: the left edge shows the pixels from x = 10.
        let left = bitmap.get_pixel(0, 20).0[0] as i32;
        assert!((left - 50).abs() <= 6, "{left}");
    }
}

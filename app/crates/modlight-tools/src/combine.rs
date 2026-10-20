//! `ImageHelpers.CombineImages`: pictures side by side or one under another, aligned,
//! spaced, wrapped after so many, on the colour of the first picture's bottom right
//! pixel when asked to.

use image::{Rgba, RgbaImage};

use crate::settings::{ImageCombinerAlignment, ImageCombinerOptions, ImageCombinerOrientation};

/// Where each picture goes: (x, y) per picture, and the whole size.
pub fn layout(
    sizes: &[(u32, u32)],
    options: &ImageCombinerOptions,
) -> (Vec<(i64, i64)>, (u32, u32)) {
    let space = options.space;
    let wrap = options.wrap_after.max(0) as usize;
    let mut positions = Vec::with_capacity(sizes.len());
    let (mut x, mut y) = (0i64, 0i64);
    let mut current = 0i64;
    let horizontal = options.orientation == ImageCombinerOrientation::Horizontal;
    for (i, (w, h)) in sizes.iter().enumerate() {
        let (w, h) = (*w as i64, *h as i64);
        let along = |s: &(u32, u32)| if horizontal { s.1 as i64 } else { s.0 as i64 };
        if wrap > 0 {
            if i % wrap == 0 {
                if i > 0 {
                    if horizontal {
                        x = 0;
                        y += current + space;
                    } else {
                        x += current + space;
                        y = 0;
                    }
                }
                current = sizes[i..(i + wrap).min(sizes.len())]
                    .iter()
                    .map(along)
                    .max()
                    .unwrap_or(0);
            }
        } else if i == 0 {
            current = sizes.iter().map(along).max().unwrap_or(0);
        }
        let own = if horizontal { h } else { w };
        // C#'s integer division truncates toward zero, as Rust's does.
        let offset = match options.alignment {
            ImageCombinerAlignment::LeftOrTop => 0,
            ImageCombinerAlignment::Center => current / 2 - own / 2,
            ImageCombinerAlignment::RightOrBottom => current - own,
        };
        if horizontal {
            positions.push((x, y + offset));
            x += w + space;
        } else {
            positions.push((x + offset, y));
            y += h + space;
        }
    }
    // `imageRects.Combine()`: the union of the rectangles.
    let mut left = i64::MAX;
    let mut top = i64::MAX;
    let mut right = i64::MIN;
    let mut bottom = i64::MIN;
    for ((px, py), (w, h)) in positions.iter().zip(sizes) {
        left = left.min(*px);
        top = top.min(*py);
        right = right.max(px + *w as i64);
        bottom = bottom.max(py + *h as i64);
    }
    if positions.is_empty() {
        return (positions, (0, 0));
    }
    (
        positions,
        ((right - left).max(1) as u32, (bottom - top).max(1) as u32),
    )
}

/// Source-over blending of one pixel (GDI+'s `DrawImage` on an ARGB bitmap).
fn blend(dst: &mut Rgba<u8>, src: Rgba<u8>) {
    let sa = src.0[3] as f32 / 255.0;
    if sa >= 1.0 {
        *dst = src;
        return;
    }
    if sa <= 0.0 {
        return;
    }
    let da = dst.0[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    for c in 0..3 {
        let s = src.0[c] as f32 / 255.0;
        let d = dst.0[c] as f32 / 255.0;
        let o = if oa > 0.0 {
            (s * sa + d * da * (1.0 - sa)) / oa
        } else {
            0.0
        };
        dst.0[c] = (o * 255.0).round() as u8;
    }
    dst.0[3] = (oa * 255.0).round() as u8;
}

/// `CombineImages(images, ...)`: `None` for fewer than two pictures.
pub fn combine(images: &[RgbaImage], options: &ImageCombinerOptions) -> Option<RgbaImage> {
    if images.len() < 2 {
        return None;
    }
    let sizes: Vec<(u32, u32)> = images.iter().map(|i| i.dimensions()).collect();
    let (positions, (w, h)) = layout(&sizes, options);
    let mut out = RgbaImage::new(w, h);
    for (i, (image, (x, y))) in images.iter().zip(&positions).enumerate() {
        if options.auto_fill_background && i == 0 && image.width() > 0 && image.height() > 0 {
            let background = *image.get_pixel(image.width() - 1, image.height() - 1);
            for p in out.pixels_mut() {
                *p = background;
            }
        }
        for (sx, sy, p) in image.enumerate_pixels() {
            let (dx, dy) = (x + sx as i64, y + sy as i64);
            if dx < 0 || dy < 0 || dx >= w as i64 || dy >= h as i64 {
                continue;
            }
            blend(out.get_pixel_mut(dx as u32, dy as u32), *p);
        }
    }
    Some(out)
}

/// `CombineImages(imageFiles, ...)`: the pictures that load, combined when two or more do.
pub fn combine_files(
    files: &[std::path::PathBuf],
    options: &ImageCombinerOptions,
) -> Option<RgbaImage> {
    let images: Vec<RgbaImage> = files
        .iter()
        .filter_map(|f| crate::images::load(f).ok())
        .collect();
    combine(&images, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(
        orientation: ImageCombinerOrientation,
        alignment: ImageCombinerAlignment,
        space: i64,
        wrap_after: i64,
    ) -> ImageCombinerOptions {
        ImageCombinerOptions {
            orientation,
            alignment,
            space,
            wrap_after,
            auto_fill_background: false,
        }
    }

    #[test]
    fn vertical_left() {
        let (pos, size) = layout(
            &[(10, 4), (6, 2), (8, 3)],
            &options(
                ImageCombinerOrientation::Vertical,
                ImageCombinerAlignment::LeftOrTop,
                2,
                0,
            ),
        );
        assert_eq!(pos, vec![(0, 0), (0, 6), (0, 10)]);
        assert_eq!(size, (10, 13));
    }

    #[test]
    fn horizontal_centred_and_bottom() {
        let sizes = [(4, 10), (2, 5), (3, 7)];
        let (pos, size) = layout(
            &sizes,
            &options(
                ImageCombinerOrientation::Horizontal,
                ImageCombinerAlignment::Center,
                1,
                0,
            ),
        );
        // 10/2 - 5/2 = 5 - 2 = 3; 10/2 - 7/2 = 5 - 3 = 2.
        assert_eq!(pos, vec![(0, 0), (5, 3), (8, 2)]);
        assert_eq!(size, (11, 10));
        let (pos, _) = layout(
            &sizes,
            &options(
                ImageCombinerOrientation::Horizontal,
                ImageCombinerAlignment::RightOrBottom,
                0,
                0,
            ),
        );
        assert_eq!(pos, vec![(0, 0), (4, 5), (6, 3)]);
    }

    #[test]
    fn wrapping() {
        let sizes = [(4, 4), (4, 6), (4, 2), (4, 3)];
        let (pos, size) = layout(
            &sizes,
            &options(
                ImageCombinerOrientation::Horizontal,
                ImageCombinerAlignment::LeftOrTop,
                1,
                2,
            ),
        );
        assert_eq!(pos, vec![(0, 0), (5, 0), (0, 7), (5, 7)]);
        assert_eq!(size, (9, 10));
        let (pos, size) = layout(
            &sizes,
            &options(
                ImageCombinerOrientation::Vertical,
                ImageCombinerAlignment::LeftOrTop,
                0,
                3,
            ),
        );
        assert_eq!(pos, vec![(0, 0), (0, 4), (0, 10), (4, 0)]);
        assert_eq!(size, (8, 12));
    }

    #[test]
    fn pixels_and_background() {
        let a = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
        let mut b = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 255, 255]));
        b.put_pixel(0, 0, Rgba([0, 0, 255, 255]));
        let mut opts = options(
            ImageCombinerOrientation::Horizontal,
            ImageCombinerAlignment::LeftOrTop,
            1,
            0,
        );
        let out = combine(&[a.clone(), b.clone()], &opts).unwrap();
        assert_eq!(out.dimensions(), (4, 2));
        assert_eq!(out.get_pixel(2, 0).0, [0, 0, 0, 0]);
        assert_eq!(out.get_pixel(3, 0).0, [0, 0, 255, 255]);
        opts.auto_fill_background = true;
        let out = combine(&[a, b], &opts).unwrap();
        assert_eq!(out.get_pixel(2, 0).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(3, 1).0, [255, 0, 0, 255]);
        assert!(combine(&[RgbaImage::new(1, 1)], &opts).is_none());
    }
}

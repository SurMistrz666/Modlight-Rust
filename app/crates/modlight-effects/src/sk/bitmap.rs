//! `SKBitmap` as the effects use it: a Bgra8888 premultiplied bitmap whose `Pixels` reads
//! and writes straight colours. The port keeps the straight colours (`Bitmap::px`) and
//! applies Skia's premultiply / unpremultiply rounding wherever upstream's bitmap would
//! store or read them, so semi-transparent pixels come out as they do on Windows.

use image::RgbaImage;
use tiny_skia::{IntSize, Pixmap};

use super::color::C;

#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub w: i32,
    pub h: i32,
    pub px: Vec<C>,
}

/// `SkPremultiplyARGBInline` / the `RGBA_to_rgbA` swizzle: `(c * a + 127) / 255`.
#[inline]
pub fn premul(c: C) -> C {
    if c.a == 255 {
        return c;
    }
    let a = c.a as u32;
    let m = |v: u8| ((v as u32 * a + 127) / 255) as u8;
    C::new(m(c.r), m(c.g), m(c.b), c.a)
}

/// Premultiplied memory read back as straight colour, as `SKBitmap.Pixels` reads it
/// (`SkUnPreMultiply`'s scale table: `((255 << 24) + a / 2) / a`, then `(s * c + 2^23) >> 24`;
/// checked against SkiaSharp for every alpha and value). Components above alpha (an
/// invalid premultiplied value an effect wrote directly into memory) clamp to 255.
#[inline]
pub fn unpremul(p: C) -> C {
    if p.a == 255 {
        return p;
    }
    if p.a == 0 {
        return C::ZERO;
    }
    let a = p.a as u32;
    let scale = ((255u32 << 24) + (a >> 1)) / a;
    let u = |v: u8| (((scale as u64 * v as u64 + (1u64 << 23)) >> 24).min(255)) as u8;
    C::new(u(p.r), u(p.g), u(p.b), p.a)
}

/// A straight colour as it reads back after being stored in a premultiplied bitmap.
#[inline]
pub fn store(c: C) -> C {
    if c.a == 255 { c } else { unpremul(premul(c)) }
}

impl Bitmap {
    /// `new SKBitmap(w, h)`: transparent.
    pub fn new(w: i32, h: i32) -> Bitmap {
        let w = w.max(0);
        let h = h.max(0);
        Bitmap {
            w,
            h,
            px: vec![C::ZERO; (w * h) as usize],
        }
    }

    pub fn filled(w: i32, h: i32, c: C) -> Bitmap {
        Bitmap {
            w,
            h,
            px: vec![store(c); (w.max(0) * h.max(0)) as usize],
        }
    }

    /// `new SKBitmap(...) { Pixels = px }`: the colours as they read back.
    pub fn from_pixels(w: i32, h: i32, mut px: Vec<C>) -> Bitmap {
        debug_assert_eq!(px.len(), (w * h) as usize);
        for p in px.iter_mut() {
            *p = store(*p);
        }
        Bitmap { w, h, px }
    }

    /// A raw premultiplied buffer (what `ApplyPixelOperation` writes through its pointer).
    pub fn from_premul_raw(w: i32, h: i32, raw: Vec<C>) -> Bitmap {
        Bitmap {
            w,
            h,
            px: raw.into_iter().map(unpremul).collect(),
        }
    }

    pub fn premul_raw(&self) -> Vec<C> {
        self.px.iter().map(|c| premul(*c)).collect()
    }

    pub fn from_image(image: &RgbaImage) -> Bitmap {
        let px = image
            .pixels()
            .map(|p| C::new(p.0[0], p.0[1], p.0[2], p.0[3]))
            .collect();
        Bitmap {
            w: image.width() as i32,
            h: image.height() as i32,
            px,
        }
    }

    pub fn to_image(&self) -> RgbaImage {
        let mut raw = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            raw.extend_from_slice(&[p.r, p.g, p.b, p.a]);
        }
        RgbaImage::from_raw(self.w.max(0) as u32, self.h.max(0) as u32, raw)
            .unwrap_or_else(|| RgbaImage::new(0, 0))
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.w + x) as usize
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> C {
        self.px[(y * self.w + x) as usize]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: C) {
        let i = (y * self.w + x) as usize;
        self.px[i] = store(c);
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// The premultiplied pixmap Skia would draw from (`None` for an empty bitmap).
    pub fn to_pixmap(&self) -> Pixmap {
        let mut data = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let q = premul(*p);
            data.extend_from_slice(&[q.r, q.g, q.b, q.a]);
        }
        let size = IntSize::from_wh(self.w.max(1) as u32, self.h.max(1) as u32).unwrap();
        if self.is_empty() {
            return Pixmap::new(1, 1).unwrap();
        }
        Pixmap::from_vec(data, size).unwrap()
    }

    pub fn from_pixmap(pixmap: &Pixmap) -> Bitmap {
        let px = pixmap
            .data()
            .chunks_exact(4)
            .map(|q| unpremul(C::new(q[0], q[1], q[2], q[3])))
            .collect();
        Bitmap {
            w: pixmap.width() as i32,
            h: pixmap.height() as i32,
            px,
        }
    }
}

/// `AdjustmentImageEffectBase.ApplyPixelOperation` on a Bgra8888 bitmap: the operation
/// sees the premultiplied memory as an `SKColor` and its result is written back raw.
pub fn pixel_op(source: &Bitmap, mut op: impl FnMut(C) -> C) -> Bitmap {
    let px = source.px.iter().map(|c| unpremul(op(premul(*c)))).collect();
    Bitmap {
        w: source.w,
        h: source.h,
        px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premul_round_trip_is_lossless_for_stored_values() {
        for a in 1..=255u8 {
            for v in 0..=255u8 {
                let p = premul(C::new(v, v, v, a));
                assert_eq!(premul(unpremul(p)), p, "{v} {a}");
            }
        }
    }
}

//! Skia's CPU blurs, as SkiaSharp 3.119 (Skia m119) runs them:
//!
//! - `SKImageFilter.CreateBlur`: `SkBlurImageFilter`'s `cpu_blur` with its `GaussPass`
//!   (the three box passes of the SVG spec folded into one pass with circular buffers, for
//!   windows below 255) and `TentPass` (two passes, for larger sigmas), on premultiplied
//!   8-bit pixels, transparent outside the source (decal).
//! - `SKMaskFilter.CreateBlur(Normal, sigma)`: `SkMaskBlurFilter`, the same Gauss plan for
//!   sigma >= 2 and Lindeberg's discrete Gaussian (Bessel functions) below.

/// `calculate_window`: the box size of the SVG approximation.
pub fn calculate_window(sigma: f64) -> i32 {
    let possible = (sigma * 3.0 * (2.0 * std::f64::consts::PI).sqrt() / 4.0 + 0.5).floor() as i32;
    possible.max(1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PassKind {
    Gauss,
    Tent,
}

/// One-dimensional blur pass with its circular buffers.
struct Pass {
    kind: PassKind,
    window: i32,
    border: i32,
    factor: u64,
    half: u32,
    buf: Vec<[u32; 4]>,
    b1: usize,
    b2: usize,
    end: usize,
    c0: usize,
    c1: usize,
    c2: usize,
    sum0: [u32; 4],
    sum1: [u32; 4],
    sum2: [u32; 4],
}

impl Pass {
    /// `GaussPass::MakeMaker`, then `TentPass::MakeMaker` when the window is too large.
    fn new(sigma: f64) -> Pass {
        let window = calculate_window(sigma);
        if window < 255 {
            let pass_size = (window - 1) as usize;
            let b1 = pass_size;
            let b2 = 2 * pass_size;
            let end = b2
                + if window & 1 == 1 {
                    pass_size
                } else {
                    pass_size + 1
                };
            let border = if window & 1 == 1 {
                3 * ((window - 1) / 2)
            } else {
                3 * (window / 2) - 1
            };
            let w2 = (window * window) as u64;
            let w3 = w2 * window as u64;
            let divisor = if window & 1 == 1 { w3 } else { w3 + w2 };
            Pass::with(PassKind::Gauss, window, border, divisor, b1, b2, end)
        } else {
            let window = 3 * window / 2;
            let pass_size = (window - 1) as usize;
            let b1 = pass_size;
            let end = b1 + pass_size;
            let divisor = (window as u64) * (window as u64);
            Pass::with(PassKind::Tent, window, window - 1, divisor, b1, end, end)
        }
    }

    fn with(
        kind: PassKind,
        window: i32,
        border: i32,
        divisor: u64,
        b1: usize,
        b2: usize,
        end: usize,
    ) -> Pass {
        let factor = ((1.0 / divisor as f64) * (1u64 << 32) as f64).round() as u64;
        Pass {
            kind,
            window,
            border,
            factor,
            half: ((divisor + 1) >> 1) as u32,
            buf: vec![[0; 4]; end.max(1)],
            b1,
            b2,
            end,
            c0: 0,
            c1: b1,
            c2: b2,
            sum0: [0; 4],
            sum1: [0; 4],
            sum2: [0; 4],
        }
    }

    fn start(&mut self) {
        self.sum0 = [0; 4];
        match self.kind {
            PassKind::Gauss => {
                self.sum1 = [0; 4];
                self.sum2 = [self.half; 4];
            }
            PassKind::Tent => {
                self.sum1 = [self.half; 4];
                self.sum2 = [0; 4];
            }
        }
        for b in self.buf.iter_mut() {
            *b = [0; 4];
        }
        self.c0 = 0;
        self.c1 = self.b1;
        self.c2 = self.b2;
    }

    #[inline]
    fn step(&mut self, lead: [u32; 4]) -> [u8; 4] {
        let mut out = [0u8; 4];
        match self.kind {
            PassKind::Gauss => {
                for k in 0..4 {
                    self.sum0[k] = self.sum0[k].wrapping_add(lead[k]);
                    self.sum1[k] = self.sum1[k].wrapping_add(self.sum0[k]);
                    self.sum2[k] = self.sum2[k].wrapping_add(self.sum1[k]);
                    out[k] = ((self.sum2[k] as u64 * self.factor) >> 32) as u8;
                }
                if self.b2 < self.end {
                    let b = self.buf[self.c2];
                    for k in 0..4 {
                        self.sum2[k] = self.sum2[k].wrapping_sub(b[k]);
                    }
                    self.buf[self.c2] = self.sum1;
                    self.c2 = if self.c2 + 1 < self.end {
                        self.c2 + 1
                    } else {
                        self.b2
                    };
                }
                if self.b1 < self.b2 {
                    let b = self.buf[self.c1];
                    for k in 0..4 {
                        self.sum1[k] = self.sum1[k].wrapping_sub(b[k]);
                    }
                    self.buf[self.c1] = self.sum0;
                    self.c1 = if self.c1 + 1 < self.b2 {
                        self.c1 + 1
                    } else {
                        self.b1
                    };
                }
                if self.b1 > 0 {
                    let b = self.buf[self.c0];
                    for k in 0..4 {
                        self.sum0[k] = self.sum0[k].wrapping_sub(b[k]);
                    }
                    self.buf[self.c0] = lead;
                    self.c0 = if self.c0 + 1 < self.b1 {
                        self.c0 + 1
                    } else {
                        0
                    };
                }
            }
            PassKind::Tent => {
                for k in 0..4 {
                    self.sum0[k] = self.sum0[k].wrapping_add(lead[k]);
                    self.sum1[k] = self.sum1[k].wrapping_add(self.sum0[k]);
                    out[k] = ((self.sum1[k] as u64 * self.factor) >> 32) as u8;
                }
                if self.b1 < self.end {
                    let b = self.buf[self.c1];
                    for k in 0..4 {
                        self.sum1[k] = self.sum1[k].wrapping_sub(b[k]);
                    }
                    self.buf[self.c1] = self.sum0;
                    self.c1 = if self.c1 + 1 < self.end {
                        self.c1 + 1
                    } else {
                        self.b1
                    };
                }
                if self.b1 > 0 {
                    let b = self.buf[self.c0];
                    for k in 0..4 {
                        self.sum0[k] = self.sum0[k].wrapping_sub(b[k]);
                    }
                    self.buf[self.c0] = lead;
                    self.c0 = if self.c0 + 1 < self.b1 {
                        self.c0 + 1
                    } else {
                        0
                    };
                }
            }
        }
        out
    }

    /// `Pass::blur`: the source covers `[src_left, src_right)` of a destination line of
    /// `dst_right` pixels; reads `src[s0 + i * ss]`, writes `dst[d0 + i * ds]`.
    #[allow(clippy::too_many_arguments)]
    fn blur(
        &mut self,
        src_left: i32,
        src_right: i32,
        dst_right: i32,
        src: &[[u8; 4]],
        s0: usize,
        ss: usize,
        dst: &mut [[u8; 4]],
        d0: usize,
        ds: usize,
    ) {
        self.start();
        let src_end = src_right - self.border;
        let dst_end = dst_right;
        let mut src_idx = src_left - self.border;
        let mut dst_idx = 0;
        let mut sc = s0;
        let mut dc = d0;
        let load = |p: [u8; 4]| [p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32];
        if dst_idx < src_idx {
            while dst_idx < src_idx {
                dst[dc] = [0; 4];
                dc += ds;
                dst_idx += 1;
            }
        } else if src_idx < dst_idx {
            let common_end = dst_idx.min(src_end);
            if src_idx < common_end {
                let n = common_end - src_idx;
                for _ in 0..n {
                    self.step(load(src[sc]));
                    sc += ss;
                }
                src_idx += n;
            }
            if src_idx < dst_idx {
                let n = dst_idx - src_idx;
                for _ in 0..n {
                    self.step([0; 4]);
                }
                src_idx += n;
            }
        }
        debug_assert_eq!(src_idx, dst_idx);
        let common_end = dst_end.min(src_end);
        if dst_idx < common_end {
            let n = common_end - dst_idx;
            for _ in 0..n {
                dst[dc] = self.step(load(src[sc]));
                sc += ss;
                dc += ds;
            }
            dst_idx += n;
        }
        if dst_idx < dst_end {
            let n = dst_end - dst_idx;
            for _ in 0..n {
                dst[dc] = self.step([0; 4]);
                dc += ds;
            }
        }
    }
}

/// `cpu_blur`: a premultiplied source of `sw` x `sh` placed at (`sx`, `sy`) in a
/// destination of `dw` x `dh`; returns the destination's premultiplied pixels (transparent
/// where the blur does not reach).
#[allow(clippy::too_many_arguments)]
pub fn blur_premul(
    src: &[[u8; 4]],
    sw: i32,
    sh: i32,
    sx: i32,
    sy: i32,
    dw: i32,
    dh: i32,
    sigma_x: f32,
    sigma_y: f32,
) -> Vec<[u8; 4]> {
    let mut dst = vec![[0u8; 4]; (dw.max(0) * dh.max(0)) as usize];
    if sw <= 0 || sh <= 0 || dw <= 0 || dh <= 0 {
        return dst;
    }
    let sigma_x = (sigma_x.max(0.0) as f64).min(532.0);
    let sigma_y = (sigma_y.max(0.0) as f64).min(532.0);
    let mut px = Pass::new(sigma_x);
    let mut py = Pass::new(sigma_y);
    if px.window <= 1 && py.window <= 1 {
        for y in 0..sh {
            let ty = y + sy;
            if ty < 0 || ty >= dh {
                continue;
            }
            for x in 0..sw {
                let tx = x + sx;
                if tx >= 0 && tx < dw {
                    dst[(ty * dw + tx) as usize] = src[(y * sw + x) as usize];
                }
            }
        }
        return dst;
    }

    // Horizontal pass into an intermediate of `sh` rows the width of the destination.
    let (inter, iw, ix): (std::borrow::Cow<[[u8; 4]]>, i32, i32) = if px.window > 1 {
        let mut tmp = vec![[0u8; 4]; (dw * sh) as usize];
        for y in 0..sh {
            px.blur(
                sx,
                sx + sw,
                dw,
                src,
                (y * sw) as usize,
                1,
                &mut tmp,
                (y * dw) as usize,
                1,
            );
        }
        (std::borrow::Cow::Owned(tmp), dw, 0)
    } else {
        (std::borrow::Cow::Borrowed(src), sw, sx)
    };

    if py.window > 1 {
        let mut col_out = vec![[0u8; 4]; dh as usize];
        for x in 0..iw {
            let tx = x + ix;
            if tx < 0 || tx >= dw {
                continue;
            }
            py.blur(
                sy,
                sy + sh,
                dh,
                &inter,
                x as usize,
                iw as usize,
                &mut col_out,
                0,
                1,
            );
            for (y, v) in col_out.iter().enumerate() {
                dst[y * dw as usize + tx as usize] = *v;
            }
        }
    } else {
        for y in 0..sh {
            let ty = y + sy;
            if ty < 0 || ty >= dh {
                continue;
            }
            for x in 0..iw {
                let tx = x + ix;
                if tx >= 0 && tx < dw {
                    dst[(ty * dw + tx) as usize] = inter[(y * iw + x) as usize];
                }
            }
        }
    }
    dst
}

/// The output bounds Skia gives a blur: the source outset by three sigmas.
pub fn blur_outset(sigma: f32) -> i32 {
    (3.0 * sigma.max(0.0)).ceil() as i32
}

// ---------- mask filter blur ----------

/// `SkGaussFilter` (sigma < 2): Lindeberg's discrete Gaussian, normalised; index 0 is
/// the centre.
fn bessel_gauss(sigma: f64) -> Vec<f64> {
    let var = sigma * sigma;
    let besseli0 = |t: f64| {
        let q = t * t / 4.0;
        let mut sum = 1.0;
        let mut factor = 1.0;
        let mut k = 1.0;
        while factor > 1.0 / 1_000_000.0 {
            factor *= q / (k * k);
            sum += factor;
            k += 1.0;
        }
        sum
    };
    let besseli1 = |t: f64| {
        let q = t * t / 4.0;
        let mut sum = t / 2.0;
        let mut factor = sum;
        let mut k = 1.0;
        while factor > 1.0 / 1_000_000.0 {
            factor *= q / (k * (k + 1.0));
            sum += factor;
            k += 1.0;
        }
        sum
    };
    let d = var.exp();
    let mut b = vec![besseli0(var), besseli1(var)];
    let mut gauss = vec![b[0] / d, b[1] / d];
    let mut n = 1usize;
    while gauss[n] > 0.001 && n < 8 {
        let next = -(2.0 * n as f64 / var) * b[n] + b[n - 1];
        b.push(next);
        gauss.push(next / d);
        n += 1;
    }
    gauss.truncate(n);
    let mut sum = 0.0;
    for g in gauss.iter().skip(1).rev() {
        sum += 2.0 * g;
    }
    sum += gauss[0];
    for g in gauss.iter_mut() {
        *g /= sum;
    }
    let mut sum = 0.0;
    for g in gauss.iter().skip(1).rev() {
        sum += 2.0 * g;
    }
    gauss[0] = 1.0 - sum;
    gauss
}

/// Blurs a premultiplied layer as `SKMaskFilter.CreateBlur(Normal, sigma)` blurs the
/// coverage of what is drawn with it (the layer is the solid-colour drawing).
pub fn mask_blur_layer(layer: &[[u8; 4]], w: i32, h: i32, sigma: f32) -> Vec<[u8; 4]> {
    let sigma = (sigma.max(0.0) as f64).min(135.0);
    if 3.0 * sigma <= 1.0 {
        return layer.to_vec();
    }
    if sigma >= 2.0 {
        return blur_premul(layer, w, h, 0, 0, w, h, sigma as f32, sigma as f32);
    }
    let g = bessel_gauss(sigma);
    let weights: Vec<u32> = g.iter().map(|d| (d * 65536.0).round() as u32).collect();
    let r = weights.len() as i32 - 1;
    let conv = |get: &dyn Fn(i32) -> [u8; 4], n: i32, i: i32| -> [u8; 4] {
        let mut acc = [0u32; 4];
        for k in -r..=r {
            let j = i + k;
            if j < 0 || j >= n {
                continue;
            }
            let p = get(j);
            let wgt = weights[k.unsigned_abs() as usize];
            for c in 0..4 {
                acc[c] += p[c] as u32 * wgt;
            }
        }
        let mut out = [0u8; 4];
        for c in 0..4 {
            out[c] = ((acc[c] + (1 << 15)) >> 16).min(255) as u8;
        }
        out
    };
    let mut tmp = vec![[0u8; 4]; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            tmp[(y * w + x) as usize] = conv(&|j| layer[(y * w + j) as usize], w, x);
        }
    }
    let mut out = vec![[0u8; 4]; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            out[(y * w + x) as usize] = conv(&|j| tmp[(j * w + x) as usize], h, y);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_and_constant_regions() {
        assert_eq!(calculate_window(3.0), 6);
        assert_eq!(calculate_window(0.5), 1);
        // A constant image stays constant in its interior.
        let src = vec![[100u8, 50, 25, 255]; 40 * 40];
        let out = blur_premul(&src, 40, 40, 0, 0, 40, 40, 2.0, 2.0);
        assert_eq!(out[20 * 40 + 20], [100, 50, 25, 255]);
        // Edges fade towards transparent.
        assert!(out[0][3] < 255);
    }

    #[test]
    fn bessel_kernel_sums_to_one() {
        let g = bessel_gauss(1.0);
        let s: f64 = g[0] + 2.0 * g[1..].iter().sum::<f64>();
        assert!((s - 1.0).abs() < 1e-9);
    }
}

//! Skia's anti-aliased rectangles (`SkScan::AntiFillRect`, `SkScan::AntiFrameRect`):
//! coverage in 1/256 of a pixel (FDot8), with the blitters' own rounding, instead of
//! the path rasteriser's quarter-pixel rows.

use tiny_skia::{Mask, Rect};

/// A coverage mask placed at (`x`, `y`) on the canvas.
pub struct Coverage {
    pub x: i32,
    pub y: i32,
    pub mask: Mask,
}

/// Collects blits into a coverage grid (pieces never overlap in Skia; if they do, the
/// coverages combine as two draws would).
struct Sink {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    data: Vec<u8>,
}

impl Sink {
    fn new(l: i32, t: i32, r: i32, b: i32) -> Sink {
        let (w, h) = ((r - l).max(1), (b - t).max(1));
        Sink {
            x0: l,
            y0: t,
            w,
            h,
            data: vec![0; (w * h) as usize],
        }
    }

    fn put(&mut self, x: i32, y: i32, a: u32) {
        let (lx, ly) = (x - self.x0, y - self.y0);
        if a == 0 || lx < 0 || ly < 0 || lx >= self.w || ly >= self.h {
            return;
        }
        let v = &mut self.data[(ly * self.w + lx) as usize];
        let a = a.min(255);
        let old = *v as u32;
        *v = (old + a - (old * a + 127) / 255) as u8;
    }

    fn blit_v(&mut self, x: i32, y: i32, height: i32, a: u32) {
        for k in 0..height {
            self.put(x, y + k, a);
        }
    }

    fn blit_h(&mut self, x: i32, y: i32, width: i32, a: u32) {
        for k in 0..width {
            self.put(x + k, y, a);
        }
    }

    fn blit_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for j in 0..h {
            self.blit_h(x, y + j, w, 255);
        }
    }

    fn finish(self) -> Coverage {
        let mut mask = Mask::new(self.w as u32, self.h as u32).unwrap();
        mask.data_mut().copy_from_slice(&self.data);
        Coverage {
            x: self.x0,
            y: self.y0,
            mask,
        }
    }
}

/// `SkScalarToFixed` then `SkFixedToFDot8`.
fn fdot8(v: f32) -> i32 {
    let fixed = (v * 65536.0) as i32;
    (fixed + 0x80) >> 8
}

fn floor8(v: i32) -> i32 {
    v >> 8
}

fn ceil8(v: i32) -> i32 {
    (v + 0xFF) >> 8
}

/// `SkAlphaMul(value, alpha256)`.
fn alpha_mul(v: u32, a256: u32) -> u32 {
    (v * a256) >> 8
}

/// `InvAlphaMul`: a + b - a*b/255, rounded.
fn inv_alpha_mul(a: u32, b: u32) -> u32 {
    let prod = a * b + 128;
    let r = (prod + (prod >> 8)) >> 8;
    a + b - r
}

fn do_scanline(s: &mut Sink, l: i32, top: i32, r: i32, alpha: u32) {
    if l >> 8 == (r - 1) >> 8 {
        s.blit_v(l >> 8, top, 1, alpha_mul(alpha, (r - l) as u32));
        return;
    }
    let mut left = l >> 8;
    if l & 0xFF != 0 {
        s.blit_v(left, top, 1, alpha_mul(alpha, (256 - (l & 0xFF)) as u32));
        left += 1;
    }
    let rite = r >> 8;
    let width = rite - left;
    if width > 0 {
        s.blit_h(left, top, width, alpha);
    }
    if r & 0xFF != 0 {
        s.blit_v(rite, top, 1, alpha_mul(alpha, (r & 0xFF) as u32));
    }
}

fn antifilldot8(s: &mut Sink, l: i32, t: i32, r: i32, b: i32, fill_inner: bool) {
    if l >= r || t >= b {
        return;
    }
    let mut top = t >> 8;
    if top == (b - 1) >> 8 {
        do_scanline(s, l, top, r, (b - t - 1) as u32);
        return;
    }
    if t & 0xFF != 0 {
        do_scanline(s, l, top, r, (256 - (t & 0xFF)) as u32);
        top += 1;
    }
    let bot = b >> 8;
    let height = bot - top;
    if height > 0 {
        let mut left = l >> 8;
        if left == (r - 1) >> 8 {
            s.blit_v(left, top, height, (r - l - 1) as u32);
        } else {
            if l & 0xFF != 0 {
                s.blit_v(left, top, height, (256 - (l & 0xFF)) as u32);
                left += 1;
            }
            let rite = r >> 8;
            let width = rite - left;
            if width > 0 && fill_inner {
                s.blit_rect(left, top, width, height);
            }
            if r & 0xFF != 0 {
                s.blit_v(rite, top, height, (r & 0xFF) as u32);
            }
        }
    }
    if b & 0xFF != 0 {
        do_scanline(s, l, bot, r, (b & 0xFF) as u32);
    }
}

fn inner_scanline(s: &mut Sink, l: i32, top: i32, r: i32, alpha: u32) {
    if l >> 8 == (r - 1) >> 8 {
        s.blit_v(
            l >> 8,
            top,
            1,
            inv_alpha_mul(alpha, (255 - (r - l)).max(0) as u32),
        );
        return;
    }
    let mut left = l >> 8;
    if l & 0xFF != 0 {
        s.blit_v(left, top, 1, inv_alpha_mul(alpha, (l & 0xFF) as u32));
        left += 1;
    }
    let rite = r >> 8;
    let width = rite - left;
    if width > 0 {
        s.blit_h(left, top, width, alpha);
    }
    if r & 0xFF != 0 {
        s.blit_v(rite, top, 1, inv_alpha_mul(alpha, (!r & 0xFF) as u32));
    }
}

fn innerstrokedot8(s: &mut Sink, l: i32, t: i32, r: i32, b: i32) {
    let mut top = t >> 8;
    if top == (b - 1) >> 8 {
        let alpha = 256 - (b - t);
        if alpha > 0 {
            inner_scanline(s, l, top, r, alpha as u32);
        }
        return;
    }
    if t & 0xFF != 0 {
        inner_scanline(s, l, top, r, (t & 0xFF) as u32);
        top += 1;
    }
    let bot = b >> 8;
    let height = bot - top;
    if height > 0 {
        if l & 0xFF != 0 {
            s.blit_v(l >> 8, top, height, (l & 0xFF) as u32);
        }
        if r & 0xFF != 0 {
            s.blit_v(r >> 8, top, height, (!r & 0xFF) as u32);
        }
    }
    if b & 0xFF != 0 {
        inner_scanline(s, l, bot, r, (!b & 0xFF) as u32);
    }
}

/// `AntiFillRect` of a device-space rectangle.
pub fn fill(r: Rect) -> Coverage {
    let (l, t, rr, b) = (
        fdot8(r.left()),
        fdot8(r.top()),
        fdot8(r.right()),
        fdot8(r.bottom()),
    );
    let mut s = Sink::new(floor8(l), floor8(t), ceil8(rr), ceil8(b));
    antifilldot8(&mut s, l, t, rr, b, true);
    s.finish()
}

fn align_thin(e1: &mut i32, e2: &mut i32) {
    if floor8(*e1) == floor8(*e2) {
        *e2 -= *e1 & 0xFF;
        *e1 &= !0xFF;
    }
}

/// `AntiFrameRect`: a device-space rectangle stroked with a stroke of the given device
/// width and height.
pub fn frame(r: Rect, stroke: (f32, f32)) -> Coverage {
    let to8 = |v: f32| (v * 256.0) as i32;
    let rx = stroke.0 * 0.5;
    let ry = stroke.1 * 0.5;
    let mut ol = to8(r.left() - rx);
    let mut ot = to8(r.top() - ry);
    let mut or = to8(r.right() + rx);
    let mut ob = to8(r.bottom() + ry);
    let mut s = Sink::new(floor8(ol), floor8(ot), ceil8(or), ceil8(ob));
    let rx = stroke.0 - rx;
    let ry = stroke.1 - ry;
    let mut il = to8(r.left() + rx);
    let mut it = to8(r.top() + ry);
    let mut ir = to8(r.right() - rx);
    let mut ib = to8(r.bottom() - ry);
    if stroke.0 < 1.0 || stroke.1 < 1.0 {
        align_thin(&mut ol, &mut il);
        align_thin(&mut ot, &mut it);
        align_thin(&mut ir, &mut or);
        align_thin(&mut ib, &mut ob);
    }
    antifilldot8(&mut s, ol, ot, or, ob, false);
    let (oxl, oyt, oxr, oyb) = (ceil8(ol), ceil8(ot), floor8(or), floor8(ob));
    let fill_check = |s: &mut Sink, l: i32, t: i32, r: i32, b: i32| {
        if l < r && t < b {
            s.blit_rect(l, t, r - l, b - t);
        }
    };
    if il >= ir || it >= ib {
        fill_check(&mut s, oxl, oyt, oxr, oyb);
    } else {
        let (nl, nt, nr, nb) = (floor8(il), floor8(it), ceil8(ir), ceil8(ib));
        fill_check(&mut s, oxl, oyt, oxr, nt);
        fill_check(&mut s, oxl, nt, nl, nb);
        fill_check(&mut s, nr, nt, oxr, nb);
        fill_check(&mut s, oxl, nb, oxr, oyb);
        innerstrokedot8(&mut s, il, it, ir, ib);
    }
    s.finish()
}

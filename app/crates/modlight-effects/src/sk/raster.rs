//! Anti-aliased path coverage as Skia m119 computes it for most effect shapes: analytic
//! (exact area) coverage, which Skia uses whenever a path has few points for its size
//! (`ShouldUseAAA`); other paths keep tiny-skia's 4x4 supersampling, Skia's other
//! algorithm.

use tiny_skia::{Mask, Path, PathSegment, Point, Transform};

/// `SkScan::ShouldUseAAA` for a device-space path.
pub fn should_use_analytic(path: &Path, is_rect: bool) -> bool {
    if is_rect {
        return true;
    }
    let b = path.bounds();
    let points = path.points().len() as f32;
    points < b.width().max(b.height()) / 2.0 - 10.0
}

/// `cheap_distance`: max + min / 2.
fn cheap_distance(dx: i32, dy: i32) -> i32 {
    let (dx, dy) = (dx.abs(), dy.abs());
    if dx > dy {
        dx + (dy >> 1)
    } else {
        dy + (dx >> 1)
    }
}

/// `diff_to_shift` with the anti-aliasing shift of 2.
fn diff_to_shift(dx: i32, dy: i32) -> i32 {
    let dist = (cheap_distance(dx, dy) + (1 << 4)) >> 5;
    (32 - (dist as u32).leading_zeros() as i32) >> 1
}

/// FDot6 coordinates of the supersampled (x4) edge builder.
fn fdot6(v: f32) -> i32 {
    (v * 256.0) as i32
}

/// `SkQuadraticEdge`: the number of lines a quad becomes (`1 << shift`).
fn quad_segments(p0: Point, p1: Point, p2: Point) -> usize {
    let dx = ((fdot6(p1.x) << 1) - fdot6(p0.x) - fdot6(p2.x)) >> 2;
    let dy = ((fdot6(p1.y) << 1) - fdot6(p0.y) - fdot6(p2.y)) >> 2;
    let shift = diff_to_shift(dx, dy).clamp(1, 6);
    1 << shift
}

/// `SkCubicEdge`: the number of lines a cubic becomes.
fn cubic_segments(p0: Point, p1: Point, p2: Point, p3: Point) -> usize {
    let delta = |a: i32, b: i32, c: i32, d: i32| {
        let one = ((a * 8 - b * 15 + 6 * c + d) * 19) >> 9;
        let two = ((a + 6 * b - c * 15 + d * 8) * 19) >> 9;
        one.abs().max(two.abs())
    };
    let dx = delta(fdot6(p0.x), fdot6(p1.x), fdot6(p2.x), fdot6(p3.x));
    let dy = delta(fdot6(p0.y), fdot6(p1.y), fdot6(p2.y), fdot6(p3.y));
    let shift = (diff_to_shift(dx, dy) + 1).clamp(1, 6);
    1 << shift
}

/// Flattens a path (after `ts`) into line segments, cutting curves into as many lines
/// as Skia's edge builder does.
pub fn flatten(path: &Path, ts: Transform) -> Vec<(Point, Point)> {
    let mut lines = Vec::new();
    let map = |p: Point| {
        let mut q = [p];
        ts.map_points(&mut q);
        q[0]
    };
    let mut start = Point::zero();
    let mut last = Point::zero();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                if last != start {
                    lines.push((last, start));
                }
                start = map(p);
                last = start;
            }
            PathSegment::LineTo(p) => {
                let p = map(p);
                lines.push((last, p));
                last = p;
            }
            PathSegment::QuadTo(c, p) => {
                let c = map(c);
                let p = map(p);
                let n = quad_segments(last, c, p);
                let mut prev = last;
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let mt = 1.0 - t;
                    let q = Point::from_xy(
                        mt * mt * last.x + 2.0 * mt * t * c.x + t * t * p.x,
                        mt * mt * last.y + 2.0 * mt * t * c.y + t * t * p.y,
                    );
                    lines.push((prev, q));
                    prev = q;
                }
                last = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let c1 = map(c1);
                let c2 = map(c2);
                let p = map(p);
                let n = cubic_segments(last, c1, c2, p);
                let mut prev = last;
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let mt = 1.0 - t;
                    let q = Point::from_xy(
                        mt * mt * mt * last.x
                            + 3.0 * mt * mt * t * c1.x
                            + 3.0 * mt * t * t * c2.x
                            + t * t * t * p.x,
                        mt * mt * mt * last.y
                            + 3.0 * mt * mt * t * c1.y
                            + 3.0 * mt * t * t * c2.y
                            + t * t * t * p.y,
                    );
                    lines.push((prev, q));
                    prev = q;
                }
                last = p;
            }
            PathSegment::Close => {
                if last != start {
                    lines.push((last, start));
                }
                last = start;
            }
        }
    }
    if last != start {
        lines.push((last, start));
    }
    lines
}

/// `SnapY`: to the nearest quarter pixel.
fn snap_y(y: f32) -> f32 {
    (y * 4.0).round() / 4.0
}

/// The edges Skia's analytic AA works on: line ends with y snapped to quarter pixels, and
/// curve steps snapped as `SkAnalyticQuadraticEdge::updateQuadratic` snaps them (whole
/// pixels for steps of two pixels or more, quarters below).
pub fn flatten_analytic(path: &Path, ts: Transform) -> Vec<(Point, Point)> {
    let map = |p: Point| {
        let mut q = [p];
        ts.map_points(&mut q);
        q[0]
    };
    let mut out = Vec::new();
    // One curve as a run of points, walked from its top so the snapping goes downwards.
    let curve = |pts: Vec<Point>, out: &mut Vec<(Point, Point)>| {
        let reversed =
            pts.first().map(|p| p.y).unwrap_or(0.0) > pts.last().map(|p| p.y).unwrap_or(0.0);
        let mut pts = pts;
        if reversed {
            pts.reverse();
        }
        let n = pts.len();
        let last_y = snap_y(pts[n - 1].y);
        let mut snapped = Point::from_xy(pts[0].x, snap_y(pts[0].y));
        let mut old = pts[0];
        let mut segs = Vec::with_capacity(n);
        for (i, p) in pts.iter().enumerate().skip(1) {
            let next = if i == n - 1 {
                Point::from_xy(p.x, last_y)
            } else if (p.y - old.y).abs() >= 2.0 {
                let dy = p.y - snapped.y;
                let slope = if dy != 0.0 {
                    (p.x - snapped.x) / dy
                } else {
                    0.0
                };
                let ny = last_y.min(p.y.round());
                Point::from_xy(p.x - slope * (p.y - ny), ny)
            } else {
                Point::from_xy(p.x, last_y.min(snap_y(p.y)))
            };
            segs.push((snapped, next));
            snapped = next;
            old = *p;
        }
        if reversed {
            for (a, b) in segs.into_iter().rev() {
                out.push((b, a));
            }
        } else {
            out.extend(segs);
        }
    };
    let mut start = Point::zero();
    let mut last = Point::zero();
    let line = |a: Point, b: Point, out: &mut Vec<(Point, Point)>| {
        out.push((
            Point::from_xy(a.x, snap_y(a.y)),
            Point::from_xy(b.x, snap_y(b.y)),
        ));
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                if last != start {
                    line(last, start, &mut out);
                }
                start = map(p);
                last = start;
            }
            PathSegment::LineTo(p) => {
                let p = map(p);
                line(last, p, &mut out);
                last = p;
            }
            PathSegment::QuadTo(c, p) => {
                let (c, p) = (map(c), map(p));
                let n = quad_segments(last, c, p);
                let mut pts = vec![last];
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let mt = 1.0 - t;
                    pts.push(Point::from_xy(
                        mt * mt * last.x + 2.0 * mt * t * c.x + t * t * p.x,
                        mt * mt * last.y + 2.0 * mt * t * c.y + t * t * p.y,
                    ));
                }
                curve(pts, &mut out);
                last = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let (c1, c2, p) = (map(c1), map(c2), map(p));
                let n = cubic_segments(last, c1, c2, p);
                let mut pts = vec![last];
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let mt = 1.0 - t;
                    pts.push(Point::from_xy(
                        mt * mt * mt * last.x
                            + 3.0 * mt * mt * t * c1.x
                            + 3.0 * mt * t * t * c2.x
                            + t * t * t * p.x,
                        mt * mt * mt * last.y
                            + 3.0 * mt * mt * t * c1.y
                            + 3.0 * mt * t * t * c2.y
                            + t * t * t * p.y,
                    ));
                }
                curve(pts, &mut out);
                last = p;
            }
            PathSegment::Close => {
                if last != start {
                    line(last, start, &mut out);
                }
                last = start;
            }
        }
    }
    if last != start {
        line(last, start, &mut out);
    }
    out
}

/// Exact-area coverage (non-zero winding, overlaps saturating) of closed line loops in a
/// `w` x `h` mask.
pub fn coverage(lines: &[(Point, Point)], w: u32, h: u32) -> Mask {
    let (wi, hi) = (w as usize, h as usize);
    let stride = wi + 2;
    let mut acc = vec![0f32; stride * hi + 2];
    for &(a, b) in lines {
        let (mut p0, mut p1) = (a, b);
        if p0.y == p1.y {
            continue;
        }
        let dir = if p0.y < p1.y { 1.0 } else { -1.0 };
        if p0.y > p1.y {
            std::mem::swap(&mut p0, &mut p1);
        }
        // Points left or right of the mask act as if on its edge.
        let clampx = |x: f32| x.clamp(0.0, w as f32);
        let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
        let y_start = p0.y.max(0.0);
        let y_end = p1.y.min(h as f32);
        if y_start >= y_end {
            continue;
        }
        let mut x = p0.x + dxdy * (y_start - p0.y);
        let mut y = y_start;
        while y < y_end {
            let row = y.floor();
            let next = (row + 1.0).min(y_end);
            let dy = next - y;
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (xa, xb) = (clampx(x), clampx(xnext));
            let (x0, x1) = if xa < xb { (xa, xb) } else { (xb, xa) };
            let line = row as usize * stride;
            let x0f = x0.floor();
            let x0i = x0f as usize;
            let x1c = x1.ceil();
            let x1i = x1c as usize;
            if x1i <= x0i + 1 {
                let xmf = 0.5 * (x0 + x1) - x0f;
                acc[line + x0i] += d - d * xmf;
                acc[line + x0i + 1] += d * xmf;
            } else {
                let s = 1.0 / (x1 - x0);
                let x0frac = x0 - x0f;
                let a0 = 0.5 * s * (1.0 - x0frac) * (1.0 - x0frac);
                let x1frac = x1 - x1c + 1.0;
                let am = 0.5 * s * x1frac * x1frac;
                acc[line + x0i] += d * a0;
                if x1i == x0i + 2 {
                    acc[line + x0i + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0frac);
                    acc[line + x0i + 1] += d * (a1 - a0);
                    for xi in x0i + 2..x1i - 1 {
                        acc[line + xi] += d * s;
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    acc[line + x1i - 1] += d * (1.0 - a2 - am);
                }
                acc[line + x1i] += d * am;
            }
            x = xnext;
            y = next;
        }
    }
    let mut mask = Mask::new(w, h).unwrap();
    let data = mask.data_mut();
    for row in 0..hi {
        let mut sum = 0f32;
        for col in 0..wi {
            sum += acc[row * stride + col];
            let c = sum.abs().min(1.0);
            data[row * wi + col] = (c * 255.0 + 0.5) as u8;
        }
    }
    mask
}

/// Skia's supersampled anti-aliasing (`SkScan::SAAFillPath`, SHIFT 2): four sub-scanlines
/// per pixel row sampled at their centres, span ends rounded to quarter pixels, coverage
/// summed as `SuperBlitter::blitH` does (64 per full sub-row, 63 on the last).
pub fn supersampled(lines: &[(Point, Point)], w: u32, h: u32) -> Mask {
    struct Edge {
        x0: f32,
        y0: f32,
        slope: f32,
        top: i32,
        bot: i32,
        winding: i32,
    }
    let mut edges = Vec::with_capacity(lines.len());
    for &(a, b) in lines {
        let (mut p0, mut p1) = (a, b);
        let mut winding = 1;
        if p0.y > p1.y {
            std::mem::swap(&mut p0, &mut p1);
            winding = -1;
        }
        let (x0, y0, x1, y1) = (p0.x * 4.0, p0.y * 4.0, p1.x * 4.0, p1.y * 4.0);
        let top = (y0 + 0.5).floor() as i32;
        let bot = (y1 + 0.5).floor() as i32;
        if top == bot {
            continue;
        }
        edges.push(Edge {
            x0,
            y0,
            slope: (x1 - x0) / (y1 - y0),
            top,
            bot,
            winding,
        });
    }
    let (wi, hi) = (w as i32, h as i32);
    let mut acc = vec![0u32; (wi * hi).max(0) as usize];
    let max_x = wi * 4;
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for sy in 0..hi * 4 {
        xs.clear();
        for e in &edges {
            if sy >= e.top && sy < e.bot {
                xs.push((e.x0 + e.slope * (sy as f32 + 0.5 - e.y0), e.winding));
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let row = (sy >> 2) as usize * wi as usize;
        let max_value = 64 - (((sy & 3) + 1) >> 2) as u32;
        let mut wind = 0;
        let mut left = 0f32;
        for &(x, wdir) in xs.iter() {
            let before = wind;
            wind += wdir;
            if before == 0 && wind != 0 {
                left = x;
            } else if before != 0 && wind == 0 {
                let l = ((left + 0.5).floor() as i32).clamp(0, max_x);
                let r = ((x + 0.5).floor() as i32).clamp(0, max_x);
                if r <= l {
                    continue;
                }
                let (start, stop) = (l, r);
                let mut fb = start & 3;
                let mut fe = stop & 3;
                let mut n = (stop >> 2) - (start >> 2) - 1;
                if n < 0 {
                    fb = fe - fb;
                    n = 0;
                    fe = 0;
                } else if fb == 0 {
                    n += 1;
                } else {
                    fb = 4 - fb;
                }
                let mut px = (start >> 2) as usize;
                if fb != 0 {
                    acc[row + px] += (fb as u32) << 4;
                    px += 1;
                }
                for _ in 0..n {
                    if px < wi as usize {
                        acc[row + px] += max_value;
                    }
                    px += 1;
                }
                if fe != 0 && px < wi as usize {
                    acc[row + px] += (fe as u32) << 4;
                }
            }
        }
    }
    let mut mask = Mask::new(w, h).unwrap();
    for (d, a) in mask.data_mut().iter_mut().zip(&acc) {
        *d = (*a - (*a >> 8)).min(255) as u8;
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{PathBuilder, Rect};

    #[test]
    fn half_covered_pixels_of_a_rect() {
        let path = PathBuilder::from_rect(Rect::from_ltrb(1.5, 1.0, 4.0, 3.0).unwrap());
        let m = coverage(&flatten(&path, Transform::identity()), 6, 4);
        let d = m.data();
        assert_eq!(d[6 + 1], 128);
        assert_eq!(d[6 + 2], 255);
        assert_eq!(d[6 + 4], 0);
        assert_eq!(d[0], 0);
    }
}

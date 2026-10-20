//! Points and rectangles with SkiaSharp's semantics (`SKPoint`, `SKRect`, `SKRectI`),
//! which is what upstream's core (`ShareX.ImageEditor.Core`) computes with.

/// `SKPoint`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    pub fn offset(self, dx: f32, dy: f32) -> Point {
        Point::new(self.x + dx, self.y + dy)
    }

    pub fn distance(self, other: Point) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Rotates around `center` by `angle_deg` (clockwise, y down), as the core's
    /// `RotatePoint` helpers do.
    pub fn rotate_around(self, center: Point, angle_deg: f32) -> Point {
        if angle_deg == 0.0 {
            return self;
        }
        let rad = angle_deg * std::f32::consts::PI / 180.0;
        let (sin, cos) = rad.sin_cos();
        let dx = self.x - center.x;
        let dy = self.y - center.y;
        Point::new(
            center.x + dx * cos - dy * sin,
            center.y + dx * sin + dy * cos,
        )
    }
}

/// `SKRect` (left, top, right, bottom).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub const EMPTY: Rect = Rect::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Avalonia's `Rect(x, y, width, height)`.
    pub fn from_xywh(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(x, y, x + width, y + height)
    }

    /// The normalized rectangle spanned by two points (`Math.Min` / `Math.Max`).
    pub fn from_points(a: Point, b: Point) -> Rect {
        Rect::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn mid_x(&self) -> f32 {
        self.left + self.width() / 2.0
    }

    pub fn mid_y(&self) -> f32 {
        self.top + self.height() / 2.0
    }

    pub fn center(&self) -> Point {
        Point::new(self.mid_x(), self.mid_y())
    }

    pub fn top_left(&self) -> Point {
        Point::new(self.left, self.top)
    }

    pub fn bottom_right(&self) -> Point {
        Point::new(self.right, self.bottom)
    }

    pub fn is_empty(&self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    /// `SKRect.Contains(x, y)`: the right and bottom edges are outside.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }

    /// Avalonia's `Rect.Contains`: every edge is inside.
    pub fn contains_inclusive(&self, p: Point) -> bool {
        p.x >= self.left && p.x <= self.right && p.y >= self.top && p.y <= self.bottom
    }

    /// `SKRect.Inflate(rect, x, y)`.
    pub fn inflate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(
            self.left - dx,
            self.top - dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    /// `SKRect.Union`.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    /// `SKRectI` made by truncating each edge, as `(int)rect.Left` does.
    pub fn truncate(&self) -> RectI {
        RectI::new(
            self.left as i32,
            self.top as i32,
            self.right as i32,
            self.bottom as i32,
        )
    }
}

/// `SKRectI`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RectI {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RectI {
    pub const EMPTY: RectI = RectI::new(0, 0, 0, 0);

    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> RectI {
        RectI {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// `SKRectI.Intersect`: the overlap, or `Empty` when there is none.
    pub fn intersect(&self, other: &RectI) -> RectI {
        if self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
        {
            RectI::new(
                self.left.max(other.left),
                self.top.max(other.top),
                self.right.min(other.right),
                self.bottom.min(other.bottom),
            )
        } else {
            RectI::EMPTY
        }
    }
}

/// Distance from `p` to the segment `a`-`b` (`CurvedSegmentHelper.DistanceToSegment`).
pub fn distance_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.001 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

/// `PointInTriangle` of the number and speech balloon annotations.
pub fn point_in_triangle(p: Point, a: Point, b: Point, c: Point) -> bool {
    fn sign(p1: Point, p2: Point, p3: Point) -> f32 {
        (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
    }
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_excludes_right_and_bottom_edges() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(!r.contains(Point::new(10.0, 5.0)));
        assert!(r.contains_inclusive(Point::new(10.0, 10.0)));
    }

    #[test]
    fn rotation_is_clockwise_with_y_down() {
        let p = Point::new(10.0, 0.0).rotate_around(Point::default(), 90.0);
        assert!((p.x).abs() < 1e-4 && (p.y - 10.0).abs() < 1e-4);
    }

    #[test]
    fn intersect_without_overlap_is_empty() {
        let a = RectI::new(0, 0, 5, 5);
        assert_eq!(a.intersect(&RectI::new(6, 6, 9, 9)), RectI::EMPTY);
        assert_eq!(a.intersect(&RectI::new(2, 3, 9, 9)), RectI::new(2, 3, 5, 5));
    }
}

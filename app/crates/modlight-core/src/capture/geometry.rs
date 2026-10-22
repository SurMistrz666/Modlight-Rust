//! Rectangles of the capture code: `Avalonia.Rect` (doubles, the region overlay's
//! image-pixel space) and `System.Drawing.Rectangle` (integers, the desktop), with the
//! helpers `RegionSelectionOverlay` and `CaptureHelpers` give them.

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A point in the overlay's image-pixel space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    pub fn distance(self, other: Point) -> f64 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// `Avalonia.Rect`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn left(&self) -> f64 {
        self.x
    }

    pub fn top(&self) -> f64 {
        self.y
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// `Rect.Contains`: the right and bottom edges belong to the rectangle.
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.right()
            && point.y >= self.y
            && point.y <= self.bottom()
    }

    /// `RegionSelectionOverlay.IsValid`.
    pub fn is_valid(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }

    /// `RegionSelectionOverlay.NormalizeAndClamp`.
    pub fn normalize_and_clamp(first: Point, second: Point, bounds: (f64, f64)) -> Rect {
        let left = first.x.min(second.x).clamp(0.0, bounds.0);
        let top = first.y.min(second.y).clamp(0.0, bounds.1);
        let right = first.x.max(second.x).clamp(0.0, bounds.0);
        let bottom = first.y.max(second.y).clamp(0.0, bounds.1);
        Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    /// `RegionSelectionOverlay.Intersect`: empty (default) when they do not overlap.
    pub fn intersect(&self, bounds: &Rect) -> Rect {
        let left = self.left().max(bounds.left());
        let top = self.top().max(bounds.top());
        let right = self.right().min(bounds.right());
        let bottom = self.bottom().min(bounds.bottom());
        if right > left && bottom > top {
            Rect::new(left, top, right - left, bottom - top)
        } else {
            Rect::default()
        }
    }

    /// The eight handle centres, clockwise from the top left (`GetHandlePoints`).
    pub fn handle_points(&self) -> [Point; 8] {
        let c = self.center();
        [
            Point::new(self.left(), self.top()),
            Point::new(c.x, self.top()),
            Point::new(self.right(), self.top()),
            Point::new(self.right(), c.y),
            Point::new(self.right(), self.bottom()),
            Point::new(c.x, self.bottom()),
            Point::new(self.left(), self.bottom()),
            Point::new(self.left(), c.y),
        ]
    }
}

/// `RegionResizeHandle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeHandle {
    None,
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl ResizeHandle {
    const ORDER: [ResizeHandle; 8] = [
        ResizeHandle::TopLeft,
        ResizeHandle::Top,
        ResizeHandle::TopRight,
        ResizeHandle::Right,
        ResizeHandle::BottomRight,
        ResizeHandle::Bottom,
        ResizeHandle::BottomLeft,
        ResizeHandle::Left,
    ];

    /// `RegionSelectionOverlay.HitTestHandle` with its default tolerance of 10.
    pub fn hit_test(selection: &Rect, point: Point) -> ResizeHandle {
        if !selection.is_valid() {
            return ResizeHandle::None;
        }
        const TOLERANCE: f64 = 10.0;
        for (handle, p) in Self::ORDER.iter().zip(selection.handle_points()) {
            if (point.x - p.x).abs() <= TOLERANCE && (point.y - p.y).abs() <= TOLERANCE {
                return *handle;
            }
        }
        ResizeHandle::None
    }
}

/// `System.Drawing.Rectangle`: a rectangle of the desktop.
///
/// On macOS the desktop is the global display space in points (origin at the top left
/// of the main display, y down), the space `CGWindowList` and ScreenCaptureKit use. On
/// Windows it is upstream's virtual screen (origin at the top left of the primary
/// display, y down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl IRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> IRect {
        IRect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// `Rectangle.IsEmpty`: every member zero.
    pub fn is_empty(&self) -> bool {
        self.x == 0 && self.y == 0 && self.width == 0 && self.height == 0
    }

    pub fn has_area(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// `Rectangle.Contains(Point)`: the right and bottom edges are outside.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn contains_rect(&self, other: &IRect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// `Rectangle.Intersect`: `Empty` when they do not overlap.
    pub fn intersect(&self, other: &IRect) -> IRect {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right >= left && bottom >= top {
            IRect::new(left, top, right - left, bottom - top)
        } else {
            IRect::default()
        }
    }

    /// `Rectangle.Union`.
    pub fn union(&self, other: &IRect) -> IRect {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        IRect::new(left, top, right - left, bottom - top)
    }

    /// `CaptureHelpers.CreateRectangle(x, y, x2, y2)`: from two corners in any order.
    pub fn from_corners(x: i32, y: i32, x2: i32, y2: i32) -> IRect {
        let (left, right) = if x <= x2 { (x, x2) } else { (x2, x) };
        let (top, bottom) = if y <= y2 { (y, y2) } else { (y2, y) };
        IRect::new(left, top, right - left + 1, bottom - top + 1)
    }

    pub fn to_rect(self) -> Rect {
        Rect::new(
            self.x as f64,
            self.y as f64,
            self.width as f64,
            self.height as f64,
        )
    }

    /// ShareX's text form, "X, Y, Width, Height".
    pub fn to_text(self) -> String {
        format!("{}, {}, {}, {}", self.x, self.y, self.width, self.height)
    }

    /// Read ShareX's text form or Newtonsoft's object form.
    pub fn parse_value(value: &Value) -> Option<IRect> {
        match value {
            Value::String(text) => {
                let parts: Vec<i32> = text
                    .split(',')
                    .map(|p| p.trim().parse::<i32>())
                    .collect::<Result<_, _>>()
                    .ok()?;
                match parts.as_slice() {
                    [x, y, w, h] => Some(IRect::new(*x, *y, *w, *h)),
                    _ => None,
                }
            }
            Value::Object(map) => {
                let read = |k: &str| map.get(k).and_then(Value::as_i64).unwrap_or(0) as i32;
                Some(IRect::new(
                    read("X"),
                    read("Y"),
                    read("Width"),
                    read("Height"),
                ))
            }
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for IRect {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        IRect::parse_value(&value).ok_or_else(|| de::Error::custom("not a rectangle"))
    }
}

impl Serialize for IRect {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_text())
    }
}

/// `System.Drawing.Point`, written by ShareX as `"10, 20"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IPoint {
    pub x: i32,
    pub y: i32,
}

impl<'de> Deserialize<'de> for IPoint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let pair = match &value {
            Value::String(text) => text
                .split_once(',')
                .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?))),
            Value::Object(map) => Some((
                map.get("X").and_then(Value::as_i64).unwrap_or(0) as i32,
                map.get("Y").and_then(Value::as_i64).unwrap_or(0) as i32,
            )),
            _ => None,
        };
        pair.map(|(x, y)| IPoint { x, y })
            .ok_or_else(|| de::Error::custom("not a point"))
    }
}

impl Serialize for IPoint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{}, {}", self.x, self.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_and_clamp_orders_and_bounds_the_corners() {
        let r = Rect::normalize_and_clamp(
            Point::new(300.0, 40.0),
            Point::new(-20.0, 900.0),
            (200.0, 500.0),
        );
        assert_eq!(r, Rect::new(0.0, 40.0, 200.0, 460.0));
        assert!(
            !Rect::normalize_and_clamp(Point::new(5.0, 5.0), Point::new(5.0, 9.0), (10.0, 10.0))
                .is_valid()
        );
    }

    #[test]
    fn intersection_of_disjoint_rectangles_is_empty() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            a.intersect(&Rect::new(10.0, 0.0, 5.0, 5.0)),
            Rect::default()
        );
        assert_eq!(
            a.intersect(&Rect::new(5.0, 5.0, 50.0, 50.0)),
            Rect::new(5.0, 5.0, 5.0, 5.0)
        );
    }

    #[test]
    fn handles_are_hit_within_ten_pixels() {
        let r = Rect::new(100.0, 100.0, 200.0, 100.0);
        assert_eq!(
            ResizeHandle::hit_test(&r, Point::new(91.0, 109.0)),
            ResizeHandle::TopLeft
        );
        assert_eq!(
            ResizeHandle::hit_test(&r, Point::new(200.0, 100.0)),
            ResizeHandle::Top
        );
        assert_eq!(
            ResizeHandle::hit_test(&r, Point::new(300.0, 150.0)),
            ResizeHandle::Right
        );
        assert_eq!(
            ResizeHandle::hit_test(&r, Point::new(200.0, 150.0)),
            ResizeHandle::None
        );
        assert_eq!(
            ResizeHandle::hit_test(&Rect::default(), Point::new(0.0, 0.0)),
            ResizeHandle::None
        );
    }

    #[test]
    fn rectangles_read_and_write_as_sharex_does() {
        let r: IRect = serde_json::from_value(json!("10, 20, 300, 400")).unwrap();
        assert_eq!(r, IRect::new(10, 20, 300, 400));
        assert_eq!(serde_json::to_value(r).unwrap(), json!("10, 20, 300, 400"));
        let o: IRect =
            serde_json::from_value(json!({"X": 1, "Y": 2, "Width": 3, "Height": 4})).unwrap();
        assert_eq!(o, IRect::new(1, 2, 3, 4));
        assert!(IRect::default().is_empty());
        assert_eq!(IRect::from_corners(10, 10, 5, 2), IRect::new(5, 2, 6, 9));
        let p: IPoint = serde_json::from_value(json!("-5, 7")).unwrap();
        assert_eq!(p, IPoint { x: -5, y: 7 });
    }

    #[test]
    fn integer_rectangles_follow_system_drawing() {
        let a = IRect::new(0, 0, 100, 100);
        assert!(a.contains(0, 0));
        assert!(!a.contains(100, 50));
        assert_eq!(
            a.intersect(&IRect::new(50, 50, 100, 100)),
            IRect::new(50, 50, 50, 50)
        );
        assert_eq!(a.intersect(&IRect::new(200, 0, 10, 10)), IRect::default());
        assert_eq!(
            a.union(&IRect::new(-10, 20, 5, 5)),
            IRect::new(-10, 0, 110, 100)
        );
        assert!(a.contains_rect(&IRect::new(10, 10, 90, 90)));
    }
}

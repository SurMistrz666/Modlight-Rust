//! A frame drawn around an area as four thin windows, one per side, that cover nothing of
//! the area itself: the recording's dashed frame and the scrolling capture's red one.
//!
//! One window over the whole area with a see-through middle would be simpler, but a window
//! is only see-through when the renderer makes it so, and here it does not: eframe's wgpu
//! painter takes transparency from the main window, which is opaque, and wgpu draws a
//! Windows window opaque in any case (a DXGI swap chain made from its handle). The middle
//! of such a window came out black, over the very area being recorded or scrolled. Strips
//! need no transparency at all.

use egui::{Rect, pos2};

/// Which side of the area a strip runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// One strip of a frame: where it is on the desktop (points), and how far along the
/// frame's outline it starts, clockwise from the top left corner, so that a dash pattern
/// runs on round the corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    pub side: Side,
    pub rect: Rect,
    pub start: f32,
}

/// The four strips of a frame `width` points wide around `area`, all outside it; the top
/// and bottom strips take the corners.
pub fn strips(area: Rect, width: f32) -> [Strip; 4] {
    let outer = area.expand(width);
    let across = outer.width();
    let down = area.height();
    [
        Strip {
            side: Side::Top,
            rect: Rect::from_min_max(outer.min, pos2(outer.max.x, area.min.y)),
            start: 0.0,
        },
        Strip {
            side: Side::Right,
            rect: Rect::from_min_max(pos2(area.max.x, area.min.y), pos2(outer.max.x, area.max.y)),
            start: across,
        },
        Strip {
            side: Side::Bottom,
            rect: Rect::from_min_max(pos2(outer.min.x, area.max.y), outer.max),
            start: across + down,
        },
        Strip {
            side: Side::Left,
            rect: Rect::from_min_max(pos2(outer.min.x, area.min.y), pos2(area.min.x, area.max.y)),
            start: 2.0 * across + down,
        },
    ]
}

/// A strip's dashes in its own window (origin at its top left): pieces `dash` points long
/// along the outline, taking turns, the first piece of the outline `false`. The outline
/// runs left to right along the top, down the right, right to left along the bottom and
/// up the left, so the pieces meet at the corners as one dashed line would.
pub fn dashes(strip: &Strip, dash: f32) -> Vec<(Rect, bool)> {
    let size = strip.rect.size();
    let length = match strip.side {
        Side::Top | Side::Bottom => size.x,
        Side::Right | Side::Left => size.y,
    };
    let mut pieces = Vec::new();
    let usable = |value: f32| value.is_finite() && value > 0.0;
    if !usable(dash) || !usable(length) || !strip.start.is_finite() {
        return pieces;
    }
    let mut along = 0.0_f32;
    while along < length {
        let index = ((strip.start + along) / dash).floor();
        let mut end = ((index + 1.0) * dash - strip.start).min(length);
        if end <= along {
            // Rounding put `along` on the boundary itself: the next piece starts here.
            end = (along + dash).min(length);
        }
        let second = (index as i64).rem_euclid(2) == 1;
        let rect = match strip.side {
            Side::Top => Rect::from_min_max(pos2(along, 0.0), pos2(end, size.y)),
            Side::Bottom => {
                Rect::from_min_max(pos2(size.x - end, 0.0), pos2(size.x - along, size.y))
            }
            Side::Right => Rect::from_min_max(pos2(0.0, along), pos2(size.x, end)),
            Side::Left => Rect::from_min_max(pos2(0.0, size.y - end), pos2(size.x, size.y - along)),
        };
        pieces.push((rect, second));
        along = end;
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn area() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(40.0, 23.0))
    }

    #[test]
    fn the_strips_surround_the_area_and_cover_none_of_it() {
        let area = area();
        let s = strips(area, 1.0);
        let outer = area.expand(1.0);
        for strip in &s {
            assert!(
                !strip.rect.intersects(area.shrink(0.001)),
                "{strip:?} covers the area"
            );
            assert!(outer.contains_rect(strip.rect));
        }
        // Together they are the outline exactly: their areas add up to the ring's.
        let total: f32 = s.iter().map(|strip| strip.rect.area()).sum();
        assert!((total - (outer.area() - area.area())).abs() < 0.001);
        assert_eq!(
            s[0].rect,
            Rect::from_min_max(pos2(99.0, 49.0), pos2(141.0, 50.0))
        );
        assert_eq!(
            s[1].rect,
            Rect::from_min_max(pos2(140.0, 50.0), pos2(141.0, 73.0))
        );
        assert_eq!(
            s[2].rect,
            Rect::from_min_max(pos2(99.0, 73.0), pos2(141.0, 74.0))
        );
        assert_eq!(
            s[3].rect,
            Rect::from_min_max(pos2(99.0, 50.0), pos2(100.0, 73.0))
        );
        // Each starts where the one before it ends along the outline.
        assert_eq!(s[1].start, 42.0);
        assert_eq!(s[2].start, 42.0 + 23.0);
        assert_eq!(s[3].start, 2.0 * 42.0 + 23.0);
    }

    #[test]
    fn the_dashes_fill_each_strip_and_take_turns_round_the_corners() {
        for strip in strips(area(), 1.0) {
            let pieces = dashes(&strip, 5.0);
            let covered: f32 = pieces.iter().map(|(r, _)| r.area()).sum();
            assert!((covered - strip.rect.area()).abs() < 0.001, "{strip:?}");
            for (r, _) in &pieces {
                assert!(Rect::from_min_size(pos2(0.0, 0.0), strip.rect.size()).contains_rect(*r));
            }
            // Next to each other along the strip, the pieces alternate unless one was cut
            // by the strip's own end.
            for pair in pieces.windows(2) {
                assert_ne!(pair[0].1, pair[1].1, "{strip:?}");
            }
        }
        // The top starts with a dark piece; the outline's 10th to 15th point is dark too.
        let top = strips(area(), 1.0)[0];
        let pieces = dashes(&top, 5.0);
        assert_eq!(
            pieces[0],
            (Rect::from_min_max(pos2(0.0, 0.0), pos2(5.0, 1.0)), false)
        );
        assert!(pieces[1].1);
        assert_eq!(
            pieces[2],
            (Rect::from_min_max(pos2(10.0, 0.0), pos2(15.0, 1.0)), false)
        );
        // The right side carries on from 42 points along: 40..45 is dark, so its first
        // 3 points are dark, then accent from 45.
        let right = strips(area(), 1.0)[1];
        let pieces = dashes(&right, 5.0);
        assert_eq!(
            pieces[0],
            (Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 3.0)), false)
        );
        assert_eq!(
            pieces[1],
            (Rect::from_min_max(pos2(0.0, 3.0), pos2(1.0, 8.0)), true)
        );
    }

    #[test]
    fn the_bottom_and_left_run_backwards() {
        let s = strips(area(), 1.0);
        // The bottom starts at its right end, 42 + 23 = 65 points along: the start of an
        // accent piece.
        let bottom = dashes(&s[2], 5.0);
        assert_eq!(
            bottom[0],
            (Rect::from_min_max(pos2(37.0, 0.0), pos2(42.0, 1.0)), true)
        );
        // The left starts at its bottom end.
        let left = dashes(&s[3], 5.0);
        assert_eq!(left[0].0.max.y, 23.0);
    }

    #[test]
    fn odd_sizes_and_bad_input_draw_nothing_wrong() {
        let thin = strips(Rect::from_min_size(pos2(0.0, 0.0), vec2(0.5, 0.0)), 1.0);
        for strip in &thin {
            for (r, _) in dashes(strip, 5.0) {
                assert!(r.is_finite() && r.width() >= 0.0 && r.height() >= 0.0);
            }
        }
        let top = strips(area(), 1.0)[0];
        assert!(dashes(&top, 0.0).is_empty());
        assert!(dashes(&top, f32::NAN).is_empty());
        // Fractional places (a Retina half point) still end on the strip's end.
        let fractional = strips(Rect::from_min_size(pos2(10.5, 3.5), vec2(17.5, 9.5)), 1.0);
        for strip in &fractional {
            let pieces = dashes(strip, 5.0);
            assert!(!pieces.is_empty());
            let covered: f32 = pieces.iter().map(|(r, _)| r.area()).sum();
            assert!((covered - strip.rect.area()).abs() < 0.01);
        }
    }
}

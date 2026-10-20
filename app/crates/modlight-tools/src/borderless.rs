//! "Active window borderless" on macOS (`BorderlessWindowManager`): another app's window
//! frame cannot be taken off without private APIs, so the closest behaviour is kept: the
//! active window is moved and sized to fill its screen (or the screen's area free of the
//! menu bar and the Dock, upstream's "exclude taskbar area"), and the same hotkey on the
//! same window puts it back where it was. The window's own frame stays.
//!
//! Frames here are in points with the origin at the top left of the main display (the
//! Accessibility API's coordinates).

/// A rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Frame {
        Frame {
            x,
            y,
            width,
            height,
        }
    }

    fn intersection_area(&self, other: &Frame) -> f64 {
        let w = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let h = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }

    fn center_distance(&self, other: &Frame) -> f64 {
        let (ax, ay) = (self.x + self.width / 2.0, self.y + self.height / 2.0);
        let (bx, by) = (other.x + other.width / 2.0, other.y + other.height / 2.0);
        ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
    }
}

/// One display: its whole area and the part free of the menu bar and the Dock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub frame: Frame,
    pub visible: Frame,
}

/// `CaptureHelpers.GetScreenBounds(handle, useWorkingArea)`: the screen the window is
/// mostly on (`Screen.FromHandle`; the nearest when it is on none), whole or its working
/// area. `None` without screens.
pub fn target_frame(window: &Frame, screens: &[Screen], working_area: bool) -> Option<Frame> {
    let best = screens.iter().max_by(|a, b| {
        let (ia, ib) = (
            window.intersection_area(&a.frame),
            window.intersection_area(&b.frame),
        );
        ia.partial_cmp(&ib)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                window
                    .center_distance(&b.frame)
                    .partial_cmp(&window.center_distance(&a.frame))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    })?;
    Some(if working_area {
        best.visible
    } else {
        best.frame
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screens() -> Vec<Screen> {
        vec![
            Screen {
                frame: Frame::new(0.0, 0.0, 1440.0, 900.0),
                visible: Frame::new(0.0, 25.0, 1440.0, 800.0),
            },
            Screen {
                frame: Frame::new(1440.0, -100.0, 1280.0, 720.0),
                visible: Frame::new(1440.0, -75.0, 1280.0, 695.0),
            },
        ]
    }

    #[test]
    fn the_screen_the_window_is_mostly_on() {
        let s = screens();
        let on_main = Frame::new(100.0, 100.0, 800.0, 600.0);
        assert_eq!(target_frame(&on_main, &s, false), Some(s[0].frame));
        assert_eq!(target_frame(&on_main, &s, true), Some(s[0].visible));
        let mostly_right = Frame::new(1300.0, 0.0, 800.0, 400.0);
        assert_eq!(target_frame(&mostly_right, &s, false), Some(s[1].frame));
        let off_screen = Frame::new(5000.0, 0.0, 100.0, 100.0);
        assert_eq!(target_frame(&off_screen, &s, false), Some(s[1].frame));
        assert_eq!(target_frame(&on_main, &[], false), None);
    }
}

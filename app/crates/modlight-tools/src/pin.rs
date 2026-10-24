//! A pinned picture's geometry and its scale and opacity steps (`PinToScreenWindow`,
//! `Helpers.GetPosition`): what size its window takes, where it opens, how the wheel and
//! the keys change it.

use crate::settings::{ContentAlignment, PinToScreenOptions};

/// `ShadowMargin`.
pub const SHADOW_MARGIN: f64 = 10.0;

/// The scale and opacity limits of a pinned picture.
pub const SCALE_MIN: i64 = 20;
pub const SCALE_MAX: i64 = 500;
pub const OPACITY_MIN: i64 = 10;
pub const OPACITY_MAX: i64 = 100;

/// The toolbar shows only on a window at least this big.
pub const TOOLBAR_MIN_WIDTH: f64 = 155.0;
pub const TOOLBAR_MIN_HEIGHT: f64 = 55.0;

/// One pinned picture's live state.
#[derive(Clone, Debug, PartialEq)]
pub struct PinState {
    pub scale: i64,
    pub opacity: i64,
    pub minimized: bool,
}

impl PinState {
    pub fn new(options: &PinToScreenOptions) -> PinState {
        PinState {
            scale: options.initial_scale.clamp(SCALE_MIN, SCALE_MAX),
            opacity: options.initial_opacity.clamp(OPACITY_MIN, OPACITY_MAX),
            minimized: false,
        }
    }

    /// `ChangeScale`: true when the size changed.
    pub fn change_scale(&mut self, delta: i64) -> bool {
        if self.minimized {
            return false;
        }
        let value = (self.scale + delta).clamp(SCALE_MIN, SCALE_MAX);
        let changed = value != self.scale;
        self.scale = value;
        changed
    }

    /// `ChangeOpacity`.
    pub fn change_opacity(&mut self, delta: i64) {
        if self.minimized {
            return;
        }
        self.opacity = (self.opacity + delta).clamp(OPACITY_MIN, OPACITY_MAX);
    }

    /// `ResetImage`.
    pub fn reset(&mut self) {
        self.scale = 100;
        self.opacity = 100;
    }

    /// The window's opacity (`Opacity`): full while minimised.
    pub fn window_opacity(&self) -> f64 {
        if self.minimized {
            1.0
        } else {
            self.opacity as f64 / 100.0
        }
    }

    /// The text of the toolbar's reset button.
    pub fn scale_text(&self) -> String {
        format!("{}%", self.scale)
    }
}

/// The border's width when it is on (`Math.Clamp(BorderSize, 1, 30)`).
pub fn border(options: &PinToScreenOptions) -> f64 {
    if options.border {
        options.border_size.clamp(1, 30) as f64
    } else {
        0.0
    }
}

/// `GetOuterInset`: border plus shadow margin.
pub fn outer_inset(options: &PinToScreenOptions) -> f64 {
    border(options) + if options.shadow { SHADOW_MARGIN } else { 0.0 }
}

/// `GetDisplayedImageSize`: `image` is the picture's size in the window's units.
pub fn displayed_image_size(
    image: (f64, f64),
    state: &PinState,
    options: &PinToScreenOptions,
) -> (f64, f64) {
    if state.minimized {
        return (
            options.minimize_size.width.max(1) as f64,
            options.minimize_size.height.max(1) as f64,
        );
    }
    // `Math.Round`: halves to even.
    let scale = state.scale as f64 / 100.0;
    (
        (image.0 * scale).round_ties_even().max(1.0),
        (image.1 * scale).round_ties_even().max(1.0),
    )
}

/// `ResizeWindow`'s size: the picture, the border twice, the shadow margin twice.
pub fn window_size(
    image: (f64, f64),
    state: &PinState,
    options: &PinToScreenOptions,
) -> (f64, f64) {
    let (w, h) = displayed_image_size(image, state, options);
    let extra = border(options) * 2.0
        + if options.shadow {
            SHADOW_MARGIN * 2.0
        } else {
            0.0
        };
    ((w + extra).max(1.0), (h + extra).max(1.0))
}

/// `ResizeWindow(keepCenter)`: the new top left keeping the centre where it was.
pub fn keep_center(position: (f64, f64), old: (f64, f64), new: (f64, f64)) -> (f64, f64) {
    (
        position.0 + ((old.0 - new.0) / 2.0).round_ties_even(),
        position.1 + ((old.1 - new.1) / 2.0).round_ties_even(),
    )
}

/// `Helpers.GetPosition(placement, offset, background, objectSize)`.
pub fn position(
    placement: ContentAlignment,
    offset: f64,
    background: (f64, f64, f64, f64),
    size: (f64, f64),
) -> (f64, f64) {
    let (bx, by, bw, bh) = background;
    let mid_x = (bw / 2.0 - size.0 / 2.0).round_ties_even();
    let mid_y = (bh / 2.0 - size.1 / 2.0).round_ties_even();
    let right = bw - size.0;
    let bottom = bh - size.1;
    use ContentAlignment::*;
    let (x, y) = match placement {
        TopLeft => (offset, offset),
        TopCenter => (mid_x, offset),
        TopRight => (right - offset, offset),
        MiddleLeft => (offset, mid_y),
        MiddleCenter => (mid_x, mid_y),
        MiddleRight => (right - offset, mid_y),
        BottomLeft => (offset, bottom - offset),
        BottomCenter => (mid_x, bottom - offset),
        BottomRight => (right - offset, bottom - offset),
    };
    (bx + x, by + y)
}

/// Whether the toolbar may show (`UpdateToolbarVisibility`).
pub fn toolbar_visible(pointer_inside: bool, window: (f64, f64)) -> bool {
    pointer_inside && window.0 >= TOOLBAR_MIN_WIDTH && window.1 >= TOOLBAR_MIN_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Size;

    #[test]
    fn sizes() {
        let options = PinToScreenOptions::default();
        let mut state = PinState::new(&options);
        // 2 px border and 10 px shadow margin on each side.
        assert_eq!(
            window_size((300.0, 200.0), &state, &options),
            (324.0, 224.0)
        );
        state.change_scale(options.scale_step);
        assert_eq!(state.scale, 110);
        assert_eq!(
            window_size((300.0, 200.0), &state, &options),
            (354.0, 244.0)
        );
        state.minimized = true;
        assert!(!state.change_scale(10));
        assert_eq!(
            window_size((300.0, 200.0), &state, &options),
            (124.0, 124.0)
        );
        let options = PinToScreenOptions {
            shadow: false,
            border: false,
            minimize_size: Size {
                width: 0,
                height: 5,
            },
            ..PinToScreenOptions::default()
        };
        assert_eq!(window_size((3.0, 2.0), &state, &options), (1.0, 5.0));
        assert_eq!(outer_inset(&options), 0.0);
    }

    #[test]
    fn limits() {
        let options = PinToScreenOptions {
            initial_scale: 900,
            initial_opacity: 3,
            ..PinToScreenOptions::default()
        };
        let mut state = PinState::new(&options);
        assert_eq!((state.scale, state.opacity), (500, 10));
        assert!(!state.change_scale(10));
        state.change_opacity(-10);
        assert_eq!(state.opacity, 10);
        state.reset();
        assert_eq!(state.scale_text(), "100%");
        assert_eq!(state.window_opacity(), 1.0);
    }

    #[test]
    fn placement() {
        let area = (0.0, 25.0, 1440.0, 800.0);
        assert_eq!(
            position(ContentAlignment::BottomRight, 10.0, area, (324.0, 224.0)),
            (1106.0, 591.0)
        );
        assert_eq!(
            position(ContentAlignment::MiddleCenter, 10.0, area, (324.0, 224.0)),
            (558.0, 313.0)
        );
        assert_eq!(
            position(ContentAlignment::TopLeft, 10.0, area, (324.0, 224.0)),
            (10.0, 35.0)
        );
        assert_eq!(
            keep_center((100.0, 100.0), (324.0, 224.0), (354.0, 244.0)),
            (85.0, 90.0)
        );
        assert!(!toolbar_visible(true, (154.0, 100.0)));
        assert!(toolbar_visible(true, (155.0, 55.0)));
    }

    /// `Math.Round` rounds half to even: a 5 x 3 picture at 50% is 2 x 2, a window one
    /// point narrower keeps its left edge, and an odd margin centres to the even side.
    #[test]
    fn halves_round_to_even_as_dotnet() {
        let options = PinToScreenOptions {
            initial_scale: 50,
            shadow: false,
            border: false,
            ..PinToScreenOptions::default()
        };
        let state = PinState::new(&options);
        assert_eq!(
            displayed_image_size((5.0, 3.0), &state, &options),
            (2.0, 2.0)
        );
        assert_eq!(
            displayed_image_size((7.0, 1.0), &state, &options),
            (4.0, 1.0)
        );
        assert_eq!(
            keep_center((100.0, 100.0), (101.0, 103.0), (100.0, 100.0)),
            (100.0, 102.0)
        );
        assert_eq!(
            position(
                ContentAlignment::MiddleCenter,
                0.0,
                (0.0, 0.0, 101.0, 103.0),
                (100.0, 100.0)
            ),
            (0.0, 2.0)
        );
    }
}

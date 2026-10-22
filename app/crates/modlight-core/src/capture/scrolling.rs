//! Scrolling capture's stitching (`ScrollingCaptureManager.CombineImages`,
//! `ImageHelpers.CompareImages`): each new picture of the scrolled area is matched
//! against the bottom of the result row by row, and only the rows it adds are appended.

use image::RgbaImage;

/// `ScrollingCaptureStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollingStatus {
    #[default]
    Failed,
    PartiallySuccessful,
    Successful,
}

/// The stitcher's memory between pictures (`bestMatchCount`, `bestMatchIndex`,
/// `bestIgnoreBottomOffset`, `status`).
#[derive(Clone, Debug, Default)]
pub struct Stitcher {
    pub result: Option<RgbaImage>,
    pub status: ScrollingStatus,
    best_match_count: usize,
    best_match_index: usize,
    best_ignore_bottom_offset: usize,
    pub auto_ignore_bottom_edge: bool,
}

/// `ImageHelpers.CompareImages`: same size, same bytes.
pub fn same_image(a: &RgbaImage, b: &RgbaImage) -> bool {
    a.dimensions() == b.dimensions() && a.as_raw() == b.as_raw()
}

fn row(image: &RgbaImage, y: usize, from: usize, len: usize) -> &[u8] {
    let stride = image.width() as usize * 4;
    let start = y * stride + from * 4;
    &image.as_raw()[start..start + len * 4]
}

impl Stitcher {
    pub fn new(auto_ignore_bottom_edge: bool) -> Stitcher {
        Stitcher {
            auto_ignore_bottom_edge,
            ..Stitcher::default()
        }
    }

    /// `CombineImages`: append `current` to the result. False when nothing matched (the
    /// capture stops there).
    pub fn add(&mut self, current: &RgbaImage) -> bool {
        let Some(result) = self.result.as_ref() else {
            self.status = ScrollingStatus::Successful;
            self.result = Some(current.clone());
            return true;
        };
        if result.width() != current.width() || current.height() == 0 {
            self.status = ScrollingStatus::Failed;
            return false;
        }
        let width = current.width() as usize;
        let height = current.height() as usize;
        let result_height = result.height() as usize;
        let match_limit = height / 2;
        let ignore_side = (width / 20).max(50).min(width / 3);
        let compare_width = width.saturating_sub(ignore_side * 2);
        // `rect`: from the result's height minus the picture's.
        let rect_top = result_height as i64 - height as i64;

        let ignore_max = height / 3;
        let mut ignore_bottom = (height / 10).max(50);
        if self.auto_ignore_bottom_edge {
            for i in 0..=ignore_max {
                if i >= height || i >= result_height {
                    break;
                }
                if row(result, result_height - 1 - i, ignore_side, compare_width)
                    != row(current, height - 1 - i, ignore_side, compare_width)
                {
                    ignore_bottom += i;
                    break;
                }
            }
            ignore_bottom = ignore_bottom.max(self.best_ignore_bottom_offset);
        }
        let ignore_bottom = ignore_bottom.min(ignore_max);
        let rect_bottom = rect_top + height as i64 - ignore_bottom as i64 - 1;

        let mut match_count = 0usize;
        let mut match_index = 0usize;
        let mut current_y = height as i64 - 1;
        while current_y >= 0 && match_count < match_limit {
            let mut count = 0usize;
            let mut y = 0i64;
            while current_y - y >= 0 && count < match_limit {
                let result_row = rect_bottom - y;
                if result_row < 0 || result_row as usize >= result_height {
                    break;
                }
                if row(result, result_row as usize, ignore_side, compare_width)
                    == row(
                        current,
                        (current_y - y) as usize,
                        ignore_side,
                        compare_width,
                    )
                {
                    count += 1;
                    y += 1;
                } else {
                    break;
                }
            }
            if count > match_count {
                match_count = count;
                match_index = current_y as usize;
            }
            current_y -= 1;
        }

        let mut best_guess = false;
        let mut ignore_bottom = ignore_bottom;
        if match_count == 0 && self.best_match_count > 0 {
            match_count = self.best_match_count;
            match_index = self.best_match_index;
            ignore_bottom = self.best_ignore_bottom_offset;
            best_guess = true;
        }
        if match_count > 0 {
            let match_height = height as i64 - match_index as i64 - 1;
            if match_height > 0 {
                let match_height = match_height as usize;
                if match_count > self.best_match_count {
                    self.best_match_count = match_count;
                    self.best_match_index = match_index;
                    self.best_ignore_bottom_offset = ignore_bottom;
                }
                let keep = result_height - ignore_bottom.min(result_height);
                let mut combined = RgbaImage::new(result.width(), (keep + match_height) as u32);
                let stride = width * 4;
                let raw = combined.as_mut();
                raw[..keep * stride].copy_from_slice(&result.as_raw()[..keep * stride]);
                let from = (match_index + 1) * stride;
                raw[keep * stride..]
                    .copy_from_slice(&current.as_raw()[from..from + match_height * stride]);
                if best_guess {
                    self.status = ScrollingStatus::PartiallySuccessful;
                } else if self.status != ScrollingStatus::PartiallySuccessful {
                    self.status = ScrollingStatus::Successful;
                }
                self.result = Some(combined);
                return true;
            }
        }
        self.status = ScrollingStatus::Failed;
        false
    }
}

/// What moves the page (`InputHelpers.SendMouseWheel`, `SendKeyPress`): `SendInput` on
/// Windows as upstream, synthetic input on a Mac (it needs the Accessibility
/// permission), a fake in tests.
pub trait Scroller: Send + Sync {
    /// Bring the window forward (`selectedWindow.Activate()`).
    fn activate(&self, window: Option<&super::source::WindowInfo>);
    /// `Home` (`AutoScrollTop`).
    fn scroll_to_top(&self);
    /// One step of `ScrollMethod`.
    fn scroll(&self, method: super::options::ScrollMethod, amount: i64);
}

/// `ScrollingCaptureManager.StartCapture`'s loop: capture, stop when two pictures are
/// the same, scroll, stitch, wait. `stop` ends it early.
pub fn run(
    source: &dyn super::source::CaptureSource,
    scroller: &dyn Scroller,
    rect: super::geometry::IRect,
    window: Option<&super::source::WindowInfo>,
    options: &super::options::ScrollingCaptureOptions,
    stop: &std::sync::atomic::AtomicBool,
) -> (Option<RgbaImage>, ScrollingStatus) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let mut stitcher = Stitcher::new(options.auto_ignore_bottom_edge);
    scroller.activate(window);
    std::thread::sleep(Duration::from_millis(options.start_delay.max(0) as u64));
    if options.auto_scroll_top {
        scroller.scroll_to_top();
        std::thread::sleep(Duration::from_millis(options.scroll_delay.max(0) as u64));
    }
    let mut previous: Option<RgbaImage> = None;
    while !stop.load(Ordering::SeqCst) {
        let Ok(last) = source.capture_rect(rect, false) else {
            break;
        };
        if previous.as_ref().is_some_and(|p| same_image(p, &last)) {
            break;
        }
        scroller.scroll(options.scroll_method, options.scroll_amount);
        let started = Instant::now();
        if !stitcher.add(&last) {
            break;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        previous = Some(last);
        let delay = Duration::from_millis(options.scroll_delay.max(0) as u64);
        if let Some(rest) = delay.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    let status = stitcher.status;
    (stitcher.result, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// A long page with a distinct pattern on every row, and a fixed footer.
    fn page(height: u32) -> RgbaImage {
        RgbaImage::from_fn(300, height, |x, y| {
            Rgba([
                (y % 251) as u8,
                ((y / 251) * 40 % 255) as u8,
                ((x * 7 + y * 13) % 256) as u8,
                255,
            ])
        })
    }

    fn view(page: &RgbaImage, top: u32, height: u32) -> RgbaImage {
        image::imageops::crop_imm(page, 0, top, page.width(), height).to_image()
    }

    #[test]
    fn scrolled_views_stitch_back_into_the_page() {
        let page = page(1200);
        let mut stitcher = Stitcher::new(false);
        let mut top = 0;
        let mut previous: Option<RgbaImage> = None;
        loop {
            let shot = view(&page, top.min(1200 - 400), 400);
            if previous.as_ref().is_some_and(|p| same_image(p, &shot)) {
                break;
            }
            assert!(stitcher.add(&shot));
            previous = Some(shot);
            top += 150;
        }
        assert_eq!(stitcher.status, ScrollingStatus::Successful);
        let result = stitcher.result.unwrap();
        // The first view minus its ignored bottom, then what each scroll added.
        assert!(result.height() >= 1100, "{}", result.height());
        let ignored = 50;
        assert_eq!(
            view(&result, 0, 400 - ignored).as_raw(),
            view(&page, 0, 400 - ignored).as_raw()
        );
    }

    #[test]
    fn a_fixed_footer_is_ignored() {
        let mut page = page(1000);
        let mut stitcher = Stitcher::new(true);
        // Footer: the bottom 30 rows of every view are the same.
        let footer = RgbaImage::from_pixel(300, 30, Rgba([9, 9, 9, 255]));
        let mut shots = Vec::new();
        for top in [0u32, 120, 240] {
            let mut shot = view(&page, top, 400);
            image::imageops::replace(&mut shot, &footer, 0, 370);
            shots.push(shot);
        }
        for shot in &shots {
            assert!(stitcher.add(shot));
        }
        assert_eq!(stitcher.status, ScrollingStatus::Successful);
        let result = stitcher.result.as_ref().unwrap();
        assert!(result.height() > 400);
        page.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
    }

    /// A fake window over a long page: each scroll moves it down 150 rows.
    struct FakePage {
        page: RgbaImage,
        top: std::sync::Mutex<u32>,
        height: u32,
    }

    impl Scroller for FakePage {
        fn activate(&self, _window: Option<&crate::capture::source::WindowInfo>) {}
        fn scroll_to_top(&self) {
            *self.top.lock().unwrap() = 0;
        }
        fn scroll(&self, _method: crate::capture::options::ScrollMethod, amount: i64) {
            let mut top = self.top.lock().unwrap();
            *top = (*top + 75 * amount as u32).min(self.page.height() - self.height);
        }
    }

    impl crate::capture::source::CaptureSource for FakePage {
        fn screens(&self) -> Vec<crate::capture::source::ScreenInfo> {
            Vec::new()
        }
        fn cursor_position(&self) -> (i32, i32) {
            (0, 0)
        }
        fn windows(&self) -> Vec<crate::capture::source::WindowInfo> {
            Vec::new()
        }
        fn active_window(&self) -> Option<crate::capture::source::WindowInfo> {
            None
        }
        fn capture_rect(
            &self,
            _rect: crate::capture::geometry::IRect,
            _cursor: bool,
        ) -> Result<RgbaImage, crate::capture::source::CaptureError> {
            Ok(view(&self.page, *self.top.lock().unwrap(), self.height))
        }
        fn capture_window(
            &self,
            _window: &crate::capture::source::WindowInfo,
            _how: crate::capture::source::WindowCapture,
        ) -> Result<RgbaImage, crate::capture::source::CaptureError> {
            Err(crate::capture::source::CaptureError::Nothing)
        }
    }

    #[test]
    fn the_loop_scrolls_to_the_end_and_stops() {
        let fake = FakePage {
            page: page(1000),
            top: std::sync::Mutex::new(300),
            height: 400,
        };
        let options = crate::capture::options::ScrollingCaptureOptions {
            start_delay: 0,
            scroll_delay: 0,
            auto_scroll_top: true,
            auto_ignore_bottom_edge: false,
            ..Default::default()
        };
        let stop = std::sync::atomic::AtomicBool::new(false);
        let (result, status) = run(
            &fake,
            &fake,
            crate::capture::geometry::IRect::new(0, 0, 300, 400),
            None,
            &options,
            &stop,
        );
        assert_eq!(status, ScrollingStatus::Successful);
        let result = result.unwrap();
        assert!(result.height() > 900, "{}", result.height());
    }

    #[test]
    fn unrelated_pictures_do_not_stitch() {
        let mut stitcher = Stitcher::new(false);
        assert!(stitcher.add(&RgbaImage::from_pixel(300, 200, Rgba([1, 2, 3, 255]))));
        let other = RgbaImage::from_fn(300, 200, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, 77, 255])
        });
        assert!(!stitcher.add(&other));
        assert_eq!(stitcher.status, ScrollingStatus::Failed);
        assert!(
            !stitcher.add(&RgbaImage::new(100, 200)),
            "a different width never stitches"
        );
    }
}

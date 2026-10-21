//! The recorder's frames on Windows: the region copied with GDI at the requested rate on
//! a thread of its own, as tightly packed BGRA of the frame size.
//!
//! GDI rather than DXGI Desktop Duplication: it is upstream's default recorder input
//! (ffmpeg's `gdigrab` copies the screen the same way, `SRCCOPY | CAPTUREBLT`), it reads
//! any area of the virtual desktop in one call, across monitor edges, where duplication
//! works per output and would have to compose regions spanning two monitors, it needs no
//! recovery when the desktop switches (UAC, a mode change, a full-screen game), and it
//! draws the pointer the same way screenshots do. Its cost is a copy of the region from
//! the compositor per frame, which a region of a 1080p screen at 30 or 60 frames per
//! second affords.
//!
//! This app's own windows (the recording frame and its toolbar) are kept out of the
//! video with `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` (Windows 10 2004 and
//! later) while the frames are taken; windows opened during the recording are added as
//! they appear, and every window changed gets its earlier affinity back on stop.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use modlight_core::capture::geometry::IRect;
use modlight_core::recording::session::FrameSource;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

use super::desktop::{PhysicalPixels, monitors, primary_scale, rect_to_pixels, virtual_bounds};
use super::gdi::{Cursor, Dib, ScreenDc};
use super::winlist::{is_visible, is_window, own_windows};

type Latest = Arc<Mutex<Option<Arc<Vec<u8>>>>>;

/// How long `start` waits for the capture thread to report that it runs.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// The recorder's frames: the region's pixels at the requested rate, tightly packed
/// BGRA, without this app's own windows.
pub struct ScreenFrames {
    /// The region in physical pixels, its size even.
    pixels: IRect,
    cursor: bool,
    fps: i64,
    latest: Latest,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// A frame size: even (the encoders' 4:2:0 needs it), at least 2.
fn even(value: i32) -> i32 {
    (value.max(2) / 2 * 2).max(2)
}

impl ScreenFrames {
    /// `region` is in the app's screen points; the frames are physical pixels (the
    /// primary monitor's scale, like every capture here).
    pub fn new(region: IRect, cursor: bool, fps: i64) -> ScreenFrames {
        let pixels = rect_to_pixels(region, primary_scale());
        ScreenFrames {
            pixels: IRect::new(pixels.x, pixels.y, even(pixels.width), even(pixels.height)),
            cursor,
            fps,
            latest: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

impl FrameSource for ScreenFrames {
    fn size(&self) -> (u32, u32) {
        (self.pixels.width as u32, self.pixels.height as u32)
    }

    fn start(&mut self) -> Result<(), String> {
        if self.thread.is_some() {
            return Ok(());
        }
        self.stop.store(false, Ordering::SeqCst);
        let (ready, started) = mpsc::channel();
        let job = Job {
            rect: self.pixels,
            cursor: self.cursor,
            interval: Duration::from_secs_f64(1.0 / self.fps.clamp(1, 240) as f64),
            latest: self.latest.clone(),
            stop: self.stop.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("modlight-screen-frames".into())
            .spawn(move || job.run(ready))
            .map_err(|e| format!("the screen capture thread could not start: {e}"))?;
        match started.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => {
                self.thread = Some(thread);
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                self.stop.store(true, Ordering::SeqCst);
                let _ = thread.join();
                Err("the screen capture did not start".into())
            }
        }
    }

    fn latest(&mut self) -> Option<Arc<Vec<u8>>> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ScreenFrames {
    fn drop(&mut self) {
        FrameSource::stop(self);
    }
}

/// What the capture thread needs.
struct Job {
    rect: IRect,
    cursor: bool,
    interval: Duration,
    latest: Latest,
    stop: Arc<AtomicBool>,
}

impl Job {
    fn run(self, ready: mpsc::Sender<Result<(), String>>) {
        let _pixels = PhysicalPixels::enter();
        let Some(mut dib) = Dib::new(self.rect.width as u32, self.rect.height as u32) else {
            let _ = ready.send(Err("the frame buffer could not be made".into()));
            return;
        };
        let Some(screen) = ScreenDc::new() else {
            let _ = ready.send(Err("the screen is not readable".into()));
            return;
        };
        let mut hidden = Exclusion::default();
        hidden.update();
        let _ = ready.send(Ok(()));
        let mut bounds = virtual_bounds(&monitors());
        let mut bounds_read = Instant::now();
        let mut next = Instant::now();
        while !self.stop.load(Ordering::SeqCst) {
            // Windows of this app opened since the last frame stay out of it too.
            hidden.update();
            if bounds_read.elapsed() > Duration::from_secs(1) {
                // A monitor may come or go during a long recording.
                bounds = virtual_bounds(&monitors());
                bounds_read = Instant::now();
            }
            let part = self.rect.intersect(&bounds);
            if part != self.rect {
                // Off every monitor stays black.
                dib.fill([0, 0, 0, 255]);
            }
            if part.has_area() {
                dib.blit_screen(&screen, part, part.x - self.rect.x, part.y - self.rect.y);
            }
            if self.cursor
                && let Some(cursor) = Cursor::current()
            {
                cursor.draw(dib.dc(), self.rect.x, self.rect.y);
            }
            let mut frame = dib.bgra().to_vec();
            // GDI leaves the fourth byte undefined; a frame is opaque.
            for pixel in frame.as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
            *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(frame));
            next += self.interval;
            let now = Instant::now();
            if next > now {
                // Wait in short steps, so a stop is not held up by a slow rate.
                while !self.stop.load(Ordering::SeqCst) {
                    let left = next.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    std::thread::sleep(left.min(Duration::from_millis(50)));
                }
            } else {
                // Behind: carry on at once.
                next = now;
            }
        }
        hidden.restore();
    }
}

/// This app's windows kept out of screen captures while the frames are taken.
#[derive(Default)]
struct Exclusion {
    /// Windows already looked at (by handle value).
    seen: HashSet<isize>,
    /// Windows whose affinity was changed here, to put back.
    changed: Vec<HWND>,
}

impl Exclusion {
    fn update(&mut self) {
        for hwnd in own_windows() {
            // Hidden helper windows (the event loop's, the tray's) are never on screen.
            if !is_visible(hwnd) || !self.seen.insert(hwnd as isize) {
                continue;
            }
            let mut current = 0u32;
            // SAFETY: plain calls on a window of this process.
            unsafe {
                if GetWindowDisplayAffinity(hwnd, &mut current) == 0 || current != WDA_NONE {
                    // Unreadable, or protected by its owner already: left as it is.
                    continue;
                }
                if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) != 0 {
                    self.changed.push(hwnd);
                } else {
                    log::debug!("a window of the app could not be kept out of the recording");
                }
            }
        }
    }

    fn restore(&mut self) {
        for hwnd in self.changed.drain(..) {
            if is_window(hwnd) {
                // SAFETY: a window of this process whose affinity was WDA_NONE.
                unsafe { SetWindowDisplayAffinity(hwnd, WDA_NONE) };
            }
        }
        self.seen.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_even_and_physical() {
        assert_eq!(even(7), 6);
        assert_eq!(even(8), 8);
        assert_eq!(even(1), 2);
        assert_eq!(even(-5), 2);
        let scale = primary_scale();
        let frames = ScreenFrames::new(IRect::new(0, 0, 101, 51), false, 30);
        let (width, height) = frames.size();
        assert_eq!(width % 2, 0);
        assert_eq!(height % 2, 0);
        assert_eq!(width as i32, even((101.0 * scale).round() as i32));
        assert_eq!(height as i32, even((51.0 * scale).round() as i32));
    }

    #[test]
    fn frames_of_a_small_region() {
        let mut frames = ScreenFrames::new(IRect::new(0, 0, 64, 32), true, 30);
        let (width, height) = frames.size();
        assert!(frames.latest().is_none());
        frames.start().unwrap();
        // Starting twice is harmless.
        frames.start().unwrap();
        let started = Instant::now();
        let mut seen = 0;
        while started.elapsed() < Duration::from_secs(5) && seen < 3 {
            if let Some(frame) = frames.latest() {
                assert_eq!(frame.len(), (width * height * 4) as usize);
                assert!(frame.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
                seen += 1;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        assert!(seen >= 3);
        frames.stop();
        assert!(frames.thread.is_none());
        // Stopping again, and dropping, are harmless.
        frames.stop();
    }

    #[test]
    fn frames_partly_off_the_desktop() {
        let scale = primary_scale();
        let bounds = virtual_bounds(&monitors());
        let left = super::super::desktop::to_points(bounds.x, scale);
        let mut frames = ScreenFrames::new(IRect::new(left - 20, 0, 40, 20), false, 60);
        frames.start().unwrap();
        let started = Instant::now();
        let frame = loop {
            if let Some(frame) = frames.latest() {
                break frame;
            }
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        };
        frames.stop();
        // The first pixel lies left of every monitor: black.
        assert_eq!(&frame[..4], &[0, 0, 0, 255]);
    }
}

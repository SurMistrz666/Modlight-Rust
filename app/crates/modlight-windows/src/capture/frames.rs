//! The recorder's frames on Windows: the region read at the requested rate on a thread
//! of its own, as tightly packed BGRA of the frame size.
//!
//! The pixels come from DXGI Desktop Duplication when it can serve them, and from GDI
//! otherwise, part by part ([`Duplication`]). GDI alone used to be the choice: it is
//! upstream's default recorder input (ffmpeg's `gdigrab` copies the screen the same way,
//! `SRCCOPY | CAPTUREBLT`) and it reads any area of the virtual desktop in one call, with
//! no recovery to do. But it cannot see flip-model swap chains presented with independent
//! flip, which is how DX12, Vulkan and many DX11 games run in fullscreen and borderless
//! fullscreen, so recording one gave a black video. GDI stays as the fallback, and the
//! pointer is drawn with it either way: duplication leaves the pointer out.
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

use super::desktop::{
    Monitor, PhysicalPixels, monitors, primary_scale, rect_to_pixels, virtual_bounds,
};
use super::duplication::Duplication;
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

/// The monitors are the same ones, in the same places.
fn same_monitors(a: &[Monitor], b: &[Monitor]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| a.device == b.device && a.bounds == b.bounds)
}

/// One frame of `rect` (physical pixels, the size of `dib`): its pixels through
/// `duplication`, which leaves to GDI what it cannot serve, then the pointer when asked,
/// drawn by GDI as in screenshots (duplication leaves it out), then opaque.
fn next_frame(
    dib: &mut Dib,
    duplication: &mut Duplication,
    screen: &ScreenDc,
    bounds: IRect,
    rect: IRect,
    cursor: bool,
) -> Vec<u8> {
    duplication.draw(dib, screen, bounds);
    if cursor && let Some(cursor) = Cursor::current() {
        cursor.draw(dib.dc(), rect.x, rect.y);
    }
    let mut frame = dib.bgra().to_vec();
    // GDI leaves the fourth byte undefined, and duplication's is of no use: a frame is
    // opaque.
    for pixel in frame.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    frame
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
        let mut screens = monitors();
        let mut bounds = virtual_bounds(&screens);
        let mut bounds_read = Instant::now();
        // Its D3D objects live and die on this thread, with this variable.
        let mut duplication = Duplication::new(self.rect);
        let mut next = Instant::now();
        while !self.stop.load(Ordering::SeqCst) {
            // Windows of this app opened since the last frame stay out of it too.
            hidden.update();
            if bounds_read.elapsed() > Duration::from_secs(1) {
                // A monitor may come or go during a long recording.
                let current = monitors();
                if !same_monitors(&screens, &current) {
                    duplication.rebuild();
                }
                bounds = virtual_bounds(&current);
                screens = current;
                bounds_read = Instant::now();
            }
            let frame = next_frame(
                &mut dib,
                &mut duplication,
                &screen,
                bounds,
                self.rect,
                self.cursor,
            );
            *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(frame));
            // After the frame: the first one is GDI's and never waits for a driver.
            duplication.recover();
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
    fn a_monitor_list_changes_with_a_monitor() {
        let monitor = |device: &str, x| Monitor {
            device: device.into(),
            bounds: IRect::new(x, 0, 1920, 1080),
            work: IRect::new(x, 0, 1920, 1040),
            primary: x == 0,
        };
        let two = [monitor("1", 0), monitor("2", 1920)];
        assert!(same_monitors(&two, &two.clone()));
        // The taskbar moving is no change here.
        let mut taskbar = two.clone();
        taskbar[0].work = IRect::new(0, 40, 1920, 1040);
        assert!(same_monitors(&two, &taskbar));
        assert!(!same_monitors(&two, &two[..1]));
        assert!(!same_monitors(&two, &[monitor("1", 0), monitor("2", 2560)]));
        assert!(!same_monitors(&two, &[monitor("1", 0), monitor("3", 1920)]));
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

    /// Puts a magenta square of this test on the desktop and checks that the frames
    /// leave it out while they run and that it is in pictures of the screen again after.
    /// Run by hand: `cargo test -p modlight-windows -- --ignored own_windows_stay_out`.
    #[test]
    #[ignore = "puts a window on the desktop"]
    fn own_windows_stay_out_of_the_frames() {
        use super::super::desktop::rect_to_points;
        use super::super::gdi::capture_pixels;
        use super::super::transparent::Backdrop;
        use windows_sys::Win32::UI::WindowsAndMessaging::HWND_TOPMOST;

        let _pixels = PhysicalPixels::enter();
        let rect = IRect::new(0, 0, 64, 64);
        let square = Backdrop::new(rect, HWND_TOPMOST).expect("a window");
        square.set_color(0x00FF_00FF);
        let magenta = |bgra: &[u8]| bgra[0] > 200 && bgra[1] < 60 && bgra[2] > 200;
        let centre = |rgba: &image::RgbaImage| {
            let p = rgba.get_pixel(32, 32).0;
            [p[2], p[1], p[0], p[3]]
        };
        assert!(magenta(&centre(&capture_pixels(rect, false).unwrap())));
        let mut frames = ScreenFrames::new(rect_to_points(rect, primary_scale()), false, 30);
        frames.start().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let frame = frames.latest().unwrap();
        let width = frames.size().0 as usize;
        let at = (32 * width + 32) * 4;
        assert!(!magenta(&frame[at..at + 4]), "the square is in the frame");
        frames.stop();
        square.set_color(0x00FF_00FF);
        assert!(magenta(&centre(&capture_pixels(rect, false).unwrap())));
    }

    /// Records a window of this test through Desktop Duplication, with the recorder's own
    /// frame code, while the window changes colour: the frames have the region's size, are
    /// opaque, and show each colour, in BGRA order, from duplication and not from GDI.
    /// Run by hand on a desktop with a real display adapter (a remote session has none):
    /// `cargo test -p modlight-windows -- --ignored --nocapture frames_through_duplication`.
    /// A test binary is not per-monitor DPI aware as a process, so this may go through the
    /// fallback from `DuplicateOutput1` to `DuplicateOutput`; it prints which call worked.
    #[test]
    #[ignore = "puts a window on the desktop and needs a real display adapter"]
    fn frames_through_duplication() {
        use super::super::transparent::Backdrop;
        use windows_sys::Win32::UI::WindowsAndMessaging::HWND_TOPMOST;

        let _pixels = PhysicalPixels::enter();
        let screens = monitors();
        let primary = screens[0].bounds;
        let rect = IRect::new(primary.x + 40, primary.y + 40, 96, 64);
        let bounds = virtual_bounds(&screens);
        let square = Backdrop::new(rect, HWND_TOPMOST).expect("a window");
        let mut dib = Dib::new(rect.width as u32, rect.height as u32).unwrap();
        let screen = ScreenDc::new().unwrap();
        let mut duplication = Duplication::new(rect);
        // Opened before the colours change: a new duplication may need a change of the
        // screen for its first picture.
        next_frame(&mut dib, &mut duplication, &screen, bounds, rect, false);
        duplication.recover();
        let centre = ((rect.height / 2 * rect.width + rect.width / 2) * 4) as usize;
        // `0x00BBGGRR` for the window; the frame's first three bytes are B, G, R. Loose,
        // for a desktop whose colours are managed (HDR, night light).
        let shows = |pixel: &[u8], bgr: [u8; 3]| {
            (0..3).all(|i| {
                if bgr[i] == 255 {
                    pixel[i] > 150
                } else {
                    pixel[i] < 100
                }
            })
        };
        for (color, bgr) in [
            (0x00FF_00FFu32, [255, 0, 255]),
            (0x0000_FF00, [0, 255, 0]),
            (0x00FF_0000, [255, 0, 0]),
        ] {
            square.set_color(color);
            let started = Instant::now();
            loop {
                let frame = next_frame(&mut dib, &mut duplication, &screen, bounds, rect, false);
                assert_eq!(frame.len(), (rect.width * rect.height * 4) as usize);
                assert!(frame.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
                if duplication.pictures() == 1 && shows(&frame[centre..centre + 3], bgr) {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "colour {color:#08X} did not come through Desktop Duplication:\n{}",
                    duplication.describe()
                );
                duplication.recover();
                std::thread::sleep(Duration::from_millis(15));
            }
        }
        println!("{}", duplication.describe());
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

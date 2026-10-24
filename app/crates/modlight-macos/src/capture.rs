//! Pictures of the screen on a Mac (the counterpart of upstream's GDI/DXGI `Screenshot`):
//! displays and the pointer from CoreGraphics, the window list from `CGWindowList`,
//! pixels from ScreenCaptureKit, and a ScreenCaptureKit stream for the recorder.
//!
//! Taking pixels needs the Screen Recording permission: the first capture makes macOS ask
//! (the prompt names the app). Nothing here is ever called by QA or automated runs; they
//! use `modlight_core::capture::source::FakeSource`.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::source::{
    CaptureError, CaptureSource, ScreenInfo, WindowCapture, WindowInfo,
};
use modlight_core::recording::session::FrameSource;

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::ProtocolObject;
#[cfg(target_os = "macos")]
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
#[cfg(target_os = "macos")]
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
#[cfg(target_os = "macos")]
use objc2_core_graphics::CGImage;
#[cfg(target_os = "macos")]
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString,
};
#[cfg(target_os = "macos")]
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCScreenshotManager, SCShareableContent, SCStream,
    SCStreamConfiguration, SCStreamOutput, SCStreamOutputType, SCWindow,
};

/// How long a ScreenCaptureKit call may take before the capture gives up.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The names the app uses on every system (`platform::capture::ScreenSource`).
pub type ScreenSource = MacSource;
pub type ScreenFrames = MacFrames;

/// The Mac's screen, as the capture code sees it.
#[derive(Default)]
pub struct MacSource {
    /// The visible frame of each display (`WorkingArea`), read on the main thread.
    working_areas: Mutex<HashMap<u32, IRect>>,
    names: Mutex<HashMap<u32, String>>,
}

impl MacSource {
    pub fn new() -> Arc<MacSource> {
        let source = Arc::new(MacSource::default());
        source.refresh_on_main_thread();
        source
    }

    /// Read what only the main thread may ask AppKit for (the visible frames and the
    /// displays' names). Called when a capture starts.
    pub fn refresh_on_main_thread(&self) {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSScreen;
            use objc2_foundation::MainThreadMarker;
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            // Cocoa's y is turned around by the primary display's height
            // (`crate::windows::visible_in_desktop`), not by `mainScreen`'s: that is the
            // screen with the key window, and any display of another height put every
            // working area off by the difference.
            let (screens, _) = crate::windows::cocoa_screens(mtm);
            let frames: Vec<crate::windows::CocoaScreen> =
                screens.iter().map(|(_, s)| *s).collect();
            let mut areas = self.working_areas.lock().unwrap();
            for (index, (id, _)) in screens.iter().enumerate() {
                if *id == 0 {
                    continue;
                }
                if let Some((x, y, w, h)) = crate::windows::visible_in_desktop(&frames, index) {
                    areas.insert(
                        *id,
                        IRect::new(
                            x.round() as i32,
                            y.round() as i32,
                            w.round() as i32,
                            h.round() as i32,
                        ),
                    );
                }
            }
            let mut names = self.names.lock().unwrap();
            for screen in NSScreen::screens(mtm).iter() {
                let description = screen.deviceDescription();
                let key = NSString::from_str("NSScreenNumber");
                let Some(number) = description.objectForKey(&key) else {
                    continue;
                };
                let Ok(number) = number.downcast::<NSNumber>() else {
                    continue;
                };
                names.insert(
                    number.unsignedIntValue(),
                    screen.localizedName().to_string(),
                );
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn displays() -> Vec<(u32, IRect, f64, bool)> {
    use objc2_core_graphics::{
        CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode, CGGetActiveDisplayList,
        CGMainDisplayID,
    };
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    // SAFETY: the buffer holds 16 ids and the count is written back.
    let error = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
    if error.0 != 0 {
        return Vec::new();
    }
    let main = CGMainDisplayID();
    ids[..count as usize]
        .iter()
        .map(|&id| {
            let b = CGDisplayBounds(id);
            let rect = IRect::new(
                b.origin.x.round() as i32,
                b.origin.y.round() as i32,
                b.size.width.round() as i32,
                b.size.height.round() as i32,
            );
            let mode = CGDisplayCopyDisplayMode(id);
            let scale = mode
                .as_deref()
                .map(|m| {
                    let points = CGDisplayMode::width(Some(m)).max(1) as f64;
                    CGDisplayMode::pixel_width(Some(m)) as f64 / points
                })
                .filter(|s| s.is_finite() && *s > 0.0)
                .unwrap_or(1.0);
            (id, rect, scale, id == main)
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn displays() -> Vec<(u32, IRect, f64, bool)> {
    Vec::new()
}

/// A CoreFoundation number in a window description.
#[cfg(target_os = "macos")]
fn number(dict: &NSDictionary<NSString, objc2::runtime::AnyObject>, key: &str) -> Option<f64> {
    let value = dict.objectForKey(&NSString::from_str(key))?;
    value.downcast::<NSNumber>().ok().map(|n| n.doubleValue())
}

#[cfg(target_os = "macos")]
fn text(dict: &NSDictionary<NSString, objc2::runtime::AnyObject>, key: &str) -> String {
    dict.objectForKey(&NSString::from_str(key))
        .and_then(|v| v.downcast::<NSString>().ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Every normal window on screen, front to back (`WindowsList.GetVisibleWindowsList`).
#[cfg(target_os = "macos")]
fn window_list() -> Vec<(WindowInfo, i64)> {
    use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID};
    let Some(array) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    // SAFETY: CFArray of CFDictionary is toll-free bridged to NSArray of NSDictionary.
    let array: &NSArray<NSDictionary<NSString, objc2::runtime::AnyObject>> = unsafe {
        &*(CFRetained::as_ptr(&array).as_ptr()
            as *const NSArray<NSDictionary<NSString, objc2::runtime::AnyObject>>)
    };
    let own = std::process::id() as i32;
    let mut windows = Vec::new();
    for dict in array.iter() {
        let layer = number(&dict, "kCGWindowLayer").unwrap_or(0.0) as i64;
        let pid = number(&dict, "kCGWindowOwnerPID").unwrap_or(0.0) as i32;
        if pid == own {
            continue;
        }
        let Some(bounds) = dict
            .objectForKey(&NSString::from_str("kCGWindowBounds"))
            .and_then(|b| b.downcast::<NSDictionary>().ok())
        else {
            continue;
        };
        // SAFETY: the bounds dictionary holds NSString keys and NSNumber values.
        let bounds: Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> =
            unsafe { Retained::cast_unchecked(bounds) };
        let rect = IRect::new(
            number(&bounds, "X").unwrap_or(0.0).round() as i32,
            number(&bounds, "Y").unwrap_or(0.0).round() as i32,
            number(&bounds, "Width").unwrap_or(0.0).round() as i32,
            number(&bounds, "Height").unwrap_or(0.0).round() as i32,
        );
        if rect.width < 2 || rect.height < 2 {
            continue;
        }
        windows.push((
            WindowInfo {
                id: number(&dict, "kCGWindowNumber").unwrap_or(0.0) as u32,
                title: text(&dict, "kCGWindowName"),
                process_name: text(&dict, "kCGWindowOwnerName"),
                pid,
                rect,
                on_screen: true,
            },
            layer,
        ));
    }
    windows
}

#[cfg(not(target_os = "macos"))]
fn window_list() -> Vec<(WindowInfo, i64)> {
    Vec::new()
}

/// Wait for a ScreenCaptureKit completion handler.
fn wait<T>(rx: mpsc::Receiver<T>) -> Option<T> {
    rx.recv_timeout(TIMEOUT).ok()
}

#[cfg(target_os = "macos")]
fn shareable_content() -> Result<Retained<SCShareableContent>, CaptureError> {
    let (tx, rx) = mpsc::channel();
    let block = block2::RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let result = if content.is_null() {
                Err(error_of(error))
            } else {
                // SAFETY: a non-null content object handed to the handler; retain it.
                Ok(unsafe { Retained::retain(content) }.unwrap())
            };
            let _ = tx.send(result);
        },
    );
    // SAFETY: the block outlives the call; the handler sends one result.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    wait(rx).unwrap_or(Err(CaptureError::Failed(
        "ScreenCaptureKit did not answer".into(),
    )))
}

#[cfg(target_os = "macos")]
fn error_of(error: *mut NSError) -> CaptureError {
    if error.is_null() {
        return CaptureError::Failed("unknown error".into());
    }
    // SAFETY: a non-null error handed to the handler.
    let error = unsafe { &*error };
    // SCStreamErrorUserDeclined is -3801.
    if error.code() == -3801 {
        CaptureError::NotPermitted
    } else {
        CaptureError::Failed(error.localizedDescription().to_string())
    }
}

#[cfg(target_os = "macos")]
fn capture_image(
    filter: &SCContentFilter,
    config: &SCStreamConfiguration,
) -> Result<RgbaImage, CaptureError> {
    let (tx, rx) = mpsc::channel();
    let block = block2::RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let result = if image.is_null() {
            Err(error_of(error))
        } else {
            // SAFETY: a valid CGImage for the duration of the handler.
            to_rgba(unsafe { &*image })
        };
        let _ = tx.send(result);
    });
    // SAFETY: filter and configuration are valid; the handler sends one result.
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            filter,
            config,
            Some(&block),
        )
    };
    wait(rx).unwrap_or(Err(CaptureError::Failed(
        "ScreenCaptureKit did not answer".into(),
    )))
}

/// A CGImage's pixels as straight RGBA. ScreenCaptureKit's SDR pictures are 32-bit BGRA
/// (premultiplied alpha first, little endian); other 32-bit layouts are read too.
#[cfg(target_os = "macos")]
fn to_rgba(image: &CGImage) -> Result<RgbaImage, CaptureError> {
    use objc2_core_graphics::{CGBitmapInfo, CGDataProvider, CGImageAlphaInfo};
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let bpp = CGImage::bits_per_pixel(Some(image));
    let row = CGImage::bytes_per_row(Some(image));
    if bpp != 32 || width == 0 || height == 0 {
        return Err(CaptureError::Failed(format!(
            "unsupported picture: {bpp} bits per pixel"
        )));
    }
    let info = CGImage::bitmap_info(Some(image));
    let alpha = CGImage::alpha_info(Some(image));
    let provider = CGImage::data_provider(Some(image)).ok_or(CaptureError::Nothing)?;
    let data = CGDataProvider::data(Some(&provider)).ok_or(CaptureError::Nothing)?;
    let bytes = data.to_vec();
    let little = (info.0 & CGBitmapInfo::ByteOrderInfoMask.0)
        == objc2_core_graphics::CGImageByteOrderInfo::Order32Little.0;
    let alpha_first = matches!(
        alpha,
        CGImageAlphaInfo::PremultipliedFirst
            | CGImageAlphaInfo::First
            | CGImageAlphaInfo::NoneSkipFirst
    );
    let premultiplied = matches!(
        alpha,
        CGImageAlphaInfo::PremultipliedFirst | CGImageAlphaInfo::PremultipliedLast
    );
    let has_alpha = !matches!(
        alpha,
        CGImageAlphaInfo::NoneSkipFirst | CGImageAlphaInfo::NoneSkipLast | CGImageAlphaInfo::None
    );
    let mut out = RgbaImage::new(width as u32, height as u32);
    for y in 0..height {
        let line = &bytes[y * row..(y * row + width * 4).min(bytes.len())];
        for x in 0..width.min(line.len() / 4) {
            let p = &line[x * 4..x * 4 + 4];
            // Memory order to A, R, G, B.
            let (a, r, g, b) = match (little, alpha_first) {
                (true, true) => (p[3], p[2], p[1], p[0]),
                (true, false) => (p[0], p[3], p[2], p[1]),
                (false, true) => (p[0], p[1], p[2], p[3]),
                (false, false) => (p[3], p[0], p[1], p[2]),
            };
            let a = if has_alpha { a } else { 255 };
            let un = |c: u8| -> u8 {
                if premultiplied && a > 0 && a < 255 {
                    ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
                } else {
                    c
                }
            };
            out.put_pixel(x as u32, y as u32, image::Rgba([un(r), un(g), un(b), a]));
        }
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
fn cg_rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

#[cfg(target_os = "macos")]
fn display_for(content: &SCShareableContent, id: u32) -> Option<Retained<SCDisplay>> {
    // SAFETY: plain property reads.
    unsafe { content.displays() }
        .iter()
        .find(|d| unsafe { d.displayID() } == id)
}

impl MacSource {
    /// One display's part of a desktop rectangle, at `scale` pixels per point.
    #[cfg(target_os = "macos")]
    fn capture_display_part(
        &self,
        content: &SCShareableContent,
        id: u32,
        display_bounds: IRect,
        part: IRect,
        scale: f64,
        cursor: bool,
    ) -> Result<RgbaImage, CaptureError> {
        let display = display_for(content, id).ok_or(CaptureError::Nothing)?;
        let empty: Retained<NSArray<SCWindow>> = NSArray::new();
        // SAFETY: a valid display and an empty exclusion list.
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &empty,
            )
        };
        // SAFETY: plain setters on a fresh configuration.
        let config = unsafe { SCStreamConfiguration::new() };
        let width = (part.width as f64 * scale).round().max(1.0) as usize;
        let height = (part.height as f64 * scale).round().max(1.0) as usize;
        unsafe {
            config.setWidth(width);
            config.setHeight(height);
            config.setSourceRect(cg_rect(
                (part.x - display_bounds.x) as f64,
                (part.y - display_bounds.y) as f64,
                part.width as f64,
                part.height as f64,
            ));
            config.setShowsCursor(cursor);
            config.setScalesToFit(false);
            config.setPixelFormat(objc2_core_video::kCVPixelFormatType_32BGRA);
        }
        capture_image(&filter, &config)
    }
}

impl CaptureSource for MacSource {
    fn screens(&self) -> Vec<ScreenInfo> {
        let areas = self.working_areas.lock().unwrap().clone();
        let names = self.names.lock().unwrap().clone();
        displays()
            .into_iter()
            .map(|(id, bounds, scale, primary)| ScreenInfo {
                id,
                bounds,
                working_area: areas.get(&id).copied().unwrap_or(bounds),
                scale,
                primary,
                name: names.get(&id).cloned().unwrap_or_default(),
            })
            .collect()
    }

    fn cursor_position(&self) -> (i32, i32) {
        #[cfg(target_os = "macos")]
        {
            use objc2_core_graphics::CGEvent;
            let event = CGEvent::new(None);
            let point = CGEvent::location(event.as_deref());
            (point.x.round() as i32, point.y.round() as i32)
        }
        #[cfg(not(target_os = "macos"))]
        (0, 0)
    }

    fn windows(&self) -> Vec<WindowInfo> {
        window_list()
            .into_iter()
            .filter(|(_, layer)| *layer == 0)
            .map(|(w, _)| w)
            .collect()
    }

    fn active_window(&self) -> Option<WindowInfo> {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSWorkspace;
            let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
            let pid = app.processIdentifier();
            self.windows().into_iter().find(|w| w.pid == pid)
        }
        #[cfg(not(target_os = "macos"))]
        None
    }

    fn capture_rect(&self, rect: IRect, cursor: bool) -> Result<RgbaImage, CaptureError> {
        #[cfg(target_os = "macos")]
        {
            let rect = rect.intersect(&self.screen_bounds());
            if !rect.has_area() {
                return Err(CaptureError::Nothing);
            }
            let scale = self.scale_for(rect);
            let content = shareable_content()?;
            let mut out = RgbaImage::new(
                (rect.width as f64 * scale).round() as u32,
                (rect.height as f64 * scale).round() as u32,
            );
            for (id, bounds, display_scale, _) in displays() {
                let part = bounds.intersect(&rect);
                if !part.has_area() {
                    continue;
                }
                let mut picture =
                    self.capture_display_part(&content, id, bounds, part, display_scale, cursor)?;
                let (w, h) = (
                    (part.width as f64 * scale).round() as u32,
                    (part.height as f64 * scale).round() as u32,
                );
                if picture.dimensions() != (w, h) {
                    picture = image::imageops::resize(
                        &picture,
                        w,
                        h,
                        image::imageops::FilterType::Triangle,
                    );
                }
                let x = ((part.x - rect.x) as f64 * scale).round() as i64;
                let y = ((part.y - rect.y) as f64 * scale).round() as i64;
                image::imageops::replace(&mut out, &picture, x, y);
            }
            Ok(out)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (rect, cursor);
            Err(CaptureError::NotPermitted)
        }
    }

    fn capture_window(
        &self,
        window: &WindowInfo,
        how: WindowCapture,
    ) -> Result<RgbaImage, CaptureError> {
        #[cfg(target_os = "macos")]
        {
            let content = shareable_content()?;
            // SAFETY: plain property reads.
            let sc_window = unsafe { content.windows() }
                .iter()
                .find(|w| unsafe { w.windowID() } == window.id)
                .ok_or(CaptureError::Nothing)?;
            // SAFETY: a valid window.
            let filter = unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(
                    SCContentFilter::alloc(),
                    &sc_window,
                )
            };
            // SAFETY: plain reads and setters.
            let (rect, scale) = unsafe { (filter.contentRect(), filter.pointPixelScale() as f64) };
            let config = unsafe { SCStreamConfiguration::new() };
            unsafe {
                config.setWidth((rect.size.width * scale).round().max(1.0) as usize);
                config.setHeight((rect.size.height * scale).round().max(1.0) as usize);
                config.setShowsCursor(how.cursor);
                config.setIgnoreShadowsSingleWindow(!how.shadow);
                config.setShouldBeOpaque(!how.transparent);
                config.setPixelFormat(objc2_core_video::kCVPixelFormatType_32BGRA);
            }
            capture_image(&filter, &config)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (window, how);
            Err(CaptureError::NotPermitted)
        }
    }

    fn activate_window(&self, window: &WindowInfo) {
        // Raising one particular window needs the Accessibility permission; the app is
        // brought forward instead, which raises its windows.
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
            if let Some(app) =
                NSRunningApplication::runningApplicationWithProcessIdentifier(window.pid)
            {
                app.activateWithOptions(NSApplicationActivationOptions::empty());
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = window;
    }
}

// ---------------------------------------------------------------- the recorder's frames

#[cfg(target_os = "macos")]
struct OutputIvars {
    latest: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    size: (u32, u32),
}

#[cfg(target_os = "macos")]
define_class!(
    // SAFETY: NSObject has no subclassing requirements; the ivars are Send + Sync.
    #[unsafe(super(NSObject))]
    #[name = "ModlightStreamOutput"]
    #[ivars = OutputIvars]
    struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(
            &self,
            _stream: &SCStream,
            sample: &objc2_core_media::CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind != SCStreamOutputType::Screen {
                return;
            }
            // SAFETY: a valid sample buffer for the duration of the call.
            let Some(buffer) = (unsafe { sample.image_buffer() }) else {
                return;
            };
            if let Some(frame) = copy_frame(&buffer, self.ivars().size) {
                *self.ivars().latest.lock().unwrap() = Some(Arc::new(frame));
            }
        }
    }
);

#[cfg(target_os = "macos")]
impl StreamOutput {
    fn new(latest: Arc<Mutex<Option<Arc<Vec<u8>>>>>, size: (u32, u32)) -> Retained<StreamOutput> {
        let this = StreamOutput::alloc().set_ivars(OutputIvars { latest, size });
        // SAFETY: NSObject's init on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// The pixels of a frame, tightly packed BGRA of the expected size.
#[cfg(target_os = "macos")]
fn copy_frame(buffer: &objc2_core_video::CVImageBuffer, size: (u32, u32)) -> Option<Vec<u8>> {
    use objc2_core_video::{
        CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
        CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
        CVPixelBufferUnlockBaseAddress,
    };
    let (w, h) = (size.0 as usize, size.1 as usize);
    // SAFETY: the buffer is locked for reading while its bytes are copied.
    unsafe {
        if CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) != 0 {
            return None;
        }
        let width = CVPixelBufferGetWidth(buffer);
        let height = CVPixelBufferGetHeight(buffer);
        let row = CVPixelBufferGetBytesPerRow(buffer);
        let base = CVPixelBufferGetBaseAddress(buffer) as *const u8;
        let mut out = None;
        if !base.is_null() && width >= w && height >= h {
            let mut frame = vec![0u8; w * h * 4];
            for y in 0..h {
                let source = std::slice::from_raw_parts(base.add(y * row), w * 4);
                frame[y * w * 4..(y + 1) * w * 4].copy_from_slice(source);
            }
            out = Some(frame);
        }
        CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly);
        out
    }
}

/// The recorder's frames: a ScreenCaptureKit stream of one display's part of the area,
/// without this app's own windows (the recording frame and toolbar stay out of the video).
pub struct MacFrames {
    region: IRect,
    cursor: bool,
    fps: i64,
    size: (u32, u32),
    latest: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    #[cfg(target_os = "macos")]
    stream: Option<(Retained<SCStream>, Retained<StreamOutput>)>,
}

// SAFETY: the stream objects are only touched by the recording thread that owns this.
unsafe impl Send for MacFrames {}

impl MacFrames {
    /// The display that holds most of `region` decides the scale (like upstream's
    /// `ddagrab`, which records the monitor with the largest intersection).
    pub fn new(region: IRect, cursor: bool, fps: i64) -> MacFrames {
        let scale = displays()
            .into_iter()
            .max_by_key(|(_, bounds, _, _)| {
                let part = bounds.intersect(&region);
                part.width as i64 * part.height as i64
            })
            .map(|(_, _, scale, _)| scale)
            .unwrap_or(1.0);
        let even = |v: f64| ((v.round() as u32) / 2 * 2).max(2);
        MacFrames {
            region,
            cursor,
            fps,
            size: (
                even(region.width as f64 * scale),
                even(region.height as f64 * scale),
            ),
            latest: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "macos")]
            stream: None,
        }
    }
}

impl FrameSource for MacFrames {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn start(&mut self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            let content = shareable_content().map_err(|e| e.to_string())?;
            let (id, bounds, _, _) = displays()
                .into_iter()
                .max_by_key(|(_, bounds, _, _)| {
                    let part = bounds.intersect(&self.region);
                    part.width as i64 * part.height as i64
                })
                .ok_or("no display")?;
            let part = bounds.intersect(&self.region);
            let display = display_for(&content, id).ok_or("display gone")?;
            let own = std::process::id() as i32;
            // SAFETY: plain property reads.
            let apps: Vec<_> = unsafe { content.applications() }
                .iter()
                .filter(|a| unsafe { a.processID() } == own)
                .collect();
            let apps = NSArray::from_retained_slice(&apps);
            let none: Retained<NSArray<SCWindow>> = NSArray::new();
            // SAFETY: valid display and lists.
            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &apps,
                    &none,
                )
            };
            let config = unsafe { SCStreamConfiguration::new() };
            unsafe {
                config.setWidth(self.size.0 as usize);
                config.setHeight(self.size.1 as usize);
                config.setSourceRect(cg_rect(
                    (part.x - bounds.x) as f64,
                    (part.y - bounds.y) as f64,
                    part.width as f64,
                    part.height as f64,
                ));
                config.setShowsCursor(self.cursor);
                config.setScalesToFit(true);
                config.setPixelFormat(objc2_core_video::kCVPixelFormatType_32BGRA);
                config.setQueueDepth(5);
                config.setMinimumFrameInterval(objc2_core_media::CMTime::with_seconds(
                    1.0 / self.fps.max(1) as f64,
                    600,
                ));
            }
            // SAFETY: valid filter and configuration, no delegate.
            let stream = unsafe {
                SCStream::initWithFilter_configuration_delegate(
                    SCStream::alloc(),
                    &filter,
                    &config,
                    None,
                )
            };
            let output = StreamOutput::new(self.latest.clone(), self.size);
            let queue = dispatch2::DispatchQueue::new("org.modlight.recorder", None);
            // SAFETY: the output object outlives the stream (both kept in self).
            unsafe {
                stream
                    .addStreamOutput_type_sampleHandlerQueue_error(
                        ProtocolObject::from_ref(&*output),
                        SCStreamOutputType::Screen,
                        Some(&queue),
                    )
                    .map_err(|e| e.localizedDescription().to_string())?;
            }
            let (tx, rx) = mpsc::channel();
            let block = block2::RcBlock::new(move |error: *mut NSError| {
                let _ = tx.send(if error.is_null() {
                    Ok(())
                } else {
                    Err(error_of(error).to_string())
                });
            });
            // SAFETY: the handler sends one result.
            unsafe { stream.startCaptureWithCompletionHandler(Some(&block)) };
            wait(rx).unwrap_or(Err("ScreenCaptureKit did not start".into()))?;
            self.stream = Some((stream, output));
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        Err("screen recording needs macOS".into())
    }

    fn latest(&mut self) -> Option<Arc<Vec<u8>>> {
        self.latest.lock().unwrap().clone()
    }

    fn stop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some((stream, _output)) = self.stream.take() {
            let (tx, rx) = mpsc::channel();
            let block = block2::RcBlock::new(move |_error: *mut NSError| {
                let _ = tx.send(());
            });
            // SAFETY: the handler signals once.
            unsafe { stream.stopCaptureWithCompletionHandler(Some(&block)) };
            let _ = wait(rx);
        }
    }
}

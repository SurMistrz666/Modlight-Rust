//! Captures (`ShareX/CaptureHelpers`: `CaptureBase` and its kinds, `Screenshot`,
//! `RegionCaptureTasks`): what is taken, how it becomes a task's picture, and the last
//! region every later "last region" capture repeats.
//!
//! The non-interactive kinds run here against a [`source::CaptureSource`]; the region
//! kinds need the overlay, so they produce a [`region::RegionRequest`] the window shows
//! and hand the overlay's outcome back to [`finish_region`].

pub mod geometry;
pub mod options;
pub mod region;
pub mod scrolling;
pub mod source;

use std::sync::{Arc, Mutex};

use geometry::IRect;
use region::{RegionKind, RegionOutcome, RegionRequest};
use source::{CaptureError, CaptureSource, WindowCapture, WindowInfo};

use crate::tasks::info::TaskMetadata;
use crate::tasks::settings::{AfterCaptureTasks, TaskSettings};

/// Whether the region capture can switch to annotation tools (the image editor's,
/// drawn on the frozen desktop; the app's overlay hosts the editor).
pub const ANNOTATION_TOOLS_AVAILABLE: bool = true;

/// `CaptureType` plus the parameters some kinds carry.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureKind {
    Fullscreen,
    /// `CaptureMonitor(bounds)`.
    Monitor(IRect),
    ActiveMonitor,
    /// `CaptureWindow(handle)`.
    Window(WindowInfo),
    ActiveWindow,
    /// `CaptureRegion(type)`.
    Region(RegionKind),
    CustomRegion,
    LastRegion,
    /// `CaptureCustomWindow`: the window whose title is in the settings.
    CustomWindow,
}

impl CaptureKind {
    /// Whether the overlay is needed (a region kind without a last region to repeat).
    pub fn is_region(&self) -> bool {
        matches!(self, CaptureKind::Region(_))
    }
}

/// What earlier captures left behind (`CaptureRegion.lastRegionCaptureType`,
/// `RegionCaptureIntegration.LastRegionRectangle`, `RegionCaptureLightForm.LastScreenSelectionRectangle`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LastRegion {
    pub kind: RegionKind,
    /// The default region capture's last area, desktop points.
    pub rect: IRect,
    /// The light and transparent captures' last area, desktop points.
    pub light_rect: IRect,
}

static LAST: Mutex<LastRegion> = Mutex::new(LastRegion {
    kind: RegionKind::Default,
    rect: IRect::new(0, 0, 0, 0),
    light_rect: IRect::new(0, 0, 0, 0),
});

pub fn last_region() -> LastRegion {
    *LAST.lock().unwrap()
}

pub fn set_last_region(last: LastRegion) {
    *LAST.lock().unwrap() = last;
}

/// What a capture produced, before it becomes a task.
#[derive(Clone, Debug)]
pub struct Captured {
    pub metadata: TaskMetadata,
    /// The capture changed the picture (annotations): no annotation afterwards
    /// (`AllowAnnotation = false`).
    pub modified: bool,
    pub kind: CaptureKind,
}

/// What a capture needs next.
#[allow(clippy::large_enum_variant)]
pub enum CaptureStep {
    /// The picture is taken.
    Done(Captured),
    /// The overlay has to ask where.
    Region(RegionRequest),
    /// The kind needed something that is not there (no window with that title...).
    Message(String),
    /// Nothing was taken (cancelled, empty, refused).
    Nothing(Option<CaptureError>),
}

/// The process whose window is the desktop, which never names a capture: upstream's
/// "explorer" on Windows; Finder plays its part on macOS.
const DESKTOP_PROCESS: &str = if cfg!(windows) { "explorer" } else { "Finder" };

/// `CreateMetadata(insideRect, "explorer")`: the active window names the capture when the
/// captured area lies inside it, unless it is the desktop's.
fn metadata_for(
    source: &dyn CaptureSource,
    inside: Option<IRect>,
    ignore: Option<&str>,
) -> TaskMetadata {
    let mut metadata = TaskMetadata::default();
    if let Some(window) = source.active_window() {
        let ignored = ignore.is_some_and(|p| window.process_name.eq_ignore_ascii_case(p));
        let inside_ok = inside.is_none_or(|r| r.is_empty() || window.rect.contains_rect(&r));
        if !ignored && inside_ok {
            metadata.window_title = Some(window.title.clone());
            metadata.process_name = Some(window.process_name.clone());
        }
    }
    metadata
}

fn with_window(mut metadata: TaskMetadata, window: Option<&WindowInfo>) -> TaskMetadata {
    if let Some(window) = window {
        metadata.window_title = Some(window.title.clone());
        metadata.process_name = Some(window.process_name.clone());
    }
    metadata
}

fn done(kind: CaptureKind, metadata: TaskMetadata, image: image::RgbaImage) -> CaptureStep {
    CaptureStep::Done(Captured {
        metadata: TaskMetadata {
            image: Some(Arc::new(image)),
            ..metadata
        },
        modified: false,
        kind,
    })
}

/// The overlay's starting point for a region capture (`ExecuteRegionCaptureAvaloniaAsync`,
/// `ExecuteRegionCaptureLight`, `ExecuteRegionCaptureTransparent`).
pub fn region_request(
    kind: RegionKind,
    settings: &TaskSettings,
    source: &dyn CaptureSource,
) -> Result<RegionRequest, CaptureError> {
    let capture = &settings.capture_settings;
    let active_monitor = capture.surface_options.active_monitor_mode;
    let bounds = if active_monitor {
        source.active_screen().map(|s| s.bounds).unwrap_or_default()
    } else {
        source.screen_bounds()
    };
    let scale = source.scale_for(bounds);
    let image = if kind == RegionKind::Transparent {
        None
    } else {
        // The overlay draws the pointer itself, from its own picture.
        Some(Arc::new(source.capture_rect(bounds, false)?))
    };
    let cursor = if kind == RegionKind::Default && capture.show_cursor {
        source.cursor_image().map(|(picture, (x, y))| {
            (
                Arc::new(picture),
                geometry::Point::new((x - bounds.x) as f64 * scale, (y - bounds.y) as f64 * scale),
            )
        })
    } else {
        None
    };
    let windows = if kind == RegionKind::Default && capture.surface_options.detect_windows {
        source
            .windows()
            .into_iter()
            .filter(|w| w.on_screen)
            .collect()
    } else {
        Vec::new()
    };
    Ok(RegionRequest {
        kind,
        screen_bounds: bounds,
        scale,
        image,
        options: capture.surface_options.clone(),
        enable_annotations: !settings.advanced_settings.region_capture_disable_annotation,
        windows,
        screens: source.screens(),
        cursor,
        last_region: last_region().rect,
        pointer: source.cursor_position(),
    })
}

/// Run a capture as far as it goes without the overlay.
pub fn capture(
    kind: CaptureKind,
    settings: &TaskSettings,
    source: &dyn CaptureSource,
) -> CaptureStep {
    let capture = &settings.capture_settings;
    let cursor = capture.show_cursor;
    let result = match &kind {
        CaptureKind::Fullscreen => {
            // `CaptureFullscreen`: the whole desktop, named after the working area's owner.
            let bounds = source.screen_bounds();
            let metadata = metadata_for(source, Some(working_area(source)), Some(DESKTOP_PROCESS));
            source
                .capture_rect(bounds, cursor)
                .map(|image| (metadata, image))
        }
        CaptureKind::Monitor(bounds) => {
            let metadata = metadata_for(source, Some(*bounds), Some(DESKTOP_PROCESS));
            source
                .capture_rect(*bounds, cursor)
                .map(|image| (metadata, image))
        }
        CaptureKind::ActiveMonitor => {
            let screen = source.active_screen();
            let bounds = screen.as_ref().map(|s| s.bounds).unwrap_or_default();
            let area = screen.map(|s| s.working_area).unwrap_or_default();
            let metadata = metadata_for(source, Some(area), Some(DESKTOP_PROCESS));
            source
                .capture_rect(bounds, cursor)
                .map(|image| (metadata, image))
        }
        CaptureKind::Window(window) => {
            source.activate_window(window);
            let metadata = with_window(TaskMetadata::default(), Some(window));
            capture_window(source, window, settings).map(|image| (metadata, image))
        }
        CaptureKind::ActiveWindow => match source.active_window() {
            Some(window) => {
                let metadata = with_window(TaskMetadata::default(), Some(&window));
                capture_window(source, &window, settings).map(|image| (metadata, image))
            }
            None => Err(CaptureError::Nothing),
        },
        CaptureKind::CustomWindow => {
            let title = capture.capture_custom_window.trim();
            if title.is_empty() {
                return CaptureStep::Nothing(None);
            }
            match search_window(source, title) {
                Some(window) => {
                    source.activate_window(&window);
                    let metadata = with_window(TaskMetadata::default(), Some(&window));
                    capture_window(source, &window, settings).map(|image| (metadata, image))
                }
                None => {
                    return CaptureStep::Message(
                        crate::l10n::t(crate::l10n::s::UnableToFindAWindowWithSpecifiedWindowTitle)
                            .to_string(),
                    );
                }
            }
        }
        CaptureKind::CustomRegion => {
            let rect = capture.capture_custom_region;
            let metadata = metadata_for(source, Some(rect), Some(DESKTOP_PROCESS));
            source
                .capture_rect(rect, cursor)
                .map(|image| (metadata, image))
        }
        CaptureKind::LastRegion => {
            let last = last_region();
            let rect = match last.kind {
                RegionKind::Default => last.rect,
                RegionKind::Light | RegionKind::Transparent => last.light_rect,
            };
            if rect.is_empty() {
                // Nothing to repeat yet: the capture it would repeat runs instead.
                return match region_request(last.kind, settings, source) {
                    Ok(request) => CaptureStep::Region(request),
                    Err(error) => CaptureStep::Nothing(Some(error)),
                };
            }
            source
                .capture_rect(rect, cursor)
                .map(|image| (TaskMetadata::default(), image))
        }
        CaptureKind::Region(region_kind) => {
            return match region_request(*region_kind, settings, source) {
                Ok(request) => CaptureStep::Region(request),
                Err(error) => CaptureStep::Nothing(Some(error)),
            };
        }
    };
    match result {
        Ok((metadata, image)) => done(kind, metadata, image),
        Err(error) => CaptureStep::Nothing(Some(error)),
    }
}

fn working_area(source: &dyn CaptureSource) -> IRect {
    let screens = source.screens();
    let mut iter = screens.iter().map(|s| s.working_area);
    let Some(first) = iter.next() else {
        return IRect::default();
    };
    iter.fold(first, |acc, r| acc.union(&r))
}

/// `CaptureWindow.Execute`'s picture: alone with its shadow and transparency when the
/// settings ask for that, else the area it covers.
///
/// On Windows the platform always takes the window itself, as upstream's
/// `Screenshot.CaptureWindow` does: it reads the window's rectangle again (a minimised
/// window picked from the menu is restored first, and its old rectangle lies off
/// screen) and honours "capture client area". On macOS a window that is not taken
/// transparently is the area it covers.
fn capture_window(
    source: &dyn CaptureSource,
    window: &WindowInfo,
    settings: &TaskSettings,
) -> Result<image::RgbaImage, CaptureError> {
    let capture = &settings.capture_settings;
    let how = WindowCapture {
        transparent: capture.capture_transparent && !capture.capture_client_area,
        shadow: capture.capture_shadow,
        client_area: capture.capture_client_area,
        cursor: capture.show_cursor,
    };
    if cfg!(windows) || how.transparent {
        source.capture_window(window, how)
    } else {
        source.capture_rect(window.rect, capture.show_cursor)
    }
}

/// `NativeMethods.SearchWindow`: the exact title first, then a title containing it.
pub fn search_window(source: &dyn CaptureSource, title: &str) -> Option<WindowInfo> {
    let windows = source.windows();
    windows
        .iter()
        .find(|w| w.title == title)
        .or_else(|| {
            let needle = title.to_lowercase();
            windows
                .iter()
                .find(|w| w.title.to_lowercase().contains(&needle))
        })
        .cloned()
}

/// The overlay closed: the picture, and what later captures remember.
pub fn finish_region(
    request: &RegionRequest,
    outcome: &RegionOutcome,
    source: &dyn CaptureSource,
    modified: bool,
) -> Option<Captured> {
    let RegionOutcome::Completed {
        image_rect,
        screen_rect,
        window,
    } = outcome
    else {
        return None;
    };
    let image = match &request.image {
        Some(frozen) => region::crop(frozen, *image_rect),
        // The transparent capture takes the area after the overlay has gone.
        None => source.capture_rect(*screen_rect, false).ok()?,
    };
    let mut last = last_region();
    match request.kind {
        RegionKind::Default => last.rect = *screen_rect,
        RegionKind::Light | RegionKind::Transparent => last.light_rect = *screen_rect,
    }
    last.kind = request.kind;
    set_last_region(last);
    let metadata = with_window(
        TaskMetadata {
            image: Some(Arc::new(image)),
            ..TaskMetadata::default()
        },
        window.as_ref(),
    );
    Some(Captured {
        metadata,
        modified,
        kind: CaptureKind::Region(request.kind),
    })
}

/// `CaptureBase.AfterCapture`: the task settings the captured picture runs with.
pub fn after_capture_settings(captured: &Captured, settings: &TaskSettings) -> TaskSettings {
    let mut settings = settings.clone();
    if settings
        .after_capture_job
        .has(AfterCaptureTasks::AnnotateImage)
        && captured.modified
    {
        settings.after_capture_job = settings
            .after_capture_job
            .remove(AfterCaptureTasks::AnnotateImage);
    }
    let region_like = matches!(
        captured.kind,
        CaptureKind::Region(_) | CaptureKind::LastRegion
    );
    if settings.image_settings.image_effect_only_region_capture && !region_like {
        settings.after_capture_job = settings
            .after_capture_job
            .remove(AfterCaptureTasks::AddImageEffects);
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use source::FakeSource;

    fn picture(step: CaptureStep) -> Captured {
        match step {
            CaptureStep::Done(captured) => captured,
            CaptureStep::Region(_) => panic!("asked for the overlay"),
            CaptureStep::Message(m) => panic!("message: {m}"),
            CaptureStep::Nothing(e) => panic!("nothing: {e:?}"),
        }
    }

    #[test]
    fn every_simple_kind_takes_its_area() {
        let source = FakeSource::sample();
        let settings = TaskSettings::default();
        let full = picture(capture(CaptureKind::Fullscreen, &settings, &source));
        assert_eq!(full.metadata.image.unwrap().dimensions(), (2720, 900));
        source.set_cursor(1500, 300);
        let monitor = picture(capture(CaptureKind::ActiveMonitor, &settings, &source));
        assert_eq!(monitor.metadata.image.unwrap().dimensions(), (1280, 720));
        let second = picture(capture(
            CaptureKind::Monitor(IRect::new(0, 0, 1440, 900)),
            &settings,
            &source,
        ));
        assert_eq!(second.metadata.image.unwrap().dimensions(), (1440, 900));
        let window = picture(capture(CaptureKind::ActiveWindow, &settings, &source));
        assert_eq!(window.metadata.window_title.as_deref(), Some("Notes"));
        assert_eq!(window.metadata.process_name.as_deref(), Some("Notes"));
        assert_eq!(window.metadata.image.unwrap().dimensions(), (640, 420));
    }

    #[test]
    fn windows_are_taken_by_themselves_on_windows_only_when_transparent_on_macos() {
        let source = FakeSource::sample();
        let mut settings = TaskSettings::default();
        settings.capture_settings.capture_transparent = false;
        settings.capture_settings.capture_client_area = true;
        picture(capture(CaptureKind::ActiveWindow, &settings, &source));
        let taken = source.windows_taken.lock().unwrap().clone();
        if cfg!(windows) {
            // The platform reads the rectangle again and keeps to the client area.
            assert_eq!(taken.len(), 1);
            assert_eq!(taken[0].0, "Notes");
            assert!(taken[0].1.client_area && !taken[0].1.transparent);
        } else {
            assert!(taken.is_empty(), "the area it covers: {taken:?}");
        }
        settings.capture_settings.capture_client_area = false;
        settings.capture_settings.capture_transparent = true;
        picture(capture(CaptureKind::ActiveWindow, &settings, &source));
        let taken = source.windows_taken.lock().unwrap().clone();
        let last = taken.last().unwrap();
        assert!(last.1.transparent && !last.1.client_area);
    }

    #[test]
    fn custom_region_and_window_come_from_the_settings() {
        let source = FakeSource::sample();
        let mut settings = TaskSettings::default();
        settings.capture_settings.capture_custom_region = IRect::new(10, 20, 30, 40);
        let custom = picture(capture(CaptureKind::CustomRegion, &settings, &source));
        assert_eq!(custom.metadata.image.unwrap().dimensions(), (30, 40));
        settings.capture_settings.capture_custom_window = "termin".into();
        let window = picture(capture(CaptureKind::CustomWindow, &settings, &source));
        assert_eq!(window.metadata.window_title.as_deref(), Some("Terminal"));
        settings.capture_settings.capture_custom_window = "Nope".into();
        assert!(matches!(
            capture(CaptureKind::CustomWindow, &settings, &source),
            CaptureStep::Message(_)
        ));
    }

    #[test]
    fn last_region_asks_first_then_repeats() {
        set_last_region(LastRegion::default());
        let source = FakeSource::sample();
        let settings = TaskSettings::default();
        let CaptureStep::Region(request) = capture(CaptureKind::LastRegion, &settings, &source)
        else {
            panic!("no overlay");
        };
        assert_eq!(request.kind, RegionKind::Default);
        assert_eq!(request.windows.len(), 3);
        let outcome = RegionOutcome::Completed {
            image_rect: IRect::new(100, 100, 50, 50),
            screen_rect: IRect::new(100, 100, 50, 50),
            window: None,
        };
        let captured = finish_region(&request, &outcome, &source, false).unwrap();
        assert_eq!(captured.metadata.image.unwrap().dimensions(), (50, 50));
        assert_eq!(last_region().rect, IRect::new(100, 100, 50, 50));
        let again = picture(capture(CaptureKind::LastRegion, &settings, &source));
        assert_eq!(again.metadata.image.unwrap().dimensions(), (50, 50));
        set_last_region(LastRegion::default());
    }

    #[test]
    fn annotation_is_dropped_after_an_annotated_capture() {
        let mut settings = TaskSettings::default();
        settings.after_capture_job = settings
            .after_capture_job
            .with(AfterCaptureTasks::AnnotateImage)
            .with(AfterCaptureTasks::AddImageEffects);
        settings.image_settings.image_effect_only_region_capture = true;
        let captured = Captured {
            metadata: TaskMetadata::default(),
            modified: true,
            kind: CaptureKind::Fullscreen,
        };
        let after = after_capture_settings(&captured, &settings);
        assert!(
            !after
                .after_capture_job
                .has(AfterCaptureTasks::AnnotateImage)
        );
        assert!(
            !after
                .after_capture_job
                .has(AfterCaptureTasks::AddImageEffects)
        );
        assert!(
            after
                .after_capture_job
                .has(AfterCaptureTasks::UploadImageToHost)
        );
    }
}

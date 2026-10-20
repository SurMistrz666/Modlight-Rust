//! Pictures the tools take of the screen: `RegionCaptureTasks.GetRegionImage` (the
//! region overlay, its answer back to a tool instead of a task) and
//! `Screenshot.CaptureFullscreen`. Both go through the capture source, so QA runs read the
//! fake desktop and only the real app asks ScreenCaptureKit.

use std::sync::Arc;

use image::RgbaImage;
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::region::{RegionKind, RegionSession};
use modlight_core::capture::source::CaptureError;
use modlight_core::capture::{self, CaptureKind, CaptureStep};
use modlight_core::l10n::{s, t};
use modlight_core::tasks::host::MessageIcon;
use modlight_core::tasks::settings::TaskSettings;

use crate::region_overlay::{OverlayPurpose, RegionOverlay};
use crate::state::AppState;

/// Who asked for a picture of the screen, and gets it back.
#[derive(Clone, Debug, PartialEq)]
pub enum RegionFor {
    /// The QR code window's "Scan screen" / "Scan region".
    QrScan(u64),
    /// The pin startup window's "Select region".
    PinStartup(u64),
    /// `PinToScreenFromScreen`.
    PinFromScreen(Box<TaskSettings>),
    /// `OCRImage(taskSettings)`.
    Ocr(Box<TaskSettings>),
    /// The OCR window's "Select region".
    OcrWindow(u64),
    /// The analyze image window's "Select region" (and its automatic start).
    AnalyzeWindow(u64),
}

/// A picture of the screen and where it was (desktop points).
pub type ScreenPicture = Option<(Arc<RgbaImage>, IRect)>;

impl AppState {
    /// `RegionCaptureTasks.GetRegionImage`: the overlay; the area comes back to `target`.
    pub fn tool_region(&mut self, target: RegionFor, settings: TaskSettings) {
        if self.captures.overlay.is_some() || self.captures.busy {
            return;
        }
        self.captures.busy = true;
        let source = self.captures.source.clone();
        self.rt.spawn_blocking(move || {
            let step = capture::capture(
                CaptureKind::Region(RegionKind::Default),
                &settings,
                source.as_ref(),
            );
            move |state: &mut AppState, _ctx: &egui::Context| {
                state.captures.busy = false;
                match step {
                    CaptureStep::Region(request) => {
                        let mut session = RegionSession::new(request);
                        session.render_scale = session.request.scale;
                        state.captures.overlay = Some(RegionOverlay::new(
                            session,
                            OverlayPurpose::Tool(target),
                            settings,
                            state.time,
                        ));
                    }
                    CaptureStep::Done(captured) => {
                        let picture = captured.metadata.image.map(|i| {
                            let rect = IRect::new(0, 0, i.width() as i32, i.height() as i32);
                            (i, rect)
                        });
                        state.tool_picture(target, picture);
                    }
                    CaptureStep::Message(_) => state.tool_picture(target, None),
                    CaptureStep::Nothing(error) => {
                        state.tool_capture_refused(error);
                        state.tool_picture(target, None);
                    }
                }
            }
        });
    }

    /// `new Screenshot().CaptureFullscreen()` for a tool.
    pub fn tool_fullscreen(&mut self, target: RegionFor, settings: TaskSettings) {
        let source = self.captures.source.clone();
        self.rt.spawn_blocking(move || {
            let bounds = source.screen_bounds();
            let step = capture::capture(CaptureKind::Fullscreen, &settings, source.as_ref());
            move |state: &mut AppState, _ctx: &egui::Context| match step {
                CaptureStep::Done(captured) => {
                    let picture = captured.metadata.image.map(|i| (i, bounds));
                    state.tool_picture(target, picture);
                }
                CaptureStep::Nothing(error) => {
                    state.tool_capture_refused(error);
                    state.tool_picture(target, None);
                }
                _ => state.tool_picture(target, None),
            }
        });
    }

    /// macOS refused the pixels: said out loud, as the captures do.
    fn tool_capture_refused(&mut self, error: Option<CaptureError>) {
        if error == Some(CaptureError::NotPermitted) {
            self.uploads
                .windows
                .messages
                .push_back(crate::dialogs::message::MessageBox::new(
                    &format!("Modlight - {}", t(s::Error)),
                    &CaptureError::NotPermitted.to_string(),
                    MessageIcon::Warning,
                ));
        }
    }

    /// The overlay closed (`capture_flow` hands the finished area over).
    pub fn tool_region_closed(&mut self, target: RegionFor, picture: ScreenPicture) {
        self.tool_picture(target, picture);
    }

    /// A picture (or none) reaches the tool that asked.
    pub fn tool_picture(&mut self, target: RegionFor, picture: ScreenPicture) {
        match target {
            RegionFor::QrScan(key) => super::qr::scanned_picture(self, key, picture),
            RegionFor::PinStartup(key) => self.pin_region_done(Some(key), None, picture),
            RegionFor::PinFromScreen(settings) => {
                self.pin_region_done(None, Some(*settings), picture)
            }
            RegionFor::Ocr(settings) => self.ocr_region_done(None, Some(*settings), picture),
            RegionFor::OcrWindow(key) => self.ocr_region_done(Some(key), None, picture),
            RegionFor::AnalyzeWindow(key) => self.analyze_region_done(key, picture),
        }
    }
}

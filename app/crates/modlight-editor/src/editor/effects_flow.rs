//! `EditorView.EffectsHost` and `MainViewModel.EffectPreview`: running an effect from the
//! browser, the dialog's live preview, applying (one undoable step) and cancelling, and
//! the Edit operations that go through the core.

use std::sync::Arc;

use crate::color::Color;
use crate::effects::{
    self, DialogState, EffectContext, ExecutionMode, Operation, ParamValue, PreviewState,
};
use crate::ui::icons;

use super::{Editor, SidePanel, toolbar};

impl Editor {
    /// The effect browser's list of recent effects (newest first).
    pub fn recent_effects(&self) -> Vec<String> {
        effects::resolve_ids(&effects::registry(), &self.options.recent_effects)
    }

    pub fn favorite_effects(&self) -> Vec<String> {
        effects::resolve_ids(&effects::registry(), &self.options.favorite_effects)
    }

    /// `RegisterRecentEffect`.
    fn register_recent(&mut self, id: &str) {
        let mut list = self.recent_effects();
        list.retain(|e| !e.eq_ignore_ascii_case(id));
        list.insert(0, id.to_string());
        list.truncate(effects::MAX_RECENT_EFFECTS);
        self.options.recent_effects = list;
    }

    /// Right click on an effect: off the recent list, or in/out of the favourites.
    pub fn effect_right_clicked(&mut self, id: &str, in_recent: bool) {
        if in_recent {
            let mut list = self.recent_effects();
            list.retain(|e| !e.eq_ignore_ascii_case(id));
            self.options.recent_effects = list;
            return;
        }
        let mut favorites = self.favorite_effects();
        if favorites.iter().any(|f| f.eq_ignore_ascii_case(id)) {
            favorites.retain(|f| !f.eq_ignore_ascii_case(id));
        } else {
            favorites.push(id.to_string());
        }
        self.options.favorite_effects = favorites;
    }

    /// An effect chosen in the browser (`EffectItem.Execute` → `OnEffectDialogRequested`).
    pub fn run_effect(&mut self, id: &str) {
        let registry = effects::registry();
        let Some(effect) = registry.get(id) else {
            return;
        };
        self.register_recent(effect.id());
        if let Some(op) = effects::operation_for(effect.id()) {
            if effect.parameters().is_empty() {
                self.run_operation(op, &effects::ParamValues::default());
                self.close_effects_panel();
            } else {
                self.open_effect_dialog(effect.as_ref(), Some(op));
            }
            return;
        }
        if effect.execution_mode() == ExecutionMode::Immediate {
            if !self.has_image() {
                return;
            }
            self.start_effect_preview();
            let status = format!("Applied {}", effect.name());
            let applied = self.apply_effect_with(|img| {
                effect.apply(
                    img,
                    &effects::ParamValues::default(),
                    &EffectContext::default(),
                )
            });
            if applied {
                self.effect_applied_notification(&status);
            }
            self.close_effects_panel();
            return;
        }
        self.open_effect_dialog(effect.as_ref(), None);
    }

    fn open_effect_dialog(&mut self, effect: &dyn effects::ImageEffect, op: Option<Operation>) {
        let mut dialog = DialogState::new(effect);
        if matches!(op, Some(Operation::CropImage | Operation::ResizeImage)) {
            let (w, h) = self.image_size();
            // Runtime defaults: the image's size (set before the dialog shows, so they do
            // not count as edits for the resize anchor).
            dialog.values.set("width", ParamValue::Number(w as f64));
            dialog.values.set("height", ParamValue::Number(h as f64));
        }
        self.start_effect_preview();
        self.effects_panel = Some(SidePanel::Effect(dialog));
        self.is_effects_panel_open = true;
        toolbar::update_active_states(self);
        self.preview_effect_dialog();
    }

    /// `StartEffectPreview`.
    pub fn start_effect_preview(&mut self) {
        if let Some(source) = self.core.source().cloned() {
            self.preview = Some(PreviewState {
                original: source,
                latest: None,
            });
            self.invalidate();
        }
    }

    /// The dialog's values changed (`RequestPreview` → `PreviewEffect`).
    pub fn preview_effect_dialog(&mut self) {
        let Some(SidePanel::Effect(dialog)) = &self.effects_panel else {
            return;
        };
        let Some(preview) = &self.preview else {
            return;
        };
        let Some(effect) = effects::registry().get(&dialog.effect_id) else {
            return;
        };
        // The resize the core would refuse is not previewed either: with the aspect
        // ratio kept, one box typed in can ask for a side in the millions, whichever
        // library implements the effect.
        if effects::operation_for(effect.id()) == Some(Operation::ResizeImage) {
            let (w, h) =
                effects::builtin::resize_target(preview.original.dimensions(), &dialog.values);
            if !modlight_core::image_decode::fits(w as u64, h as u64) {
                log::warn!("Modlight: the editor did not preview a resize to {w}x{h}");
                return;
            }
        }
        let context = EffectContext { is_preview: true };
        match effect.apply(&preview.original, &dialog.values, &context) {
            // An empty picture (a crop outside the image) is no preview: the canvas
            // cannot show it (a texture of nothing is a GPU validation error) and Apply
            // refuses it anyway.
            Ok(result) if result.width() == 0 || result.height() == 0 => {
                log::warn!("effect preview failed: the effect gave an empty picture");
            }
            Ok(result) => {
                if let Some(p) = &mut self.preview {
                    p.latest = Some(Arc::new(result));
                }
            }
            Err(message) => log::warn!("effect preview failed: {message}"),
        }
        self.invalidate();
    }

    /// Changes one value of the open dialog and refreshes the preview.
    pub fn set_effect_value(&mut self, key: &str, value: ParamValue) {
        if let Some(SidePanel::Effect(dialog)) = &mut self.effects_panel {
            dialog.set(key, value);
        }
        self.preview_effect_dialog();
    }

    /// The dialog's Apply.
    pub fn apply_effect_dialog(&mut self) {
        let Some(SidePanel::Effect(dialog)) = self.effects_panel.clone() else {
            return;
        };
        let Some(effect) = effects::registry().get(&dialog.effect_id) else {
            return;
        };
        if let Some(op) = effects::operation_for(effect.id()) {
            // `RestorePreEffectSourceImage`, the operation through the core, `EndEffectPreview`.
            self.preview = None;
            self.run_operation(op, &dialog.values);
            self.close_effects_panel();
            return;
        }
        let status = format!("Applied {}", effect.name());
        let values = dialog.values.clone();
        let applied =
            self.apply_effect_with(|img| effect.apply(img, &values, &EffectContext::default()));
        if applied {
            self.effect_applied_notification(&status);
        }
        self.close_effects_panel();
    }

    /// `ApplyEffect(Func)`: the latest preview, or the effect on the original, becomes
    /// the image in one undoable step.
    fn apply_effect_with(
        &mut self,
        effect: impl FnOnce(&image::RgbaImage) -> Result<image::RgbaImage, String>,
    ) -> bool {
        let Some(preview) = self.preview.take() else {
            return false;
        };
        let result = match preview.latest {
            Some(latest) => (*latest).clone(),
            None => match effect(&preview.original) {
                Ok(r) => r,
                Err(message) => {
                    log::warn!("effect failed: {message}");
                    self.invalidate();
                    return false;
                }
            },
        };
        let applied = self
            .core
            .apply_image_operation(move |_| Some(result), false, None);
        self.pump();
        applied
    }

    /// `CancelEffectPreview`.
    pub fn cancel_effect_preview(&mut self) {
        if self.preview.take().is_some() {
            self.invalidate();
        }
    }

    /// The image the view shows: the live preview while an effect dialog is open.
    pub fn display_image(&self) -> Option<Arc<image::RgbaImage>> {
        if let Some(p) = &self.preview {
            return Some(p.latest.clone().unwrap_or_else(|| p.original.clone()));
        }
        self.core.source().cloned()
    }

    fn effect_applied_notification(&mut self, status: &str) {
        let message = if status.trim().is_empty() {
            modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_ImageEffectApplied)
                .to_string()
        } else if status.ends_with(['.', '!', '?']) {
            status.to_string()
        } else {
            format!("{status}.")
        };
        self.show_notification(&message, Some(icons::PANEL_EFFECTS));
    }

    /// The Edit operations (`EditorOperationKind`) with their notifications.
    pub fn run_operation(&mut self, op: Operation, v: &effects::ParamValues) {
        use modlight_core::l10n::{editor as s, t, tf};
        let ok;
        let (message, icon) = match op {
            Operation::AutoCropImage => {
                ok = self.core.auto_crop(v.int("tolerance"));
                (
                    t(s::MainViewModel_ImageAutoCropped).to_string(),
                    icons::TOOL_CROP,
                )
            }
            Operation::CropImage => {
                let (x, y, w, h) = (v.int("x"), v.int("y"), v.int("width"), v.int("height"));
                ok = w > 0
                    && h > 0
                    && self.core.crop(crate::geom::Rect::from_xywh(
                        x as f32, y as f32, w as f32, h as f32,
                    ));
                if ok {
                    self.pump();
                    self.image_cropped_notification();
                }
                return;
            }
            Operation::ResizeImage => {
                let (w, h) = effects::builtin::resize_target(self.image_size(), v);
                ok = self.core.resize_image(w, h);
                (
                    t(s::MainViewModel_ImageResized).to_string(),
                    icons::TOOL_IMAGE,
                )
            }
            Operation::ResizeCanvas => {
                let color: Color = v.color("background_color");
                ok = self.core.resize_canvas(
                    v.int("top"),
                    v.int("right"),
                    v.int("bottom"),
                    v.int("left"),
                    color,
                );
                (
                    t(s::MainViewModel_CanvasResized).to_string(),
                    icons::PANEL_BACKGROUND,
                )
            }
            Operation::Rotate90Clockwise => {
                ok = self.core.rotate_90_clockwise();
                (
                    t(s::MainViewModel_ImageRotated90Clockwise).to_string(),
                    icons::ACTION_ROTATE_RIGHT,
                )
            }
            Operation::Rotate90CounterClockwise => {
                ok = self.core.rotate_90_counter_clockwise();
                (
                    t(s::MainViewModel_ImageRotated90CounterClockwise).to_string(),
                    icons::ACTION_ROTATE_LEFT,
                )
            }
            Operation::Rotate180 => {
                ok = self.core.rotate_180();
                (
                    t(s::MainViewModel_ImageRotated180).to_string(),
                    icons::ACTION_ROTATE_RIGHT,
                )
            }
            Operation::RotateCustomAngle => {
                let angle = v.number("angle").clamp(-360.0, 360.0) as f32;
                ok = self
                    .core
                    .rotate_custom_angle(angle, v.boolean("auto_resize"));
                let rounded = (angle as f64 * 100.0).round() / 100.0;
                let formatted = format_angle(rounded);
                (
                    tf(s::MainViewModel_ImageRotatedByDegrees, &[formatted.into()]),
                    icons::ACTION_ROTATE_RIGHT,
                )
            }
            Operation::FlipHorizontal => {
                ok = self.core.flip_horizontal();
                (
                    t(s::MainViewModel_ImageFlippedHorizontally).to_string(),
                    icons::PANEL_EFFECTS,
                )
            }
            Operation::FlipVertical => {
                ok = self.core.flip_vertical();
                (
                    t(s::MainViewModel_ImageFlippedVertically).to_string(),
                    icons::PANEL_EFFECTS,
                )
            }
        };
        self.pump();
        if ok {
            self.show_notification(&message, Some(icon));
        }
    }
}

/// `ToString("0.##", InvariantCulture)`.
fn format_angle(value: f64) -> String {
    let s = format!("{value:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Editor, EditorConfig};
    use crate::options::ImageEditorOptions;
    use image::{Rgba, RgbaImage};

    fn editor() -> Editor {
        let mut e = Editor::new(ImageEditorOptions::default(), EditorConfig::default());
        e.load_image(
            RgbaImage::from_pixel(40, 20, Rgba([200, 100, 50, 255])),
            None,
        );
        e
    }

    #[test]
    fn immediate_effect_applies_in_one_undo_step() {
        let mut e = editor();
        e.run_effect("invert");
        assert_eq!(
            e.core.source().unwrap().get_pixel(0, 0).0,
            [55, 155, 205, 255]
        );
        assert_eq!(e.recent_effects()[0], "invert");
        e.undo();
        assert_eq!(
            e.core.source().unwrap().get_pixel(0, 0).0,
            [200, 100, 50, 255]
        );
    }

    #[test]
    fn dialog_previews_then_applies_or_cancels() {
        let mut e = editor();
        e.run_effect("brightness");
        assert!(e.preview.is_some());
        e.set_effect_value("amount", ParamValue::Number(20.0));
        let shown = e.display_image().unwrap();
        assert_eq!(shown.get_pixel(0, 0).0[0], 251);
        assert_eq!(e.core.source().unwrap().get_pixel(0, 0).0[0], 200);
        e.cancel_effect_preview();
        e.close_effects_panel();
        assert_eq!(e.display_image().unwrap().get_pixel(0, 0).0[0], 200);
        e.run_effect("brightness");
        e.set_effect_value("amount", ParamValue::Number(20.0));
        e.apply_effect_dialog();
        assert_eq!(e.core.source().unwrap().get_pixel(0, 0).0[0], 251);
        assert!(e.preview.is_none());
        assert!(!e.is_effects_panel_open);
    }

    /// Resize image with "Maintain aspect ratio" works out the other side from the box
    /// typed in: on a long narrow picture (a scrolling capture) a large width gives a
    /// height in the millions. The preview and Apply went ahead and crashed (an overflow
    /// in the resampler, or the allocation); now the picture stays as it is.
    #[test]
    fn a_resize_far_past_the_picture_limits_is_refused() {
        let mut e = Editor::new(ImageEditorOptions::default(), EditorConfig::default());
        e.load_image(RgbaImage::from_pixel(10, 4000, Rgba([1, 2, 3, 255])), None);
        e.run_effect("resize_image");
        e.set_effect_value("maintain_aspect_ratio", ParamValue::Bool(true));
        e.set_effect_value("width", ParamValue::Number(10000.0));
        e.apply_effect_dialog();
        assert_eq!(e.image_size(), (10, 4000));
        assert!(!e.core.can_undo());
        // A size within the limits still resizes, keeping the ratio.
        e.run_effect("resize_image");
        e.set_effect_value("maintain_aspect_ratio", ParamValue::Bool(true));
        e.set_effect_value("width", ParamValue::Number(20.0));
        e.apply_effect_dialog();
        assert_eq!(e.image_size(), (20, 8000));
    }

    /// An effect over a band of the picture stays over that band when the picture is
    /// turned. Upstream (and the port) turned the band's corners, which swaps its width
    /// and height, and then turned the band by the angle as well: a blur over a line of
    /// text ended up across it, the text in the clear.
    #[test]
    fn an_effect_stays_on_its_band_when_the_image_turns() {
        let mut e = Editor::new(ImageEditorOptions::default(), EditorConfig::default());
        e.load_image(
            RgbaImage::from_pixel(100, 60, Rgba([255, 255, 255, 255])),
            None,
        );
        let mut band = crate::model::Annotation::effect(crate::model::EffectKind::Highlight);
        band.start = crate::geom::Point::new(10.0, 10.0);
        band.end = crate::geom::Point::new(90.0, 20.0);
        let id = band.id;
        e.core.add_annotation(band);
        e.refresh_effect(id);
        e.pump();
        let yellow = |img: &RgbaImage, x: u32, y: u32| img.get_pixel(x, y).0 == [255, 255, 0, 255];
        let before = e.snapshot().unwrap();
        assert!(yellow(&before, 50, 15) && !yellow(&before, 50, 40));

        e.run_operation(
            Operation::Rotate90Clockwise,
            &effects::ParamValues::default(),
        );
        // The band now runs down the picture: x 40 to 50, y 10 to 90.
        let after = e.snapshot().unwrap();
        assert_eq!(after.dimensions(), (60, 100));
        for (x, y) in [(45, 15), (45, 50), (45, 85)] {
            assert!(yellow(&after, x, y), "({x}, {y}) is not highlighted");
        }
        for (x, y) in [(15, 50), (30, 50), (55, 50)] {
            assert!(!yellow(&after, x, y), "({x}, {y}) is highlighted");
        }

        // The same with a custom angle of 90 (the arbitrary rotation's path).
        e.undo();
        e.run_operation(Operation::RotateCustomAngle, &{
            let mut v = effects::ParamValues::defaults(
                &effects::registry()
                    .get("rotate_custom_angle")
                    .unwrap()
                    .parameters(),
            );
            v.set("angle", ParamValue::Number(90.0));
            v
        });
        let turned = e.snapshot().unwrap();
        assert_eq!(turned.dimensions(), (60, 100));
        assert!(yellow(&turned, 45, 85) && !yellow(&turned, 15, 50));
    }

    #[test]
    fn operations_go_through_the_core() {
        let mut e = editor();
        e.run_effect("rotate_90_clockwise");
        assert_eq!(e.image_size(), (20, 40));
        e.run_effect("resize_image");
        e.set_effect_value("width", ParamValue::Number(10.0));
        e.apply_effect_dialog();
        assert_eq!(e.image_size(), (10, 40));
        assert_eq!(format_angle(45.0), "45");
        assert_eq!(format_angle(12.345), "12.35");
    }
}

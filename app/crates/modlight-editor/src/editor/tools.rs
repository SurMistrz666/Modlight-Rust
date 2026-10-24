//! `MainViewModel.ToolOptions`: the option values the toolbar shows, which tool's
//! defaults each one writes back to (`Options.*`), which controls are visible for the
//! active tool or the selected annotation, and `EditorView.ToolbarHandlers`' `Apply*`
//! that pushes a change onto the selected annotation.

use crate::color::Color;
use crate::model::{
    Annotation, ArrowStyle, BorderStyle, CursorType, EditorTool, EffectKind, Kind, StepType,
    TextAlign,
};

use super::Editor;

pub const MIN_EFFECT_STRENGTH: f32 = 1.0;
pub const MAX_BLUR_STRENGTH: f32 = 200.0;
pub const MAX_PIXELATE_STRENGTH: f32 = 200.0;
pub const MAX_MAGNIFY_STRENGTH: f32 = 10.0;
pub const MAX_SPOTLIGHT_STRENGTH: f32 = 100.0;

/// `GetMaxEffectStrength`.
pub fn max_effect_strength(tool: EditorTool) -> f32 {
    match tool {
        EditorTool::Blur => MAX_BLUR_STRENGTH,
        EditorTool::Pixelate => MAX_PIXELATE_STRENGTH,
        EditorTool::Magnify => MAX_MAGNIFY_STRENGTH,
        EditorTool::Spotlight => MAX_SPOTLIGHT_STRENGTH,
        _ => 30.0,
    }
}

/// The option values (`[ObservableProperty]`s of the tool options).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolState {
    pub active_tool: EditorTool,
    pub selected_color: String,
    pub stroke_width: i32,
    pub corner_radius: i32,
    pub fill_color: String,
    pub text_color: String,
    pub font_size: f32,
    pub step_start_number: i32,
    pub step_type: StepType,
    pub text_align: TextAlign,
    pub font_family: String,
    pub border_style: BorderStyle,
    pub arrow_style: ArrowStyle,
    pub cursor_type: CursorType,
    pub effect_strength: f32,
    pub spotlight_blur: f32,
    pub effect_ellipse: bool,
    pub shadow_enabled: bool,
    pub shadow_color: String,
    pub shadow_blur_radius: f64,
    pub shadow_opacity: f64,
    pub shadow_offset_x: f64,
    pub shadow_offset_y: f64,
    pub speech_balloon_tail: bool,
    pub text_bold: bool,
    pub text_italic: bool,
    pub number_counter: i32,
}

impl Default for ToolState {
    fn default() -> Self {
        ToolState {
            active_tool: EditorTool::Rectangle,
            selected_color: "#EF4444".into(),
            stroke_width: 4,
            corner_radius: 4,
            fill_color: "#00000000".into(),
            text_color: "#FF000000".into(),
            font_size: 30.0,
            step_start_number: 1,
            step_type: StepType::Numeric,
            text_align: TextAlign::Center,
            font_family: "Segoe UI".into(),
            border_style: BorderStyle::Solid,
            arrow_style: ArrowStyle::Classic,
            cursor_type: CursorType::Default,
            effect_strength: 1.0,
            spotlight_blur: 0.0,
            effect_ellipse: false,
            shadow_enabled: true,
            shadow_color: crate::model::DEFAULT_SHADOW_COLOR.into(),
            shadow_blur_radius: crate::model::DEFAULT_SHADOW_BLUR_RADIUS,
            shadow_opacity: crate::model::DEFAULT_SHADOW_OPACITY,
            shadow_offset_x: crate::model::DEFAULT_SHADOW_OFFSET_X,
            shadow_offset_y: crate::model::DEFAULT_SHADOW_OFFSET_Y,
            speech_balloon_tail: true,
            text_bold: true,
            text_italic: false,
            number_counter: 1,
        }
    }
}

fn argb(c: Color) -> String {
    c.to_hex_argb()
}

/// `IsRememberableAnnotationTool`.
pub fn is_rememberable(tool: EditorTool) -> bool {
    matches!(
        tool,
        EditorTool::Rectangle
            | EditorTool::Ellipse
            | EditorTool::Line
            | EditorTool::Arrow
            | EditorTool::Freehand
            | EditorTool::Text
            | EditorTool::SpeechBalloon
            | EditorTool::Step
            | EditorTool::Cursor
            | EditorTool::Highlight
            | EditorTool::SmartEraser
            | EditorTool::Blur
            | EditorTool::Pixelate
            | EditorTool::Magnify
            | EditorTool::Spotlight
    )
}

/// Which option controls the toolbar shows (`Show*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Visibility {
    pub border_color: bool,
    pub fill_color: bool,
    pub text_color: bool,
    pub thickness: bool,
    pub font_size: bool,
    pub step_start_number: bool,
    pub step_type: bool,
    pub text_align: bool,
    pub font_family: bool,
    pub border_style: bool,
    pub arrow_style: bool,
    pub cursor_type: bool,
    pub corner_radius: bool,
    pub strength: bool,
    pub spotlight_blur: bool,
    pub effect_ellipse: bool,
    pub shadow: bool,
    pub speech_balloon_tail: bool,
    pub text_style: bool,
    pub text_italic: bool,
}

impl Visibility {
    /// `ShowToolOptionsSeparator`: anything at all.
    pub fn any(&self) -> bool {
        self.border_color
            || self.fill_color
            || self.text_color
            || self.thickness
            || self.font_size
            || self.step_start_number
            || self.step_type
            || self.text_align
            || self.font_family
            || self.border_style
            || self.arrow_style
            || self.cursor_type
            || self.corner_radius
            || self.strength
            || self.spotlight_blur
            || self.effect_ellipse
            || self.text_style
            || self.shadow
            || self.speech_balloon_tail
    }
}

impl Editor {
    /// The annotation the Select tool shows options for (`SelectedAnnotation`).
    pub fn vm_selected_annotation(&self) -> Option<&Annotation> {
        self.vm_selected.and_then(|id| self.annotation(id))
    }

    fn selected_tool_type(&self) -> Option<EditorTool> {
        self.vm_selected_annotation().map(|a| a.tool)
    }

    /// The tool whose options are shown: the active one, or in Select mode the tool
    /// that made the selected annotation.
    pub fn effective_tool(&self) -> EditorTool {
        let tool = self.tools.active_tool;
        if tool == EditorTool::Select
            && let Some(t) = self.selected_tool_type()
        {
            return t;
        }
        tool
    }

    pub fn visibility(&self) -> Visibility {
        use EditorTool::*;
        let active = self.tools.active_tool;
        let sel = self.vm_selected_annotation();
        let sel_tool = sel.map(|a| a.tool);
        let either = |set: &[EditorTool]| {
            if active == Select {
                sel_tool.is_some_and(|t| set.contains(&t))
            } else {
                set.contains(&active)
            }
        };
        let sel_kind = |f: &dyn Fn(&Annotation) -> bool| sel.is_some_and(f);
        Visibility {
            border_color: either(&[
                Rectangle,
                Ellipse,
                Line,
                Arrow,
                Freehand,
                SpeechBalloon,
                Text,
                Step,
            ]),
            fill_color: either(&[Rectangle, Ellipse, SpeechBalloon, Step, Highlight]),
            text_color: either(&[Text, SpeechBalloon, Step]),
            thickness: either(&[
                Rectangle,
                Ellipse,
                Line,
                Arrow,
                Freehand,
                SpeechBalloon,
                Step,
                Text,
            ]),
            font_size: either(&[Text, Step, SpeechBalloon]),
            step_start_number: active == Step,
            step_type: active == Step,
            text_align: either(&[Text, SpeechBalloon]),
            font_family: either(&[Text, SpeechBalloon]),
            border_style: either(&[Rectangle, Ellipse, Line, Freehand]),
            arrow_style: if active == Select {
                sel_kind(&|a| matches!(a.kind, Kind::Arrow { .. }))
            } else {
                active == Arrow
            },
            cursor_type: if active == Select {
                sel_kind(&|a| matches!(a.kind, Kind::Cursor { .. }))
            } else {
                active == Cursor
            },
            corner_radius: either(&[Rectangle, SpeechBalloon]),
            strength: either(&[Blur, Pixelate, Magnify, Spotlight]),
            spotlight_blur: if active == Select {
                sel_kind(&|a| matches!(a.kind, Kind::Spotlight(_)))
            } else {
                active == Spotlight
            },
            effect_ellipse: if active == Select {
                sel_kind(&|a| {
                    a.effect_kind() == Some(EffectKind::Magnify)
                        || matches!(a.kind, Kind::Spotlight(_))
                })
            } else {
                matches!(active, Magnify | Spotlight)
            },
            shadow: either(&[
                Rectangle,
                Ellipse,
                Line,
                Arrow,
                Freehand,
                Text,
                SpeechBalloon,
                Step,
            ]),
            speech_balloon_tail: if active == Select {
                sel_kind(&|a| matches!(a.kind, Kind::SpeechBalloon(_) | Kind::Number(_)))
            } else {
                matches!(active, SpeechBalloon | Step)
            },
            text_style: either(&[Text, SpeechBalloon, Step]),
            text_italic: either(&[Text, SpeechBalloon]),
        }
    }

    /// `EffectStrengthMaximum`.
    pub fn effect_strength_maximum(&self) -> f32 {
        let tool = if self.tools.active_tool == EditorTool::Select {
            self.selected_tool_type().unwrap_or(EditorTool::Select)
        } else {
            self.tools.active_tool
        };
        max_effect_strength(tool)
    }

    // ---- property setters with the view model's write-back and the view's apply ----

    /// `SelectedColor` (`OnSelectedColorChanged` + `ApplySelectedColor`).
    pub fn set_selected_color(&mut self, hex: &str) {
        self.tools.selected_color = hex.to_string();
        let color = Color::parse_or_transparent(hex);
        let write_default =
            !(self.tools.active_tool == EditorTool::Select && self.vm_selected.is_some());
        if write_default {
            match self.tools.active_tool {
                EditorTool::Step => self.options.step_border_color_hex = argb(color),
                EditorTool::SpeechBalloon => {
                    self.options.speech_balloon_border_color_hex = argb(color)
                }
                EditorTool::Text => self.options.text_border_color_hex = argb(color),
                _ => self.options.border_color_hex = argb(color),
            }
        }
        let hex = hex.to_string();
        self.with_selected(|a| a.stroke_color = hex);
    }

    /// `StrokeWidth` (`OnStrokeWidthChanged` + `ApplySelectedStrokeWidth`).
    pub fn set_stroke_width(&mut self, width: i32) {
        self.tools.stroke_width = width;
        match self.tools.active_tool {
            EditorTool::Step => self.options.step_thickness = width,
            EditorTool::SpeechBalloon => self.options.speech_balloon_thickness = width,
            EditorTool::Text => self.options.text_thickness = width,
            EditorTool::Select if self.vm_selected.is_some() => {
                match self.vm_selected_annotation().map(|a| &a.kind) {
                    Some(Kind::Number(_)) => self.options.step_thickness = width,
                    Some(Kind::SpeechBalloon(_)) => self.options.speech_balloon_thickness = width,
                    Some(Kind::Text(_)) => self.options.text_thickness = width,
                    Some(Kind::SmartEraser { .. }) => {}
                    _ => self.options.thickness = width,
                }
            }
            _ => self.options.thickness = width,
        }
        self.with_selected(|a| a.stroke_width = width as f32);
    }

    /// `CornerRadius` (`OnCornerRadiusChanged` + `ApplySelectedCornerRadius`).
    pub fn set_corner_radius(&mut self, value: i32) {
        let clamped = value.max(0);
        self.tools.corner_radius = clamped;
        let mut applies = matches!(
            self.tools.active_tool,
            EditorTool::Rectangle | EditorTool::SpeechBalloon
        );
        if self.tools.active_tool == EditorTool::Select
            && let Some(a) = self.vm_selected_annotation()
        {
            applies = a.is_rectangle() || matches!(a.kind, Kind::SpeechBalloon(_));
        }
        if applies {
            self.options.corner_radius = clamped;
        }
        self.with_selected(|a| match &mut a.kind {
            Kind::Rectangle { corner_radius, .. } => *corner_radius = clamped,
            Kind::SpeechBalloon(b) => b.corner_radius = clamped,
            _ => {}
        });
    }

    /// `FillColor` (`OnFillColorChanged` + `ApplySelectedFillColor`).
    pub fn set_fill_color(&mut self, hex: &str) {
        self.tools.fill_color = hex.to_string();
        let color = argb(Color::parse_or_transparent(hex));
        match self.tools.active_tool {
            EditorTool::Step => self.options.step_fill_color_hex = color,
            EditorTool::SpeechBalloon => self.options.speech_balloon_fill_color_hex = color,
            EditorTool::Highlight => self.options.highlight_fill_color_hex = color,
            _ => self.options.fill_color_hex = color,
        }
        let hex = hex.to_string();
        let mut refresh = false;
        self.with_selected(|a| {
            a.fill_color = hex;
            refresh = a.effect_kind() == Some(EffectKind::Highlight);
        });
        if refresh {
            self.refresh_selected_effect();
        }
    }

    /// `TextColor` (`OnTextColorChanged` + `ApplySelectedTextColor`).
    pub fn set_text_color(&mut self, hex: &str) {
        self.tools.text_color = hex.to_string();
        let color = argb(Color::parse_or_transparent(hex));
        let mut target = self.tools.active_tool;
        if target == EditorTool::Select
            && let Some(t) = self.selected_tool_type()
        {
            target = t;
        }
        match target {
            EditorTool::Step => self.options.step_text_color_hex = color,
            EditorTool::SpeechBalloon => self.options.speech_balloon_text_color_hex = color,
            EditorTool::Text => self.options.text_text_color_hex = color,
            _ => {}
        }
        let hex = hex.to_string();
        self.with_selected(|a| match &mut a.kind {
            Kind::Text(t) => t.text_color = hex,
            Kind::SpeechBalloon(b) => b.text.text_color = hex,
            Kind::Number(n) => n.text_color = hex,
            _ => {}
        });
    }

    /// `FontSize` (`OnFontSizeChanged`); the toolbar also applies it (`OnFontSizeChanged`
    /// of the view).
    pub fn set_font_size(&mut self, size: f32, apply: bool) {
        self.tools.font_size = size;
        let mut is_step = self.tools.active_tool == EditorTool::Step;
        let mut is_balloon = self.tools.active_tool == EditorTool::SpeechBalloon;
        if self.tools.active_tool == EditorTool::Select
            && let Some(a) = self.vm_selected_annotation()
        {
            is_step |= matches!(a.kind, Kind::Number(_));
            is_balloon |= matches!(a.kind, Kind::SpeechBalloon(_));
        }
        if is_step {
            self.options.step_font_size = size;
        } else if is_balloon {
            self.options.speech_balloon_font_size = size;
        } else {
            self.options.text_font_size = size;
        }
        if apply {
            self.with_selected(|a| match &mut a.kind {
                Kind::Text(t) => t.font_size = size,
                Kind::Number(n) => n.font_size = size,
                Kind::SpeechBalloon(b) => b.text.font_size = size,
                _ => {}
            });
        }
    }

    /// `StepStartNumber`: clamped to 1..=10; it restarts the counter.
    pub fn set_step_start_number(&mut self, value: i32) {
        let clamped = value.clamp(1, 10);
        self.tools.step_start_number = clamped;
        self.tools.number_counter = clamped;
        // The view recalculates the counter from the existing steps.
        self.recalculate_number_counter();
    }

    /// `SelectedStepType`: written back, then applied to every step (as the view does).
    pub fn set_step_type(&mut self, step_type: StepType) {
        self.tools.step_type = step_type;
        self.options.step_type = step_type;
        let mut any = false;
        let mut changed = false;
        for a in self.core.annotations_mut() {
            if let Kind::Number(n) = &mut a.kind {
                changed |= n.step_type != step_type;
                n.step_type = step_type;
                any = true;
            }
        }
        if any {
            self.is_dirty |= changed;
            self.invalidate();
        }
    }

    /// `SelectedTextHorizontalAlignment` (+ `ApplySelectedTextHorizontalAlignment`).
    pub fn set_text_align(&mut self, align: TextAlign) {
        self.tools.text_align = align;
        let (is_text, is_balloon) = self.text_target();
        if is_text {
            self.options.text_horizontal_alignment = align;
        } else if is_balloon {
            self.options.speech_balloon_text_horizontal_alignment = align;
        }
        self.with_selected(|a| match &mut a.kind {
            Kind::Text(t) => t.align = align,
            Kind::SpeechBalloon(b) => b.text.align = align,
            _ => {}
        });
    }

    fn text_target(&self) -> (bool, bool) {
        let mut is_text = self.tools.active_tool == EditorTool::Text;
        let mut is_balloon = self.tools.active_tool == EditorTool::SpeechBalloon;
        if self.tools.active_tool == EditorTool::Select
            && let Some(a) = self.vm_selected_annotation()
        {
            is_text = matches!(a.kind, Kind::Text(_));
            is_balloon = matches!(a.kind, Kind::SpeechBalloon(_));
        }
        (is_text, is_balloon)
    }

    /// `SelectedFontFamily` (+ `ApplySelectedFontFamily` from the toolbar).
    pub fn set_font_family(&mut self, family: &str, apply: bool) {
        let family = if family.trim().is_empty() {
            "Segoe UI".to_string()
        } else {
            family.to_string()
        };
        self.tools.font_family = family.clone();
        let (is_text, is_balloon) = self.text_target();
        if is_text {
            self.options.text_font_family = family.clone();
        } else if is_balloon {
            self.options.speech_balloon_font_family = family.clone();
        }
        if apply {
            self.with_selected(|a| match &mut a.kind {
                Kind::Text(t) => t.font_family = family,
                Kind::SpeechBalloon(b) => b.text.font_family = family,
                _ => {}
            });
        }
    }

    /// `SelectedBorderStyle` (+ `ApplySelectedBorderStyle`).
    pub fn set_border_style(&mut self, style: BorderStyle, apply: bool) {
        self.tools.border_style = style;
        let mut supports = matches!(
            self.tools.active_tool,
            EditorTool::Rectangle | EditorTool::Ellipse | EditorTool::Line | EditorTool::Freehand
        );
        if self.tools.active_tool == EditorTool::Select
            && let Some(t) = self.selected_tool_type()
        {
            supports = matches!(
                t,
                EditorTool::Rectangle
                    | EditorTool::Ellipse
                    | EditorTool::Line
                    | EditorTool::Freehand
            );
        }
        if supports {
            self.options.border_style = style;
        }
        if apply {
            self.with_selected(|a| {
                if !a.is_smart_eraser() {
                    a.set_border_style(style)
                }
            });
        }
    }

    /// `SelectedArrowStyle` (+ `ApplySelectedArrowStyle`).
    pub fn set_arrow_style(&mut self, style: ArrowStyle, apply: bool) {
        self.tools.arrow_style = style;
        let mut is_arrow = self.tools.active_tool == EditorTool::Arrow;
        if self.tools.active_tool == EditorTool::Select
            && let Some(a) = self.vm_selected_annotation()
        {
            is_arrow = matches!(a.kind, Kind::Arrow { .. });
        }
        if is_arrow {
            self.options.arrow_style = style;
        }
        if apply {
            self.with_selected(|a| {
                if matches!(a.kind, Kind::Arrow { .. }) {
                    crate::model::set_arrow_style(a, style);
                }
            });
        }
    }

    /// `SelectedCursorType` (+ `ApplySelectedCursorType`).
    pub fn set_cursor_type(&mut self, cursor: CursorType, apply: bool) {
        self.tools.cursor_type = cursor;
        let mut is_cursor = self.tools.active_tool == EditorTool::Cursor;
        if self.tools.active_tool == EditorTool::Select
            && let Some(a) = self.vm_selected_annotation()
        {
            is_cursor = matches!(a.kind, Kind::Cursor { .. });
        }
        if is_cursor {
            self.options.cursor_type = cursor;
        }
        if apply {
            let bitmap = crate::cursor::annotation_bitmap(cursor);
            self.with_selected(|a| {
                if let Kind::Cursor {
                    cursor_type,
                    bitmap: b,
                } = &mut a.kind
                {
                    *cursor_type = cursor;
                    if bitmap.is_some() {
                        *b = bitmap;
                    }
                }
            });
        }
    }

    /// `EffectStrength`: clamped to the tool's range; written back unless the options
    /// are being loaded (+ `ApplySelectedEffectStrength` from the toolbar).
    pub fn set_effect_strength(&mut self, value: f32, apply: bool) {
        let max = self.effect_strength_maximum();
        let clamped = value.clamp(MIN_EFFECT_STRENGTH, max);
        self.tools.effect_strength = clamped;
        if !self.loading_options {
            let tool = if self.tools.active_tool == EditorTool::Select {
                self.selected_tool_type().unwrap_or(EditorTool::Select)
            } else {
                self.tools.active_tool
            };
            match tool {
                EditorTool::Blur => self.options.blur_strength = clamped,
                EditorTool::Pixelate => self.options.pixelate_strength = clamped,
                EditorTool::Magnify => self.options.magnifier_strength = clamped,
                EditorTool::Spotlight => self.options.spotlight_strength = clamped,
                _ => {}
            }
        }
        if apply {
            let mut refresh = false;
            self.with_selected(|a| match &mut a.kind {
                Kind::Effect(e) => {
                    e.amount = clamped;
                    refresh = true;
                }
                Kind::Spotlight(s) => {
                    s.darken_opacity =
                        (clamped / MAX_SPOTLIGHT_STRENGTH * 255.0).clamp(0.0, 255.0) as u8;
                }
                _ => {}
            });
            if refresh {
                self.refresh_selected_effect();
            }
        }
    }

    /// `SpotlightBlur` (+ `ApplySelectedSpotlightBlur`).
    pub fn set_spotlight_blur(&mut self, value: f32, apply: bool) {
        let clamped = value.clamp(0.0, MAX_BLUR_STRENGTH);
        self.tools.spotlight_blur = clamped;
        if !self.loading_options {
            let is_spotlight = self.tools.active_tool == EditorTool::Spotlight
                || (self.tools.active_tool == EditorTool::Select
                    && matches!(
                        self.vm_selected_annotation().map(|a| &a.kind),
                        Some(Kind::Spotlight(_))
                    ));
            if is_spotlight {
                self.options.spotlight_blur = clamped;
            }
        }
        if apply {
            self.with_selected(|a| {
                if let Kind::Spotlight(s) = &mut a.kind {
                    s.blur_amount = clamped;
                }
            });
        }
    }

    /// `EffectEllipse` (+ `ApplySelectedEffectEllipse`).
    pub fn set_effect_ellipse(&mut self, value: bool, apply: bool) {
        self.tools.effect_ellipse = value;
        let mut is_magnify = self.tools.active_tool == EditorTool::Magnify;
        let mut is_spotlight = self.tools.active_tool == EditorTool::Spotlight;
        if self.tools.active_tool == EditorTool::Select
            && let Some(a) = self.vm_selected_annotation()
        {
            is_magnify = a.effect_kind() == Some(EffectKind::Magnify);
            is_spotlight = matches!(a.kind, Kind::Spotlight(_));
        }
        if is_magnify {
            self.options.magnifier_ellipse = value;
        } else if is_spotlight {
            self.options.spotlight_ellipse = value;
        }
        if apply {
            let mut refresh = false;
            self.with_selected(|a| match &mut a.kind {
                Kind::Effect(e) if e.effect == Some(EffectKind::Magnify) => {
                    e.is_ellipse = value;
                    refresh = true;
                }
                Kind::Spotlight(s) => s.is_ellipse = value,
                _ => {}
            });
            if refresh {
                self.refresh_selected_effect();
            }
        }
    }

    /// `ShadowEnabled` (+ `ApplySelectedShadowState`).
    pub fn set_shadow_enabled(&mut self, value: bool, apply: bool) {
        self.tools.shadow_enabled = value;
        self.options.shadow = value;
        if apply {
            self.with_selected(|a| a.shadow.enabled = value);
        }
    }

    /// The shadow popup's settings; each change re-applies them to the selection
    /// (`ShadowSettingsChanged` → `ApplySelectedShadowSettings`).
    pub fn set_shadow_settings(
        &mut self,
        color: &str,
        blur: f64,
        opacity: f64,
        offset_x: f64,
        offset_y: f64,
        apply: bool,
    ) {
        self.tools.shadow_color = color.to_string();
        if let Some(c) = Color::parse(color) {
            self.options.shadow_color_hex = argb(c);
        }
        self.tools.shadow_blur_radius = blur.max(0.0);
        self.options.shadow_blur_radius = self.tools.shadow_blur_radius;
        self.tools.shadow_opacity = opacity.clamp(0.0, 1.0);
        self.options.shadow_opacity = self.tools.shadow_opacity;
        self.tools.shadow_offset_x = offset_x;
        self.options.shadow_offset_x = offset_x;
        self.tools.shadow_offset_y = offset_y;
        self.options.shadow_offset_y = offset_y;
        if apply {
            let o = self.options.clone();
            self.with_selected(|a| {
                a.shadow.color = o.shadow_color_hex.clone();
                a.shadow.blur_radius = o.shadow_blur_radius;
                a.shadow.opacity = o.shadow_opacity;
                a.shadow.offset_x = o.shadow_offset_x;
                a.shadow.offset_y = o.shadow_offset_y;
            });
        }
    }

    /// `SpeechBalloonTail` (+ `ApplySelectedSpeechBalloonTail`).
    pub fn set_speech_balloon_tail(&mut self, value: bool, apply: bool) {
        self.tools.speech_balloon_tail = value;
        let mut balloon = self.tools.active_tool == EditorTool::SpeechBalloon;
        let mut step = self.tools.active_tool == EditorTool::Step;
        if self.tools.active_tool == EditorTool::Select {
            let kind = self.vm_selected_annotation().map(|a| &a.kind);
            balloon = matches!(kind, Some(Kind::SpeechBalloon(_)));
            step = matches!(kind, Some(Kind::Number(_)));
        }
        if balloon {
            self.options.speech_balloon_tail = value;
        } else if step {
            self.options.step_tail = value;
        }
        if apply {
            self.with_selected(|a| match &mut a.kind {
                Kind::SpeechBalloon(b) => b.tail_enabled = value,
                Kind::Number(n) => n.tail_enabled = value,
                _ => {}
            });
        }
    }

    /// `TextBold` (+ `ApplySelectedTextBold`).
    pub fn set_text_bold(&mut self, value: bool, apply: bool) {
        self.tools.text_bold = value;
        let (mut is_text, mut is_balloon) = (
            self.tools.active_tool == EditorTool::Text,
            self.tools.active_tool == EditorTool::SpeechBalloon,
        );
        let mut is_step = self.tools.active_tool == EditorTool::Step;
        if self.tools.active_tool == EditorTool::Select
            && let Some(a) = self.vm_selected_annotation()
        {
            is_text = matches!(a.kind, Kind::Text(_));
            is_balloon = matches!(a.kind, Kind::SpeechBalloon(_));
            is_step = matches!(a.kind, Kind::Number(_));
        }
        if is_text {
            self.options.text_bold = value;
        } else if is_balloon {
            self.options.speech_balloon_text_bold = value;
        } else if is_step {
            self.options.step_text_bold = value;
        }
        if apply {
            self.with_selected(|a| match &mut a.kind {
                Kind::Text(t) => t.bold = value,
                Kind::SpeechBalloon(b) => b.text.bold = value,
                Kind::Number(n) => n.bold = value,
                _ => {}
            });
        }
    }

    /// `TextItalic` (+ `ApplySelectedTextItalic`).
    pub fn set_text_italic(&mut self, value: bool, apply: bool) {
        self.tools.text_italic = value;
        let (is_text, is_balloon) = self.text_target();
        if is_text {
            self.options.text_italic = value;
        } else if is_balloon {
            self.options.speech_balloon_text_italic = value;
        }
        if apply {
            self.with_selected(|a| match &mut a.kind {
                Kind::Text(t) => t.italic = value,
                Kind::SpeechBalloon(b) => b.text.italic = value,
                _ => {}
            });
        }
    }

    /// `LoadOptionsForTool`.
    pub(crate) fn load_options_for_tool(&mut self, tool: EditorTool) {
        let o = self.options.clone();
        self.set_shadow_settings(
            &o.shadow_color_hex,
            o.shadow_blur_radius,
            o.shadow_opacity,
            o.shadow_offset_x,
            o.shadow_offset_y,
            false,
        );
        match tool {
            EditorTool::Rectangle
            | EditorTool::Ellipse
            | EditorTool::Line
            | EditorTool::Freehand => {
                self.set_selected_color(&argb(o.border_color()));
                self.set_fill_color(&argb(o.fill_color()));
                self.set_stroke_width(o.thickness);
                self.set_corner_radius(o.corner_radius);
                self.set_shadow_enabled(o.shadow, false);
                self.set_border_style(o.border_style, false);
                self.set_font_size(o.text_font_size, false);
            }
            EditorTool::Arrow => {
                self.set_selected_color(&argb(o.border_color()));
                self.set_fill_color(&argb(o.fill_color()));
                self.set_stroke_width(o.thickness);
                self.set_corner_radius(o.corner_radius);
                self.set_shadow_enabled(o.shadow, false);
                self.set_font_size(o.text_font_size, false);
                self.set_arrow_style(o.arrow_style, false);
            }
            EditorTool::Cursor => self.set_cursor_type(o.cursor_type, false),
            EditorTool::Text => {
                self.set_selected_color(&argb(o.text_border_color()));
                self.set_text_color(&argb(o.text_text_color()));
                self.set_stroke_width(o.text_thickness);
                self.set_shadow_enabled(o.shadow, false);
                self.set_font_size(o.text_font_size, false);
                self.set_font_family(&o.text_font_family, false);
                self.set_text_align(o.text_horizontal_alignment);
                self.set_text_bold(o.text_bold, false);
                self.set_text_italic(o.text_italic, false);
            }
            EditorTool::SpeechBalloon => {
                self.set_selected_color(&argb(o.speech_balloon_border_color()));
                self.set_fill_color(&argb(o.speech_balloon_fill_color()));
                self.set_text_color(&argb(o.speech_balloon_text_color()));
                self.set_stroke_width(o.speech_balloon_thickness);
                self.set_corner_radius(o.corner_radius);
                self.set_shadow_enabled(o.shadow, false);
                self.set_speech_balloon_tail(o.speech_balloon_tail, false);
                self.set_font_size(o.speech_balloon_font_size, false);
                self.set_font_family(&o.speech_balloon_font_family, false);
                self.set_text_align(o.speech_balloon_text_horizontal_alignment);
                self.set_text_bold(o.speech_balloon_text_bold, false);
                self.set_text_italic(o.speech_balloon_text_italic, false);
            }
            EditorTool::Step => {
                self.set_selected_color(&argb(o.step_border_color()));
                self.set_fill_color(&argb(o.step_fill_color()));
                self.set_text_color(&argb(o.step_text_color()));
                self.set_stroke_width(o.step_thickness);
                self.set_shadow_enabled(o.shadow, false);
                self.set_speech_balloon_tail(o.step_tail, false);
                self.set_font_size(o.step_font_size, false);
                self.set_step_type(o.step_type);
                self.set_text_bold(o.step_text_bold, false);
                self.set_text_italic(o.text_italic, false);
            }
            EditorTool::Highlight => self.set_fill_color(&argb(o.highlight_fill_color())),
            EditorTool::Blur => self.set_effect_strength(o.blur_strength, false),
            EditorTool::Pixelate => self.set_effect_strength(o.pixelate_strength, false),
            EditorTool::Magnify => {
                self.set_effect_strength(o.magnifier_strength, false);
                self.set_effect_ellipse(o.magnifier_ellipse, false);
            }
            EditorTool::Spotlight => {
                self.set_effect_strength(o.spotlight_strength, false);
                self.set_spotlight_blur(o.spotlight_blur, false);
                self.set_effect_ellipse(o.spotlight_ellipse, false);
            }
            _ => {}
        }
    }

    /// `ActiveTool` (`OnActiveToolChanged`): remember the tool, load its options.
    pub(crate) fn set_active_tool_property(&mut self, tool: EditorTool) {
        self.tools.active_tool = tool;
        self.core.active_tool = tool;
        if is_rememberable(tool) {
            self.options.last_used_annotation_tool = tool;
        }
        self.loading_options = true;
        self.load_options_for_tool(tool);
        self.loading_options = false;
    }

    /// `OnSelectionChanged` of the view: the options follow the selected annotation.
    pub(crate) fn sync_options_from_selection(&mut self) {
        let Some(a) = self.vm_selected_annotation().cloned() else {
            return;
        };
        // The setters below hand the annotation its own values: not an edit.
        let dirty = self.is_dirty;
        self.sync_options_from(&a);
        self.is_dirty = dirty;
    }

    fn sync_options_from(&mut self, a: &Annotation) {
        let skip_stroke = a.is_image() || a.is_effect() || matches!(a.kind, Kind::Spotlight(_));
        if !skip_stroke && !a.is_smart_eraser() {
            self.set_selected_color(&a.stroke_color.clone());
            self.set_stroke_width(a.stroke_width as i32);
            self.set_shadow_enabled(a.shadow.enabled, false);
            let shadow_color = Color::parse(&a.shadow.color)
                .map(argb)
                .unwrap_or_else(|| a.shadow.color.clone());
            self.set_shadow_settings(
                &shadow_color,
                a.shadow.blur_radius,
                a.shadow.opacity,
                a.shadow.offset_x,
                a.shadow.offset_y,
                false,
            );
        }
        match &a.kind {
            Kind::Number(n) => {
                self.set_font_size(n.font_size, false);
                self.set_text_bold(n.bold, false);
                self.set_fill_color(&a.fill_color);
                self.set_speech_balloon_tail(n.tail_enabled, false);
                if !n.text_color.is_empty() {
                    let c = argb(Color::parse_or_transparent(&n.text_color));
                    self.set_text_color(&c);
                }
            }
            Kind::Text(t) => {
                self.set_font_size(t.font_size, false);
                self.set_font_family(&t.font_family, false);
                self.set_text_align(t.align);
                self.set_text_bold(t.bold, false);
                self.set_text_italic(t.italic, false);
                if !t.text_color.is_empty() {
                    let c = argb(Color::parse_or_transparent(&t.text_color));
                    self.set_text_color(&c);
                }
            }
            Kind::SpeechBalloon(b) => {
                self.set_font_size(b.text.font_size, false);
                self.set_font_family(&b.text.font_family, false);
                self.set_text_align(b.text.align);
                self.set_text_bold(b.text.bold, false);
                self.set_text_italic(b.text.italic, false);
                self.set_fill_color(&a.fill_color);
                self.set_corner_radius(b.corner_radius);
                self.set_speech_balloon_tail(b.tail_enabled, false);
                if !b.text.text_color.is_empty() {
                    let c = argb(Color::parse_or_transparent(&b.text.text_color));
                    self.set_text_color(&c);
                }
            }
            Kind::Rectangle {
                border_style,
                corner_radius,
            } => {
                self.set_border_style(*border_style, false);
                self.set_fill_color(&a.fill_color);
                self.set_corner_radius(*corner_radius);
            }
            Kind::Line { border_style, .. } | Kind::Freehand { border_style, .. } => {
                self.set_border_style(*border_style, false)
            }
            Kind::Cursor { cursor_type, .. } => self.set_cursor_type(*cursor_type, false),
            Kind::Arrow { style, .. } => self.set_arrow_style(*style, false),
            Kind::Ellipse { border_style } => {
                self.set_border_style(*border_style, false);
                self.set_fill_color(&a.fill_color);
            }
            Kind::Spotlight(s) => {
                let strength = (s.darken_opacity as f32 / 255.0 * MAX_SPOTLIGHT_STRENGTH).round();
                self.set_effect_strength(strength, false);
                self.set_spotlight_blur(s.blur_amount, false);
                self.set_effect_ellipse(s.is_ellipse, false);
            }
            Kind::Effect(e) => {
                self.set_effect_strength(e.amount.trunc(), false);
                if e.effect == Some(EffectKind::Magnify) {
                    self.set_effect_ellipse(e.is_ellipse, false);
                }
                if e.effect == Some(EffectKind::Highlight) {
                    self.set_fill_color(&a.fill_color);
                }
            }
            _ => {}
        }
    }
}

//! How an upstream effect class becomes an `ImageEffect`: its id, name, category, icon,
//! description and execution mode, its parameter list (built with the same helpers as
//! `EffectParameters`) and the function that applies it.

use std::sync::Arc;

use image::RgbaImage;
use modlight_editor::effects::{
    Category, EffectContext, ExecutionMode, ImageEffect, ParamDef, ParamKind, ParamValues,
};

use crate::sk::{Bitmap, C};

/// What `apply` hands an effect: the parameter values, read with upstream's conversions.
pub struct P<'a> {
    pub values: &'a ParamValues,
    pub context: &'a EffectContext,
}

impl P<'_> {
    /// `FloatSlider` (`(float)value`) or a double numeric read as `float`.
    pub fn f(&self, key: &str) -> f32 {
        self.values.number(key) as f32
    }

    /// `DoubleNumeric` (`double`).
    pub fn d(&self, key: &str) -> f64 {
        self.values.number(key)
    }

    /// `IntSlider` (`(int)Math.Round(value)`) and `IntNumeric` (rounded away from zero).
    pub fn i(&self, key: &str) -> i32 {
        self.values.int(key)
    }

    pub fn b(&self, key: &str) -> bool {
        self.values.boolean(key)
    }

    pub fn c(&self, key: &str) -> C {
        C::from_editor(self.values.color(key))
    }

    /// An enum parameter: the C# member name of the chosen option.
    pub fn e(&self, key: &str) -> String {
        self.values.choice(key)
    }

    pub fn s(&self, key: &str) -> String {
        self.values.text(key)
    }
}

pub type Run = fn(&Bitmap, &P) -> Result<Bitmap, String>;

/// One effect class.
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub icon: char,
    pub description: &'static str,
    pub immediate: bool,
    pub params: fn() -> Vec<ParamDef>,
    pub run: Run,
}

fn no_params() -> Vec<ParamDef> {
    Vec::new()
}

impl Def {
    pub const fn new(
        id: &'static str,
        name: &'static str,
        category: Category,
        icon: char,
        description: &'static str,
        run: Run,
    ) -> Def {
        Def {
            id,
            name,
            category,
            icon,
            description,
            immediate: false,
            params: no_params,
            run,
        }
    }

    pub const fn params(mut self, params: fn() -> Vec<ParamDef>) -> Def {
        self.params = params;
        self
    }

    pub const fn immediate(mut self) -> Def {
        self.immediate = true;
        self
    }

    pub fn arc(self) -> Arc<dyn ImageEffect> {
        Arc::new(self)
    }

    /// Runs the effect on a bitmap with the given values (tests and the classic library).
    pub fn apply_bitmap(&self, source: &Bitmap, values: &ParamValues) -> Result<Bitmap, String> {
        let context = EffectContext::default();
        (self.run)(
            source,
            &P {
                values,
                context: &context,
            },
        )
    }
}

impl ImageEffect for Def {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn category(&self) -> Category {
        self.category
    }
    fn icon(&self) -> char {
        self.icon
    }
    fn description(&self) -> &str {
        self.description
    }
    fn parameters(&self) -> Vec<ParamDef> {
        (self.params)()
    }
    fn execution_mode(&self) -> ExecutionMode {
        if self.immediate {
            ExecutionMode::Immediate
        } else {
            ExecutionMode::Previewable
        }
    }
    fn apply(
        &self,
        source: &RgbaImage,
        values: &ParamValues,
        context: &EffectContext,
    ) -> Result<RgbaImage, String> {
        // Upstream's source is a premultiplied bitmap: semi-transparent colours read back
        // through that storage.
        let mut bitmap = Bitmap::from_image(source);
        for c in bitmap.px.iter_mut() {
            *c = crate::sk::store(*c);
        }
        let out = (self.run)(&bitmap, &P { values, context })?;
        Ok(out.to_image())
    }
}

// ---------- parameter builders (`EffectParameters`) ----------

/// `EffectParameters.IntSlider` with its defaults (tick 1, snapping, `{}{0:0}`).
pub fn int_slider(key: &str, label: &str, min: i32, max: i32, default: i32) -> ParamDef {
    ParamDef::int_slider(key, label, min, max, default)
}

/// `EffectParameters.FloatSlider` with its defaults.
pub fn float_slider(key: &str, label: &str, min: f64, max: f64, default: f64) -> ParamDef {
    ParamDef::float_slider(key, label, min, max, default)
}

/// `EffectParameters.IntNumeric` (increment 1, format `0`).
pub fn int_numeric(key: &str, label: &str, min: i32, max: i32, default: i32) -> ParamDef {
    ParamDef::int_numeric(key, label, min, max, default)
}

/// `EffectParameters.DoubleNumeric` (increment 1, format `0.0`).
pub fn double_numeric(key: &str, label: &str, min: f64, max: f64, default: f64) -> ParamDef {
    ParamDef {
        key: key.into(),
        label: label.into(),
        description: None,
        kind: ParamKind::Numeric {
            min,
            max,
            default,
            increment: 1.0,
            format: "0.0".into(),
            integer: false,
        },
    }
}

pub fn checkbox(key: &str, label: &str, default: bool) -> ParamDef {
    ParamDef::checkbox(key, label, default)
}

pub fn color(key: &str, label: &str, default: C) -> ParamDef {
    ParamDef::color(key, label, default.to_editor())
}

/// `EffectParameters.Enum`: `(label, C# member name)` pairs and the default member.
pub fn choice(key: &str, label: &str, options: &[(&str, &str)], default: &str) -> ParamDef {
    let index = options
        .iter()
        .position(|o| o.1 == default)
        .expect("enum default must be one of the options");
    ParamDef::enumeration(key, label, options, index)
}

pub fn text(key: &str, label: &str, default: &str) -> ParamDef {
    ParamDef::text(key, label, default)
}

pub fn file_path(key: &str, label: &str, default: &str, filter: Option<&str>) -> ParamDef {
    ParamDef::file_path(key, label, default, filter)
}

/// The optional arguments of the `EffectParameters` helpers.
pub trait ParamExt {
    fn tick(self, tick: f64) -> Self;
    fn snap(self, snap: bool) -> Self;
    fn fmt(self, format: &str) -> Self;
    fn incr(self, increment: f64) -> Self;
    fn desc(self, description: &str) -> Self;
}

impl ParamExt for ParamDef {
    fn tick(mut self, value: f64) -> Self {
        if let ParamKind::Slider { tick, .. } = &mut self.kind {
            *tick = value;
        }
        self
    }
    fn snap(mut self, value: bool) -> Self {
        if let ParamKind::Slider { snap, .. } = &mut self.kind {
            *snap = value;
        }
        self
    }
    fn fmt(mut self, value: &str) -> Self {
        match &mut self.kind {
            ParamKind::Slider { format, .. } | ParamKind::Numeric { format, .. } => {
                *format = value.to_string()
            }
            _ => {}
        }
        self
    }
    fn incr(mut self, value: f64) -> Self {
        if let ParamKind::Numeric { increment, .. } = &mut self.kind {
            *increment = value;
        }
        self
    }
    fn desc(mut self, value: &str) -> Self {
        self.description = Some(value.to_string());
        self
    }
}

/// `tickFrequency: t, isSnapToTickEnabled: false, valueStringFormat: f`: the common
/// fine-grained slider.
pub fn fine(p: ParamDef, tick: f64, format: &str) -> ParamDef {
    p.tick(tick).snap(false).fmt(format)
}

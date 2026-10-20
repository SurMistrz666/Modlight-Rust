//! Image effects: the registry the effect library plugs into, the parameter schema
//! (`Core/ImageEffects/Parameters`), the dialog state (`EffectParameterState`), the
//! browser's catalogue with recent and favourite effects (`EffectBrowserPanel`), and the
//! live preview flow (`MainViewModel.EffectPreview`, `EditorView.EffectsHost`).
//!
//! The contract for effect implementations is written down in `app/docs/EDITOR.md`.

pub mod builtin;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use image::RgbaImage;

use crate::color::Color;

/// `ImageEffectCategory`, in its declaration order (the browser's order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Manipulations,
    Adjustments,
    Filters,
    Drawings,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Manipulations,
        Category::Adjustments,
        Category::Filters,
        Category::Drawings,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Manipulations => "Manipulations",
            Category::Adjustments => "Adjustments",
            Category::Filters => "Filters",
            Category::Drawings => "Drawings",
        }
    }

    /// `EffectBrowserLocalization.GetCategoryName`.
    pub fn display_name(self) -> String {
        localized(&format!("EffectBrowserPanel_Category_{}", self.name()))
            .unwrap_or_else(|| self.name().to_string())
    }
}

fn localized(name: &str) -> Option<String> {
    modlight_core::l10n::lookup(&format!("ShareX.ImageEditor:{name}"))
        .map(|key| modlight_core::l10n::t(key).to_string())
}

/// `EffectExecutionMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    #[default]
    Previewable,
    Immediate,
}

/// One parameter of an effect (`EffectParameter` and its subclasses).
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: ParamKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    /// `SliderEffectParameter` (`IntSlider` rounds the value; `FloatSlider` does not).
    Slider {
        min: f64,
        max: f64,
        default: f64,
        tick: f64,
        snap: bool,
        format: String,
        integer: bool,
    },
    /// `CheckboxEffectParameter` (a toggle switch).
    Checkbox { default: bool },
    /// `EnumEffectParameter`: labels and the value each stands for.
    Enum {
        options: Vec<(String, String)>,
        default_index: usize,
    },
    /// `ColorEffectParameter`.
    Color { default: Color },
    /// `NumericEffectParameter` (`IntNumeric` rounds away from zero).
    Numeric {
        min: f64,
        max: f64,
        default: f64,
        increment: f64,
        format: String,
        integer: bool,
    },
    /// `TextEffectParameter`.
    Text { default: String },
    /// `FilePathEffectParameter` (`filter` is `"Images|*.png;*.jpg"`).
    FilePath {
        default: String,
        filter: Option<String>,
    },
}

impl ParamDef {
    /// `EffectParameters.IntSlider`.
    pub fn int_slider(key: &str, label: &str, min: i32, max: i32, default: i32) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::Slider {
                min: min as f64,
                max: max as f64,
                default: default as f64,
                tick: 1.0,
                snap: true,
                format: "{}{0:0}".into(),
                integer: true,
            },
        }
    }

    /// `EffectParameters.FloatSlider`.
    pub fn float_slider(key: &str, label: &str, min: f64, max: f64, default: f64) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::Slider {
                min,
                max,
                default,
                tick: 1.0,
                snap: true,
                format: "{}{0:0}".into(),
                integer: false,
            },
        }
    }

    pub fn checkbox(key: &str, label: &str, default: bool) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::Checkbox { default },
        }
    }

    pub fn color(key: &str, label: &str, default: Color) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::Color { default },
        }
    }

    /// `EffectParameters.IntNumeric`.
    pub fn int_numeric(key: &str, label: &str, min: i32, max: i32, default: i32) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::Numeric {
                min: min as f64,
                max: max as f64,
                default: default as f64,
                increment: 1.0,
                format: "0".into(),
                integer: true,
            },
        }
    }

    pub fn enumeration(
        key: &str,
        label: &str,
        options: &[(&str, &str)],
        default_index: usize,
    ) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::Enum {
                options: options
                    .iter()
                    .map(|(l, v)| (l.to_string(), v.to_string()))
                    .collect(),
                default_index,
            },
        }
    }

    pub fn text(key: &str, label: &str, default: &str) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::Text {
                default: default.into(),
            },
        }
    }

    pub fn file_path(key: &str, label: &str, default: &str, filter: Option<&str>) -> ParamDef {
        ParamDef {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParamKind::FilePath {
                default: default.into(),
                filter: filter.map(str::to_string),
            },
        }
    }

    pub fn default_value(&self) -> ParamValue {
        match &self.kind {
            ParamKind::Slider { default, .. } => ParamValue::Number(*default),
            ParamKind::Checkbox { default } => ParamValue::Bool(*default),
            ParamKind::Enum {
                options,
                default_index,
            } => ParamValue::Choice(
                options
                    .get(*default_index)
                    .map(|o| o.1.clone())
                    .unwrap_or_default(),
            ),
            ParamKind::Color { default } => ParamValue::Color(*default),
            ParamKind::Numeric { default, .. } => ParamValue::Number(*default),
            ParamKind::Text { default } | ParamKind::FilePath { default, .. } => {
                ParamValue::Text(default.clone())
            }
        }
    }
}

/// A parameter's current value.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Number(f64),
    /// An emptied numeric box (`decimal?` null): the effect gets its default.
    Empty,
    Bool(bool),
    Choice(String),
    Color(Color),
    Text(String),
}

/// The values handed to `apply`, by key, with upstream's conversions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamValues {
    values: Vec<(String, ParamValue)>,
    defs: Vec<ParamDef>,
    /// Which numeric boxes changed last (`NumericParameterState.LastChangedSequence`).
    pub sequence: HashMap<String, u64>,
}

impl ParamValues {
    pub fn defaults(defs: &[ParamDef]) -> ParamValues {
        ParamValues {
            values: defs
                .iter()
                .map(|d| (d.key.clone(), d.default_value()))
                .collect(),
            defs: defs.to_vec(),
            sequence: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: ParamValue) {
        if let Some(slot) = self.values.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            self.values.push((key.to_string(), value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&ParamValue> {
        self.values.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn def(&self, key: &str) -> Option<&ParamDef> {
        self.defs.iter().find(|d| d.key == key)
    }

    /// A slider or numeric value as the effect receives it: `IntSlider` rounds
    /// (`Math.Round`, to even), `IntNumeric` rounds away from zero, an empty box gives
    /// the default.
    pub fn number(&self, key: &str) -> f64 {
        let def = self.def(key);
        let raw = match self.get(key) {
            Some(ParamValue::Number(v)) => Some(*v),
            _ => None,
        };
        match def.map(|d| &d.kind) {
            Some(ParamKind::Slider {
                default, integer, ..
            }) => {
                let v = raw.unwrap_or(*default);
                if *integer { round_half_even(v) } else { v }
            }
            Some(ParamKind::Numeric {
                default, integer, ..
            }) => {
                let v = raw.unwrap_or(*default);
                if *integer { v.round() } else { v }
            }
            _ => raw.unwrap_or(0.0),
        }
    }

    pub fn int(&self, key: &str) -> i32 {
        self.number(key) as i32
    }

    pub fn boolean(&self, key: &str) -> bool {
        match (self.get(key), self.def(key).map(|d| &d.kind)) {
            (Some(ParamValue::Bool(b)), _) => *b,
            (_, Some(ParamKind::Checkbox { default })) => *default,
            _ => false,
        }
    }

    pub fn choice(&self, key: &str) -> String {
        match self.get(key) {
            Some(ParamValue::Choice(c)) => c.clone(),
            _ => match self.def(key).map(|d| d.default_value()) {
                Some(ParamValue::Choice(c)) => c,
                _ => String::new(),
            },
        }
    }

    pub fn color(&self, key: &str) -> Color {
        match self.get(key) {
            Some(ParamValue::Color(c)) => *c,
            _ => match self.def(key).map(|d| d.default_value()) {
                Some(ParamValue::Color(c)) => c,
                _ => Color::TRANSPARENT,
            },
        }
    }

    pub fn text(&self, key: &str) -> String {
        match self.get(key) {
            Some(ParamValue::Text(t)) => t.clone(),
            _ => match self.def(key).map(|d| d.default_value()) {
                Some(ParamValue::Text(t)) => t,
                _ => String::new(),
            },
        }
    }
}

fn round_half_even(v: f64) -> f64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - v.signum()
    } else {
        r
    }
}

/// `ImageEffectContext`.
#[derive(Clone, Copy, Debug, Default)]
pub struct EffectContext {
    pub is_preview: bool,
}

/// An image effect (`ImageEffectBase`). Implementations are pure: they read the source
/// and the values and return a new image (straight RGBA, any size).
pub trait ImageEffect: Send + Sync {
    /// The stable id (`"gaussian_blur"`); recent and favourite lists store it.
    fn id(&self) -> &str;
    /// The English name; the browser shows the translation `EffectBrowserPanel_Effect_<id>`
    /// when the string tables have one.
    fn name(&self) -> &str;
    fn category(&self) -> Category;
    /// A Lucide glyph (`IconKey`), e.g. `modlight_core::icons::CIRCLE_GAUGE`.
    fn icon(&self) -> char;
    fn description(&self) -> &str {
        ""
    }
    fn parameters(&self) -> Vec<ParamDef> {
        Vec::new()
    }
    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Previewable
    }
    fn apply(
        &self,
        source: &RgbaImage,
        values: &ParamValues,
        context: &EffectContext,
    ) -> Result<RgbaImage, String>;
}

/// `BrowserLabel`: immediate effects plain, the others with an ellipsis (they open a
/// dialog).
pub fn browser_label(effect: &dyn ImageEffect) -> String {
    let name = display_name(effect);
    if effect.execution_mode() == ExecutionMode::Immediate {
        name
    } else {
        format!("{name}...")
    }
}

/// `EffectBrowserLocalization.GetEffectName`.
pub fn display_name(effect: &dyn ImageEffect) -> String {
    localized(&format!("EffectBrowserPanel_Effect_{}", effect.id()))
        .unwrap_or_else(|| effect.name().to_string())
}

/// The effects the editor knows (`DiscoveredEffectRegistry`): the built-in ones plus
/// whatever the effect library registers.
#[derive(Default, Clone)]
pub struct EffectRegistry {
    effects: Vec<Arc<dyn ImageEffect>>,
}

impl EffectRegistry {
    pub fn with_builtin() -> EffectRegistry {
        let mut r = EffectRegistry::default();
        for effect in builtin::effects() {
            r.register(effect);
        }
        r
    }

    /// Adds an effect; one with the same id replaces the earlier one.
    pub fn register(&mut self, effect: Arc<dyn ImageEffect>) {
        self.effects
            .retain(|e| !e.id().eq_ignore_ascii_case(effect.id()));
        self.effects.push(effect);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn ImageEffect>> {
        self.effects
            .iter()
            .find(|e| e.id().eq_ignore_ascii_case(id))
            .cloned()
    }

    /// Every effect, by category then name (`OrderBy(Category).ThenBy(Name)`).
    pub fn all(&self) -> Vec<Arc<dyn ImageEffect>> {
        let mut list = self.effects.clone();
        list.sort_by(|a, b| {
            a.category()
                .cmp(&b.category())
                .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
        });
        list
    }

    pub fn by_category(&self, category: Category) -> Vec<Arc<dyn ImageEffect>> {
        self.all()
            .into_iter()
            .filter(|e| e.category() == category)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }
}

fn global() -> &'static RwLock<EffectRegistry> {
    static REGISTRY: OnceLock<RwLock<EffectRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(EffectRegistry::with_builtin()))
}

/// The process-wide registry.
pub fn registry() -> EffectRegistry {
    global().read().unwrap().clone()
}

/// Adds effects to the process-wide registry (the effect library calls this once).
pub fn register_effects(effects: impl IntoIterator<Item = Arc<dyn ImageEffect>>) {
    let mut r = global().write().unwrap();
    for e in effects {
        r.register(e);
    }
}

/// `EditorOperationKind`: effects the editor routes through its core so annotations
/// follow the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    AutoCropImage,
    CropImage,
    ResizeImage,
    ResizeCanvas,
    Rotate90Clockwise,
    Rotate90CounterClockwise,
    Rotate180,
    RotateCustomAngle,
    FlipHorizontal,
    FlipVertical,
}

/// `EditorOperationCatalog`'s mappings.
pub fn operation_for(id: &str) -> Option<Operation> {
    Some(match id.to_ascii_lowercase().as_str() {
        "auto_crop_image" => Operation::AutoCropImage,
        "crop_image" => Operation::CropImage,
        "resize_image" => Operation::ResizeImage,
        "resize_canvas" => Operation::ResizeCanvas,
        "rotate_90_clockwise" => Operation::Rotate90Clockwise,
        "rotate_90_counter_clockwise" => Operation::Rotate90CounterClockwise,
        "rotate_180" => Operation::Rotate180,
        "rotate_custom_angle" => Operation::RotateCustomAngle,
        "flip_horizontal" => Operation::FlipHorizontal,
        "flip_vertical" => Operation::FlipVertical,
        _ => return None,
    })
}

/// Old ids still found in stored recent and favourite lists (`EffectAliases`).
pub fn resolve_alias(id: &str) -> String {
    let normalized = normalize_effect_id(id);
    match normalized.as_str() {
        "resize" => "resize_image".into(),
        "canvas" => "resize_canvas".into(),
        "crop" => "crop_image".into(),
        "auto_crop" | "editor_auto_crop" => "auto_crop_image".into(),
        "rotate" => "rotate_custom_angle".into(),
        "rotate_90" => "rotate_90_clockwise".into(),
        "rotate_90_cc" => "rotate_90_counter_clockwise".into(),
        _ => normalized,
    }
}

/// `EffectItem.NormalizeEffectId`.
pub fn normalize_effect_id(value: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = false;
    for c in value.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// The right sidebar's effect dialog (`SchemaDrivenEffectDialog`).
#[derive(Clone, Debug, PartialEq)]
pub struct DialogState {
    pub effect_id: String,
    pub params: Vec<ParamDef>,
    pub values: ParamValues,
    next_sequence: u64,
}

impl DialogState {
    pub fn new(effect: &dyn ImageEffect) -> DialogState {
        let params = effect.parameters();
        DialogState {
            effect_id: effect.id().to_string(),
            values: ParamValues::defaults(&params),
            params,
            next_sequence: 1,
        }
    }

    /// Changes a value (and records the order numeric boxes changed in).
    pub fn set(&mut self, key: &str, value: ParamValue) {
        if matches!(
            self.params.iter().find(|p| p.key == key).map(|p| &p.kind),
            Some(ParamKind::Numeric { .. })
        ) {
            self.values
                .sequence
                .insert(key.to_string(), self.next_sequence);
            self.next_sequence += 1;
        }
        self.values.set(key, value);
    }

    /// `ResolveResizeImageAspectRatioAnchor`: the box changed last wins.
    pub fn resize_anchor(&self) -> ResizeAnchor {
        let w = self.values.sequence.get("width").copied().unwrap_or(0);
        let h = self.values.sequence.get("height").copied().unwrap_or(0);
        if w == h {
            ResizeAnchor::LargestDimension
        } else if w > h {
            ResizeAnchor::Width
        } else {
            ResizeAnchor::Height
        }
    }
}

/// `ResizeImageEffectAspectRatioAnchor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeAnchor {
    LargestDimension,
    Width,
    Height,
}

/// A live preview session (`_preEffectImage` and `_latestEffectPreviewImage`).
#[derive(Clone, Debug)]
pub struct PreviewState {
    pub original: Arc<RgbaImage>,
    pub latest: Option<Arc<RgbaImage>>,
}

impl PartialEq for PreviewState {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.original, &other.original)
    }
}

/// The browser's lists (`EffectBrowserPanel`): recent (at most 10, newest first) and
/// favourites, both kept in the options by id.
pub const MAX_RECENT_EFFECTS: usize = 10;

pub fn resolve_ids(registry: &EffectRegistry, ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let id = resolve_alias(id);
        if registry.get(&id).is_some() && !out.iter().any(|o| o.eq_ignore_ascii_case(&id)) {
            out.push(id);
        }
    }
    out
}

/// `EffectCategory.Filter`: name or description contains the text (case-insensitive).
pub fn matches_search(effect: &dyn ImageEffect, search: &str) -> bool {
    let search = search.trim().to_lowercase();
    if search.is_empty() {
        return true;
    }
    browser_label(effect).to_lowercase().contains(&search)
        || effect.description().to_lowercase().contains(&search)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_holds_the_operations_and_the_sample_effects() {
        let r = EffectRegistry::with_builtin();
        for id in [
            "invert",
            "grayscale",
            "brightness",
            "gaussian_blur",
            "rotate_custom_angle",
            "resize_image",
            "flip_vertical",
        ] {
            assert!(r.get(id).is_some(), "{id}");
        }
        let all = r.all();
        assert_eq!(all[0].category(), Category::Manipulations);
        assert!(operation_for("ROTATE_180").is_some());
        assert_eq!(resolve_alias("Auto crop"), "auto_crop_image");
    }

    #[test]
    fn slider_values_round_like_upstream() {
        let defs = vec![ParamDef::int_slider("radius", "Radius", 1, 200, 15)];
        let mut v = ParamValues::defaults(&defs);
        v.set("radius", ParamValue::Number(2.5));
        assert_eq!(v.int("radius"), 2);
        v.set("radius", ParamValue::Number(3.5));
        assert_eq!(v.int("radius"), 4);
    }

    #[test]
    fn resize_anchor_follows_the_last_box_changed() {
        let r = EffectRegistry::with_builtin();
        let mut d = DialogState::new(r.get("resize_image").unwrap().as_ref());
        assert_eq!(d.resize_anchor(), ResizeAnchor::LargestDimension);
        d.set("height", ParamValue::Number(10.0));
        d.set("width", ParamValue::Number(10.0));
        assert_eq!(d.resize_anchor(), ResizeAnchor::Width);
    }
}

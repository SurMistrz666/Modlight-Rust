//! The image effects of Modlight App for macOS.
//!
//! - `fx`: every effect of upstream `ShareX.ImageEditor/Core/ImageEffects` (the editor's
//!   effect browser), implemented against `modlight_editor::effects::ImageEffect` with the
//!   same ids, names, categories, icons, parameters and pixel arithmetic.
//! - `sk`: the parts of SkiaSharp those effects rely on, with Skia's rounding.
//! - `classic`: `ShareX.ImageEffectsLib`, the preset-based effects of the task settings
//!   ("Image effects" after capture), their JSON format and the `.sxie` packager.
//!
//! Pure pixel code: no GUI.

// The pixel code follows upstream's C# line by line (index loops, explicit clamps written
// as upstream writes them, long parameter lists), which keeps it reviewable against the
// source.
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::manual_clamp,
    clippy::excessive_precision,
    clippy::chunks_exact_to_as_chunks,
    clippy::approx_constant,
    clippy::collapsible_else_if,
    clippy::type_complexity
)]

pub mod def;
pub mod dotnet;
pub mod fx;
pub mod helpers;
pub mod sk;

use std::sync::Arc;

use modlight_editor::effects::ImageEffect;

/// Every effect, as the editor's registry takes them.
pub fn effects() -> Vec<Arc<dyn ImageEffect>> {
    fx::defs().into_iter().map(|d| d.arc()).collect()
}

/// Adds every effect to the editor's process-wide registry (call once at start).
pub fn register() {
    modlight_editor::effects::register_effects(effects());
}

/// One effect by id, for callers that need the concrete definition.
pub fn find(id: &str) -> Option<def::Def> {
    fx::defs()
        .into_iter()
        .find(|d| d.id.eq_ignore_ascii_case(id))
}

//! `ShareX.ImageEffectsLib`: the classic image effects of the task settings ("Add
//! image effects" after capture), their presets and `.sxie` packages.

pub mod apply;
pub mod gdi;
pub mod model;
pub mod package;
pub mod sample;

pub use model::{ClassicEffect, Kind, Preset, Value};

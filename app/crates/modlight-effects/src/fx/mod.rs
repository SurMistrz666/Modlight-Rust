//! The effect classes of upstream `Core/ImageEffects`, by category.

pub mod adjustments;

use crate::def::Def;

/// Every effect class the editor discovers (those with a parameterless constructor).
pub fn defs() -> Vec<Def> {
    let mut all = Vec::new();
    all.extend(adjustments::defs());
    all
}

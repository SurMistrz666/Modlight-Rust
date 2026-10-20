//! The effect classes of upstream `Core/ImageEffects`, by category.

pub mod adjustments;
pub mod manipulations;
pub mod remove_background;

use crate::def::Def;

/// Every effect class the editor discovers (those with a parameterless constructor).
pub fn defs() -> Vec<Def> {
    let mut all = Vec::new();
    all.extend(adjustments::defs());
    all.extend(manipulations::defs());
    all.push(remove_background::def());
    all
}

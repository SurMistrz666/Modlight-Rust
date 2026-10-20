//! The effect classes of upstream `Core/ImageEffects`, by category.

pub mod adjustments;
pub mod filters_a;
pub mod filters_b;
pub mod filters_c;
pub mod filters_d;
pub mod filters_e;
pub mod filters_f;
pub mod filters_g;
pub mod filters_h;
pub mod filters_i;
pub mod filters_j;
pub mod filters_k;
pub mod manipulations;
pub mod remove_background;

use crate::def::Def;

/// Every effect class the editor discovers (those with a parameterless constructor).
pub fn defs() -> Vec<Def> {
    let mut all = Vec::new();
    all.extend(adjustments::defs());
    all.extend(manipulations::defs());
    all.push(remove_background::def());
    all.extend(filters_a::defs());
    all.extend(filters_b::defs());
    all.extend(filters_c::defs());
    all.extend(filters_d::defs());
    all.extend(filters_e::defs());
    all.extend(filters_f::defs());
    all.extend(filters_g::defs());
    all.extend(filters_h::defs());
    all.extend(filters_i::defs());
    all.extend(filters_j::defs());
    all.extend(filters_k::defs());
    all
}

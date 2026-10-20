//! The effects this crate brings itself: the manipulations the editor routes through its
//! core (`EditorOperationCatalog`) and a few simple effects that prove the registry end
//! to end (invert, grayscale, brightness, Gaussian blur, flip). The full library
//! (`Core/ImageEffects`, about 240 classes) registers the rest through
//! `effects::register_effects`.

use std::sync::Arc;

use image::{Rgba, RgbaImage};
use modlight_core::icons as l;

use super::{Category, EffectContext, ExecutionMode, ImageEffect, ParamDef, ParamValues};
use crate::color::Color;
use crate::ops;
use crate::raster;

/// Every built-in effect.
pub fn effects() -> Vec<Arc<dyn ImageEffect>> {
    vec![
        Arc::new(AutoCrop),
        Arc::new(CropImage),
        Arc::new(ResizeImage),
        Arc::new(ResizeCanvas),
        Arc::new(Rotate90 {
            id: "rotate_90_clockwise",
            name: "Rotate 90\u{b0} clockwise",
            icon: l::ROTATE_CW,
            description: "Rotates the image 90 degrees clockwise.",
            angle: 90,
        }),
        Arc::new(Rotate90 {
            id: "rotate_90_counter_clockwise",
            name: "Rotate 90\u{b0} counter clockwise",
            icon: l::ROTATE_CCW,
            description: "Rotates the image 90 degrees counter-clockwise.",
            angle: -90,
        }),
        Arc::new(Rotate90 {
            id: "rotate_180",
            name: "Rotate 180\u{b0}",
            icon: l::ROTATE_CW,
            description: "Rotates the image by 180 degrees.",
            angle: 180,
        }),
        Arc::new(RotateCustom),
        Arc::new(FlipFixed { vertical: false }),
        Arc::new(FlipFixed { vertical: true }),
        Arc::new(Flip),
        Arc::new(Invert),
        Arc::new(Grayscale),
        Arc::new(Brightness),
        Arc::new(GaussianBlur),
    ]
}

/// `AdjustmentImageEffectBase.ApplyColorMatrix`: Skia's 4x5 matrix on straight colours
/// in 0..1, the fifth column added.
pub fn apply_color_matrix(source: &RgbaImage, m: &[f32; 20]) -> RgbaImage {
    let mut out = source.clone();
    for p in out.pixels_mut() {
        let c = [
            p.0[0] as f32 / 255.0,
            p.0[1] as f32 / 255.0,
            p.0[2] as f32 / 255.0,
            p.0[3] as f32 / 255.0,
        ];
        let mut r = [0u8; 4];
        for (row, value) in r.iter_mut().enumerate() {
            let k = row * 5;
            let v = m[k] * c[0] + m[k + 1] * c[1] + m[k + 2] * c[2] + m[k + 3] * c[3] + m[k + 4];
            *value = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        *p = Rgba(r);
    }
    out
}

struct AutoCrop;
impl ImageEffect for AutoCrop {
    fn id(&self) -> &str {
        "auto_crop_image"
    }
    fn name(&self) -> &str {
        "Auto crop image"
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        l::SCAN
    }
    fn description(&self) -> &str {
        "Automatically crops the image using tolerance on edge pixels."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![ParamDef::int_slider("tolerance", "Tolerance", 0, 255, 0)]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        // A transparent match colour means "the top-left pixel".
        let corner = *source
            .get_pixel_checked(0, 0)
            .unwrap_or(&Rgba([0, 0, 0, 0]));
        Ok(ops::auto_crop(source, corner, v.int("tolerance")))
    }
}

struct CropImage;
impl ImageEffect for CropImage {
    fn id(&self) -> &str {
        "crop_image"
    }
    fn name(&self) -> &str {
        "Crop image"
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        l::CROP
    }
    fn description(&self) -> &str {
        "Crops the image."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int_numeric("x", "X", 0, 100000, 0),
            ParamDef::int_numeric("y", "Y", 0, 100000, 0),
            ParamDef::int_numeric("width", "Width", 1, 100000, 100),
            ParamDef::int_numeric("height", "Height", 1, 100000, 100),
        ]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        let rect = crate::geom::RectI::new(
            v.int("x"),
            v.int("y"),
            v.int("x") + v.int("width"),
            v.int("y") + v.int("height"),
        );
        Ok(raster::crop(source, rect))
    }
}

struct ResizeImage;
impl ImageEffect for ResizeImage {
    fn id(&self) -> &str {
        "resize_image"
    }
    fn name(&self) -> &str {
        "Resize image"
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        l::SCALING
    }
    fn description(&self) -> &str {
        "Resizes the image."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int_numeric("width", "Width", 0, 10000, 0),
            ParamDef::int_numeric("height", "Height", 0, 10000, 0),
            ParamDef::checkbox("maintain_aspect_ratio", "Maintain aspect ratio", false),
        ]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        let (w, h) = resize_target(source.dimensions(), v);
        Ok(ops::resize(source, w, h))
    }
}

/// `ResizeImageEffect.GetTargetSize` with the anchor of the box changed last.
pub fn resize_target(source: (u32, u32), v: &ParamValues) -> (u32, u32) {
    let w = v.int("width");
    let h = v.int("height");
    let keep = v.boolean("maintain_aspect_ratio");
    if keep && w > 0 && h > 0 {
        let ws = v.sequence.get("width").copied().unwrap_or(0);
        let hs = v.sequence.get("height").copied().unwrap_or(0);
        let (sw, sh) = (source.0 as f64, source.1 as f64);
        if ws > hs {
            let nh = ((w as f64 / sw * sh).round() as i64).max(1) as u32;
            return (w as u32, nh);
        }
        if hs > ws {
            let nw = ((h as f64 / sh * sw).round() as i64).max(1) as u32;
            return (nw, h as u32);
        }
    }
    ops::resolve_resize_target(source, (w, h), keep)
}

struct ResizeCanvas;
impl ImageEffect for ResizeCanvas {
    fn id(&self) -> &str {
        "resize_canvas"
    }
    fn name(&self) -> &str {
        "Resize canvas"
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        l::EXPAND
    }
    fn description(&self) -> &str {
        "Resizes the canvas."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int_numeric("top", "Top", -10000, 10000, 0),
            ParamDef::int_numeric("right", "Right", -10000, 10000, 0),
            ParamDef::int_numeric("bottom", "Bottom", -10000, 10000, 0),
            ParamDef::int_numeric("left", "Left", -10000, 10000, 0),
            ParamDef::color("background_color", "Background color", Color::TRANSPARENT),
        ]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        ops::resize_canvas(
            source,
            v.int("left"),
            v.int("top"),
            v.int("right"),
            v.int("bottom"),
            v.color("background_color"),
        )
        .ok_or_else(|| "Canvas size after padding must be greater than 0".to_string())
    }
}

struct Rotate90 {
    id: &'static str,
    name: &'static str,
    icon: char,
    description: &'static str,
    angle: i32,
}
impl ImageEffect for Rotate90 {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        self.icon
    }
    fn description(&self) -> &str {
        self.description
    }
    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Immediate
    }
    fn apply(
        &self,
        source: &RgbaImage,
        _: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        Ok(ops::rotate_orthogonal(source, self.angle))
    }
}

struct RotateCustom;
impl ImageEffect for RotateCustom {
    fn id(&self) -> &str {
        "rotate_custom_angle"
    }
    fn name(&self) -> &str {
        "Rotate"
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        l::ROTATE_CW
    }
    fn description(&self) -> &str {
        "Rotates the image by a custom angle."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float_slider("angle", "Angle", -360.0, 360.0, 0.0),
            ParamDef::checkbox("auto_resize", "Auto resize", true),
        ]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        Ok(ops::rotate_custom(
            source,
            v.number("angle") as f32,
            v.boolean("auto_resize"),
        ))
    }
}

struct FlipFixed {
    vertical: bool,
}
impl ImageEffect for FlipFixed {
    fn id(&self) -> &str {
        if self.vertical {
            "flip_vertical"
        } else {
            "flip_horizontal"
        }
    }
    fn name(&self) -> &str {
        if self.vertical {
            "Flip vertical"
        } else {
            "Flip horizontal"
        }
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        if self.vertical {
            l::FLIP_VERTICAL
        } else {
            l::FLIP_HORIZONTAL
        }
    }
    fn description(&self) -> &str {
        if self.vertical {
            "Flips the image vertically."
        } else {
            "Flips the image horizontally."
        }
    }
    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Immediate
    }
    fn apply(
        &self,
        source: &RgbaImage,
        _: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        Ok(if self.vertical {
            ops::flip_vertical(source)
        } else {
            ops::flip_horizontal(source)
        })
    }
}

/// `FlipImageEffect` (id "flip"): either or both directions.
struct Flip;
impl ImageEffect for Flip {
    fn id(&self) -> &str {
        "flip"
    }
    fn name(&self) -> &str {
        "Flip"
    }
    fn category(&self) -> Category {
        Category::Manipulations
    }
    fn icon(&self) -> char {
        l::FLIP_HORIZONTAL
    }
    fn description(&self) -> &str {
        "Flips the image horizontally or vertically."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::checkbox("horizontally", "Horizontally", false),
            ParamDef::checkbox("vertically", "Vertically", false),
        ]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        let mut out = source.clone();
        if v.boolean("horizontally") {
            out = ops::flip_horizontal(&out);
        }
        if v.boolean("vertically") {
            out = ops::flip_vertical(&out);
        }
        Ok(out)
    }
}

struct Invert;
impl ImageEffect for Invert {
    fn id(&self) -> &str {
        "invert"
    }
    fn name(&self) -> &str {
        "Invert"
    }
    fn category(&self) -> Category {
        Category::Adjustments
    }
    fn icon(&self) -> char {
        l::REFRESH_CCW_DOT
    }
    fn description(&self) -> &str {
        "Inverts image colors."
    }
    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Immediate
    }
    fn apply(
        &self,
        source: &RgbaImage,
        _: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        Ok(apply_color_matrix(
            source,
            &[
                -1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
            ],
        ))
    }
}

struct Grayscale;
impl ImageEffect for Grayscale {
    fn id(&self) -> &str {
        "grayscale"
    }
    fn name(&self) -> &str {
        "Grayscale"
    }
    fn category(&self) -> Category {
        Category::Adjustments
    }
    fn icon(&self) -> char {
        l::ECLIPSE
    }
    fn description(&self) -> &str {
        "Converts the image to grayscale."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![ParamDef::float_slider(
            "strength", "Strength", 0.0, 100.0, 100.0,
        )]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        let strength = v.number("strength") as f32;
        let (r, g, b) = (0.2126f32, 0.7152f32, 0.0722f32);
        if strength >= 100.0 {
            return Ok(apply_color_matrix(
                source,
                &[
                    r, g, b, 0.0, 0.0, r, g, b, 0.0, 0.0, r, g, b, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                    0.0,
                ],
            ));
        }
        if strength <= 0.0 {
            return Ok(source.clone());
        }
        let s = strength / 100.0;
        let i = 1.0 - s;
        Ok(apply_color_matrix(
            source,
            &[
                r * s + i,
                g * s,
                b * s,
                0.0,
                0.0,
                r * s,
                g * s + i,
                b * s,
                0.0,
                0.0,
                r * s,
                g * s,
                b * s + i,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ],
        ))
    }
}

struct Brightness;
impl ImageEffect for Brightness {
    fn id(&self) -> &str {
        "brightness"
    }
    fn name(&self) -> &str {
        "Brightness"
    }
    fn category(&self) -> Category {
        Category::Adjustments
    }
    fn icon(&self) -> char {
        l::SUN_MEDIUM
    }
    fn description(&self) -> &str {
        "Adjusts image brightness."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![ParamDef::float_slider(
            "amount", "Amount", -100.0, 100.0, 0.0,
        )]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        let value = v.number("amount") as f32 / 100.0;
        Ok(apply_color_matrix(
            source,
            &[
                1.0, 0.0, 0.0, 0.0, value, 0.0, 1.0, 0.0, 0.0, value, 0.0, 0.0, 1.0, 0.0, value,
                0.0, 0.0, 0.0, 1.0, 0.0,
            ],
        ))
    }
}

struct GaussianBlur;
impl ImageEffect for GaussianBlur {
    fn id(&self) -> &str {
        "gaussian_blur"
    }
    fn name(&self) -> &str {
        "Gaussian blur"
    }
    fn category(&self) -> Category {
        Category::Filters
    }
    fn icon(&self) -> char {
        l::CIRCLE_GAUGE
    }
    fn description(&self) -> &str {
        "Applies a Gaussian blur effect."
    }
    fn parameters(&self) -> Vec<ParamDef> {
        vec![ParamDef::int_slider("radius", "Radius", 1, 200, 15)]
    }
    fn apply(
        &self,
        source: &RgbaImage,
        v: &ParamValues,
        _: &EffectContext,
    ) -> Result<RgbaImage, String> {
        // Extended by its edges (twice the radius), blurred with sigma = radius / 3,
        // cut back.
        let radius = v.int("radius").max(1) as u32;
        let sigma = radius as f32 / 3.0;
        let pad = radius * 2;
        let extended = raster::extend_clamped(source, pad, pad, pad, pad);
        let blurred = raster::gaussian_blur(&extended, sigma);
        Ok(
            image::imageops::crop_imm(&blurred, pad, pad, source.width(), source.height())
                .to_image(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::ParamValue;

    #[test]
    fn invert_and_grayscale_change_pixels_as_upstream() {
        let img = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 64, 255]));
        let inv = Invert
            .apply(&img, &ParamValues::default(), &EffectContext::default())
            .unwrap();
        assert_eq!(inv.get_pixel(0, 0).0, [0, 255, 191, 255]);
        let g = Grayscale;
        let v = ParamValues::defaults(&g.parameters());
        let gray = g.apply(&img, &v, &EffectContext::default()).unwrap();
        let p = gray.get_pixel(0, 0).0;
        assert_eq!(p[0], p[1]);
        assert_eq!(p[0], (0.2126f32 * 255.0 + 0.0722 * 64.0).round() as u8);
    }

    #[test]
    fn brightness_adds_to_every_channel() {
        let img = RgbaImage::from_pixel(1, 1, Rgba([100, 100, 100, 255]));
        let b = Brightness;
        let mut v = ParamValues::defaults(&b.parameters());
        v.set("amount", ParamValue::Number(10.0));
        let out = b.apply(&img, &v, &EffectContext::default()).unwrap();
        assert_eq!(out.get_pixel(0, 0).0, [126, 126, 126, 255]);
    }
}

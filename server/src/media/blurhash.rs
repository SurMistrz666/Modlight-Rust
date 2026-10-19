//! `encode` of the `blurhash` npm package, version 2.0.5 (src/encode.ts,
//! src/base83.ts, src/utils.ts), which `lib/media/image.ts` calls.
//!
//! Ported operation for operation, in the same evaluation order, because the
//! hash is stored on the upload row and both servers must write the same
//! string for the same pixels. Two details of that version are kept on
//! purpose although other BlurHash implementations differ:
//!
//! - the AC maximum is the largest SIGNED component (`Math.max(...val)`), not
//!   the largest magnitude;
//! - every intermediate is an IEEE double, summed x-major as the JS loops do.
//!
//! `tests/fixtures/blurhash_vectors.json` holds hashes computed by the
//! reference package itself; the test below replays them.

use std::f64::consts::PI;

const DIGIT_CHARACTERS: &[u8; 83] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";

/// `ValidationError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub &'static str);

/// `encode83`.
pub fn encode83(n: f64, length: i32) -> String {
    let mut result = String::new();
    for i in 1..=length {
        let digit = (n.floor() / 83f64.powi(length - i)) % 83.0;
        result.push(DIGIT_CHARACTERS[digit.floor() as usize] as char);
    }
    result
}

/// `sRGBToLinear`.
pub fn srgb_to_linear(value: u8) -> f64 {
    let v = value as f64 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// `linearTosRGB`.
pub fn linear_to_srgb(value: f64) -> i64 {
    // Math.max(0, Math.min(1, value)); NaN stays NaN in both.
    let v = value.clamp(0.0, 1.0);
    if v <= 0.0031308 {
        (v * 12.92 * 255.0 + 0.5).trunc() as i64
    } else {
        ((1.055 * v.powf(1.0 / 2.4) - 0.055) * 255.0 + 0.5).trunc() as i64
    }
}

/// `sign`.
fn sign(n: f64) -> f64 {
    if n < 0.0 { -1.0 } else { 1.0 }
}

/// `signPow`.
fn sign_pow(val: f64, exp: f64) -> f64 {
    sign(val) * val.abs().powf(exp)
}

const BYTES_PER_PIXEL: usize = 4;

/// `multiplyBasisFunction`, with the basis function of `encode` inlined.
fn multiply_basis_function(pixels: &[u8], width: usize, height: usize, cx: usize, cy: usize) -> [f64; 3] {
    let mut r = 0.0f64;
    let mut g = 0.0f64;
    let mut b = 0.0f64;
    let bytes_per_row = width * BYTES_PER_PIXEL;
    let normalisation = if cx == 0 && cy == 0 { 1.0 } else { 2.0 };

    for x in 0..width {
        let bytes_per_pixel_x = BYTES_PER_PIXEL * x;
        for y in 0..height {
            let base_pixel_index = bytes_per_pixel_x + y * bytes_per_row;
            let basis = normalisation
                * (PI * cx as f64 * x as f64 / width as f64).cos()
                * (PI * cy as f64 * y as f64 / height as f64).cos();
            r += basis * srgb_to_linear(pixels[base_pixel_index]);
            g += basis * srgb_to_linear(pixels[base_pixel_index + 1]);
            b += basis * srgb_to_linear(pixels[base_pixel_index + 2]);
        }
    }

    let scale = 1.0 / (width * height) as f64;
    [r * scale, g * scale, b * scale]
}

/// `encodeDC`.
fn encode_dc(value: [f64; 3]) -> i64 {
    let rounded_r = linear_to_srgb(value[0]);
    let rounded_g = linear_to_srgb(value[1]);
    let rounded_b = linear_to_srgb(value[2]);
    (rounded_r << 16) + (rounded_g << 8) + rounded_b
}

/// `encodeAC`.
fn encode_ac(value: [f64; 3], maximum_value: f64) -> f64 {
    let quant = |v: f64| (sign_pow(v / maximum_value, 0.5) * 9.0 + 9.5).floor().clamp(0.0, 18.0).floor();
    let quant_r = quant(value[0]);
    let quant_g = quant(value[1]);
    let quant_b = quant(value[2]);
    quant_r * 19.0 * 19.0 + quant_g * 19.0 + quant_b
}

/// `encode(pixels, width, height, componentX, componentY)`: `pixels` is RGBA,
/// 4 bytes per pixel, row major.
pub fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    component_x: u32,
    component_y: u32,
) -> Result<String, ValidationError> {
    if !(1..=9).contains(&component_x) || !(1..=9).contains(&component_y) {
        return Err(ValidationError("BlurHash must have between 1 and 9 components"));
    }
    if width as usize * height as usize * 4 != pixels.len() {
        return Err(ValidationError("Width and height must match the pixels array"));
    }
    let (width, height) = (width as usize, height as usize);

    let mut factors: Vec<[f64; 3]> = Vec::with_capacity((component_x * component_y) as usize);
    for y in 0..component_y as usize {
        for x in 0..component_x as usize {
            factors.push(multiply_basis_function(pixels, width, height, x, y));
        }
    }

    let dc = factors[0];
    let ac = &factors[1..];

    let mut hash = String::new();

    let size_flag = (component_x - 1) + (component_y - 1) * 9;
    hash += &encode83(size_flag as f64, 1);

    let maximum_value: f64;
    if !ac.is_empty() {
        // Math.max(...ac.map((val) => Math.max(...val))): the largest SIGNED value.
        let actual_maximum_value = ac.iter().map(|v| v[0].max(v[1]).max(v[2])).fold(f64::NEG_INFINITY, f64::max);
        let quantised_maximum_value = (actual_maximum_value * 166.0 - 0.5).floor().clamp(0.0, 82.0).floor();
        maximum_value = (quantised_maximum_value + 1.0) / 166.0;
        hash += &encode83(quantised_maximum_value, 1);
    } else {
        maximum_value = 1.0;
        hash += &encode83(0.0, 1);
    }

    hash += &encode83(encode_dc(dc) as f64, 4);

    for factor in ac {
        hash += &encode83(encode_ac(*factor, maximum_value), 2);
    }

    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_what_the_reference_rejects() {
        assert_eq!(encode(&[0; 4], 1, 1, 0, 4), Err(ValidationError("BlurHash must have between 1 and 9 components")));
        assert_eq!(encode(&[0; 4], 1, 1, 4, 10), Err(ValidationError("BlurHash must have between 1 and 9 components")));
        assert_eq!(encode(&[0; 8], 1, 1, 4, 4), Err(ValidationError("Width and height must match the pixels array")));
    }

    #[test]
    fn base83_digits() {
        assert_eq!(encode83(0.0, 1), "0");
        assert_eq!(encode83(82.0, 1), "~");
        assert_eq!(encode83(83.0, 2), "10");
        // 16777215 = 29*83^3 + 28*83^2 + 30*83 + 10
        assert_eq!(encode83(16_777_215.0, 4), "TSUA");
    }

    /// The xorshift32 the vector generator used (see the fixture's header in
    /// `_scratch/srv-library/media/blurhash/vectors.js`).
    fn pixels(pattern: &str, seed: u32, w: usize, h: usize) -> Vec<u8> {
        let mut s = if seed == 0 { 1 } else { seed };
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                match pattern {
                    "noise" => {
                        let v = next();
                        out[i..i + 4].copy_from_slice(&v.to_le_bytes());
                    }
                    "gradient" => {
                        let v = next() & 15;
                        out[i] = ((x * 255) as f64 / (w.max(2) - 1) as f64).floor() as u8;
                        out[i + 1] = ((y * 255) as f64 / (h.max(2) - 1) as f64).floor() as u8;
                        out[i + 2] = ((seed as u64 * 37 + v as u64) & 255) as u8;
                        out[i + 3] = 255;
                    }
                    _ => {
                        out[i] = (seed & 255) as u8;
                        out[i + 1] = ((seed >> 8) & 255) as u8;
                        out[i + 2] = ((seed >> 16) & 255) as u8;
                        out[i + 3] = 255;
                    }
                }
            }
        }
        out
    }

    /// Hashes the reference package (blurhash 2.0.5 under Node 24 in the
    /// reference container) computed for these exact pixels.
    #[test]
    fn matches_the_reference_package_hash_for_hash() {
        let vectors: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../tests/fixtures/blurhash_vectors.json")).unwrap();
        assert!(vectors.len() > 500);
        for v in &vectors {
            let get = |k: &str| v[k].as_u64().unwrap();
            let (w, h) = (get("w") as u32, get("h") as u32);
            let px = pixels(v["pattern"].as_str().unwrap(), get("seed") as u32, w as usize, h as usize);
            let hash = encode(&px, w, h, get("cx") as u32, get("cy") as u32).unwrap();
            assert_eq!(hash, v["hash"].as_str().unwrap(), "{v}");
        }
    }

    #[test]
    fn a_flat_colour_has_no_ac_energy() {
        // One grey pixel: DC only, every AC component quantises to the middle.
        let hash = encode(&[128, 128, 128, 255], 1, 1, 4, 4).unwrap();
        assert_eq!(hash.len(), 4 + 2 * 16);
        let ac = &hash.as_bytes()[6..];
        assert!(ac.chunks(2).all(|c| c == &ac[..2]));
    }
}

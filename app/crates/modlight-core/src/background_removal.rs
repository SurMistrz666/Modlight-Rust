//! The background remover's model inference (`ShareX.Tools` `BackgroundRemovalService`):
//! the picture resized to the model's input with Catmull-Rom (as Skia draws it), the
//! ImageNet normalisation, the ONNX model run by ONNX Runtime (the `ort` crate, linked in;
//! nothing installed on the system), the output turned into a mask (a sigmoid when the
//! model returns logits, then stretched to 0..1), the mask scaled back and laid over the
//! picture's alpha (`DstIn`).
//!
//! Devices: upstream's GPU is DirectML on the adapter with the most video memory; here it
//! is Core ML (the Neural Engine and the GPU as macOS chooses). "Auto" tries Core ML and
//! falls back to the CPU, as upstream falls back from DirectML.
//!
//! One session is kept (the model, its size and time, the device) so a second picture does
//! not load the model again; [`release`] drops it when the tool closes.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use anyhow::{Context, anyhow, bail};
use image::RgbaImage;

/// `BackgroundRemovalDevice`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RemovalDevice {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

/// What the caller chooses for one removal: the model file and the device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoveBackgroundOptions {
    /// The `.onnx` file; `None` when no model is installed.
    pub model: Option<PathBuf>,
    pub device: RemovalDevice,
}

/// `BackgroundRemovalResult`: the picture and how the run went.
#[derive(Clone, Debug)]
pub struct Removal {
    pub image: RgbaImage,
    pub session_cached: bool,
    pub execution_device: String,
    pub session_ms: u128,
    pub preprocessing_ms: u128,
    pub inference_ms: u128,
    pub postprocessing_ms: u128,
}

/// Removes the background of `image` with the chosen model.
pub fn remove_background(
    image: &RgbaImage,
    options: &RemoveBackgroundOptions,
) -> anyhow::Result<RgbaImage> {
    let Some(model) = &options.model else {
        bail!("The background removal model is not installed.");
    };
    Ok(remove(image, model, options.device)?.image)
}

/// The ImageNet mean and standard deviation the models were trained with.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STANDARD_DEVIATION: [f32; 3] = [0.229, 0.224, 0.225];

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionKey {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    device: RemovalDevice,
}

struct Cached {
    key: SessionKey,
    session: ort::session::Session,
    device: String,
}

static SESSION: Mutex<Option<Cached>> = Mutex::new(None);
static ENVIRONMENT: std::sync::OnceLock<Result<ort::environment::Environment, String>> =
    std::sync::OnceLock::new();

fn environment() -> anyhow::Result<&'static ort::environment::Environment> {
    ENVIRONMENT
        .get_or_init(|| {
            ort::init()
                .with_name("Modlight")
                .with_telemetry(false)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
}

/// Drop the kept session (the tool closed: `BackgroundRemovalService.Dispose`).
pub fn release() {
    if let Ok(mut session) = SESSION.lock() {
        *session = None;
    }
}

/// ONNX Runtime's errors (some carry the builder, which stays on this thread).
fn failed(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{error}")
}

/// `CreateSession`: Core ML for "GPU", the CPU for "CPU", Core ML or else the CPU for
/// "Auto"; returns the session and the name of what runs it.
fn create_session(
    model: &Path,
    device: RemovalDevice,
) -> anyhow::Result<(ort::session::Session, String)> {
    use ort::session::Session;
    use ort::session::builder::GraphOptimizationLevel;
    let env = environment()?;
    let cpu = || -> anyhow::Result<Session> {
        Session::builder(env)
            .map_err(failed)?
            .commit_from_file(model)
            .map_err(failed)
    };
    let core_ml = || -> anyhow::Result<Session> {
        Session::builder(env)
            .map_err(failed)?
            .with_memory_pattern(false)
            .map_err(failed)?
            .with_parallel_execution(false)
            .map_err(failed)?
            .with_optimization_level(GraphOptimizationLevel::All)
            .map_err(failed)?
            .with_execution_providers([ort::ep::CoreML::default().build().error_on_failure()])
            .map_err(failed)?
            .commit_from_file(model)
            .map_err(failed)
    };
    match device {
        RemovalDevice::Cpu => Ok((cpu()?, "CPU".into())),
        RemovalDevice::Gpu => Ok((core_ml()?, "Core ML".into())),
        RemovalDevice::Auto => match core_ml() {
            Ok(session) => Ok((session, "Core ML".into())),
            Err(error) => {
                log::info!("Core ML is unavailable for the background model: {error}");
                Ok((cpu()?, "CPU (GPU unavailable)".into()))
            }
        },
    }
}

/// `RemoveBackground(source, model, device)`.
pub fn remove(image: &RgbaImage, model: &Path, device: RemovalDevice) -> anyhow::Result<Removal> {
    if image.width() == 0 || image.height() == 0 {
        bail!("Selected image is empty.");
    }
    if !model.is_file() {
        bail!(
            "The background removal model file is missing. ({})",
            model.display()
        );
    }
    let mut guard = SESSION
        .lock()
        .map_err(|_| anyhow!("the background removal session is unusable"))?;
    let started = Instant::now();
    let metadata = std::fs::metadata(model).context("model file")?;
    let key = SessionKey {
        path: std::fs::canonicalize(model).unwrap_or_else(|_| model.to_path_buf()),
        size: metadata.len(),
        modified: metadata.modified().ok(),
        device,
    };
    let cached = guard.as_ref().is_some_and(|c| c.key == key);
    if !cached {
        *guard = None;
        let (session, device_name) = create_session(model, device)?;
        *guard = Some(Cached {
            key,
            session,
            device: device_name,
        });
    }
    let session_ms = started.elapsed().as_millis();
    let Some(current) = guard.as_mut() else {
        bail!("the background removal session is unusable");
    };
    let execution_device = current.device.clone();
    let mut removal = run_model(&mut current.session, image)?;
    removal.session_cached = cached;
    removal.execution_device = execution_device;
    removal.session_ms = session_ms;
    Ok(removal)
}

/// `RunModel`.
fn run_model(session: &mut ort::session::Session, source: &RgbaImage) -> anyhow::Result<Removal> {
    let input = session
        .inputs()
        .first()
        .ok_or_else(|| anyhow!("The ONNX model does not expose an input tensor."))?;
    let input_name = input.name().to_string();
    let dimensions: Vec<i64> = match input.dtype() {
        ort::value::ValueType::Tensor { shape, .. } => shape.to_vec(),
        _ => Vec::new(),
    };
    let (width, height, channels_last) = resolve_input_shape(&dimensions);

    let phase = Instant::now();
    let resized = resize_premultiplied(
        &premultiply(source),
        (source.width(), source.height()),
        width,
        height,
    );
    let tensor = input_tensor(&resized, width, height, channels_last);
    let shape: Vec<i64> = if channels_last {
        vec![1, height as i64, width as i64, 3]
    } else {
        vec![1, 3, height as i64, width as i64]
    };
    let preprocessing_ms = phase.elapsed().as_millis();

    let phase = Instant::now();
    let value = ort::value::Tensor::from_array((shape, tensor)).map_err(failed)?;
    let outputs = session
        .run(vec![(
            std::borrow::Cow::<str>::from(input_name),
            ort::session::SessionInputValue::from(value),
        )])
        .map_err(failed)?;
    let (output_shape, values) = match outputs.values().next() {
        Some(output) => {
            let (shape, data) = output
                .try_extract_tensor::<f32>()
                .map_err(|_| anyhow!("The ONNX model did not return a usable output tensor."))?;
            (shape.to_vec(), data.to_vec())
        }
        None => bail!("The ONNX model did not return a usable output tensor."),
    };
    drop(outputs);
    let inference_ms = phase.elapsed().as_millis();

    let phase = Instant::now();
    let (mask_width, mask_height, mask) = mask_from_output(&values, &output_shape)?;
    let mask = resize_alpha(
        &mask,
        mask_width,
        mask_height,
        source.width(),
        source.height(),
    );
    let image = apply_alpha_mask(source, &mask);
    Ok(Removal {
        image,
        session_cached: false,
        execution_device: String::new(),
        session_ms: 0,
        preprocessing_ms,
        inference_ms,
        postprocessing_ms: phase.elapsed().as_millis(),
    })
}

/// `ResolveInputShape`: width, height and whether the channels come last; 1024 x 1024
/// unless the model fixes them.
pub fn resolve_input_shape(dimensions: &[i64]) -> (u32, u32, bool) {
    let mut width = 1024u32;
    let mut height = 1024u32;
    if dimensions.len() != 4 {
        return (width, height, false);
    }
    let channel_index = if dimensions[1] == 3 {
        1
    } else if dimensions[3] == 3 {
        3
    } else {
        1
    };
    let channels_last = channel_index == 3;
    let (height_index, width_index) = if channels_last { (1, 2) } else { (2, 3) };
    if dimensions[height_index] > 0 {
        height = dimensions[height_index] as u32;
    }
    if dimensions[width_index] > 0 {
        width = dimensions[width_index] as u32;
    }
    (width, height, channels_last)
}

/// `ResolveOutputShape`: the mask's width and height.
pub fn resolve_output_shape(dimensions: &[i64]) -> (i64, i64) {
    let n = dimensions.len();
    if n >= 4 {
        let channels_last = dimensions[n - 1] == 1 && dimensions[n - 3] > 1;
        if channels_last {
            (dimensions[n - 2], dimensions[n - 3])
        } else {
            (dimensions[n - 1], dimensions[n - 2])
        }
    } else if n >= 2 {
        (dimensions[n - 1], dimensions[n - 2])
    } else {
        (1, 1)
    }
}

/// Premultiplied RGBA, as Skia keeps a decoded picture (8 bits a channel).
pub fn premultiply(image: &RgbaImage) -> Vec<[u8; 4]> {
    image
        .pixels()
        .map(|p| {
            let a = p[3] as u32;
            let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
            [m(p[0]), m(p[1]), m(p[2]), p[3]]
        })
        .collect()
}

/// Skia's Catmull-Rom cubic (`SKCubicResampler.CatmullRom`, B = 0, C = 1/2).
fn catmull_rom(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

/// The four taps and weights of one destination coordinate (pixel centres mapped, edges
/// clamped, the kernel not widened when shrinking: Skia's cubic sampling).
fn taps(dst: u32, src_len: u32, dst_len: u32) -> ([usize; 4], [f32; 4]) {
    let s = (dst as f32 + 0.5) * src_len as f32 / dst_len as f32 - 0.5;
    let base = s.floor();
    let t = s - base;
    let mut index = [0usize; 4];
    let mut weight = [0f32; 4];
    for k in 0..4 {
        let i = base as i64 - 1 + k as i64;
        index[k] = i.clamp(0, src_len as i64 - 1) as usize;
        weight[k] = catmull_rom(t - (k as f32 - 1.0));
    }
    let sum: f32 = weight.iter().sum();
    if sum.abs() > f32::EPSILON {
        for w in &mut weight {
            *w /= sum;
        }
    }
    (index, weight)
}

/// `ResizeBitmap`: premultiplied pixels drawn into `width` x `height` with the cubic;
/// colours kept within the alpha, 8 bits a channel again.
pub fn resize_premultiplied(
    pixels: &[[u8; 4]],
    src: (u32, u32),
    width: u32,
    height: u32,
) -> Vec<[u8; 4]> {
    let (sw, sh) = src;
    if (sw, sh) == (width, height) {
        return pixels.to_vec();
    }
    // Horizontal pass into floats, then vertical.
    let columns: Vec<([usize; 4], [f32; 4])> = (0..width).map(|x| taps(x, sw, width)).collect();
    let mut horizontal = vec![[0f32; 4]; (width * sh) as usize];
    for y in 0..sh as usize {
        let row = &pixels[y * sw as usize..(y + 1) * sw as usize];
        for (x, (index, weight)) in columns.iter().enumerate() {
            let mut acc = [0f32; 4];
            for k in 0..4 {
                let p = row[index[k]];
                for c in 0..4 {
                    acc[c] += p[c] as f32 * weight[k];
                }
            }
            horizontal[y * width as usize + x] = acc;
        }
    }
    let rows: Vec<([usize; 4], [f32; 4])> = (0..height).map(|y| taps(y, sh, height)).collect();
    let mut out = vec![[0u8; 4]; (width * height) as usize];
    for (y, (index, weight)) in rows.iter().enumerate() {
        for x in 0..width as usize {
            let mut acc = [0f32; 4];
            for k in 0..4 {
                let p = horizontal[index[k] * width as usize + x];
                for c in 0..4 {
                    acc[c] += p[c] * weight[k];
                }
            }
            let a = acc[3].clamp(0.0, 255.0);
            out[y * width as usize + x] = [
                acc[0].clamp(0.0, a).round() as u8,
                acc[1].clamp(0.0, a).round() as u8,
                acc[2].clamp(0.0, a).round() as u8,
                a.round() as u8,
            ];
        }
    }
    out
}

/// `CreateInputTensor`: the premultiplied pixels back to straight colour, scaled to 0..1
/// and normalised, planar (`[1, 3, H, W]`) or interleaved (`[1, H, W, 3]`).
pub fn input_tensor(pixels: &[[u8; 4]], width: u32, height: u32, channels_last: bool) -> Vec<f32> {
    let count = (width * height) as usize;
    let mut out = vec![0f32; count * 3];
    for (i, p) in pixels.iter().enumerate().take(count) {
        let alpha_scale = if p[3] > 0 && p[3] < 255 {
            255.0 / p[3] as f32
        } else {
            1.0
        };
        let channel = |c: usize| {
            ((255f32.min(p[c] as f32 * alpha_scale) / 255.0) - MEAN[c]) / STANDARD_DEVIATION[c]
        };
        let (r, g, b) = (channel(0), channel(1), channel(2));
        if channels_last {
            out[i * 3] = r;
            out[i * 3 + 1] = g;
            out[i * 3 + 2] = b;
        } else {
            out[i] = r;
            out[count + i] = g;
            out[count * 2 + i] = b;
        }
    }
    out
}

/// `CreateMaskBitmap`: the model's output as an 8-bit mask (width, height, alpha values).
pub fn mask_from_output(values: &[f32], dimensions: &[i64]) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let (width, height) = resolve_output_shape(dimensions);
    if width <= 0 || height <= 0 {
        bail!("The ONNX model returned an invalid mask size.");
    }
    let count = (width * height) as usize;
    if values.len() < count {
        bail!("The ONNX model returned an incomplete mask.");
    }
    let mut values: Vec<f32> = values[..count].to_vec();
    let mut min = values.iter().copied().fold(f32::MAX, f32::min);
    let mut max = values.iter().copied().fold(f32::MIN, f32::max);
    if min < 0.0 {
        // Logits: through a sigmoid first.
        for v in &mut values {
            *v = 1.0 / (1.0 + (-*v).exp());
        }
        min = values.iter().copied().fold(f32::MAX, f32::min);
        max = values.iter().copied().fold(f32::MIN, f32::max);
    }
    let range = (max - min).max(0.000_001);
    let mask = values
        .iter()
        .map(|v| {
            let normalized = ((v - min) / range).clamp(0.0, 1.0);
            (normalized * 255.0).round_ties_even() as u8
        })
        .collect();
    Ok((width as u32, height as u32, mask))
}

/// The mask scaled to the picture (the same cubic, alpha only).
pub fn resize_alpha(
    mask: &[u8],
    width: u32,
    height: u32,
    to_width: u32,
    to_height: u32,
) -> Vec<u8> {
    let pixels: Vec<[u8; 4]> = mask.iter().map(|a| [0, 0, 0, *a]).collect();
    resize_premultiplied(&pixels, (width, height), to_width, to_height)
        .into_iter()
        .map(|p| p[3])
        .collect()
}

/// `ApplyAlphaMask`: the picture kept where the mask is (`SKBlendMode.DstIn` on
/// premultiplied pixels, written back as straight colour).
pub fn apply_alpha_mask(source: &RgbaImage, mask: &[u8]) -> RgbaImage {
    let mut out = RgbaImage::new(source.width(), source.height());
    for ((i, p), o) in source.pixels().enumerate().zip(out.pixels_mut()) {
        let m = mask.get(i).copied().unwrap_or(0) as u32;
        let a = p[3] as u32;
        let alpha = (a * m + 127) / 255;
        if alpha == 0 {
            *o = image::Rgba([0, 0, 0, 0]);
            continue;
        }
        let channel = |c: u8| {
            let premultiplied = (c as u32 * a + 127) / 255;
            let masked = (premultiplied * m + 127) / 255;
            ((masked * 255 + alpha / 2) / alpha).min(255) as u8
        };
        *o = image::Rgba([channel(p[0]), channel(p[1]), channel(p[2]), alpha as u8]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_shapes() {
        assert_eq!(
            resolve_input_shape(&[1, 3, 1024, 1024]),
            (1024, 1024, false)
        );
        assert_eq!(resolve_input_shape(&[1, 3, -1, -1]), (1024, 1024, false));
        assert_eq!(resolve_input_shape(&[1, 320, 480, 3]), (480, 320, true));
        assert_eq!(resolve_input_shape(&[1, 3, 512, 768]), (768, 512, false));
        assert_eq!(resolve_input_shape(&[3, 512]), (1024, 1024, false));
        assert_eq!(resolve_output_shape(&[1, 1, 64, 32]), (32, 64));
        assert_eq!(resolve_output_shape(&[1, 64, 32, 1]), (32, 64));
        assert_eq!(resolve_output_shape(&[64, 32]), (32, 64));
        assert_eq!(resolve_output_shape(&[5]), (1, 1));
    }

    #[test]
    fn tensors_normalise_straight_colour() {
        // Half-transparent white premultiplied is 128,128,128,128: white again.
        let pixels = vec![[128u8, 128, 128, 128], [0, 0, 0, 255]];
        let planar = input_tensor(&pixels, 2, 1, false);
        let white_r = (1.0 - MEAN[0]) / STANDARD_DEVIATION[0];
        assert!((planar[0] - white_r).abs() < 1e-4);
        assert!((planar[1] - (-MEAN[0] / STANDARD_DEVIATION[0])).abs() < 1e-4);
        assert!((planar[2] - (1.0 - MEAN[1]) / STANDARD_DEVIATION[1]).abs() < 1e-4);
        let interleaved = input_tensor(&pixels, 2, 1, true);
        assert!((interleaved[0] - white_r).abs() < 1e-4);
        assert!((interleaved[3] - (-MEAN[0] / STANDARD_DEVIATION[0])).abs() < 1e-4);
    }

    #[test]
    fn masks_from_logits_and_probabilities() {
        let (w, h, mask) = mask_from_output(&[0.25, 0.75, 0.5, 1.0], &[1, 1, 2, 2]).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(mask, [0, 170, 85, 255]);
        let (_, _, mask) = mask_from_output(&[-4.0, 4.0, 0.0, 0.0], &[1, 1, 2, 2]).unwrap();
        assert_eq!(mask[0], 0);
        assert_eq!(mask[1], 255);
        assert_eq!(mask[2], 128);
        assert!(mask_from_output(&[1.0], &[1, 1, 2, 2]).is_err());
        assert!(mask_from_output(&[1.0], &[1, 1, 0, 2]).is_err());
    }

    #[test]
    fn cubic_resizes_keep_flat_areas_and_edges() {
        let flat = vec![[40u8, 80, 120, 255]; 16];
        let out = resize_premultiplied(&flat, (4, 4), 9, 3);
        assert!(out.iter().all(|p| *p == [40, 80, 120, 255]));
        let mask = [0u8, 0, 255, 255];
        let wide = resize_alpha(&mask, 4, 1, 8, 1);
        assert_eq!(wide[0], 0);
        assert_eq!(wide[7], 255);
        assert!(wide.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn masks_cut_the_alpha() {
        let mut source = RgbaImage::from_pixel(2, 1, image::Rgba([200, 100, 50, 255]));
        source.put_pixel(1, 0, image::Rgba([200, 100, 50, 128]));
        let out = apply_alpha_mask(&source, &[255, 0]);
        assert_eq!(out.get_pixel(0, 0).0, [200, 100, 50, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [0, 0, 0, 0]);
        let out = apply_alpha_mask(&source, &[128, 255]);
        assert_eq!(out.get_pixel(0, 0)[3], 128);
        assert_eq!(out.get_pixel(1, 0)[3], 128);
        let missing = remove_background(&source, &RemoveBackgroundOptions::default());
        assert!(missing.unwrap_err().to_string().contains("not installed"));
    }

    // A tiny ONNX model written by hand: `ReduceMean` over the channels of a
    // [1, 3, 8, 8] picture, so the "mask" is the picture's normalised brightness.

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }

    fn field_varint(field: u32, v: u64, out: &mut Vec<u8>) {
        varint((field as u64) << 3, out);
        varint(v, out);
    }

    fn field_bytes(field: u32, bytes: &[u8], out: &mut Vec<u8>) {
        varint(((field as u64) << 3) | 2, out);
        varint(bytes.len() as u64, out);
        out.extend_from_slice(bytes);
    }

    fn value_info(name: &str, dims: &[u64]) -> Vec<u8> {
        let mut shape = Vec::new();
        for d in dims {
            let mut dim = Vec::new();
            field_varint(1, *d, &mut dim);
            field_bytes(1, &dim, &mut shape);
        }
        let mut tensor = Vec::new();
        field_varint(1, 1, &mut tensor); // FLOAT
        field_bytes(2, &shape, &mut tensor);
        let mut ty = Vec::new();
        field_bytes(1, &tensor, &mut ty);
        let mut info = Vec::new();
        field_bytes(1, name.as_bytes(), &mut info);
        field_bytes(2, &ty, &mut info);
        info
    }

    fn reduce_mean_model() -> Vec<u8> {
        let mut axes = Vec::new();
        field_bytes(1, b"axes", &mut axes);
        field_varint(8, 1, &mut axes);
        field_varint(20, 7, &mut axes); // INTS
        let mut keepdims = Vec::new();
        field_bytes(1, b"keepdims", &mut keepdims);
        field_varint(3, 1, &mut keepdims);
        field_varint(20, 2, &mut keepdims); // INT
        let mut node = Vec::new();
        field_bytes(1, b"input", &mut node);
        field_bytes(2, b"mask", &mut node);
        field_bytes(4, b"ReduceMean", &mut node);
        field_bytes(5, &axes, &mut node);
        field_bytes(5, &keepdims, &mut node);
        let mut graph = Vec::new();
        field_bytes(1, &node, &mut graph);
        field_bytes(2, b"test", &mut graph);
        field_bytes(11, &value_info("input", &[1, 3, 8, 8]), &mut graph);
        field_bytes(12, &value_info("mask", &[1, 1, 8, 8]), &mut graph);
        let mut opset = Vec::new();
        field_bytes(1, b"", &mut opset);
        field_varint(2, 13, &mut opset);
        let mut model = Vec::new();
        field_varint(1, 8, &mut model);
        field_bytes(2, b"modlight-test", &mut model);
        field_bytes(7, &graph, &mut model);
        field_bytes(8, &opset, &mut model);
        model
    }

    #[test]
    fn a_model_runs_on_the_cpu_and_its_mask_cuts_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("tiny.onnx");
        std::fs::write(&model, reduce_mean_model()).unwrap();
        // Left half white (kept), right half black (removed).
        let mut picture = RgbaImage::from_pixel(32, 24, image::Rgba([0, 0, 0, 255]));
        for y in 0..24 {
            for x in 0..16 {
                picture.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let first = remove(&picture, &model, RemovalDevice::Cpu).unwrap();
        assert_eq!(first.execution_device, "CPU");
        assert!(!first.session_cached);
        assert_eq!(first.image.dimensions(), (32, 24));
        assert_eq!(first.image.get_pixel(2, 12).0, [255, 255, 255, 255]);
        assert_eq!(first.image.get_pixel(30, 12)[3], 0);
        let second = remove(&picture, &model, RemovalDevice::Cpu).unwrap();
        assert!(second.session_cached);
        let options = RemoveBackgroundOptions {
            model: Some(model.clone()),
            device: RemovalDevice::Cpu,
        };
        assert_eq!(remove_background(&picture, &options).unwrap(), first.image);
        release();
        let third = remove(&picture, &model, RemovalDevice::Cpu).unwrap();
        assert!(!third.session_cached);
        // Core ML (or the CPU when it cannot take the model) gives the same cut.
        let auto = remove(&picture, &model, RemovalDevice::Auto).unwrap();
        eprintln!("Auto ran on: {}", auto.execution_device);
        assert!(
            ["Core ML", "CPU (GPU unavailable)"].contains(&auto.execution_device.as_str()),
            "{}",
            auto.execution_device
        );
        assert_eq!(auto.image.get_pixel(2, 12).0, [255, 255, 255, 255]);
        assert_eq!(auto.image.get_pixel(30, 12)[3], 0);
        release();
        assert!(remove(&picture, &dir.path().join("none.onnx"), RemovalDevice::Cpu).is_err());
    }
}

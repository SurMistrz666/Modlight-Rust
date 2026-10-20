//! The background remover's model inference (`ShareX.Tools` BackgroundRemovalService):
//! one narrow entry point. The ONNX model runs here once the tools port fills it in;
//! until then every call reports that the model is not installed.

use image::RgbaImage;

/// What the caller chooses for one removal (model and device; filled in with the tool).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoveBackgroundOptions {}

/// Removes the background of `image` with the downloaded model.
pub fn remove_background(
    image: &RgbaImage,
    options: &RemoveBackgroundOptions,
) -> anyhow::Result<RgbaImage> {
    let _ = (image, options);
    anyhow::bail!("The background removal model is not installed.")
}

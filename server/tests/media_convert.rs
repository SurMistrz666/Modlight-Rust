//! tests/integration/convert.test.ts: the libvips conversion engine on real
//! encoders, plus the libvips operations the media processing port added
//! (join, flatten, composite, text).

mod media_common;

use bytes::Bytes;
use media_common::*;
use modlight_server::media::convert::{
    ConvertOptions, ConvertTarget, conversion_budget_for, convert_image_buffer, probe_image,
};
use modlight_server::media::vips::{CompositeLayer, PngOptions, Sharp, TextInput, WebpOptions};

const FRAMES: u32 = 4;
const FRAME_W: u32 = 16;
const FRAME_H: u32 = 12;

async fn sample_image() -> Bytes {
    png_of(solid(32, 24, [120, 80, 200]), 32, 24, 3).await
}

async fn animated() -> Bytes {
    animated_gif_with(FRAMES, FRAME_W, FRAME_H, |i| [(40 + i * 50) as u8, 20, (200 - i * 40) as u8]).await
}

async fn meta(buf: &[u8], animated: bool) -> modlight_server::media::vips::Metadata {
    Sharp::new(Bytes::copy_from_slice(buf)).animated(animated).metadata().await.unwrap()
}

#[tokio::test]
async fn converts_to_png_jpg_webp_with_the_right_mime_and_ext() {
    let src = sample_image().await;
    for (to, mime, ext) in [
        (ConvertTarget::Png, "image/png", "png"),
        (ConvertTarget::Jpg, "image/jpeg", "jpg"),
        (ConvertTarget::Webp, "image/webp", "webp"),
    ] {
        let r = convert_image_buffer(src.clone(), to, ConvertOptions::default()).await.unwrap();
        assert_eq!((r.mime, r.ext), (mime, ext));
        assert!(!r.buffer.is_empty());
    }
}

#[tokio::test]
async fn wraps_an_image_into_a_valid_pdf_with_the_jpeg_as_is() {
    let src = sample_image().await;
    let r = convert_image_buffer(src, ConvertTarget::Pdf, ConvertOptions::default()).await.unwrap();
    assert_eq!((r.mime, r.ext), ("application/pdf", "pdf"));
    assert_eq!(&r.buffer[..5], b"%PDF-");
    let text = String::from_utf8_lossy(&r.buffer);
    assert!(text.contains("DCTDecode"));
    assert!(!text.contains("FlateDecode/Image"));
    assert!(text.contains("/Width 32\n/Height 24\n/ColorSpace /DeviceRGB"));
}

#[tokio::test]
async fn flattens_transparency_onto_white() {
    let rgba: Vec<u8> = std::iter::repeat_n([0u8, 0, 0, 0], 64).flatten().collect();
    let transparent = png_of(rgba, 8, 8, 4).await;
    let r = convert_image_buffer(transparent.clone(), ConvertTarget::Pdf, ConvertOptions::default()).await.unwrap();
    assert_eq!(&r.buffer[..5], b"%PDF-");
    // The flatten itself: a transparent pixel becomes white, not black.
    let white = Sharp::new(transparent).flatten([255.0, 255.0, 255.0]).raw_output().to_buffer().await.unwrap();
    assert!(white.iter().all(|b| *b == 255), "{:?}", &white[..6]);
}

#[tokio::test]
async fn reencodes_an_animated_gif_as_an_animated_webp_with_every_frame() {
    let src = animated().await;
    let probe = probe_image(src).await.unwrap();
    assert!(probe.animated);
    assert_eq!(probe.pages, FRAMES);
    assert_eq!(probe.height, FRAME_H);
    let budget = conversion_budget_for(&probe, ConvertTarget::Webp);
    assert!(budget.ok && budget.animated);
    let out = convert_image_buffer(
        probe.input.clone(),
        ConvertTarget::Webp,
        ConvertOptions { animated: true, ..Default::default() },
    )
    .await
    .unwrap();
    let m = meta(&out.buffer, true).await;
    assert_eq!(m.pages, Some(FRAMES));
    assert_eq!(m.page_height, Some(FRAME_H));
}

#[tokio::test]
async fn returns_a_single_frame_for_the_targets_that_cannot_animate() {
    let src = animated().await;
    let probe = probe_image(src.clone()).await.unwrap();
    let budget = conversion_budget_for(&probe, ConvertTarget::Png);
    assert!(budget.ok && !budget.animated);
    let out = convert_image_buffer(src, ConvertTarget::Png, ConvertOptions { animated: true, ..Default::default() })
        .await
        .unwrap();
    let m = meta(&out.buffer, false).await;
    assert_eq!(m.height, FRAME_H);
    assert_eq!(m.pages.unwrap_or(1), 1);
}

#[tokio::test]
async fn reports_undecodable_input_and_opens_an_avis_cover() {
    assert!(probe_image(Bytes::from_static(b"not an image at all")).await.is_none());
    let sequence = avif_sequence_brand().await;
    assert!(Sharp::new(sequence.clone()).metadata().await.is_err());
    let probe = probe_image(sequence).await.unwrap();
    assert!(probe.cover_only);
    let budget = conversion_budget_for(&probe, ConvertTarget::Webp);
    assert!(budget.ok && !budget.animated);
    let out = convert_image_buffer(probe.input.clone(), ConvertTarget::Png, ConvertOptions::default()).await.unwrap();
    assert_eq!(meta(&out.buffer, false).await.width, 16);
}

#[tokio::test]
async fn an_aborted_signal_is_thrown_before_any_work() {
    let controller = modlight_server::media::abort::AbortController::new();
    controller.abort();
    let err = convert_image_buffer(
        sample_image().await,
        ConvertTarget::Png,
        ConvertOptions { signal: Some(controller.signal()), ..Default::default() },
    )
    .await
    .unwrap_err();
    assert!(err.downcast_ref::<modlight_server::media::transform_queue::AbortedError>().is_some());
}

#[tokio::test]
async fn joins_separate_frames_into_one_animation() {
    let frames: Vec<Bytes> = futures_join(3).await;
    let webp = Sharp::join(frames, 1, true)
        .webp(WebpOptions {
            quality: 45,
            effort: 3,
            loop_count: Some(0),
            delay: Some(vec![83, 83, 83]),
            ..Default::default()
        })
        .to_buffer()
        .await
        .unwrap();
    assert_eq!(modlight_server::media::motion_preview::count_animated_webp_frames(&webp), 3);
    let m = meta(&webp, true).await;
    assert_eq!((m.pages, m.page_height, m.width), (Some(3), Some(10), 12));
    assert_eq!(m.delay, Some(vec![83, 83, 83]));
    assert!(Sharp::join(vec![still(4, 4).await], 1, true).to_buffer().await.is_err());
}

async fn futures_join(n: u32) -> Vec<Bytes> {
    let mut out = Vec::new();
    for i in 0..n {
        out.push(png_of(solid(12, 10, [(i * 80) as u8, 30, 90]), 12, 10, 3).await);
    }
    out
}

#[tokio::test]
async fn composites_a_layer_over_an_image_at_an_offset() {
    let base = png_of(solid(20, 10, [0, 0, 255]), 20, 10, 3).await;
    let layer = png_of(solid(4, 2, [255, 0, 0]), 4, 2, 3).await;
    let out = Sharp::new(base)
        .composite(vec![CompositeLayer { input: layer, left: 3, top: 5 }])
        .raw_output()
        .to_buffer()
        .await
        .unwrap();
    // RGBA: the composite added an alpha channel, as sharp does.
    let px = |x: usize, y: usize| &out[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4];
    assert_eq!(px(3, 5), [255, 0, 0, 255]);
    assert_eq!(px(0, 0), [0, 0, 255, 255]);
    let too_big = png_of(solid(30, 2, [1, 2, 3]), 30, 2, 3).await;
    let err = Sharp::new(png_of(solid(20, 10, [0, 0, 255]), 20, 10, 3).await)
        .composite(vec![CompositeLayer { input: too_big, left: 0, top: 0 }])
        .to_buffer()
        .await
        .unwrap_err();
    assert_eq!(err.message(), "Image to composite must have same dimensions or smaller");
}

#[tokio::test]
async fn renders_pango_markup_with_an_explicit_font_file() {
    let font = format!("{}/public/fonts/Geist-Bold.ttf", env!("CARGO_MANIFEST_DIR"));
    let png = Sharp::text(TextInput {
        text: r##"<span foreground="#a1b2c3" letter_spacing="6144">ZIP</span>"##.into(),
        font: Some("Geist 78px".into()),
        fontfile: Some(font),
        rgba: true,
    })
    .png(PngOptions::default())
    .to_buffer()
    .await
    .unwrap();
    let m = meta(&png, false).await;
    // Measured on the reference container (sharp 0.34.5, libvips 8.17.3):
    // the same markup, font and file come out 128x56.
    assert_eq!((m.width, m.height, m.channels), (128, 56, 4));
}

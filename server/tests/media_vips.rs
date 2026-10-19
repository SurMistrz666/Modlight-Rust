//! The libvips binding itself (`media::vips`): what sharp guarantees and the
//! server relies on, checked against real libvips.

mod media_common;

use std::time::{Duration, Instant};

use bytes::Bytes;
use media_common::{animated_gif, bmp, jpeg_with_exif, noise, png_claiming, solid, still};
use modlight_server::media::transform_cost::{THUMBNAIL_TIMEOUT_MS, THUMBNAIL_WEIGHT};
use modlight_server::media::transform_queue::{RunOptions, image_transforms};
use modlight_server::media::vips::image::band_format;
use modlight_server::media::vips::{
    self, Arg, FailOn, Fit, GifOptions, Image, Operation, Resize, Sharp, VipsError, WebpOptions,
    sharp::DEFAULT_LIMIT_INPUT_PIXELS,
};

#[tokio::test]
async fn refuses_the_formats_production_libvips_cannot_load() {
    // This libvips may carry ImageMagick, which reads BMP; sharp-libvips does
    // not, and neither may we.
    let err = Sharp::new(bmp(4, 3)).metadata().await.unwrap_err();
    assert_eq!(err.message(), "Input buffer contains unsupported image format");
    let err = Sharp::new(bmp(4, 3)).png(Default::default()).to_buffer().await.unwrap_err();
    assert_eq!(err.message(), "Input buffer contains unsupported image format");
    // ...and an AVIF sequence brand, which ImageMagick would also take.
    let err = Sharp::new(media_common::avif_sequence_brand().await).metadata().await.unwrap_err();
    assert_eq!(err.message(), "Input buffer contains unsupported image format");
    let err = Sharp::new(Bytes::new()).metadata().await.unwrap_err();
    assert_eq!(err.message(), "Input Buffer is empty");
}

#[tokio::test]
async fn applies_sharps_default_pixel_limit_unless_told_otherwise() {
    // 16384 x 16384 is just over 0x3FFF squared.
    let bomb = png_claiming(16_384, 16_384);
    const { assert!(16_384u64 * 16_384 > DEFAULT_LIMIT_INPUT_PIXELS) };
    let err = Sharp::new(bomb.clone()).metadata().await.unwrap_err();
    assert_eq!(err.message(), "Input image exceeds pixel limit");
    // limitInputPixels: false reads the header and nothing else.
    let meta = Sharp::new(bomb).fail_on(FailOn::None).limit_input_pixels(None).metadata().await.unwrap();
    assert_eq!((meta.width, meta.height), (16_384, 16_384));
}

#[tokio::test]
async fn reports_frames_page_height_delay_and_loop_of_an_animation() {
    let gif = animated_gif(5).await;
    let first_page = Sharp::new(gif.clone()).metadata().await.unwrap();
    assert_eq!((first_page.format, first_page.width, first_page.height), ("gif", 12, 10));
    assert_eq!(first_page.pages, Some(5));

    let all = Sharp::new(gif).animated(true).metadata().await.unwrap();
    assert_eq!((all.width, all.height, all.page_height), (12, 50, Some(10)));
    assert_eq!(all.loop_count, Some(0));
}

#[tokio::test]
async fn repeats_a_single_delay_over_every_page_it_knows_about() {
    // SetAnimationProperties: one delay is repeated for all nPages - where
    // nPages is what the INPUT options resolved, not the strip's page count.
    // A joined strip loaded as one page (upstream's test fixtures) keeps a
    // one-entry delay list; loaded animated, every frame gets it.
    let raw: Vec<u8> = (0..4).flat_map(|i| solid(6, 4, [i * 60, 0, 0])).collect();
    let animated = Sharp::raw(Bytes::from(raw), 6, 16, 3)
        .raw_page_height(4)
        .animated(true)
        .gif(GifOptions { delay: Some(vec![120]), loop_count: Some(2), ..GifOptions::default() })
        .to_buffer()
        .await
        .unwrap();
    let meta = Sharp::new(Bytes::from(animated)).animated(true).metadata().await.unwrap();
    assert_eq!(meta.pages, Some(4));
    assert_eq!(meta.delay, Some(vec![120; 4]));
    assert_eq!(meta.loop_count, Some(2));
}

#[tokio::test]
async fn orients_by_exif_and_sizes_the_result_as_seen() {
    // 40x20 stored, orientation 6: the picture people see is 20x40.
    let jpeg = jpeg_with_exif(40, 20, Some(6));
    assert_eq!(Sharp::new(jpeg.clone()).metadata().await.unwrap().orientation, Some(6));

    let (out, info) = Sharp::new(jpeg)
        .rotate()
        .resize(Resize::to(Some(10), None))
        .png(Default::default())
        .to_buffer_with_info()
        .await
        .unwrap();
    assert_eq!((info.width, info.height), (10, 20));
    let meta = Sharp::new(Bytes::from(out)).metadata().await.unwrap();
    assert_eq!((meta.width, meta.height, meta.orientation), (10, 20, None));
}

#[tokio::test]
async fn raw_output_is_rgba_after_ensure_alpha_even_for_greyscale() {
    // A real one-channel PNG (sharp itself would write RGB: its output
    // colourspace defaults to sRGB).
    let grey = vips::run_blocking(|| {
        let image =
            Image::from_memory(&[128u8; 10 * 6], 10, 6, 1, band_format::UCHAR)?.copy_with_interpretation("b-w")?;
        image.save_buffer(Operation::new("pngsave_buffer")?)
    })
    .await
    .map(Bytes::from)
    .unwrap();
    assert_eq!(Sharp::new(grey.clone()).metadata().await.unwrap().channels, 1);
    // ...and the default output colourspace is sRGB, as in sharp.
    let rgb = Sharp::new(grey.clone()).png(Default::default()).to_buffer().await.unwrap();
    assert_eq!(Sharp::new(Bytes::from(rgb)).metadata().await.unwrap().channels, 3);

    let (data, info) = Sharp::new(grey).raw_output().ensure_alpha(1.0).to_buffer_with_info().await.unwrap();
    assert_eq!((info.width, info.height, info.channels, info.format), (10, 6, 4, "raw"));
    assert_eq!(data.len(), 10 * 6 * 4);
    assert_eq!(&data[..4], &[128, 128, 128, 255]);
}

#[tokio::test]
async fn timeout_kills_an_evaluation_that_reports_progress() {
    // sharp's `.timeout({ seconds })`: measured upstream as "VipsImage: killed"
    // after about a second on a progressive encode.
    let (w, h) = (3000, 3000);
    let raw = noise(w * h * 3, 7);
    let started = Instant::now();
    let result = vips::run_blocking(move || {
        let image = Image::from_memory(&raw, w as i32, h as i32, 3, band_format::UCHAR)?;
        let blurred = Operation::new("gaussblur")?.set("in", &image).set("sigma", 300.0).call()?.image("out")?;
        blurred.set_timeout(1);
        blurred.write_to_memory()
    })
    .await;
    let err = result.expect_err("a 300-sigma blur of 9 Mpx cannot finish in a second");
    assert!(err.is_timeout(), "{err}");
    assert!(err.message().contains("killed"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
}

#[tokio::test]
async fn checks_argument_names_and_types_before_reaching_gobject() {
    let image = vips::run_blocking(|| Image::from_memory(&[0u8; 12], 2, 2, 3, band_format::UCHAR)).await.unwrap();
    let wrong_type = Operation::new("resize").unwrap().set("in", &image).set("scale", 1).call();
    assert_eq!(wrong_type.err().map(|e| e.message().to_string()), Some("resize: wrong value type for scale".into()));
    let unknown = Operation::new("resize").unwrap().set("in", &image).set("sclae", 1.0).call();
    assert_eq!(unknown.err().map(|e| e.message().to_string()), Some("resize: no argument named sclae".into()));
    let bad_nick =
        Operation::new("resize").unwrap().set("in", &image).set("scale", 1.0).set("kernel", Arg::Nick("nope")).call();
    assert!(bad_nick.is_err());
    assert!(Operation::new("no_such_operation").is_err());
    // An argument only newer releases have is skipped when unknown.
    let fine = Operation::new("resize")
        .unwrap()
        .set("in", &image)
        .set("scale", 0.5)
        .set_if_known("no_such_option", true)
        .call();
    assert_eq!(fine.unwrap().image("out").unwrap().width(), 1);
}

/// app/api/background/route.ts, lines 70-84: the wallpaper encode, through the
/// shared limiter at thumbnail weight.
async fn background(input: Bytes) -> anyhow::Result<Vec<u8>> {
    const MAX_INPUT_PIXELS: u64 = 64_000_000;
    const BACKGROUND_MAX_EDGE: u32 = 2560;
    image_transforms()
        .run(
            move |_signal| async move {
                Ok(Sharp::new(input)
                    .limit_input_pixels(Some(MAX_INPUT_PIXELS))
                    .animated(false)
                    .timeout(THUMBNAIL_TIMEOUT_MS.div_ceil(1000) as u32)
                    .rotate()
                    .resize(
                        Resize::to(Some(BACKGROUND_MAX_EDGE), Some(BACKGROUND_MAX_EDGE))
                            .fit(Fit::Inside)
                            .without_enlargement(true),
                    )
                    .webp(WebpOptions { quality: 82, ..WebpOptions::default() })
                    .to_buffer()
                    .await?)
            },
            RunOptions { weight: Some(THUMBNAIL_WEIGHT), timeout_ms: Some(THUMBNAIL_TIMEOUT_MS), signal: None },
        )
        .await
}

#[tokio::test]
async fn expresses_the_background_route_encode() {
    let wide = media_common::jpeg_of(solid(3000, 2000, [30, 60, 90]), 3000, 2000).await;
    let out = background(wide).await.unwrap();
    let meta = Sharp::new(Bytes::from(out)).metadata().await.unwrap();
    assert_eq!((meta.format, meta.width, meta.height), ("webp", 2560, 1707));

    // Small pictures are never enlarged.
    let out = background(still(300, 200).await).await.unwrap();
    let meta = Sharp::new(Bytes::from(out)).metadata().await.unwrap();
    assert_eq!((meta.width, meta.height), (300, 200));

    // The decompression bomb is refused by the decoder, not discovered by the encoder.
    let err = background(png_claiming(10_000, 10_000)).await.unwrap_err();
    assert_eq!(err.downcast_ref::<VipsError>().map(VipsError::message), Some("Input image exceeds pixel limit"));
    // Anything libvips cannot decode is "not an image" to the route.
    assert!(background(Bytes::from_static(b"nope")).await.unwrap_err().is::<VipsError>());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_pipelines_at_once_share_libvips_safely() {
    let source = still(320, 240).await;
    let jobs: Vec<_> = (0..24)
        .map(|i| {
            let source = source.clone();
            tokio::spawn(async move {
                Sharp::new(source)
                    .rotate()
                    .resize(Resize::to(Some(32 + i), Some(32)))
                    .webp(WebpOptions::default())
                    .to_buffer_with_info()
                    .await
                    .map(|(_, info)| info.width)
            })
        })
        .collect();
    for (i, job) in jobs.into_iter().enumerate() {
        assert_eq!(job.await.unwrap().unwrap(), 32 + i as u32);
    }
}

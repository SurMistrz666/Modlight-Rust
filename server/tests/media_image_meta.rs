//! tests/integration/image-meta.test.ts, against real libvips.

mod media_common;

use bytes::Bytes;
use media_common::{animated_gif, avif_sequence_brand, still};
use modlight_server::media::avif_brand::with_still_avif_brand;
use modlight_server::media::image::{
    ImageMeta, ThumbnailFit, ThumbnailOptions, image_meta_from_buffer, make_thumbnail,
};
use modlight_server::media::vips::Sharp;

// describe("image metadata")

#[tokio::test]
async fn counts_frames_so_the_conversion_menu_can_describe_this_file() {
    assert_eq!(image_meta_from_buffer(still(24, 16).await).await.frames, Some(1));
    assert_eq!(image_meta_from_buffer(animated_gif(5).await).await.frames, Some(5));
}

#[tokio::test]
async fn reports_dimensions_and_a_blurhash_for_an_ordinary_image() {
    let meta = image_meta_from_buffer(still(24, 16).await).await;
    assert_eq!(meta.width, Some(24));
    assert_eq!(meta.height, Some(16));
    assert!(meta.blurhash.as_deref().is_some_and(|h| !h.is_empty()));
}

#[tokio::test]
async fn returns_nothing_at_all_for_something_that_is_not_an_image() {
    assert_eq!(image_meta_from_buffer(Bytes::from_static(b"not an image")).await, ImageMeta::default());
    assert_eq!(image_meta_from_buffer(Bytes::new()).await, ImageMeta::default());
}

// describe("the AVIF sequence brand libvips refuses")

#[tokio::test]
async fn the_avis_brand_still_gets_dimensions_a_frame_count_and_a_blurhash() {
    // Production has exactly one such file. It converted perfectly (the convert
    // path carried the workaround) and yet had no preview, no width/height and
    // no placeholder, because this path opened the buffer with a plain sharp()
    // call that libvips rejects on the major brand alone.
    let seq = avif_sequence_brand().await;
    assert!(Sharp::new(seq.clone()).metadata().await.is_err());

    let meta = image_meta_from_buffer(seq).await;
    assert_eq!(meta.width, Some(16));
    assert_eq!(meta.height, Some(16));
    // Frame count is left UNKNOWN, not 1. Only the cover is reachable through
    // libvips, but the file itself holds a whole sequence - and since ffmpeg
    // arrived, something in this system can read it. NULL already means
    // "unknown" on this column.
    assert_eq!(meta.frames, None);
    assert!(meta.blurhash.is_some());
}

#[tokio::test]
async fn the_avis_brand_can_be_thumbnailed_once_the_reader_has_been_talked_round() {
    let seq = avif_sequence_brand().await;
    // make_thumbnail takes the buffer probe_image handed back, which is the point:
    // the route probes first and passes `probe.input` on.
    let readable = with_still_avif_brand(&seq).expect("patched");

    let thumb = make_thumbnail(
        readable,
        ThumbnailOptions { width: Some(8), fit: ThumbnailFit::Inside, ..ThumbnailOptions::default() },
    )
    .await
    .unwrap();
    assert_eq!(Sharp::new(Bytes::from(thumb)).metadata().await.unwrap().format, "webp");
}

#[tokio::test]
async fn leaves_a_well_formed_file_alone() {
    assert_eq!(with_still_avif_brand(&still(24, 16).await), None);
    assert_eq!(with_still_avif_brand(&[0u8; 4]), None);
}

// The thumbnail encoder (makeThumbnail), beyond the upstream file.

#[tokio::test]
async fn thumbnails_are_a_cover_crop_that_never_enlarges() {
    let src = still(400, 200).await;
    let webp =
        make_thumbnail(src.clone(), ThumbnailOptions { width: Some(64), height: Some(64), ..Default::default() })
            .await
            .unwrap();
    let meta = Sharp::new(Bytes::from(webp)).metadata().await.unwrap();
    assert_eq!((meta.format, meta.width, meta.height), ("webp", 64, 64));

    // A 400x200 source asked for 800x800 stays exactly as it is: with
    // `withoutEnlargement` sharp clamps the crop box to the source, so the
    // "square" is abandoned (the measurement avatar.rs works around).
    let big = make_thumbnail(src, ThumbnailOptions { width: Some(800), height: Some(800), ..Default::default() })
        .await
        .unwrap();
    let meta = Sharp::new(Bytes::from(big)).metadata().await.unwrap();
    assert_eq!((meta.width, meta.height), (400, 200));
}

#[tokio::test]
async fn thumbnails_can_be_avif() {
    use modlight_server::media::image::ThumbnailFormat;
    let avif = make_thumbnail(
        still(64, 48).await,
        ThumbnailOptions {
            width: Some(32),
            format: ThumbnailFormat::Avif,
            timeout_ms: Some(10_000),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let meta = Sharp::new(Bytes::from(avif)).metadata().await.unwrap();
    assert_eq!(meta.format, "heif");
    assert_eq!(meta.compression.as_deref(), Some("av1"));
    assert_eq!((meta.width, meta.height), (32, 24));
}

//! tests/integration/avatar-encode.test.ts, against real libvips.
//!
//! The encoder against real libvips, because the two claims that matter cannot be
//! made by a mock: that an animated GIF comes out still animated, and that the
//! budgets turn an abusive one into something a browser can hold.
//!
//! The animation claim in particular is worth a real encode. libvips reads page
//! zero and nothing else unless it is opened with `n: -1` (`animated: true`), so
//! the difference between a working feature and a frozen one is a single option
//! that no type checks and no unit test with a stubbed encoder would notice.

mod media_common;

use bytes::Bytes;
use media_common::{animated_gif_with, gif_of_pages, jpeg_of, jpeg_with_exif, noise, pages_of, png_of, solid};
use modlight_server::media::avatar::{AvatarDegradation, AvatarEncodeError, AvatarEncodeResult, encode_avatar};
use modlight_server::media::vips::Sharp;

const AVATAR_SIZE: u32 = 256;
const AVATAR_MAX_FRAMES: u32 = 120;
const AVATAR_MAX_OUTPUT_BYTES: usize = 512 * 1024;

/// A real animated GIF with `n` differently coloured pages.
async fn animated_gif(n: u32, w: u32, h: u32) -> Bytes {
    animated_gif_with(n, w, h, |i| [((20 + i * 7) % 256) as u8, ((10 + i * 13) % 256) as u8, 200]).await
}

/// Incompressible frames, so an animated encode cannot come out tiny.
async fn noisy_gif(n: u32, edge: u32) -> Bytes {
    let raw = noise((edge * edge * 3 * n) as usize, 0x5eed);
    gif_of_pages(raw, edge, edge, n, 60).await
}

fn ok(result: AvatarEncodeResult) -> (Bytes, bool, u32, Option<AvatarDegradation>) {
    match result {
        AvatarEncodeResult::Ok { bytes, animated, frames, degraded } => (bytes, animated, frames, degraded),
        AvatarEncodeResult::Failed { error } => panic!("refused: {}", error.as_str()),
    }
}

#[tokio::test]
async fn keeps_a_gif_animated_frame_for_frame() {
    let source = animated_gif(6, 60, 40).await;
    assert_eq!(pages_of(&source).await, 6);

    let (bytes, animated, frames, _) = ok(encode_avatar(source).await.unwrap());
    assert!(animated);
    assert_eq!(frames, 6);
    // The claim, measured on the bytes rather than on our own bookkeeping.
    assert_eq!(pages_of(&bytes).await, 6);

    let meta = Sharp::new(bytes).animated(true).metadata().await.unwrap();
    assert_eq!(meta.format, "webp");
    // Square, and never larger than the stored edge.
    assert!(meta.width <= AVATAR_SIZE);
    assert_eq!(Some(meta.width), meta.page_height);
}

#[tokio::test]
async fn produces_a_square_that_is_small_enough_to_sit_on_every_page() {
    let source = jpeg_of(solid(1600, 1200, [10, 120, 200]), 1600, 1200).await;

    let (bytes, animated, _, degraded) = ok(encode_avatar(source).await.unwrap());
    let meta = Sharp::new(bytes.clone()).metadata().await.unwrap();
    assert_eq!(meta.width, AVATAR_SIZE);
    assert_eq!(meta.height, AVATAR_SIZE);
    assert!(!animated);
    assert_eq!(degraded, None);
    assert!(bytes.len() < AVATAR_MAX_OUTPUT_BYTES);
}

#[tokio::test]
async fn never_upscales_a_source_smaller_than_the_stored_edge() {
    let source = png_of(solid(48, 48, [1, 2, 3]), 48, 48, 3).await;
    let (bytes, ..) = ok(encode_avatar(source).await.unwrap());
    assert_eq!(Sharp::new(bytes).metadata().await.unwrap().width, 48);
}

#[tokio::test]
async fn strips_exif_so_a_photograph_does_not_publish_where_it_was_taken() {
    let source = jpeg_with_exif(400, 400, None);
    assert!(Sharp::new(source.clone()).metadata().await.unwrap().exif.is_some());

    let (bytes, ..) = ok(encode_avatar(source).await.unwrap());
    assert_eq!(Sharp::new(bytes).metadata().await.unwrap().exif, None);
}

#[tokio::test]
async fn saves_a_sequence_past_the_frame_cap_as_a_still_and_says_which_cap_it_was() {
    // Tiny frames on purpose: the pixel budget is nowhere near, so only the
    // count can be what refused this. The owner still gets an avatar.
    let source = animated_gif(AVATAR_MAX_FRAMES + 1, 8, 8).await;
    let (bytes, animated, frames, degraded) = ok(encode_avatar(source).await.unwrap());
    assert!(!animated);
    assert_eq!(frames, 1);
    assert_eq!(degraded, Some(AvatarDegradation::Frames));
    assert_eq!(pages_of(&bytes).await, 1);
}

#[tokio::test]
async fn falls_back_to_a_still_when_the_animation_encodes_too_heavy_to_serve() {
    // Inside every pixel and frame budget, and still fat: 40 frames of noise
    // clear the ceilings and produce far more than half a megabyte of WebP. The
    // frame budget bounds the ENCODE; this is the guard on the RESULT, which is
    // the only number the viewer feels.
    let source = noisy_gif(40, 280).await;
    let (bytes, animated, _, degraded) = ok(encode_avatar(source).await.unwrap());
    assert!(bytes.len() <= AVATAR_MAX_OUTPUT_BYTES);
    assert!(!animated);
    assert_eq!(degraded, Some(AvatarDegradation::OutputSize));
}

#[tokio::test]
async fn answers_not_an_image_rather_than_failing() {
    assert_eq!(
        encode_avatar(Bytes::from_static(b"this is not an image")).await.unwrap(),
        AvatarEncodeResult::Failed { error: AvatarEncodeError::Decode }
    );
}

#[tokio::test]
async fn refuses_a_single_frame_over_the_bomb_ceiling() {
    // 10000 x 10000 declared in a few hundred bytes: refused from the header.
    let source = media_common::png_claiming(10_000, 10_000);
    assert_eq!(
        encode_avatar(source).await.unwrap(),
        AvatarEncodeResult::Failed { error: AvatarEncodeError::TooManyPixels }
    );
}

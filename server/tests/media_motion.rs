//! tests/integration/motion.test.ts: the moving thumbnail on real encoders.

mod common;
mod media_common;

use bytes::Bytes;
use media_common::*;
use modlight_server::db::models::Upload;
use modlight_server::media::abort::AbortController;
use modlight_server::media::motion::{MotionFit, MotionOptions, motion_preview};
use modlight_server::media::motion_preview::{MotionRefusal, count_animated_webp_frames};
use modlight_server::media::transform_cost::{
    MAX_TRANSFORM_BYTES, MOTION_FRAMES, max_motion_source_pixels, motion_page_budget,
};
use modlight_server::media::vips::{GifOptions, Sharp};
use modlight_server::state::AppState;
use modlight_server::storage::{LocalDriver, Storage};

const OPTS: MotionOptions = MotionOptions { width: Some(128), height: Some(128), fit: MotionFit::Cover };

fn state(root: &std::path::Path) -> AppState {
    let mut s = common::state_with(&[
        ("APP_URL", "http://127.0.0.1:8421"),
        ("NODE_ENV", "production"),
        ("SKIP_ENV_VALIDATION", "1"),
        ("UPLOAD_TMP_PATH", &root.join("tmp").to_string_lossy()),
    ]);
    s.storage = Storage::with_driver(std::sync::Arc::new(LocalDriver::new(&root.to_string_lossy())));
    s
}

fn row(mime: &str, ext: &str, frames: Option<i32>, size: usize, key: &str, checksum: Option<&str>) -> Upload {
    serde_json::from_value(serde_json::json!({
        "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "ownerId": "u", "code": "abc123", "filename": format!("f.{ext}"),
        "ext": ext, "kind": "file", "language": null, "mimeType": mime, "size": size, "storageKey": key,
        "checksum": checksum, "width": null, "height": null, "blurhash": null, "thumbKey": null, "published": true,
        "deleteToken": "t", "views": 0, "downloads": 0, "lastAccessedAt": null, "expiresAt": null, "maxViews": null,
        "passwordHash": null, "folderId": null, "deletedAt": null, "createdAt": "2026-10-20T00:00:00.000Z",
        "updatedAt": "2026-10-20T00:00:00.000Z", "passwordVersion": null, "frames": frames, "convertedAt": null,
        "originalExt": null, "durationSeconds": null, "fps": null
    }))
    .unwrap()
}

/// Frames that really differ: a moving diagonal stripe pattern.
async fn striped_gif(n: u32, w: u32, h: u32) -> Bytes {
    let mut raw = Vec::with_capacity((w * h * 3 * n) as usize);
    for i in 0..n {
        for y in 0..h {
            for x in 0..w {
                let on = (x + y + i * 20) % 64 < 32;
                raw.extend_from_slice(&[if on { 240 } else { 15 }, if on { 40 } else { 200 }, (i * 7) as u8]);
            }
        }
    }
    Bytes::from(
        Sharp::raw(Bytes::from(raw), w, h * n, 3)
            .raw_page_height(h)
            .gif(GifOptions { delay: Some(vec![50; n as usize]), loop_count: Some(0), ..GifOptions::default() })
            .to_buffer()
            .await
            .unwrap(),
    )
}

async fn page(webp: &Bytes, index: u32) -> Vec<u8> {
    Sharp::new(webp.clone()).animated(true).page(index).pages(1).raw_output().to_buffer().await.unwrap()
}

async fn store(root: &std::path::Path, name: &str, body: &[u8]) -> String {
    std::fs::write(root.join(name), body).unwrap();
    name.to_string()
}

#[tokio::test]
async fn a_moving_thumbnail_of_an_animation() {
    let dir = tempfile::tempdir().unwrap();
    let st = state(dir.path());
    let gif = striped_gif(40, 160, 120).await;
    let media = row("image/gif", "gif", Some(40), gif.len(), &store(dir.path(), "anim.gif", &gif).await, None);
    let (buffer, frames) = motion_preview(&st, &media, OPTS, AbortController::new().signal()).await.unwrap().unwrap();
    let counted = count_animated_webp_frames(&buffer);
    assert_eq!(u64::from(counted), motion_page_budget(160 * 120, 40).frames);
    assert_eq!(counted, MOTION_FRAMES);
    assert_eq!(frames, counted);
    let meta = Sharp::new(buffer.clone()).animated(true).metadata().await.unwrap();
    assert_eq!(meta.pages, Some(counted));
    assert_ne!(page(&buffer, 0).await, page(&buffer, counted - 1).await);
    assert_ne!(page(&buffer, 1).await, page(&buffer, 2).await);
    assert!(buffer.len() < 500_000);

    // keeps a short animation whole
    let short = striped_gif(4, 160, 120).await;
    let media = row("image/gif", "gif", Some(4), short.len(), &store(dir.path(), "short.gif", &short).await, None);
    let (buffer, _) = motion_preview(&st, &media, OPTS, AbortController::new().signal()).await.unwrap().unwrap();
    assert_eq!(count_animated_webp_frames(&buffer), 4);

    // serves the second request from cache, byte for byte
    let twelve = striped_gif(12, 160, 120).await;
    let checksum = "b".repeat(64);
    let media = row(
        "image/gif",
        "gif",
        Some(12),
        twelve.len(),
        &store(dir.path(), "cached.gif", &twelve).await,
        Some(&checksum),
    );
    let cold = motion_preview(&st, &media, OPTS, AbortController::new().signal()).await.unwrap().unwrap();
    let warm = motion_preview(&st, &media, OPTS, AbortController::new().signal()).await.unwrap().unwrap();
    assert_eq!(cold, warm);
}

#[tokio::test]
async fn when_there_is_no_moving_thumbnail_to_be_had() {
    let dir = tempfile::tempdir().unwrap();
    let st = state(dir.path());
    let refused = |r: anyhow::Result<Result<(Bytes, u32), MotionRefusal>>| r.unwrap().unwrap_err();

    let png = still(200, 150).await;
    let media = row("image/png", "png", Some(1), png.len(), &store(dir.path(), "still.png", &png).await, None);
    assert_eq!(
        refused(motion_preview(&st, &media, OPTS, AbortController::new().signal()).await),
        MotionRefusal::NotAnimated
    );

    assert!(!motion_page_budget(9000 * 9000, 30).ok);
    let v = motion_page_budget(4000 * 3000, 120);
    assert!(v.ok && v.frames < u64::from(MOTION_FRAMES) && v.pixels <= max_motion_source_pixels());

    // an animation that never actually changes collapses to one frame
    let frozen = gif_of_pages(solid(120, 90 * 10, [90, 90, 90]), 120, 90, 10, 80).await;
    let media = row("image/gif", "gif", Some(10), frozen.len(), &store(dir.path(), "frozen.gif", &frozen).await, None);
    assert_eq!(
        refused(motion_preview(&st, &media, OPTS, AbortController::new().signal()).await),
        MotionRefusal::NotAnimated
    );

    let junk = b"this is not a picture";
    let media =
        row("application/octet-stream", "bin", None, junk.len(), &store(dir.path(), "junk.bin", junk).await, None);
    assert_eq!(
        refused(motion_preview(&st, &media, OPTS, AbortController::new().signal()).await),
        MotionRefusal::NotAnimated
    );

    let media = row("image/gif", "gif", Some(40), MAX_TRANSFORM_BYTES as usize + 1, "does-not-exist.gif", None);
    assert_eq!(
        refused(motion_preview(&st, &media, OPTS, AbortController::new().signal()).await),
        MotionRefusal::TooLarge
    );
}

async fn lavfi(out: &std::path::Path, args: &[&str]) -> bool {
    let mut cmd = vec!["-nostdin", "-hide_banner", "-v", "error"];
    cmd.extend_from_slice(args);
    let out_s = out.to_string_lossy().into_owned();
    cmd.extend_from_slice(&["-y", &out_s]);
    tokio::process::Command::new("ffmpeg").args(&cmd).status().await.is_ok_and(|s| s.success())
}

#[tokio::test]
async fn a_moving_thumbnail_of_a_video() {
    if !modlight_server::media::ffmpeg::ffmpeg_available().await {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let st = state(dir.path());
    let src = dir.path().join("clip.mp4");
    assert!(
        lavfi(
            &src,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=25:duration=20",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p"
            ]
        )
        .await
    );
    let size = std::fs::metadata(&src).unwrap().len() as usize;
    let media = row("video/mp4", "mp4", None, size, "clip.mp4", None);
    let started = std::time::Instant::now();
    let (buffer, _) = motion_preview(&st, &media, OPTS, AbortController::new().signal()).await.unwrap().unwrap();
    assert!(started.elapsed().as_secs() < 20);
    let counted = count_animated_webp_frames(&buffer);
    assert_eq!(counted, MOTION_FRAMES);
    assert_eq!(Sharp::new(buffer.clone()).animated(true).metadata().await.unwrap().pages, Some(counted));
    assert_ne!(page(&buffer, 0).await, page(&buffer, 1).await);
    assert_ne!(page(&buffer, 3).await, page(&buffer, 4).await);
    assert_ne!(page(&buffer, 0).await, page(&buffer, counted - 1).await);

    // does not open on the very first frame of the file
    let fade = dir.path().join("fade.mp4");
    assert!(
        lavfi(
            &fade,
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=black:size=320x240:rate=25:duration=3",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=25:duration=12",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0[v]",
                "-map",
                "[v]",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ]
        )
        .await
    );
    let size = std::fs::metadata(&fade).unwrap().len() as usize;
    let media = row("video/mp4", "mp4", None, size, "fade.mp4", None);
    let (buffer, _) = motion_preview(&st, &media, OPTS, AbortController::new().signal()).await.unwrap().unwrap();
    assert!(page(&buffer, 0).await.iter().any(|v| *v > 24));
}

//! The audio/video probe path against the real ffprobe: `probeStoredAvMeta`
//! on files ffmpeg itself makes in a temporary directory (nothing binary is
//! committed). Skipped only when the machine has no ffmpeg at all.

use std::path::Path;
use std::process::Command;

use modlight_server::media::abort::AbortController;
use modlight_server::media::av_meta::{AvMeta, ProbeStoredOptions, probe_stored_av_meta, probe_stored_av_meta_with};
use modlight_server::media::av_probe::probe_av;
use modlight_server::media::ffmpeg::{AvAbortedError, ffmpeg_available};
use modlight_server::storage::LocalDriver;

fn ffmpeg(args: &[&str]) -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// One second of a 64x48 test pattern at 10 fps, and a second of silence.
fn make_fixtures(dir: &Path) -> bool {
    let video = dir.join("u/clip.mp4");
    let audio = dir.join("u/tone.wav");
    std::fs::create_dir_all(dir.join("u")).unwrap();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=64x48:rate=10:duration=1",
        "-c:v",
        "mpeg4",
        "-pix_fmt",
        "yuv420p",
        video.to_str().unwrap(),
    ]) && ffmpeg(&["-f", "lavfi", "-i", "anullsrc=r=8000:cl=mono", "-t", "1", audio.to_str().unwrap()])
}

#[tokio::test]
async fn probes_a_stored_video_into_the_rows_columns() {
    if !ffmpeg_available().await {
        eprintln!("no ffmpeg on this machine; skipped");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    assert!(make_fixtures(root.path()), "ffmpeg could not make the fixtures");
    let storage = LocalDriver::new(root.path().to_str().unwrap());

    let meta = probe_stored_av_meta(&storage, "u/clip.mp4").await.expect("a video");
    assert_eq!((meta.width, meta.height, meta.frames), (Some(64), Some(48), Some(10)));
    assert_eq!(meta.fps, Some(10.0));
    assert!((meta.duration_seconds.unwrap() - 1.0).abs() < 0.05, "{meta:?}");

    let audio = probe_stored_av_meta(&storage, "u/tone.wav").await.expect("audio");
    assert_eq!(audio, AvMeta { duration_seconds: audio.duration_seconds, ..AvMeta::default() });
    assert!((audio.duration_seconds.unwrap() - 1.0).abs() < 0.05);
}

#[tokio::test]
async fn never_fails_the_caller() {
    if !ffmpeg_available().await {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("u")).unwrap();
    // A playlist pointing at a file outside storage: the concat demuxer, which
    // the allowlist refuses even where ffprobe would agree to open it.
    std::fs::write(root.path().join("u/evil.txt"), "ffconcat version 1.0\nfile /etc/hosts\n").unwrap();
    std::fs::write(root.path().join("u/junk.bin"), b"\x00\x01\x02 not media").unwrap();
    let storage = LocalDriver::new(root.path().to_str().unwrap());

    assert_eq!(probe_stored_av_meta(&storage, "u/evil.txt").await, None);
    assert_eq!(probe_stored_av_meta(&storage, "u/junk.bin").await, None);
    assert_eq!(probe_stored_av_meta(&storage, "u/missing.mp4").await, None);

    // A caller that has gone away is "no answer", not an error.
    let gone = AbortController::new();
    gone.abort();
    let opts = ProbeStoredOptions { stage_remote: None, signal: Some(gone.signal()) };
    assert_eq!(probe_stored_av_meta_with(&storage, "u/junk.bin", opts).await, None);
}

#[tokio::test]
async fn the_probe_itself_passes_an_abort_on() {
    if !ffmpeg_available().await {
        return;
    }
    let gone = AbortController::new();
    gone.abort();
    let err = probe_av("/nonexistent", 5_000, Some(gone.signal())).await.unwrap_err();
    assert!(err.is::<AvAbortedError>());
    // ...and answers None for a file that is not media.
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk");
    std::fs::write(&junk, b"junk").unwrap();
    assert_eq!(probe_av(junk.to_str().unwrap(), 5_000, None).await.unwrap(), None);
}

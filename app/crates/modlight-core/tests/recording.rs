//! Screen recording end to end with a real ffmpeg and a fake screen: frames of a still
//! picture go through the pipe, ffmpeg writes an MP4, and a GIF through the lossless
//! first pass and the palette pass. Skipped when no ffmpeg is installed on this computer
//! (the app itself uses the one from the Components window): on a Mac Homebrew's, on
//! Windows the one `MODLIGHT_TEST_FFMPEG` names or an `ffmpeg.exe` on the PATH.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use image::{Rgba, RgbaImage};
use modlight_core::capture::geometry::IRect;
use modlight_core::recording::options::{FFmpegOptions, FFmpegVideoCodec};
use modlight_core::recording::session::{
    FrameSource, FrameSourceFactory, RecorderControl, RecordingStatus, SessionParams, StillFrames,
    run_session,
};

#[cfg(not(windows))]
fn ffmpeg() -> Option<PathBuf> {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

#[cfg(windows)]
fn ffmpeg() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MODLIGHT_TEST_FFMPEG").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("ffmpeg.exe"))
            .find(|p| p.is_file())
    })
}

fn frames() -> FrameSourceFactory {
    Arc::new(|rect: IRect, _cursor: bool| {
        let picture = RgbaImage::from_fn(rect.width as u32, rect.height as u32, |x, y| {
            Rgba([(x * 4) as u8, (y * 4) as u8, 160, 255])
        });
        Ok(Box::new(StillFrames::new(&picture)) as Box<dyn FrameSource>)
    })
}

fn params(dir: &std::path::Path, codec: FFmpegVideoCodec, two_pass: bool) -> SessionParams {
    params_with(dir, ffmpeg().unwrap(), codec, two_pass)
}

fn params_with(
    dir: &std::path::Path,
    ffmpeg_path: PathBuf,
    codec: FFmpegVideoCodec,
    two_pass: bool,
) -> SessionParams {
    let ffmpeg_options = FFmpegOptions {
        VideoCodec: codec,
        ..FFmpegOptions::default()
    };
    let extension = ffmpeg_options.extension();
    SessionParams {
        ffmpeg_path,
        ffmpeg: ffmpeg_options,
        fps: 15,
        duration: 1.0,
        draw_cursor: false,
        two_pass,
        auto_start: true,
        start_delay: 0.0,
        path: dir.join(if two_pass {
            "clip.mp4".to_string()
        } else {
            format!("clip.{extension}")
        }),
        two_pass_output: dir.join(format!("clip.{extension}")),
        frames: frames(),
    }
}

#[test]
fn a_fixed_length_recording_becomes_an_mp4() {
    if ffmpeg().is_none() {
        eprintln!("no ffmpeg on this computer: skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let control = RecorderControl::new(IRect::new(0, 0, 64, 48), 1.0);
    let outcome = run_session(
        params(dir.path(), FFmpegVideoCodec::libx264, false),
        control.clone(),
    );
    assert_eq!(outcome.error, None);
    let path = outcome.path.expect("a file");
    assert_eq!(path.extension().unwrap(), "mp4");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() > 500, "{} bytes", bytes.len());
    assert_eq!(&bytes[4..8], b"ftyp");
    assert!(control.snapshot().finished);
}

#[test]
fn a_gif_goes_through_the_lossless_first_pass() {
    if ffmpeg().is_none() {
        eprintln!("no ffmpeg on this computer: skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let control = RecorderControl::new(IRect::new(0, 0, 40, 30), 1.0);
    let outcome = run_session(params(dir.path(), FFmpegVideoCodec::gif, true), control);
    let path = outcome.path.expect("a gif");
    assert_eq!(path.extension().unwrap(), "gif");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..6], b"GIF89a");
    assert!(
        !dir.path().join("clip.mp4").exists(),
        "the first pass is removed"
    );
}

#[test]
fn stopping_and_aborting_from_the_window() {
    if ffmpeg().is_none() {
        eprintln!("no ffmpeg on this computer: skipped");
        return;
    }
    // Stop: an open-ended recording ends with a file when Stop is pressed.
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(dir.path(), FFmpegVideoCodec::libx264, false);
    p.duration = 0.0;
    let control = RecorderControl::new(IRect::new(0, 0, 32, 32), 0.0);
    let worker = {
        let control = control.clone();
        std::thread::spawn(move || run_session(p, control))
    };
    wait_for(&control, RecordingStatus::Recording);
    std::thread::sleep(Duration::from_millis(600));
    control.start_stop();
    let outcome = worker.join().unwrap();
    assert!(outcome.path.is_some(), "{outcome:?}");
    assert!(!outcome.aborted);

    // Abort: nothing is left behind.
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(dir.path(), FFmpegVideoCodec::libx264, false);
    p.duration = 0.0;
    let control = RecorderControl::new(IRect::new(0, 0, 32, 32), 0.0);
    let worker = {
        let control = control.clone();
        std::thread::spawn(move || run_session(p, control))
    };
    wait_for(&control, RecordingStatus::Recording);
    control.abort();
    let outcome = worker.join().unwrap();
    assert_eq!(outcome.path, None);
    assert!(outcome.aborted);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_paused_recording_is_joined_into_one_file() {
    if ffmpeg().is_none() {
        eprintln!("no ffmpeg on this computer: skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(dir.path(), FFmpegVideoCodec::libx264, false);
    p.duration = 0.0;
    let control = RecorderControl::new(IRect::new(0, 0, 32, 32), 0.0);
    let worker = {
        let control = control.clone();
        std::thread::spawn(move || run_session(p, control))
    };
    wait_for(&control, RecordingStatus::Recording);
    std::thread::sleep(Duration::from_millis(500));
    control.pause_resume();
    wait_for(&control, RecordingStatus::Paused);
    std::thread::sleep(Duration::from_millis(300));
    control.pause_resume();
    wait_for(&control, RecordingStatus::Recording);
    std::thread::sleep(Duration::from_millis(500));
    control.start_stop();
    let outcome = worker.join().unwrap();
    let path = outcome.path.expect("one joined file");
    assert!(!dir.path().join("clip-concat.mp4").exists());
    assert!(std::fs::metadata(path).unwrap().len() > 500);
}

fn wait_for(control: &RecorderControl, status: RecordingStatus) {
    for _ in 0..200 {
        if control.status() == status {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("never reached {status:?}, at {:?}", control.status());
}

/// A stand-in for ffmpeg: a shell script.
#[cfg(unix)]
fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A camera or a microphone alone is ffmpeg's own avfoundation input, stopped by typing
/// `q`. ffmpeg then still has to finish the file (an MP4 has its index moved to the
/// front, which takes a while for a long recording): this stand-in needs 600 ms after
/// the `q`, and the recording must wait for it rather than kill it half way.
#[cfg(unix)]
#[test]
fn a_camera_recording_is_given_the_time_to_finish_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = script(
        tools.path(),
        "ffmpeg",
        "echo 'Press [q] to stop, [?] for help' >&2\n\
         head -c 1 > /dev/null\n\
         sleep 0.6\n\
         for last; do :; done\n\
         printf finished > \"$last\"\n",
    );
    let mut p = params_with(dir.path(), ffmpeg, FFmpegVideoCodec::libx264, false);
    p.ffmpeg.VideoSource = "FaceTime HD Camera".into();
    p.duration = 0.0;
    let control = RecorderControl::new(IRect::new(0, 0, 32, 32), 0.0);
    let worker = {
        let control = control.clone();
        std::thread::spawn(move || run_session(p, control))
    };
    wait_for(&control, RecordingStatus::Recording);
    control.start_stop();
    let outcome = worker.join().unwrap();
    assert_eq!(outcome.error, None);
    let path = outcome.path.expect("the finished file");
    assert_eq!(std::fs::read(path).unwrap(), b"finished");
}

/// An ffmpeg that does not answer `q` is killed on the third request to stop, as
/// upstream's `FFmpegCLIManager.Close` does, and never before.
#[cfg(unix)]
#[test]
fn a_camera_recording_that_will_not_stop_is_killed_on_the_third_request() {
    let dir = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    // `exec`: the process that is killed is the one holding the output pipe.
    let ffmpeg = script(
        tools.path(),
        "ffmpeg",
        "echo 'Press [q] to stop, [?] for help' >&2\nexec sleep 30\n",
    );
    let mut p = params_with(dir.path(), ffmpeg, FFmpegVideoCodec::libx264, false);
    p.ffmpeg.VideoSource = "FaceTime HD Camera".into();
    p.duration = 0.0;
    let control = RecorderControl::new(IRect::new(0, 0, 32, 32), 0.0);
    let worker = {
        let control = control.clone();
        std::thread::spawn(move || run_session(p, control))
    };
    wait_for(&control, RecordingStatus::Recording);
    let started = std::time::Instant::now();
    control.start_stop();
    std::thread::sleep(Duration::from_millis(400));
    assert!(!worker.is_finished(), "killed after one request to stop");
    control.abort();
    std::thread::sleep(Duration::from_millis(400));
    assert!(!worker.is_finished(), "killed after two requests to stop");
    control.abort();
    let outcome = worker.join().unwrap();
    assert!(outcome.aborted);
    assert!(started.elapsed() < Duration::from_secs(10));
}

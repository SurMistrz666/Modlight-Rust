//! The video converter with a real ffmpeg (Homebrew's, when this Mac has one): a short
//! clip converted with upstream's arguments, a stop half way, and an encoder this ffmpeg
//! lacks reported by its last line. Skipped when no ffmpeg is installed. No window opens.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use modlight_app::tools::video::{Control, run_conversion};
use modlight_tools::settings::{ConverterVideoCodecs, VideoConverterOptions};
use modlight_tools::video::Conversion;

fn ffmpeg() -> Option<PathBuf> {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// A test clip with a tone (`size` as ffmpeg writes it, "160x120").
fn clip(ffmpeg: &PathBuf, folder: &std::path::Path, seconds: u32, size: &str) -> PathBuf {
    let path = folder.join(format!("clip-{seconds}-{size}.mov"));
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=duration={seconds}:size={size}:rate=15"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={seconds}"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-y",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    path
}

fn conversion(input: &std::path::Path, codec: ConverterVideoCodecs) -> Conversion {
    let mut c = Conversion::from_options(
        &VideoConverterOptions::default(),
        Some(&input.to_string_lossy()),
    );
    c.set_codec(codec);
    c
}

#[test]
fn converts_stops_and_reports() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no ffmpeg on this Mac: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let input = clip(&ffmpeg, dir.path(), 2, "160x120");

    // H.264 with upstream's arguments, progress reported.
    let c = conversion(&input, ConverterVideoCodecs::X264);
    let control = Arc::new(Control::default());
    let result = run_conversion(&ffmpeg, &c.arguments(), control.clone());
    assert!(result.succeeded, "{result:?}");
    let output = PathBuf::from(c.output_path());
    assert!(output.is_file() && std::fs::metadata(&output).unwrap().len() > 1000);
    assert!(*control.progress.lock().unwrap() > 50.0);

    // H.265 tagged for Apple players, and a GIF from the clip.
    for codec in [ConverterVideoCodecs::X265, ConverterVideoCodecs::Gif] {
        let c = conversion(&input, codec);
        let result = run_conversion(&ffmpeg, &c.arguments(), Arc::new(Control::default()));
        assert!(result.succeeded, "{codec:?}: {result:?}");
        assert!(PathBuf::from(c.output_path()).is_file());
    }

    // Stopped half way: `q`, then the result says so. The clip is long and large enough
    // that no AV1 encoder finishes it before the stop.
    let long = clip(&ffmpeg, dir.path(), 60, "1280x720");
    let mut c = conversion(&long, ConverterVideoCodecs::Av1);
    c.output_name = "stopped".into();
    let control = Arc::new(Control::default());
    let stopper = control.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        stopper.stop.store(true, Ordering::SeqCst);
    });
    let result = run_conversion(&ffmpeg, &c.arguments(), control);
    assert!(result.cancelled, "{result:?}");

    // An encoder this ffmpeg was built without: the last line says why.
    let c = conversion(&input, ConverterVideoCodecs::Xvid);
    let result = run_conversion(&ffmpeg, &c.arguments(), Arc::new(Control::default()));
    if !result.succeeded {
        assert!(result.error.is_some(), "{result:?}");
    }
}

//! The README's pictures, drawn from the QA fixtures (see `modlight_app::qa_showcase`):
//!
//!   cargo run --release -p modlight-app --example showcase -- <out-dir> [scale]
//!
//! writes `<out-dir>/stills/<screen>-<palette>.png` and `<out-dir>/scenes/<scene>/*.png`
//! at `scale` pixels per point (2 by default). `scripts/showcase/make-media.sh` frames
//! them and makes the README's images and video. Nothing opens a window or talks to a
//! server.

use std::path::PathBuf;

use modlight_app::qa_showcase;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(out) = args.next().map(PathBuf::from) else {
        eprintln!("usage: showcase <out-dir> [scale]");
        std::process::exit(2);
    };
    let scale = args
        .next()
        .and_then(|s| s.to_str().and_then(|s| s.parse::<f32>().ok()))
        .unwrap_or(2.0);
    let result = qa_showcase::stills(&out.join("stills"), scale).and_then(|()| {
        qa_showcase::SCENES
            .iter()
            .try_for_each(|scene| qa_showcase::scene(scene, &out.join("scenes"), scale))
    });
    if let Err(error) = result {
        eprintln!("showcase: {error}");
        std::process::exit(1);
    }
}

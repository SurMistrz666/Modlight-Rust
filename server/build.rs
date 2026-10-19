//! Link libvips (and the GLib/GObject it is built on) the way the system
//! describes it: `pkg-config --libs vips`. Homebrew on the Mac, Debian's
//! `libvips-dev` in the image; nothing is vendored or compiled here.
//! See `src/media/vips.rs`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG");

    let pkg_config = std::env::var("PKG_CONFIG").unwrap_or_else(|_| "pkg-config".to_string());
    let output = Command::new(&pkg_config).args(["--libs", "vips"]).output().unwrap_or_else(|e| {
        panic!("could not run `{pkg_config} --libs vips` ({e}): install pkg-config and libvips (brew install vips pkgconf, or apt install libvips-dev pkg-config)")
    });
    if !output.status.success() {
        panic!(
            "`{pkg_config} --libs vips` failed: {}\ninstall libvips (brew install vips, or apt install libvips-dev)",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let flags = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut tokens = flags.split_whitespace();
    while let Some(token) = tokens.next() {
        if let Some(dir) = token.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(lib) = token.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
        } else if token == "-framework"
            && let Some(framework) = tokens.next()
        {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}

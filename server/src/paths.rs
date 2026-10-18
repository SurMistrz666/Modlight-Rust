//! Where the server finds the files it ships with: `migrations/`, `messages/`,
//! `public/` and the SPA build in `web/dist/`.
//!
//! Upstream reads all of these relative to the working directory (drizzle reads
//! `./drizzle`, Next serves `./public`). The port does the same, with two
//! conveniences that do not change behaviour in production:
//! - `MODLIGHT_ROOT` points at the directory explicitly (the Docker image sets
//!   it to `/app`);
//! - without it, the working directory is used when it looks like the server
//!   directory, otherwise the crate directory this binary was built from, so
//!   `cargo run --manifest-path server/Cargo.toml` from the repository root and
//!   `cargo test` both find their files.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub migrations: PathBuf,
    pub messages: PathBuf,
    pub public: PathBuf,
    pub web_dist: PathBuf,
}

impl Paths {
    /// Resolve from `MODLIGHT_ROOT`, the working directory or the crate dir.
    pub fn resolve() -> Paths {
        if let Some(root) = std::env::var_os("MODLIGHT_ROOT").filter(|v| !v.is_empty()) {
            return Paths::at(PathBuf::from(root));
        }
        if let Ok(cwd) = std::env::current_dir()
            && cwd.join("migrations/meta/_journal.json").is_file()
        {
            return Paths::at(cwd);
        }
        Paths::at(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    }

    /// Everything under one root directory.
    pub fn at(root: impl AsRef<Path>) -> Paths {
        let root = root.as_ref().to_path_buf();
        Paths {
            migrations: root.join("migrations"),
            messages: root.join("messages"),
            public: root.join("public"),
            web_dist: root.join("web").join("dist"),
            root,
        }
    }

    /// The crate directory (tests).
    pub fn crate_dir() -> Paths {
        Paths::at(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    }
}

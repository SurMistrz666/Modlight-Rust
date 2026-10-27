//! Port of tests/unit/token-scope-coverage.test.ts: "which surfaces accept an
//! API token, and does every one of them check a scope?", asked of the source
//! tree rather than of a list somebody maintains by hand.
//!
//! Adapted to a port in progress: an inventory entry whose Rust file has no
//! call site yet is not ported and is skipped; a file with call sites that is
//! NOT in the inventory fails, and so does a call site that names no
//! capability. Capabilities are compared as a sorted list (Rust handlers need
//! not appear in the same order as the TypeScript exports did).

use std::path::{Path, PathBuf};

const ENTRY_POINTS: &[&str] = &[
    "authenticate_api_token(",
    "authenticate_api_token_with(",
    "resolve_api_caller(",
    "resolve_api_caller_with(",
    "resolve_api_owner(",
    "resolve_api_owner_with(",
];

/// Files that declare or forward the functions rather than calling them as a
/// surface (lib/api-token.ts and lib/api-caller.ts upstream).
const FORWARDERS: &[&str] = &["src/api_token.rs", "src/api_caller.rs"];

/// The inventory: every surface that accepts an API token, at the Rust path the
/// port's naming rule gives its upstream file, with the capabilities it demands.
const EXPECTED: &[(&str, &[&str])] = &[
    // POST /upload: header, legacy field before the file, legacy field after it.
    ("src/upload/sharex.rs", &["upload", "upload", "upload"]),
    ("src/upload/tus.rs", &["upload"]),
    ("src/media/serve.rs", &["read"]),
    ("src/routes/media_delete.rs", &["write"]),
    ("src/routes/api/uploads.rs", &["read", "write", "write"]),
    ("src/routes/api/uploads_check.rs", &["read"]),
    ("src/routes/api/duplicates.rs", &["read", "write"]),
    ("src/routes/api/folders.rs", &["read", "write", "write", "write"]),
    ("src/routes/api/shortlinks.rs", &["read", "write"]),
    ("src/routes/api/client_settings.rs", &["read", "write"]),
    ("src/routes/api/theme.rs", &["read", "write"]),
    ("src/routes/api/me.rs", &["identify"]),
    ("src/routes/api/releases_app.rs", &["admin"]),
    ("src/routes/api/admin_disk.rs", &["admin"]),
    ("src/routes/api/link_info.rs", &["read"]),
    // Not upstream: Downloads. Listing and downloading take any token (identify);
    // publishing and deleting an administrator's `full` one (or the publish token).
    ("src/routes/api/downloads.rs", &["admin", "identify"]),
    // Not upstream: per-account file encryption (status, the app's unlock).
    ("src/routes/api/encryption.rs", &["read", "read"]),
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Strip `#[cfg(test)] mod tests { ... }` (the unit tests call these with stubs).
fn without_tests(src: &str) -> &str {
    match src.find("#[cfg(test)]") {
        Some(i) => &src[..i],
        None => src,
    }
}

/// The argument list of every call in `src`, up to its closing parenthesis.
fn calls(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for entry in ENTRY_POINTS {
        let mut from = 0;
        while let Some(rel) = src[from..].find(entry) {
            let start = from + rel;
            // Skip definitions (`fn name(`) and paths that merely end in the name.
            let before = src[..start].trim_end();
            let is_def = before.ends_with("fn");
            let prev = src[..start].chars().last().unwrap_or(' ');
            let open = start + entry.len();
            from = open;
            if is_def || prev.is_alphanumeric() || prev == '_' {
                continue;
            }
            let mut depth = 1;
            let mut end = open;
            for (i, c) in src[open..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push(src[open..end].to_string());
        }
    }
    out
}

fn capability(args: &str) -> Option<String> {
    for cap in ["Identify", "Upload", "Read", "Write", "Admin"] {
        if args.contains(&format!("TokenCapability::{cap}")) || args.contains(&format!("C::{cap}")) {
            return Some(cap.to_lowercase());
        }
    }
    None
}

#[test]
fn every_surface_that_accepts_an_api_token_names_a_capability() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for file in files {
        let rel = file.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        let src = std::fs::read_to_string(&file).unwrap();
        let c = calls(without_tests(&src));
        if !c.is_empty() {
            found.push((rel, c));
        }
    }

    // No call site resolves a caller without saying what it needs.
    let mut unscoped = Vec::new();
    for (file, calls) in &found {
        if FORWARDERS.contains(&file.as_str()) {
            continue;
        }
        for args in calls {
            if capability(args).is_none() {
                unscoped.push(format!("{file}: ({})", args.trim()));
            }
        }
    }
    assert!(unscoped.is_empty(), "unscoped token surfaces: {unscoped:#?}");

    // A new token surface cannot appear unnoticed.
    for (file, _) in &found {
        if FORWARDERS.contains(&file.as_str()) {
            continue;
        }
        assert!(
            EXPECTED.iter().any(|(f, _)| f == file),
            "{file} accepts API tokens but is not in the inventory of tests/token_scope_coverage.rs"
        );
    }

    // Each ported surface demands the capability the inventory says it does.
    for (file, expected) in EXPECTED {
        let Some((_, calls)) = found.iter().find(|(f, _)| f == file) else {
            continue; // not ported yet
        };
        let mut caps: Vec<String> = calls.iter().filter_map(|a| capability(a)).collect();
        let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        caps.sort();
        want.sort();
        assert_eq!(caps, want, "{file}");
    }
}

#[test]
fn the_scanner_finds_calls_and_their_capability() {
    let src = "let a = resolve_api_owner(&state, &s, &h, TokenCapability::Write).await;\npub async fn resolve_api_caller(x: u8) {}\nlet b = crate::api_token::authenticate_api_token(db, Some(t), C::Upload);";
    let c = calls(src);
    assert_eq!(c.len(), 2);
    assert_eq!(capability(&c[0]).as_deref(), Some("upload"));
    assert_eq!(capability(&c[1]).as_deref(), Some("write"));
}

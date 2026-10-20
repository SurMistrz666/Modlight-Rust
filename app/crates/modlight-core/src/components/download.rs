//! Fetching one file over HTTPS: resumable, cancellable, unable to hang
//! (`ModlightComponentDownload`).
//!
//! One rule is stricter than upstream's, as the audit asked: a download whose size is
//! not known in advance is a failure. The size comes from the catalogue, the release
//! document or the manifest, never from `Content-Length`, and a body that ends short of
//! it, or runs past it, is never called complete. Nothing is used without hashing it
//! first anyway (`verify::verify_or_delete`).

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::net;
use crate::l10n::{s, t};

/// How long a silent connection is given before it is called dead: not a total
/// timeout, a no-progress timeout.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// How much has arrived, and how fast it is arriving.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DownloadProgress {
    pub received: i64,
    pub total: i64,
    pub bytes_per_second: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadOutcome {
    Completed,
    /// The person pressed cancel. The part file is kept so it can be resumed.
    Cancelled,
    /// Nothing answered, or the line went away.
    Offline,
    /// Something answered and said no, or sent a body of the wrong length.
    HttpFailed,
    /// The bytes arrived but could not be written.
    WriteFailed,
    /// The URL was not https. Nothing was requested.
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadResult {
    pub outcome: DownloadOutcome,
    pub message: String,
}

impl DownloadResult {
    fn new(outcome: DownloadOutcome, message: impl Into<String>) -> DownloadResult {
        DownloadResult {
            outcome,
            message: message.into(),
        }
    }

    pub fn completed(&self) -> bool {
        self.outcome == DownloadOutcome::Completed
    }
}

/// Download `url` to `path`, continuing an earlier attempt if one is there.
///
/// `expected_size` must be known (more than zero): without it nothing is requested.
pub async fn to_file(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    expected_size: i64,
    stall: Duration,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> DownloadResult {
    if !net::is_fetchable(url) {
        return DownloadResult::new(DownloadOutcome::Refused, url);
    }
    to_file_unchecked(client, url, path, expected_size, stall, progress, cancel).await
}

/// [`to_file`] for a caller that has already judged the URL by its own rule (the app
/// updater accepts plain http on the loopback interface, as upstream does).
pub(crate) async fn to_file_unchecked(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    expected_size: i64,
    stall: Duration,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> DownloadResult {
    if expected_size <= 0 {
        // The audit's rule: a size nobody promised cannot be checked against, and a
        // missing Content-Length must never read as "complete".
        return DownloadResult::new(
            DownloadOutcome::HttpFailed,
            t(s::Components_ErrorIncomplete),
        );
    }
    let mut restarted = false;
    loop {
        let result = attempt(client, url, path, expected_size, stall, progress, cancel).await;
        match result {
            Attempt::Done(result) => return result,
            Attempt::RangeRefused if !restarted => {
                // The file on the server is no longer as long as what we have: start
                // again from nothing rather than argue with it.
                restarted = true;
                super::verify::try_delete(path);
            }
            Attempt::RangeRefused => {
                return DownloadResult::new(
                    DownloadOutcome::HttpFailed,
                    "416 Range Not Satisfiable",
                );
            }
        }
    }
}

enum Attempt {
    Done(DownloadResult),
    RangeRefused,
}

async fn attempt(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    expected_size: i64,
    stall: Duration,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> Attempt {
    let mut resume_from: i64 = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            let length = meta.len() as i64;
            if length == expected_size {
                // Already the whole file: the caller hashes it next and finds out.
                return Attempt::Done(DownloadResult::new(DownloadOutcome::Completed, ""));
            }
            if length > expected_size {
                // Longer than the whole file means it is not this file at all.
                super::verify::try_delete(path);
                0
            } else {
                length
            }
        }
        _ => 0,
    };

    let mut request = client.get(url);
    if resume_from > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    let response = tokio::select! {
        _ = cancel.cancelled() => {
            return Attempt::Done(DownloadResult::new(DownloadOutcome::Cancelled, ""));
        }
        response = request.send() => response,
    };
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            let outcome = if error.is_status() {
                DownloadOutcome::HttpFailed
            } else {
                DownloadOutcome::Offline
            };
            return Attempt::Done(DownloadResult::new(outcome, error.to_string()));
        }
    };
    let status = response.status();
    if resume_from > 0 && status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        return Attempt::RangeRefused;
    }
    if !status.is_success() {
        return Attempt::Done(DownloadResult::new(
            DownloadOutcome::HttpFailed,
            format!(
                "{} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or_default()
            ),
        ));
    }
    let append = resume_from > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    if append {
        // The range has to start where we asked, or the two halves would be spliced at
        // the wrong byte and the digest would fail for a reason nobody could work out.
        let starts_right = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes "))
            .and_then(|v| v.split('-').next())
            .and_then(|v| v.trim().parse::<i64>().ok())
            == Some(resume_from);
        if !starts_right {
            return Attempt::Done(DownloadResult::new(
                DownloadOutcome::HttpFailed,
                "the server answered a different range",
            ));
        }
    }
    if resume_from > 0 && !append {
        // Asked for a range and got the whole thing: the server does not do ranges.
        resume_from = 0;
    }

    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return Attempt::Done(DownloadResult::new(
            DownloadOutcome::WriteFailed,
            error.to_string(),
        ));
    }
    let file = if append {
        std::fs::OpenOptions::new().append(true).open(path)
    } else {
        std::fs::File::create(path)
    };
    let mut file = match file {
        Ok(file) => std::io::BufWriter::with_capacity(128 * 1024, file),
        Err(error) => {
            return Attempt::Done(DownloadResult::new(
                DownloadOutcome::WriteFailed,
                error.to_string(),
            ));
        }
    };

    let mut received = resume_from;
    let mut since_sample: i64 = 0;
    let mut speed = 0.0;
    let mut speed_timer = Instant::now();
    let mut report_timer = Instant::now();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = file.flush();
                return Attempt::Done(DownloadResult::new(DownloadOutcome::Cancelled, ""));
            }
            chunk = tokio::time::timeout(stall, response.chunk()) => chunk,
        };
        let chunk = match chunk {
            Err(_) => {
                let _ = file.flush();
                return Attempt::Done(DownloadResult::new(
                    DownloadOutcome::Offline,
                    t(s::Components_ErrorStalled),
                ));
            }
            Ok(Err(error)) => {
                let _ = file.flush();
                return Attempt::Done(DownloadResult::new(
                    DownloadOutcome::Offline,
                    error.to_string(),
                ));
            }
            Ok(Ok(None)) => break,
            Ok(Ok(Some(chunk))) => chunk,
        };
        let length = chunk.len() as i64;
        if received + length > expected_size {
            // Longer than promised. Not this file, and not a reason to fill the disk.
            drop(file);
            super::verify::try_delete(path);
            return Attempt::Done(DownloadResult::new(
                DownloadOutcome::HttpFailed,
                t(s::Components_ErrorIncomplete),
            ));
        }
        if let Err(error) = file.write_all(&chunk) {
            return Attempt::Done(DownloadResult::new(
                DownloadOutcome::WriteFailed,
                error.to_string(),
            ));
        }
        received += length;
        since_sample += length;
        let elapsed = speed_timer.elapsed();
        if elapsed > Duration::from_millis(500) {
            speed = since_sample as f64 / elapsed.as_secs_f64().max(0.001);
            since_sample = 0;
            speed_timer = Instant::now();
        }
        if report_timer.elapsed() > Duration::from_millis(150) {
            progress(DownloadProgress {
                received,
                total: expected_size,
                bytes_per_second: speed,
            });
            report_timer = Instant::now();
        }
    }
    if let Err(error) = file.flush() {
        return Attempt::Done(DownloadResult::new(
            DownloadOutcome::WriteFailed,
            error.to_string(),
        ));
    }
    progress(DownloadProgress {
        received,
        total: expected_size,
        bytes_per_second: speed,
    });
    if received != expected_size {
        // Short body. Kept on disk on purpose - the next attempt resumes from here - and
        // never mistaken for a finished download.
        return Attempt::Done(DownloadResult::new(
            DownloadOutcome::HttpFailed,
            t(s::Components_ErrorIncomplete),
        ));
    }
    Attempt::Done(DownloadResult::new(DownloadOutcome::Completed, ""))
}

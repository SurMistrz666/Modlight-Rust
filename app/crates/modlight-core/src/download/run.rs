//! One run of yt-dlp: build the arguments, read what it says, hand back a file
//! (`YtDlpRun`, `YtDlpProgress`, `YtDlpFormatResult`, `YtDlpOutcome`).
//!
//! yt-dlp is asked to speak in lines nobody has to guess at: `--progress-template` with
//! the RAW numeric fields, and `--print` with markers of our own. Where the file went is
//! `--print after_move:%(filepath)s`, the only answer that is always true. Arguments are
//! a vector, never a string, with `--` before the URL; there is no `--exec`. Pausing is
//! stopping: the process group is killed and the identical command resumes from
//! yt-dlp's own `.part` file.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::cookies::YouTubeCookies;
use super::error::{self, VersionFacts};
use super::format::{DownloadFormat, resolution_from};
use super::link::DownloadLink;
use crate::components::process;
use crate::l10n::{Arg, format, s, t};

/// Marks a progress line. Deliberately not a word yt-dlp itself would print.
pub const PROGRESS_MARKER: &str = "@ML|";
pub const TITLE_MARKER: &str = "@MT|";
pub const FINAL_PATH_MARKER: &str = "@MF|";
pub const PREDICTED_PATH_MARKER: &str = "@MP|";
/// The chosen format's size, printed once, before the download starts.
pub const RESOLUTION_MARKER: &str = "@MR|";
/// What yt-dlp prints for a field it has no value for.
const NOT_AVAILABLE: &str = "NA";
/// Diagnosis lines kept: a fragmented download can warn once per fragment.
const MAX_ERROR_LINES: usize = 200;

/// How far along one download is. Every field is a fact yt-dlp just reported.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    /// 0 to 100, a fraction of the bytes known about.
    pub percent: f64,
    pub downloaded: i64,
    /// The size of the stream being fetched, or 0 when the server did not say.
    pub total: i64,
    /// Bytes per second, or 0 when not yet known.
    pub speed: f64,
    /// Seconds left, or -1 when not yet known.
    pub eta: i32,
    /// A stream has finished and something else is happening (merging, usually).
    pub is_working: bool,
}

/// What yt-dlp settled on, before it fetches a byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatResult {
    /// The short side in pixels, or 0 for audio and for unknown.
    pub resolution: i32,
    /// Why this is not what was asked for, ready to show. Empty when it is.
    pub note: String,
}

/// Something worth telling the row while the run is going.
#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    Progress(Progress),
    Title(String),
    Format(FormatResult),
}

/// What became of a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub success: bool,
    /// We stopped it: a cancel, or a pause.
    pub canceled: bool,
    pub file_path: Option<PathBuf>,
    pub title: Option<String>,
    /// Ready to put in front of a person. Empty on success.
    pub error: String,
    pub needs_sign_in: bool,
    pub needs_update: bool,
}

/// Everything one run is made of, fixed for the run.
#[derive(Clone, Debug)]
pub struct RunSpec {
    pub tool: PathBuf,
    pub link: DownloadLink,
    pub folder: PathBuf,
    pub format: DownloadFormat,
    /// The merger this run uses, read once at the start.
    pub ffmpeg: Option<PathBuf>,
    pub quickjs: Option<PathBuf>,
    /// The YouTube jar, already filtered by `YouTubeCookies::file_for`.
    pub cookies: Option<PathBuf>,
}

/// The argument vector (`BuildArguments`).
pub fn build_arguments(spec: &RunSpec) -> Vec<String> {
    let mut out: Vec<String> = [
        "--color",
        "never",
        "--newline",
        "--progress",
        "--no-simulate",
        "--no-playlist",
        "--windows-filenames",
        "--trim-filenames",
        "180",
        "--retries",
        "10",
        "--fragment-retries",
        "10",
        "--concurrent-fragments",
        "4",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    if let Some(cookies) = &spec.cookies {
        out.push("--cookies".into());
        out.push(cookies.to_string_lossy().into_owned());
    }
    out.push("-P".into());
    out.push(spec.folder.to_string_lossy().into_owned());
    out.push("-o".into());
    out.push("%(title).200B [%(id)s].%(ext)s".into());
    let ffmpeg = spec
        .ffmpeg
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    out.extend(
        spec.format
            .arguments(spec.link.is_youtube, ffmpeg.as_deref()),
    );
    if let Some(quickjs) = &spec.quickjs {
        out.push("--js-runtimes".into());
        out.push(format!("quickjs:{}", quickjs.to_string_lossy()));
    }
    out.push("--progress-template".into());
    out.push(format!(
        "download:{PROGRESS_MARKER}%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s"
    ));
    out.push("--print".into());
    out.push(format!("before_dl:{TITLE_MARKER}%(title)s"));
    out.push("--print".into());
    out.push(format!("after_move:{FINAL_PATH_MARKER}%(filepath)s"));
    out.push("--print".into());
    out.push(format!("video:{PREDICTED_PATH_MARKER}%(filename)s"));
    out.push("--print".into());
    out.push(format!("video:{RESOLUTION_MARKER}%(height)s|%(width)s"));
    out.push("--".into());
    out.push(spec.link.url.clone());
    out
}

fn parse_long(value: &str) -> Option<i64> {
    let text = value.trim();
    if text.is_empty() || text == NOT_AVAILABLE {
        return None;
    }
    text.parse().ok()
}

/// Invariant culture, always: yt-dlp prints "13033483.601051992" whatever the locale.
fn parse_double(value: &str) -> Option<f64> {
    let text = value.trim();
    if text.is_empty() || text == NOT_AVAILABLE {
        return None;
    }
    text.parse().ok().filter(|v: &f64| v.is_finite())
}

/// Reads both output streams, one line at a time. Pure: everything it learns is kept
/// here or returned as an event.
#[derive(Debug)]
pub struct LineReader {
    format: DownloadFormat,
    has_ffmpeg: bool,
    /// Bytes belonging to streams that have already finished this run.
    completed_bytes: i64,
    pub title: Option<String>,
    pub final_paths: Vec<PathBuf>,
    pub predicted_path: Option<PathBuf>,
    pub errors: Vec<String>,
}

impl LineReader {
    pub fn new(format: DownloadFormat, has_ffmpeg: bool) -> LineReader {
        LineReader {
            format,
            has_ffmpeg,
            completed_bytes: 0,
            title: None,
            final_paths: Vec::new(),
            predicted_path: None,
            errors: Vec::new(),
        }
    }

    /// One line from either stream.
    pub fn read(&mut self, line: &str) -> Option<RunEvent> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return None;
        }
        if let Some(payload) = line.strip_prefix(PROGRESS_MARKER) {
            return self.read_progress(payload).map(RunEvent::Progress);
        }
        if let Some(value) = line.strip_prefix(TITLE_MARKER) {
            let value = value.trim();
            if !value.is_empty() && value != NOT_AVAILABLE && self.title.is_none() {
                self.title = Some(value.to_string());
                return Some(RunEvent::Title(value.to_string()));
            }
            return None;
        }
        if let Some(value) = line.strip_prefix(FINAL_PATH_MARKER) {
            let value = value.trim();
            if !value.is_empty() {
                self.final_paths.push(PathBuf::from(value));
            }
            return None;
        }
        if let Some(value) = line.strip_prefix(PREDICTED_PATH_MARKER) {
            let value = value.trim();
            if !value.is_empty() {
                self.predicted_path = Some(PathBuf::from(value));
            }
            return None;
        }
        if let Some(payload) = line.strip_prefix(RESOLUTION_MARKER) {
            let mut fields = payload.split('|');
            let height = fields.next().map(pixels).unwrap_or(0);
            let width = fields.next().map(pixels).unwrap_or(0);
            return Some(RunEvent::Format(
                self.verdict(resolution_from(width, height)),
            ));
        }
        if self.errors.len() < MAX_ERROR_LINES {
            self.errors.push(line.to_string());
        }
        None
    }

    /// status | downloaded | total | total_estimate | speed | eta
    fn read_progress(&mut self, payload: &str) -> Option<Progress> {
        let fields: Vec<&str> = payload.split('|').collect();
        if fields.len() < 6 {
            return None;
        }
        let status = fields[0].trim();
        let downloaded = parse_long(fields[1]).unwrap_or(0);
        let total = parse_long(fields[2])
            .or_else(|| parse_double(fields[3]).map(|v| v as i64))
            .unwrap_or(0);
        let speed = parse_double(fields[4]).unwrap_or(0.0);
        let eta = parse_double(fields[5]).map(|v| v as i32).unwrap_or(-1);
        if status == "finished" {
            self.completed_bytes += downloaded;
            return Some(Progress {
                percent: 100.0,
                downloaded: self.completed_bytes,
                total: self.completed_bytes,
                is_working: true,
                eta: -1,
                ..Progress::default()
            });
        }
        if status != "downloading" {
            return None;
        }
        let done = self.completed_bytes + downloaded;
        let whole = self.completed_bytes + total;
        let percent = if whole > 0 {
            (done as f64 * 100.0 / whole as f64).clamp(0.0, 100.0)
        } else {
            0.0
        };
        Some(Progress {
            percent,
            downloaded: done,
            total: whole,
            speed,
            eta,
            is_working: false,
        })
    }

    /// Whether what yt-dlp chose is what was asked for, and how to say it if not.
    pub fn verdict(&self, resolution: i32) -> FormatResult {
        if !self.has_ffmpeg {
            return FormatResult {
                resolution,
                note: if self.format.is_audio() {
                    t(s::Download_NoteNoFFmpegAudio).to_string()
                } else {
                    t(s::Download_NoteNoFFmpegVideo).to_string()
                },
            };
        }
        let asked = self.format.max_resolution();
        if asked == 0 || resolution <= 0 || resolution == asked {
            return FormatResult {
                resolution,
                note: String::new(),
            };
        }
        let key = if resolution < asked {
            s::Download_NoteQualityLower
        } else {
            s::Download_NoteQualityHigher
        };
        FormatResult {
            resolution,
            note: format(t(key), &[Arg::from(asked), Arg::from(resolution)]),
        }
    }

    /// The line worth showing: an ERROR line first, wherever it appeared; else the last
    /// line that is not a warning.
    pub fn last_error(&self) -> String {
        if let Some(line) = self
            .errors
            .iter()
            .rev()
            .find(|l| starts_with_ignore_case(l, "ERROR:"))
        {
            return line.clone();
        }
        self.errors
            .iter()
            .rev()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !starts_with_ignore_case(l, "WARNING:"))
            .map(str::to_string)
            .unwrap_or_default()
    }

    /// Which file this run produced: yt-dlp's after-move path, then the predicted name
    /// or its sibling, then the newest file written since the run started.
    pub fn resolve_file(&self, folder: &Path, started: SystemTime) -> Option<PathBuf> {
        if let Some(path) = self.final_paths.iter().rev().find(|p| p.is_file()) {
            return Some(path.clone());
        }
        if let Some(predicted) = &self.predicted_path {
            if predicted.is_file() {
                return Some(predicted.clone());
            }
            if let (Some(directory), Some(stem)) = (predicted.parent(), predicted.file_stem())
                && let Ok(entries) = std::fs::read_dir(directory)
            {
                let prefix = format!("{}.", stem.to_string_lossy());
                let candidates: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
                    })
                    .collect();
                if let Some(found) = newest(&candidates, SystemTime::UNIX_EPOCH) {
                    return Some(found);
                }
            }
        }
        let since = started
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap_or(started);
        let entries: Vec<PathBuf> = std::fs::read_dir(folder)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        newest(&entries, since)
    }
}

/// ASCII case-insensitive prefix test that never slices inside a character.
pub fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn pixels(value: &str) -> i32 {
    parse_long(value)
        .filter(|v| *v > 0 && *v <= i32::MAX as i64)
        .unwrap_or(0) as i32
}

/// The newest regular file written at or after `not_before`, skipping yt-dlp's own
/// scratch files (a `.part` is a download that did not finish).
fn newest(candidates: &[PathBuf], not_before: SystemTime) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, SystemTime)> = None;
    for candidate in candidates {
        let extension = candidate
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(extension.as_str(), "part" | "ytdl" | "temp") {
            continue;
        }
        let Ok(meta) = std::fs::metadata(candidate) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(written) = meta.modified() else {
            continue;
        };
        if written >= not_before && best.as_ref().is_none_or(|(_, at)| written > *at) {
            best = Some((candidate.clone(), written));
        }
    }
    best.map(|(path, _)| path)
}

/// Run yt-dlp for `spec`. `events` hears progress, the title and the chosen format as
/// they arrive; cancelling `cancel` kills the process group and answers `canceled`.
pub async fn run(
    spec: &RunSpec,
    profile: &Path,
    events: Arc<dyn Fn(RunEvent) + Send + Sync>,
    cancel: &CancellationToken,
) -> Outcome {
    if let Err(error) = std::fs::create_dir_all(&spec.folder) {
        return Outcome {
            error: format(
                t(s::Download_ErrorFolder),
                &[
                    Arg::from(spec.folder.display().to_string()),
                    Arg::from(error.to_string()),
                ],
            ),
            ..Outcome::default()
        };
    }
    let started = SystemTime::now();
    let arguments = build_arguments(spec);
    let mut command = process::command(&spec.tool, &arguments);
    command
        .current_dir(&spec.folder)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Outcome {
                error: format(
                    t(s::Download_ErrorStartFailed),
                    &[Arg::from(error.to_string())],
                ),
                ..Outcome::default()
            };
        }
    };
    let guard = child.id().map(process::GroupGuard::register);
    let reader = Arc::new(Mutex::new(LineReader::new(
        spec.format,
        spec.ffmpeg.is_some(),
    )));
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(tokio::spawn(pump(stdout, reader.clone(), events.clone())));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(tokio::spawn(pump(stderr, reader.clone(), events.clone())));
    }
    let status = tokio::select! {
        status = child.wait() => status.ok(),
        _ = cancel.cancelled() => {
            if let Some(guard) = &guard {
                guard.kill();
            }
            // Only once the process is gone: a resume must not race its own leftovers.
            let _ = child.wait().await;
            None
        }
    };
    for handle in readers {
        let _ = handle.await;
    }
    drop(guard);
    let reader = std::mem::replace(
        &mut *reader.lock().unwrap(),
        LineReader::new(spec.format, false),
    );
    if cancel.is_cancelled() {
        return Outcome {
            canceled: true,
            title: reader.title,
            ..Outcome::default()
        };
    }
    let success = status.is_some_and(|s| s.success());
    if !success {
        let raw = reader.last_error();
        log::info!(
            "Download: yt-dlp exited {} for {}{}",
            status
                .and_then(|s| s.code())
                .map(|c| c.to_string())
                .unwrap_or_else(|| "by a signal".into()),
            spec.link.source,
            if raw.is_empty() {
                " with nothing to say.".to_string()
            } else {
                format!(": {raw}")
            }
        );
        let kind = error::classify(&raw);
        let version = if error::wants_version_check(kind) {
            super::version_check::facts(profile).await
        } else {
            VersionFacts::unchecked()
        };
        // Read now: the jar can be written or thrown away while a download runs.
        let signed_in = YouTubeCookies::new(profile).is_signed_in();
        let failure = error::describe(kind, &raw, spec.link.is_youtube, signed_in, &version);
        return Outcome {
            title: reader.title,
            error: failure.message,
            needs_sign_in: failure.needs_sign_in,
            needs_update: failure.needs_update,
            ..Outcome::default()
        };
    }
    match reader.resolve_file(&spec.folder, started) {
        Some(file) => Outcome {
            success: true,
            file_path: Some(file),
            title: reader.title,
            ..Outcome::default()
        },
        None => Outcome {
            title: reader.title,
            error: t(s::Download_ErrorNoOutput).to_string(),
            ..Outcome::default()
        },
    }
}

async fn pump<R: tokio::io::AsyncRead + Unpin>(
    stream: R,
    reader: Arc<Mutex<LineReader>>,
    events: Arc<dyn Fn(RunEvent) + Send + Sync>,
) {
    let mut lines = BufReader::new(stream).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            // Not UTF-8 on one line is not a reason to stop reading the rest.
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => continue,
            Err(_) => break,
        };
        let event = reader.lock().ok().and_then(|mut r| r.read(&line));
        if let Some(event) = event {
            events(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::format::{AudioFormat, MediaKind, Quality};

    fn spec(link: &str, cookies: Option<&str>, ffmpeg: Option<&str>) -> RunSpec {
        RunSpec {
            tool: PathBuf::from("/x/yt-dlp"),
            link: DownloadLink::parse(link).unwrap(),
            folder: PathBuf::from("/Users/alex/Downloads"),
            format: DownloadFormat::new(MediaKind::Video, Quality::H1080, AudioFormat::M4a),
            ffmpeg: ffmpeg.map(PathBuf::from),
            quickjs: None,
            cookies: cookies.map(PathBuf::from),
        }
    }

    #[test]
    fn the_url_comes_last_after_a_double_dash_and_nothing_executes() {
        let args = build_arguments(&spec(
            "https://youtube.com/watch?v=-rf",
            Some("/p/YouTubeSignIn/cookies.txt"),
            Some("/p/Tools/ffmpeg"),
        ));
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args[args.len() - 1], "https://youtube.com/watch?v=-rf");
        assert_eq!(args.iter().filter(|a| *a == "--").count(), 1);
        assert!(!args.iter().any(|a| a.starts_with("--exec")));
        assert!(!args.iter().any(|a| a == "--ignore-config"));
        let at = |name: &str| args.iter().position(|a| a == name).unwrap();
        assert_eq!(args[at("--cookies") + 1], "/p/YouTubeSignIn/cookies.txt");
        assert_eq!(args[at("--ffmpeg-location") + 1], "/p/Tools/ffmpeg");
        assert_eq!(args[at("-P") + 1], "/Users/alex/Downloads");
        assert_eq!(args[at("-o") + 1], "%(title).200B [%(id)s].%(ext)s");
        assert_eq!(args[at("-S") + 1], "res:1080,ext:mp4:m4a");
        assert_eq!(
            args[..5],
            [
                "--color",
                "never",
                "--newline",
                "--progress",
                "--no-simulate"
            ]
        );
        assert!(args.contains(&"--no-playlist".to_string()));
    }

    #[test]
    fn no_jar_and_no_ffmpeg_mean_no_such_arguments() {
        let args = build_arguments(&spec("https://tiktok.com/@a/video/1", None, None));
        assert!(!args.iter().any(|a| a == "--cookies"));
        assert!(!args.iter().any(|a| a.contains("ffmpeg")));
        assert!(!args.iter().any(|a| a == "--merge-output-format"));
    }

    #[test]
    fn progress_lines_are_read_in_the_invariant_culture() {
        let format = DownloadFormat::new(MediaKind::Video, Quality::Best, AudioFormat::M4a);
        let mut reader = LineReader::new(format, true);
        let event = reader
            .read("@ML|downloading|1048576|4194304|NA|13033483.601051992|12")
            .unwrap();
        assert_eq!(
            event,
            RunEvent::Progress(Progress {
                percent: 25.0,
                downloaded: 1_048_576,
                total: 4_194_304,
                speed: 13_033_483.601051992,
                eta: 12,
                is_working: false
            })
        );
        // The estimate stands in when the total is not known.
        let RunEvent::Progress(p) = reader.read("@ML|downloading|500|NA|1000.0|NA|NA").unwrap()
        else {
            panic!()
        };
        assert_eq!((p.total, p.speed, p.eta), (1000, 0.0, -1));
        // A finished stream: working, and the next stream counts on top of it.
        let RunEvent::Progress(done) = reader
            .read("@ML|finished|4194304|4194304|NA|NA|NA")
            .unwrap()
        else {
            panic!()
        };
        assert!(done.is_working);
        let RunEvent::Progress(audio) = reader
            .read("@ML|downloading|100000|200000|NA|5000|3")
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(audio.downloaded, 4_194_304 + 100_000);
        assert_eq!(audio.total, 4_194_304 + 200_000);
        assert!(reader.read("@ML|garbage").is_none());
        assert!(reader.read("@ML|error|1|2|3|4|5").is_none());
    }

    #[test]
    fn markers_carry_the_title_paths_and_format() {
        let format = DownloadFormat::new(MediaKind::Video, Quality::H1080, AudioFormat::M4a);
        let mut reader = LineReader::new(format, true);
        assert_eq!(
            reader.read("@MT|Zażółć gęślą jaźń 🎵"),
            Some(RunEvent::Title("Zażółć gęślą jaźń 🎵".into()))
        );
        assert_eq!(reader.read("@MT|Second"), None, "the first title stands");
        assert_eq!(reader.read("@MF|/d/a [x].mp4"), None);
        assert_eq!(reader.final_paths, [PathBuf::from("/d/a [x].mp4")]);
        reader.read("@MP|/d/a [x].f137.mp4");
        assert_eq!(
            reader.predicted_path,
            Some(PathBuf::from("/d/a [x].f137.mp4"))
        );
        let Some(RunEvent::Format(lower)) = reader.read("@MR|720|1280") else {
            panic!()
        };
        assert_eq!(lower.resolution, 720);
        assert!(lower.note.contains("1080") && lower.note.contains("720"));
        let Some(RunEvent::Format(vertical)) = reader.read("@MR|1920|1080") else {
            panic!()
        };
        assert_eq!(
            vertical,
            FormatResult {
                resolution: 1080,
                note: String::new()
            }
        );
        let Some(RunEvent::Format(audio)) = reader.read("@MR|NA|NA") else {
            panic!()
        };
        assert_eq!(audio.resolution, 0);
    }

    #[test]
    fn without_ffmpeg_the_format_says_why() {
        let format = DownloadFormat::new(MediaKind::Audio, Quality::Best, AudioFormat::Mp3);
        let reader = LineReader::new(format, false);
        assert_eq!(reader.verdict(0).note, t(s::Download_NoteNoFFmpegAudio));
    }

    #[test]
    fn the_error_line_beats_a_later_warning() {
        let format = DownloadFormat::new(MediaKind::Video, Quality::Best, AudioFormat::M4a);
        let mut reader = LineReader::new(format, true);
        reader.read("[youtube] Extracting URL");
        reader.read("ERROR: [youtube] x: Private video");
        reader.read("WARNING: your yt-dlp is more than 90 days old");
        assert_eq!(reader.last_error(), "ERROR: [youtube] x: Private video");
        let mut quiet = LineReader::new(format, true);
        quiet.read("something broke");
        quiet.read("WARNING: old");
        assert_eq!(quiet.last_error(), "something broke");
    }

    #[test]
    fn the_file_is_found_even_without_after_move() {
        let dir = tempfile::tempdir().unwrap();
        let format = DownloadFormat::new(MediaKind::Video, Quality::Best, AudioFormat::M4a);
        let started = SystemTime::now() - std::time::Duration::from_secs(5);
        let mut reader = LineReader::new(format, true);
        std::fs::write(dir.path().join("a [x].mp4"), b"v").unwrap();
        std::fs::write(dir.path().join("b [y].mp4.part"), b"p").unwrap();
        reader.read(&format!(
            "@MP|{}",
            dir.path().join("a [x].f137.mp4").display()
        ));
        assert_eq!(
            reader.resolve_file(dir.path(), started),
            Some(dir.path().join("a [x].mp4"))
        );
        let empty = LineReader::new(format, true);
        assert_eq!(
            empty.resolve_file(dir.path(), started),
            Some(dir.path().join("a [x].mp4"))
        );
    }
}

//! The download queue's rows and the queue itself (`DownloadItemViewModel`, the queue
//! half of `DownloadViewModel`): states, what each row says, which buttons it offers,
//! and which row runs next. No process and no window here - the tab starts runs and
//! feeds their events back in.

use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use super::Destination;
use super::format::DownloadFormat;
use super::link::DownloadLink;
use super::run::{FormatResult, Outcome, Progress};
use crate::format::local_size;
use crate::icons;
use crate::l10n::{Arg, format, s, t};

/// Where one queued download has got to (`DownloadState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Queued,
    Starting,
    Downloading,
    Working,
    Paused,
    Completed,
    Failed,
    Canceled,
}

/// What the tab has to do with a finished file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Stayed on this Mac: show the "saved" notice.
    Saved(PathBuf),
    /// Hand it to the upload flow (and, for `Both`, it also stays).
    Upload(PathBuf),
}

/// One row of the queue.
#[derive(Clone, Debug)]
pub struct DownloadItem {
    pub id: u64,
    pub link: DownloadLink,
    pub destination: Destination,
    /// What was asked for, frozen at the moment the row was queued.
    pub format: DownloadFormat,
    /// Where yt-dlp writes. Fixed for the row's whole life, so a resume finds its .part.
    pub folder: PathBuf,
    /// The folder is ours (a Modlight-only download's scratch copy).
    pub folder_is_scratch: bool,
    pub state: State,
    pub title: String,
    pub detail: String,
    pub percent: f64,
    pub size_text: String,
    pub speed_text: String,
    pub eta_text: String,
    pub file_path: Option<PathBuf>,
    /// What was asked for, until yt-dlp says what it found.
    pub format_text: String,
    requested_format_text: String,
    pub note: String,
    pub needs_sign_in: bool,
    pub needs_update: bool,
    pub updating_tool: bool,
    /// This stop is a pause.
    pausing: bool,
    cancel: Option<CancellationToken>,
}

/// A local file's size the way the rest of the app spells it, or empty for nothing.
pub fn size(bytes: i64, binary: bool) -> String {
    if bytes > 0 {
        local_size(bytes, binary, 1)
    } else {
        String::new()
    }
}

/// `TimeSpan` as "m:ss", or "h:mm:ss" from an hour.
pub fn clock(seconds: i32) -> String {
    let seconds = seconds.clamp(0, 359_999);
    let (h, m, s) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    if h >= 1 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl DownloadItem {
    pub fn new(
        id: u64,
        link: DownloadLink,
        folder: PathBuf,
        folder_is_scratch: bool,
        destination: Destination,
        format: DownloadFormat,
        format_label: &str,
    ) -> DownloadItem {
        DownloadItem {
            id,
            title: link.url.clone(),
            link,
            destination,
            format,
            folder,
            folder_is_scratch,
            state: State::Queued,
            detail: t(s::Download_StateQueued).to_string(),
            percent: 0.0,
            size_text: String::new(),
            speed_text: String::new(),
            eta_text: String::new(),
            file_path: None,
            format_text: format_label.to_string(),
            requested_format_text: format_label.to_string(),
            note: String::new(),
            needs_sign_in: false,
            needs_update: false,
            updating_tool: false,
            pausing: false,
            cancel: None,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state,
            State::Starting | State::Downloading | State::Working
        )
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self.state,
            State::Completed | State::Failed | State::Canceled
        )
    }

    pub fn show_progress(&self) -> bool {
        matches!(
            self.state,
            State::Starting | State::Downloading | State::Working | State::Paused
        )
    }

    pub fn is_indeterminate(&self) -> bool {
        matches!(self.state, State::Starting | State::Working)
    }

    pub fn can_pause(&self) -> bool {
        matches!(
            self.state,
            State::Downloading | State::Starting | State::Queued
        )
    }

    pub fn can_resume(&self) -> bool {
        self.state == State::Paused
    }

    pub fn can_cancel(&self) -> bool {
        !self.is_finished()
    }

    pub fn can_retry(&self) -> bool {
        matches!(self.state, State::Failed | State::Canceled)
    }

    pub fn can_sign_in(&self) -> bool {
        self.state == State::Failed && self.needs_sign_in
    }

    pub fn can_update_tool(&self) -> bool {
        self.state == State::Failed && self.needs_update && !self.updating_tool
    }

    pub fn can_reveal(&self) -> bool {
        self.state == State::Completed && !self.folder_is_scratch && self.file_path.is_some()
    }

    pub fn has_numbers(&self) -> bool {
        !self.speed_text.is_empty() || !self.size_text.is_empty() || !self.eta_text.is_empty()
    }

    pub fn is_failed(&self) -> bool {
        self.state == State::Failed
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Completed
    }

    /// The glyph on the left of the row: the state at a glance.
    pub fn icon(&self) -> char {
        match self.state {
            State::Queued => icons::CLOCK,
            State::Paused => icons::CIRCLE_PAUSE,
            State::Completed => icons::CIRCLE_CHECK,
            State::Failed => icons::TRIANGLE_ALERT,
            State::Canceled => icons::BAN,
            _ => icons::ARROW_DOWN_TO_LINE,
        }
    }

    /// Begin, or begin again after a pause. Returns the token the run listens to.
    pub fn start(&mut self) -> Option<CancellationToken> {
        if self.is_active() {
            return None;
        }
        self.pausing = false;
        self.needs_sign_in = false;
        self.needs_update = false;
        self.note.clear();
        self.format_text = self.requested_format_text.clone();
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        self.state = State::Starting;
        self.detail = t(s::Download_StateStarting).to_string();
        self.speed_text.clear();
        self.eta_text.clear();
        Some(token)
    }

    /// Stop, keep what has been fetched, and expect to be asked again.
    pub fn pause(&mut self) {
        if self.state == State::Queued {
            self.state = State::Paused;
            self.detail = t(s::Download_StatePaused).to_string();
            return;
        }
        if !self.is_active() {
            return;
        }
        self.pausing = true;
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
        }
    }

    pub fn resume(&mut self) {
        if self.state != State::Paused {
            return;
        }
        self.state = State::Queued;
        self.detail = t(s::Download_StateQueued).to_string();
    }

    pub fn cancel(&mut self) {
        if self.is_finished() {
            return;
        }
        self.pausing = false;
        if self.is_active() {
            if let Some(cancel) = &self.cancel {
                cancel.cancel();
            }
            return;
        }
        self.stopped(t(s::Download_StateCanceled), State::Canceled);
    }

    pub fn retry(&mut self) {
        if !self.can_retry() {
            return;
        }
        self.percent = 0.0;
        self.size_text.clear();
        self.speed_text.clear();
        self.eta_text.clear();
        self.state = State::Queued;
        self.detail = t(s::Download_StateQueued).to_string();
    }

    /// Called when the row is dropped from the list, so nothing is left running.
    pub fn dispose(&mut self) {
        self.pausing = false;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }

    pub fn named(&mut self, found: &str) {
        if !found.is_empty() {
            self.title = found.to_string();
        }
    }

    /// yt-dlp has picked a format: say what it is, and whether it is what was asked for.
    pub fn chose(&mut self, chosen: &FormatResult) {
        if chosen.resolution > 0 {
            self.format_text = format(
                t(s::Download_QualityHeight),
                &[Arg::from(chosen.resolution)],
            );
        }
        self.note = chosen.note.clone();
    }

    pub fn apply(&mut self, progress: &Progress, binary: bool) {
        if !self.is_active() {
            // A last update that crossed with a pause.
            return;
        }
        if progress.is_working {
            self.state = State::Working;
            self.detail = t(s::Download_StateWorking).to_string();
            self.speed_text.clear();
            self.eta_text.clear();
            return;
        }
        self.state = State::Downloading;
        self.detail = t(s::Download_StateDownloading).to_string();
        self.percent = progress.percent;
        self.size_text = if progress.total > 0 {
            format(
                t(s::Download_SizeFormat),
                &[
                    Arg::from(size(progress.downloaded, binary)),
                    Arg::from(size(progress.total, binary)),
                ],
            )
        } else {
            size(progress.downloaded, binary)
        };
        self.speed_text = if progress.speed > 0.0 {
            format(
                t(s::Download_SpeedFormat),
                &[Arg::from(size(progress.speed as i64, binary))],
            )
        } else {
            String::new()
        };
        self.eta_text = if progress.eta >= 0 {
            format(t(s::Download_EtaFormat), &[Arg::from(clock(progress.eta))])
        } else {
            String::new()
        };
    }

    /// The run ended. Returns what the tab must do with the file, if anything.
    pub fn finish(&mut self, outcome: &Outcome, binary: bool) -> Option<Delivery> {
        self.cancel = None;
        if let Some(title) = &outcome.title {
            self.named(title);
        }
        if outcome.canceled {
            if self.pausing {
                self.pausing = false;
                self.state = State::Paused;
                self.detail = t(s::Download_StatePaused).to_string();
                self.speed_text.clear();
                self.eta_text.clear();
                return None;
            }
            self.stopped(t(s::Download_StateCanceled), State::Canceled);
            return None;
        }
        let Some(path) = outcome.file_path.clone().filter(|_| outcome.success) else {
            self.needs_sign_in = outcome.needs_sign_in;
            self.needs_update = outcome.needs_update;
            let reason = if outcome.error.is_empty() {
                t(s::Download_ErrorUnknown).to_string()
            } else {
                outcome.error.clone()
            };
            self.stopped(&reason, State::Failed);
            return None;
        };
        self.file_path = Some(path.clone());
        self.percent = 100.0;
        self.speed_text.clear();
        self.eta_text.clear();
        self.size_text = size(
            std::fs::metadata(&path)
                .map(|m| m.len() as i64)
                .unwrap_or(0),
            binary,
        );
        self.title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = self.folder.display().to_string();
        let delivery = match self.destination {
            Destination::Disk => {
                self.detail = format(t(s::Download_SavedTo), &[Arg::from(folder)]);
                Delivery::Saved(path)
            }
            Destination::Both => {
                self.detail = format(t(s::Download_SavedAndSent), &[Arg::from(folder)]);
                Delivery::Upload(path)
            }
            Destination::Modlight => {
                self.detail = t(s::Download_Sent).to_string();
                Delivery::Upload(path)
            }
        };
        self.state = State::Completed;
        Some(delivery)
    }

    /// The upload could not even be handed over.
    pub fn delivery_failed(&mut self, message: &str) {
        self.detail = format(t(s::Download_ErrorRaw), &[Arg::from(message)]);
    }

    fn stopped(&mut self, reason: &str, last: State) {
        self.speed_text.clear();
        self.eta_text.clear();
        self.detail = reason.to_string();
        self.state = last;
    }
}

/// The queue: newest first on screen, oldest waiting row first to run, one at a time.
#[derive(Clone, Debug, Default)]
pub struct DownloadQueue {
    pub items: Vec<DownloadItem>,
    next_id: u64,
}

impl DownloadQueue {
    /// Put a row at the top. Returns its id.
    pub fn enqueue(
        &mut self,
        link: DownloadLink,
        folder: PathBuf,
        folder_is_scratch: bool,
        destination: Destination,
        format: DownloadFormat,
        format_label: &str,
    ) -> u64 {
        self.next_id += 1;
        let item = DownloadItem::new(
            self.next_id,
            link,
            folder,
            folder_is_scratch,
            destination,
            format,
            format_label,
        );
        self.items.insert(0, item);
        self.next_id
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut DownloadItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&DownloadItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn remove(&mut self, id: u64) {
        if let Some(at) = self.items.iter().position(|i| i.id == id) {
            let mut item = self.items.remove(at);
            item.dispose();
        }
    }

    pub fn clear_finished(&mut self) {
        self.items.retain(|i| !i.is_finished());
    }

    pub fn has_finished(&self) -> bool {
        self.items.iter().any(DownloadItem::is_finished)
    }

    /// The next row to start, or `None` while one is running (`Pump`).
    pub fn next_to_start(&self) -> Option<u64> {
        if self.items.iter().any(DownloadItem::is_active) {
            return None;
        }
        self.items
            .iter()
            .rev()
            .find(|i| i.state == State::Queued)
            .map(|i| i.id)
    }

    /// Every row that failed for want of a session goes back in the queue, after a
    /// session was actually captured.
    pub fn retry_sign_in_failures(&mut self) {
        for item in &mut self.items {
            if item.can_sign_in() {
                item.retry();
            }
        }
    }

    /// Stop everything: the window is going away for good.
    pub fn shutdown(&mut self) {
        for item in &mut self.items {
            item.dispose();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::format::{AudioFormat, MediaKind, Quality};

    fn queue_with(count: usize) -> DownloadQueue {
        let mut queue = DownloadQueue::default();
        for i in 0..count {
            queue.enqueue(
                DownloadLink::parse(&format!("https://youtu.be/{i}")).unwrap(),
                PathBuf::from("/d"),
                false,
                Destination::Disk,
                DownloadFormat::new(MediaKind::Video, Quality::H1080, AudioFormat::M4a),
                "1080p",
            );
        }
        queue
    }

    #[test]
    fn the_oldest_waiting_row_runs_first_and_alone() {
        let mut queue = queue_with(3);
        assert_eq!(queue.items[0].id, 3, "newest on top");
        assert_eq!(queue.next_to_start(), Some(1));
        let token = queue.get_mut(1).unwrap().start().unwrap();
        assert_eq!(queue.next_to_start(), None);
        queue.get_mut(1).unwrap().pause();
        assert!(token.is_cancelled());
        queue.get_mut(1).unwrap().finish(
            &Outcome {
                canceled: true,
                ..Outcome::default()
            },
            false,
        );
        assert_eq!(queue.get(1).unwrap().state, State::Paused);
        assert_eq!(queue.next_to_start(), Some(2));
        queue.get_mut(1).unwrap().resume();
        assert_eq!(queue.next_to_start(), Some(1));
    }

    #[test]
    fn a_waiting_row_can_be_paused_and_cancelled_without_a_process() {
        let mut queue = queue_with(1);
        let item = queue.get_mut(1).unwrap();
        item.pause();
        assert_eq!(item.state, State::Paused);
        item.cancel();
        assert_eq!(item.state, State::Canceled);
        assert!(item.can_retry());
        item.retry();
        assert_eq!(item.state, State::Queued);
    }

    #[test]
    fn a_failure_offers_what_the_run_said() {
        let mut queue = queue_with(1);
        let item = queue.get_mut(1).unwrap();
        item.start();
        item.chose(&FormatResult {
            resolution: 720,
            note: "lower".into(),
        });
        assert_eq!(item.format_text, "720p");
        item.finish(
            &Outcome {
                error: "nope".into(),
                needs_sign_in: true,
                ..Outcome::default()
            },
            false,
        );
        assert!(item.can_sign_in() && !item.can_update_tool());
        assert_eq!(item.detail, "nope");
        queue.retry_sign_in_failures();
        let item = queue.get_mut(1).unwrap();
        assert_eq!(item.state, State::Queued);
        item.start();
        assert_eq!(
            item.format_text, "1080p",
            "a new run starts from what was asked"
        );
        assert!(item.note.is_empty());
    }

    #[test]
    fn progress_is_spelled_in_local_units() {
        let mut queue = queue_with(1);
        let item = queue.get_mut(1).unwrap();
        item.start();
        item.apply(
            &Progress {
                percent: 50.0,
                downloaded: 12_400_000,
                total: 24_800_000,
                speed: 2_500_000.0,
                eta: 3725,
                is_working: false,
            },
            false,
        );
        assert_eq!(item.state, State::Downloading);
        assert!(item.size_text.contains("12.4 MB") && item.size_text.contains("24.8 MB"));
        assert!(item.speed_text.contains("2.5 MB"));
        assert!(item.eta_text.contains("1:02:05"));
        assert_eq!(clock(65), "1:05");
        assert_eq!(clock(-3), "0:00");
    }

    #[test]
    fn a_finished_file_is_delivered_where_the_destination_says() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("video [x].mp4");
        std::fs::write(&file, b"v").unwrap();
        for (destination, upload) in [
            (Destination::Disk, false),
            (Destination::Modlight, true),
            (Destination::Both, true),
        ] {
            let mut item = DownloadItem::new(
                1,
                DownloadLink::parse("https://youtu.be/x").unwrap(),
                dir.path().to_path_buf(),
                destination == Destination::Modlight,
                destination,
                DownloadFormat::new(MediaKind::Video, Quality::Best, AudioFormat::M4a),
                "Best",
            );
            item.start();
            let delivery = item.finish(
                &Outcome {
                    success: true,
                    file_path: Some(file.clone()),
                    ..Outcome::default()
                },
                false,
            );
            assert_eq!(item.state, State::Completed);
            assert_eq!(item.title, "video [x].mp4");
            match delivery {
                Some(Delivery::Upload(_)) => assert!(upload),
                Some(Delivery::Saved(_)) => assert!(!upload),
                None => panic!("nothing delivered"),
            }
            assert_eq!(item.can_reveal(), destination != Destination::Modlight);
        }
    }
}

//! The one place that decides whether a task on its way to Modlight has an account to
//! go to, and what to do when it has not (`ModlightUploadGate`).
//!
//! Sign-in is asked for here rather than at start: the app starts silently in the menu
//! bar, and a window jumping up on every boot would ask a question nobody posed. A task
//! about to upload is the moment the question becomes real.

use crate::tasks::context::TaskContext;
use crate::tasks::host::{TaskAnswer, TaskQuestion};
use crate::tasks::info::{DataType, TaskInfo, TaskJob};
use crate::tasks::settings::{
    AfterCaptureTasks, AfterUploadTasks, FileDestination, ImageDestination, TextDestination,
    UrlShortenerType,
};

/// Whether anything about this task needs a Modlight account.
pub fn targets_modlight(info: &TaskInfo) -> bool {
    info.is_upload_job() && (uses_uploader(info) || uses_shortener(info))
}

/// Whether the FILE itself goes to Modlight: narrower on purpose, because the offline
/// queue re-sends bytes and must not send a file somewhere it was never going.
pub fn uploads_to_modlight(info: &TaskInfo) -> bool {
    info.is_upload_job() && uses_uploader(info)
}

/// Targets Modlight and nobody is signed in.
pub fn is_signed_out(info: &TaskInfo, ctx: &TaskContext) -> bool {
    targets_modlight(info) && ctx.credential().is_none()
}

/// Ask for sign-in, and answer whether the upload may go ahead. The task waits on the
/// window, as upstream's worker thread does: captures taken meanwhile wait with it.
pub async fn allow(info: &TaskInfo, ctx: &TaskContext, task: u64) -> bool {
    if !is_signed_out(info, ctx) {
        return true;
    }
    log::info!("Modlight: this upload needs an account. Asking to sign in.");
    let signed_in = matches!(
        ctx.channel.ask(task, TaskQuestion::SignIn).await,
        Some(TaskAnswer::SignIn(true))
    ) && ctx.credential().is_some();
    if !signed_in {
        log::info!(
            "Modlight: still signed out, so the upload is skipped and the file is kept locally."
        );
    }
    signed_in
}

/// Make sure this task leaves something on disk even if the upload never happens
/// (`KeepLocally`): a capture exists nowhere else, and "delete once uploaded" cannot be
/// authorised by an upload that may not happen.
pub fn keep_locally(info: &mut TaskInfo) {
    let settings = &mut info.settings;
    settings.after_capture_job = settings
        .after_capture_job
        .remove(AfterCaptureTasks::DeleteFile);
    if info.job == TaskJob::Job {
        settings.after_capture_job = settings
            .after_capture_job
            .with(AfterCaptureTasks::SaveImageToFile);
    }
}

fn uses_uploader(info: &TaskInfo) -> bool {
    let settings = &info.settings;
    match info.upload_destination() {
        DataType::Image => settings.image_destination == ImageDestination::Modlight,
        DataType::Text => settings.text_destination == TextDestination::Modlight,
        DataType::File => {
            settings.file_destination_for(info.data_type) == FileDestination::Modlight
        }
        _ => false,
    }
}

/// The shortener counts too, whether it is the whole job or an after-upload step: it
/// needs the same token, and finding that out after the upload would be too late.
fn uses_shortener(info: &TaskInfo) -> bool {
    if info.job == TaskJob::ShareUrl
        || info.settings.url_shortener_destination != UrlShortenerType::Modlight
    {
        return false;
    }
    info.job == TaskJob::ShortenUrl
        || info
            .settings
            .after_upload_job
            .has(AfterUploadTasks::UseURLShortener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::settings::TaskSettings;

    fn info(job: TaskJob, data_type: DataType) -> TaskInfo {
        let mut info = TaskInfo::new(TaskSettings::default());
        info.job = job;
        info.data_type = data_type;
        info
    }

    #[test]
    fn what_needs_an_account() {
        assert!(targets_modlight(&info(TaskJob::FileUpload, DataType::File)));
        assert!(uploads_to_modlight(&info(
            TaskJob::FileUpload,
            DataType::File
        )));
        assert!(targets_modlight(&info(TaskJob::ShortenUrl, DataType::Url)));
        assert!(!uploads_to_modlight(&info(
            TaskJob::ShortenUrl,
            DataType::Url
        )));
        assert!(!targets_modlight(&info(TaskJob::ShareUrl, DataType::Url)));
        assert!(!targets_modlight(&info(TaskJob::Download, DataType::File)));
        let mut capture = info(TaskJob::Job, DataType::Image);
        assert!(
            targets_modlight(&capture),
            "UploadImageToHost is on by default"
        );
        capture.settings.after_capture_job = AfterCaptureTasks::SaveImageToFile;
        assert!(!targets_modlight(&capture));
    }

    #[test]
    fn keeping_locally_saves_captures_and_never_deletes() {
        let mut capture = info(TaskJob::Job, DataType::Image);
        capture.settings.after_capture_job =
            AfterCaptureTasks::UploadImageToHost.with(AfterCaptureTasks::DeleteFile);
        keep_locally(&mut capture);
        assert!(
            capture
                .settings
                .after_capture_job
                .has(AfterCaptureTasks::SaveImageToFile)
        );
        assert!(
            !capture
                .settings
                .after_capture_job
                .has(AfterCaptureTasks::DeleteFile)
        );

        let mut file = info(TaskJob::FileUpload, DataType::File);
        file.settings.after_capture_job = AfterCaptureTasks::DeleteFile;
        keep_locally(&mut file);
        assert_eq!(file.settings.after_capture_job, AfterCaptureTasks::None);
    }
}

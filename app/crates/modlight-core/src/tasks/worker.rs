//! One task from start to end (`WorkerTask`): what it is made from, the work before the
//! upload, the upload with its retries and its hand-over to the offline queue, and the
//! work after it. Runs on the background runtime; every window it needs is a question
//! sent to the app and awaited (see `host.rs`), where upstream blocks its thread.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::l10n::{Arg, format, s, t};
use crate::tasks::context::TaskContext;
use crate::tasks::gate;
use crate::tasks::host::{MessageIcon, TaskAnswer, TaskEffect, TaskEvent, TaskQuestion};
use crate::tasks::info::{DataType, TaskInfo, TaskJob, TaskMetadata, TaskStatus, UploadResult};
use crate::tasks::naming;
use crate::tasks::settings::{AfterCaptureTasks, AfterUploadTasks, TaskSettings};
use crate::tasks::uploader;
use crate::upload::queue::{LiveSession, TakeOver};

/// The bytes a task uploads (`WorkerTask.Data`): a file it reads, or bytes in memory.
#[derive(Clone, Debug)]
pub enum TaskData {
    File(PathBuf),
    Bytes(Arc<Vec<u8>>),
}

impl TaskData {
    pub fn len(&self) -> u64 {
        match self {
            TaskData::File(path) => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            TaskData::Bytes(bytes) => bytes.len() as u64,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> std::io::Result<Vec<u8>> {
        match self {
            TaskData::File(path) => std::fs::read(path),
            TaskData::Bytes(bytes) => Ok(bytes.as_ref().clone()),
        }
    }
}

/// What a task's window and the task list read; written by the task as it runs.
#[derive(Clone, Debug)]
pub struct TaskState {
    pub info: TaskInfo,
    pub status: TaskStatus,
    pub stop_requested: bool,
    pub early_url_copied: bool,
    pub keep_image: bool,
    /// The bytes to upload, while the task holds them.
    pub data: Option<TaskData>,
    /// The text of a text task.
    pub text: Option<String>,
    /// The picture of an image task.
    pub image: Option<Arc<image::RgbaImage>>,
    /// The server's status on the last attempt, `None` when nothing answered: what the
    /// offline queue decides on.
    pub last_answer: Option<u16>,
    /// The resumable session a large upload left on the server, for the next attempt or
    /// the offline queue to carry on from.
    pub live_session: Option<LiveSession>,
}

/// One task, as the task list holds it.
#[derive(Clone)]
pub struct WorkerTask {
    pub id: u64,
    shared: Arc<Mutex<TaskState>>,
    /// Stops the transfer in flight (`uploader.StopUpload`).
    upload_cancel: CancellationToken,
}

impl std::fmt::Debug for WorkerTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerTask").field("id", &self.id).finish()
    }
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl WorkerTask {
    fn new(settings: TaskSettings) -> WorkerTask {
        WorkerTask {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            shared: Arc::new(Mutex::new(TaskState {
                info: TaskInfo::new(settings),
                status: TaskStatus::InQueue,
                stop_requested: false,
                early_url_copied: false,
                keep_image: false,
                data: None,
                text: None,
                image: None,
                last_answer: None,
                live_session: None,
            })),
            upload_cancel: CancellationToken::new(),
        }
    }

    /// A copy of what the task is now.
    pub fn snapshot(&self) -> TaskState {
        self.lock().clone()
    }

    pub fn status(&self) -> TaskStatus {
        self.lock().status
    }

    pub fn info(&self) -> TaskInfo {
        self.lock().info.clone()
    }

    pub fn is_busy(&self) -> bool {
        self.status().is_busy()
    }

    pub fn is_working(&self) -> bool {
        self.status().is_working()
    }

    pub fn stop_requested(&self) -> bool {
        self.lock().stop_requested
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TaskState> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Read or change the task's state (the worker, and fixtures that draw a task).
    pub fn with<R>(&self, f: impl FnOnce(&mut TaskState) -> R) -> R {
        f(&mut self.lock())
    }

    // ------------------------------------------------------------------ constructors

    /// `CreateHistoryTask`: a finished upload restored from the recent list.
    pub fn history(recent: &crate::tasks::recent::RecentTask) -> WorkerTask {
        let task = WorkerTask::new(TaskSettings::default());
        task.with(|s| {
            s.status = TaskStatus::History;
            s.info.set_file_path(&recent.file_path);
            s.info.file_name = recent.file_name();
            s.info.result.url = recent.url.clone();
            s.info.result.thumbnail_url = recent.thumbnail_url.clone();
            s.info.result.deletion_url = recent.deletion_url.clone();
            s.info.result.shortened_url = recent.shortened_url.clone();
            s.info.task_end_time = Some(recent.time);
        });
        task
    }

    /// `CreateDataUploaderTask`.
    pub fn data_upload(
        data_type: DataType,
        bytes: Vec<u8>,
        file_name: &str,
        settings: TaskSettings,
    ) -> WorkerTask {
        let task = WorkerTask::new(settings);
        task.with(|s| {
            s.info.job = TaskJob::DataUpload;
            s.info.data_type = data_type;
            s.info.file_name = file_name.to_string();
            s.data = Some(TaskData::Bytes(Arc::new(bytes)));
        });
        task
    }

    /// `CreateFileUploaderTask`. `None` when the file cannot be read.
    pub fn file_upload(
        path: &Path,
        settings: TaskSettings,
        ctx: &TaskContext,
    ) -> Option<WorkerTask> {
        let task = WorkerTask::new(settings);
        let path_text = path.to_string_lossy().into_owned();
        let mut failed = None;
        task.with(|s| {
            s.info.set_file_path(&path_text);
            s.info.data_type = naming::find_data_type(&path_text, &s.info.settings);
            if s.info.settings.upload_settings.file_upload_use_name_pattern {
                let ext = naming::extension(&path_text);
                s.info.file_name = ctx.file_name(&s.info.settings, Some(&ext), None);
            }
            if s.info
                .settings
                .advanced_settings
                .process_images_during_file_upload
                && s.info.data_type == DataType::Image
            {
                s.info.job = TaskJob::Job;
                match crate::image_decode::open(path) {
                    Ok(picture) => {
                        let picture = Arc::new(picture.into_rgba8());
                        s.info.metadata.image = Some(picture.clone());
                        s.image = Some(picture);
                    }
                    Err(error) => failed = Some(error.to_string()),
                }
            } else {
                s.info.job = TaskJob::FileUpload;
                match std::fs::File::open(path) {
                    Ok(_) => s.data = Some(TaskData::File(path.to_path_buf())),
                    Err(error) => failed = Some(error.to_string()),
                }
            }
        });
        if let Some(error) = failed {
            // `e.ShowError()`: a message box, and no task.
            ctx.channel.effect(TaskEffect::MessageBox {
                text: error,
                title: "Modlight".into(),
                icon: MessageIcon::Error,
            });
            return None;
        }
        Some(task)
    }

    /// `CreateFileJobTask`: a recording (or another finished file) through the
    /// after-capture work and, when that includes uploading, the upload.
    pub fn file_job(
        path: &Path,
        metadata: TaskMetadata,
        settings: TaskSettings,
        custom_file_name: Option<&str>,
        ctx: &TaskContext,
    ) -> Option<WorkerTask> {
        let task = WorkerTask::new(settings);
        let path_text = path.to_string_lossy().into_owned();
        let upload = task.with(|s| {
            s.info.set_file_path(&path_text);
            s.info.data_type = naming::find_data_type(&path_text, &s.info.settings);
            let ext = naming::extension(&path_text);
            if let Some(name) = custom_file_name.filter(|n| !n.is_empty()) {
                s.info.file_name = naming::append_extension(name, &ext);
            } else if s.info.settings.upload_settings.file_upload_use_name_pattern {
                s.info.file_name = ctx.file_name(&s.info.settings, Some(&ext), None);
            }
            s.info.metadata = metadata;
            s.info.job = TaskJob::Job;
            s.info.is_upload_job()
        });
        if upload {
            if std::fs::File::open(path).is_err() {
                return None;
            }
            task.with(|s| s.data = Some(TaskData::File(path.to_path_buf())));
        }
        Some(task)
    }

    /// `CreateImageUploaderTask`.
    pub fn image_upload(
        metadata: TaskMetadata,
        settings: TaskSettings,
        custom_file_name: Option<&str>,
        ctx: &TaskContext,
    ) -> WorkerTask {
        let task = WorkerTask::new(settings);
        task.with(|s| {
            s.info.job = TaskJob::Job;
            s.info.data_type = DataType::Image;
            s.info.file_name = match custom_file_name.filter(|n| !n.is_empty()) {
                Some(name) => naming::append_extension(name, "bmp"),
                None => ctx.file_name(&s.info.settings, Some("bmp"), Some(&metadata)),
            };
            s.image = metadata.image.clone();
            s.info.metadata = metadata;
        });
        task
    }

    /// `CreateTextUploaderTask`.
    pub fn text_upload(text: &str, settings: TaskSettings, ctx: &TaskContext) -> WorkerTask {
        let task = WorkerTask::new(settings);
        task.with(|s| {
            s.info.job = TaskJob::TextUpload;
            s.info.data_type = DataType::Text;
            let extension = s
                .info
                .settings
                .advanced_settings
                .text_file_extension
                .clone();
            s.info.file_name = ctx.file_name(&s.info.settings, Some(&extension), None);
            s.text = Some(text.to_string());
        });
        task
    }

    /// `CreateURLShortenerTask`.
    pub fn url_shortener(url: &str, settings: TaskSettings) -> WorkerTask {
        let task = WorkerTask::new(settings);
        task.with(|s| {
            s.info.job = TaskJob::ShortenUrl;
            s.info.data_type = DataType::Url;
            s.info.file_name = format(
                t(s::UploadTask_CreateURLShortenerTask_Shorten_URL___0__),
                &[Arg::from("Modlight")],
            );
            s.info.result.url = url.to_string();
        });
        task
    }

    /// `CreateShareURLTask`.
    pub fn share_url(url: &str, settings: TaskSettings) -> WorkerTask {
        let task = WorkerTask::new(settings);
        task.with(|s| {
            s.info.job = TaskJob::ShareUrl;
            s.info.data_type = DataType::Url;
            let service =
                uploader::sharing_service_name(s.info.settings.url_sharing_service_destination);
            s.info.file_name = format(
                t(s::UploadTask_CreateShareURLTask_Share_URL___0__),
                &[Arg::from(service)],
            );
            s.info.result.url = url.to_string();
        });
        task
    }

    /// `CreateDownloadTask`. `None` when no file name can be made from the address.
    pub fn download(
        url: &str,
        upload: bool,
        settings: TaskSettings,
        ctx: &TaskContext,
    ) -> Option<WorkerTask> {
        let task = WorkerTask::new(settings);
        let mut ok = true;
        task.with(|s| {
            s.info.job = if upload {
                TaskJob::DownloadUpload
            } else {
                TaskJob::Download
            };
            let mut file_name = naming::file_name_from_url(url);
            if s.info.settings.upload_settings.file_upload_use_name_pattern {
                let ext = naming::extension(&file_name);
                file_name = ctx.file_name(&s.info.settings, Some(&ext), None);
            }
            if file_name.is_empty() {
                ok = false;
                return;
            }
            s.info.data_type = naming::find_data_type(&file_name, &s.info.settings);
            s.info.file_name = file_name;
            s.info.result.url = url.to_string();
        });
        ok.then_some(task)
    }

    // ------------------------------------------------------------------ control

    /// `Start`: only a queued task that nobody stopped.
    pub fn start(&self, ctx: &TaskContext) {
        let job = {
            let mut s = self.lock();
            if s.status != TaskStatus::InQueue || s.stop_requested {
                return;
            }
            s.info.task_start_time = Some(Utc::now());
            // `Prepare`.
            s.status = TaskStatus::Preparing;
            s.info.status = match s.info.job {
                TaskJob::Job | TaskJob::TextUpload => t(s::UploadTask_Prepare_Preparing),
                _ => t(s::UploadTask_Prepare_Starting),
            }
            .to_string();
            s.info.job
        };
        let _ = job;
        ctx.channel.event(self.id, TaskEvent::StatusChanged);
        let task = self.clone();
        let ctx = ctx.clone();
        ctx.runtime.clone().spawn(async move {
            task.thread_do_work(&ctx).await;
            task.on_task_completed(&ctx);
        });
    }

    /// `Stop`.
    pub fn stop(&self, ctx: &TaskContext) {
        let status = {
            let mut s = self.lock();
            s.stop_requested = true;
            s.status
        };
        match status {
            TaskStatus::InQueue => self.on_task_completed(ctx),
            TaskStatus::Preparing | TaskStatus::Working => {
                self.upload_cancel.cancel();
                self.with(|s| {
                    s.status = TaskStatus::Stopping;
                    s.info.status = t(s::UploadTask_Stop_Stopping).to_string();
                });
                ctx.channel.event(self.id, TaskEvent::StatusChanged);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ the work

    async fn thread_do_work(&self, ctx: &TaskContext) {
        self.create_task_reference();

        // Asked before the capture work, answered after it: with no account, the only
        // chance to make sure a capture survives is to turn the local save on before
        // the work writes it. The sign-in window itself comes later, in the upload.
        let signed_out = {
            let info = self.info();
            gate::is_signed_out(&info, ctx)
        };
        if signed_out {
            self.with(|s| gate::keep_locally(&mut s.info));
        }

        let ok = self.do_thread_job(ctx).await;
        if !ok {
            self.with(|s| s.stop_requested = true);
        }
        if self.lock().image.is_some() {
            ctx.channel.event(self.id, TaskEvent::ImageReady);
        }

        if !self.stop_requested() {
            let is_upload_job = self.lock().info.is_upload_job();
            if is_upload_job && ctx.is_upload_allowed() {
                self.do_upload_job(ctx).await;
            } else {
                self.with(|s| s.info.result.is_url_expected = false);
            }
        }

        // `finally`.
        let (early, stop, url_empty, delete) = self.with(|s| {
            s.keep_image = s.image.is_some()
                && s.info
                    .settings
                    .general_settings
                    .show_toast_notification_after_task_completed;
            s.data = None;
            let settings = &s.info.settings;
            let delete = (s.info.job == TaskJob::Job
                || (s.info.job == TaskJob::FileUpload
                    && settings
                        .advanced_settings
                        .use_after_capture_tasks_during_file_upload))
                && settings
                    .after_capture_job
                    .has(AfterCaptureTasks::DeleteFile)
                && !s.info.file_path().is_empty()
                && Path::new(s.info.file_path()).is_file();
            (
                s.early_url_copied,
                s.stop_requested,
                s.info.result.url.is_empty(),
                delete.then(|| PathBuf::from(s.info.file_path())),
            )
        });
        if early && (stop || url_empty) {
            ctx.channel.effect(TaskEffect::ClearClipboard);
        }
        if let Some(path) = delete
            && let Err(error) = std::fs::remove_file(&path)
        {
            log::warn!("Modlight: the file could not be deleted after upload: {error}");
        }

        let (stop, expected, is_error, url_empty) = self.with(|s| {
            (
                s.stop_requested,
                s.info.result.is_url_expected,
                s.info.result.is_error(),
                s.info.result.url.is_empty(),
            )
        });
        if !stop && expected && !is_error {
            if url_empty {
                self.with(|s| {
                    s.info
                        .result
                        .errors
                        .add(t(s::UploadTask_ThreadDoWork_URL_is_empty_))
                });
            } else {
                self.do_after_upload_jobs(ctx).await;
            }
        }
    }

    /// Where this one file is going, decided once (`CreateTaskReferenceHelper`): what
    /// the task was told, else the folder the upload settings name, else the folder My
    /// space is standing in.
    fn create_task_reference(&self) {
        self.with(|s| {
            let configured = s
                .info
                .settings
                .upload_settings
                .modlight_folder_id
                .clone()
                .filter(|f| !f.is_empty());
            s.info.modlight_folder_id = s
                .info
                .settings
                .modlight_folder_id
                .clone()
                .or(configured)
                .or_else(crate::upload::target::current);
        });
    }

    async fn do_thread_job(&self, ctx: &TaskContext) -> bool {
        let (is_upload, auto_clear, job) = self.with(|s| {
            (
                s.info.is_upload_job(),
                s.info.settings.advanced_settings.auto_clear_clipboard,
                s.info.job,
            )
        });
        if is_upload && auto_clear {
            ctx.channel.effect(TaskEffect::ClearClipboard);
        }

        if matches!(job, TaskJob::Download | TaskJob::DownloadUpload) {
            if !self
                .download_from_url(job == TaskJob::DownloadUpload, ctx)
                .await
            {
                return false;
            }
            if job == TaskJob::Download {
                return true;
            }
        }

        let (text_present, use_after_capture) = self.with(|s| {
            (
                s.text.as_ref().is_some_and(|t| !t.is_empty()),
                s.info
                    .settings
                    .advanced_settings
                    .use_after_capture_tasks_during_file_upload,
            )
        });
        if job == TaskJob::Job {
            if !self.do_after_capture_jobs(ctx).await {
                return false;
            }
            self.do_file_jobs(ctx);
        } else if job == TaskJob::TextUpload && text_present {
            self.do_text_jobs(ctx).await;
        } else if job == TaskJob::FileUpload && use_after_capture {
            self.do_file_jobs(ctx);
        }

        // `DoOCR`: the picture's text (the window, or straight to the clipboard).
        let (ocr, image, data_type, settings) = self.with(|s| {
            (
                s.info
                    .settings
                    .after_capture_job
                    .has(AfterCaptureTasks::DoOCR),
                s.image.clone(),
                s.info.data_type,
                s.info.settings.clone(),
            )
        });
        if ocr
            && data_type == DataType::Image
            && let Some(image) = image
        {
            ctx.channel
                .effect(TaskEffect::Ocr(image, Box::new(settings)));
        }
        true
    }

    async fn do_after_capture_jobs(&self, ctx: &TaskContext) -> bool {
        let Some(image) = self.lock().image.clone() else {
            return true;
        };
        let (settings, file_name, metadata) = self.with(|s| {
            (
                s.info.settings.clone(),
                s.info.file_name.clone(),
                s.info.metadata.clone(),
            )
        });
        let job = settings.after_capture_job;

        let mut image = image;
        // `TaskHelpers.BeautifyImage`: the editor with the Background panel open; closing
        // it without a picture ends the task.
        if job.has(AfterCaptureTasks::BeautifyImage) {
            match ctx
                .channel
                .ask(self.id, TaskQuestion::Beautify(image.clone()))
                .await
            {
                Some(TaskAnswer::Annotated(Some(beautified))) => {
                    image = beautified;
                    self.with(|s| s.image = Some(image.clone()));
                }
                _ => return false,
            }
        }
        // `TaskHelpers.ApplyImageEffects`: the host applies the preset (and shows the Image
        // effects window first when the task asks for it); no picture ends the task.
        if job.has(AfterCaptureTasks::AddImageEffects) {
            match ctx
                .channel
                .ask(
                    self.id,
                    TaskQuestion::ImageEffects(image.clone(), Box::new(settings.clone())),
                )
                .await
            {
                Some(TaskAnswer::Annotated(Some(applied))) => {
                    image = applied;
                    self.with(|s| s.image = Some(image.clone()));
                }
                _ => {
                    log::warn!("Error: Applying image effects resulted empty image.");
                    return false;
                }
            }
        }
        if job.has(AfterCaptureTasks::AnnotateImage) {
            // `TaskHelpers.AnnotateImage`: the editor, waited for; closing it without a
            // picture ends the task.
            match ctx
                .channel
                .ask(self.id, TaskQuestion::Annotate(image.clone()))
                .await
            {
                Some(TaskAnswer::Annotated(Some(annotated))) => {
                    image = annotated;
                    self.with(|s| s.image = Some(image.clone()));
                }
                _ => return false,
            }
        }
        if job.has(AfterCaptureTasks::CopyImageToClipboard) {
            ctx.channel.effect(TaskEffect::CopyImage(image.clone()));
            log::info!("Image copied to clipboard.");
        }
        if job.has(AfterCaptureTasks::PinToScreen) {
            ctx.channel.effect(TaskEffect::PinToScreen(image.clone()));
        }
        if job.has(AfterCaptureTasks::SendImageToPrinter) {
            ctx.channel.effect(TaskEffect::PrintImage(image.clone()));
        }

        self.with(|s| s.info.metadata.image = Some(image.clone()));

        if job.has(AfterCaptureTasks::SaveImageToFile)
            || job.has(AfterCaptureTasks::SaveImageToFileWithDialog)
            || job.has(AfterCaptureTasks::DoOCR)
            || job.has(AfterCaptureTasks::UploadImageToHost)
            || job.has(AfterCaptureTasks::AnalyzeImage)
        {
            let prepared =
                crate::tasks::image::prepare_image(&image, &settings, ctx.png_strip_color_space());
            let file_name = naming::change_extension(&file_name, prepared.format.extension());
            self.with(|s| {
                s.data = Some(TaskData::Bytes(Arc::new(prepared.bytes.clone())));
                s.info.file_name = file_name.clone();
            });

            if job.has(AfterCaptureTasks::SaveImageToFile)
                || job.has(AfterCaptureTasks::AnalyzeImage)
            {
                let folder = ctx.screenshots_folder(&settings, Some(&metadata));
                if let Some(path) = self
                    .handle_exists_file(&folder.join(&file_name), &settings, ctx)
                    .await
                {
                    // `Info.FilePath = filePath` names the task after the file too, so a
                    // name chosen in the file-exists window is the one uploaded.
                    self.with(|s| s.info.set_file_path(&path.to_string_lossy()));
                    match write_file(&path, &prepared.bytes) {
                        Ok(()) => {
                            log::info!("Image saved to file: {}", path.display());
                        }
                        Err(error) => ctx.channel.effect(TaskEffect::MessageBox {
                            text: format!(
                                "{}\n\"{}\"\n\n{error}",
                                t(s::ImageData_Write_Error_Message),
                                path.display()
                            ),
                            title: format!("Modlight - {}", t(s::Error)),
                            icon: MessageIcon::Error,
                        }),
                    }
                }
            }

            if job.has(AfterCaptureTasks::SaveImageToFileWithDialog) {
                let file_name = self.lock().info.file_name.clone();
                let folder = ctx.screenshots_folder(&settings, Some(&metadata));
                // Upstream asks again when writing fails; the panel is asked once per
                // failure the same way.
                loop {
                    let answer = ctx
                        .channel
                        .ask(
                            self.id,
                            TaskQuestion::SaveAs {
                                folder: folder.clone(),
                                file_name: file_name.clone(),
                            },
                        )
                        .await;
                    let Some(TaskAnswer::SaveAs(Some(path))) = answer else {
                        break;
                    };
                    self.with(|s| s.info.set_file_path(&path.to_string_lossy()));
                    match write_file(&path, &prepared.bytes) {
                        Ok(()) => {
                            log::info!("Image saved to file with dialog: {}", path.display());
                            break;
                        }
                        Err(error) => {
                            log::warn!("Modlight: the image could not be saved: {error}");
                        }
                    }
                }
            }

            if job.has(AfterCaptureTasks::SaveThumbnailImageToFile) {
                let (folder, name) = {
                    let path = self.lock().info.file_path().to_string();
                    if path.is_empty() {
                        (
                            ctx.screenshots_folder(&settings, Some(&metadata)),
                            file_name.clone(),
                        )
                    } else {
                        let path = PathBuf::from(path);
                        (
                            path.parent().map(Path::to_path_buf).unwrap_or_default(),
                            path.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        )
                    }
                };
                if let Some(path) = self
                    .create_thumbnail(&image, &folder, &name, &settings, ctx)
                    .await
                {
                    log::info!("Thumbnail saved to file: {}", path.display());
                    self.with(|s| s.info.thumbnail_file_path = path.to_string_lossy().into_owned());
                }
            }
        }
        true
    }

    /// `TaskHelpers.CreateThumbnail`.
    async fn create_thumbnail(
        &self,
        image: &image::RgbaImage,
        folder: &Path,
        file_name: &str,
        settings: &TaskSettings,
        ctx: &TaskContext,
    ) -> Option<PathBuf> {
        let images = &settings.image_settings;
        let wants = (images.thumbnail_width > 0 || images.thumbnail_height > 0)
            && (!images.thumbnail_check_size
                || (image.width() as i64 > images.thumbnail_width
                    && image.height() as i64 > images.thumbnail_height));
        if !wants {
            return None;
        }
        let stem = Path::new(file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!("{stem}{}.jpg", images.thumbnail_name);
        let path = self
            .handle_exists_file(&folder.join(name), settings, ctx)
            .await?;
        let bytes = crate::tasks::image::thumbnail_jpeg(
            image,
            images.thumbnail_width,
            images.thumbnail_height,
        )?;
        write_file(&path, &bytes).ok()?;
        Some(path)
    }

    fn do_file_jobs(&self, ctx: &TaskContext) {
        let (path, settings) =
            self.with(|s| (s.info.file_path().to_string(), s.info.settings.clone()));
        if path.is_empty() || !Path::new(&path).is_file() {
            return;
        }
        let job = settings.after_capture_job;
        let mut path = path;
        if job.has(AfterCaptureTasks::PerformActions) {
            let actions: Vec<_> = settings
                .external_programs
                .iter()
                .filter(|p| p.is_active)
                .cloned()
                .collect();
            if !actions.is_empty() {
                let file_name = self.lock().info.file_name.clone();
                let mut modified = false;
                for action in actions {
                    if let Some(result) = action.run(Path::new(&path)) {
                        modified = true;
                        path = result.path.to_string_lossy().into_owned();
                        if let Some(input) = result.pending_delete {
                            let _ = std::fs::remove_file(input);
                        }
                    }
                }
                if modified {
                    let extension =
                        crate::names::file_helpers::get_file_name_extension(&path, false);
                    let name = crate::names::file_helpers::change_file_name_extension(
                        &file_name, &extension,
                    );
                    self.with(|s| {
                        s.info.set_file_path(&path);
                        s.info.file_name = name;
                        // `LoadFileStream`: the upload reads the program's output.
                        s.data = Some(TaskData::File(PathBuf::from(&path)));
                    });
                }
            }
        }
        if job.has(AfterCaptureTasks::CopyFileToClipboard) {
            ctx.channel
                .effect(TaskEffect::CopyFile(PathBuf::from(&path)));
        } else if job.has(AfterCaptureTasks::CopyFilePathToClipboard) {
            ctx.channel.effect(TaskEffect::CopyText(path.clone()));
        } else if job.has(AfterCaptureTasks::CopyFolderPathToClipboard) {
            let folder = Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            ctx.channel.effect(TaskEffect::CopyText(folder));
        }
        if job.has(AfterCaptureTasks::ShowInExplorer) {
            ctx.channel
                .effect(TaskEffect::RevealFile(PathBuf::from(&path)));
        }
        let is_image = self.with(|s| s.info.data_type == DataType::Image);
        if job.has(AfterCaptureTasks::AnalyzeImage) && is_image {
            ctx.channel.effect(TaskEffect::AnalyzeImage(
                PathBuf::from(&path),
                Box::new(self.with(|s| s.info.settings.clone())),
            ));
        }
        if job.has(AfterCaptureTasks::ScanQRCode) && is_image {
            ctx.channel
                .effect(TaskEffect::ScanQrCode(PathBuf::from(&path)));
        }
    }

    async fn do_text_jobs(&self, ctx: &TaskContext) {
        let (settings, file_name, text) = self.with(|s| {
            (
                s.info.settings.clone(),
                s.info.file_name.clone(),
                s.text.clone().unwrap_or_default(),
            )
        });
        if settings.advanced_settings.text_task_save_as_file {
            let folder = ctx.screenshots_folder(&settings, None);
            if let Some(path) = self
                .handle_exists_file(&folder.join(&file_name), &settings, ctx)
                .await
            {
                // `File.WriteAllText(path, text, Encoding.UTF8)` writes a byte order mark;
                // the uploaded bytes below (`GetBytes`) do not carry one.
                let mut bytes = vec![0xEF, 0xBB, 0xBF];
                bytes.extend_from_slice(text.as_bytes());
                self.with(|s| s.info.set_file_path(&path.to_string_lossy()));
                match write_file(&path, &bytes) {
                    Ok(()) => {
                        log::info!("Text saved to file: {}", path.display());
                    }
                    Err(error) => log::warn!("Modlight: the text could not be saved: {error}"),
                }
            }
        }
        self.with(|s| s.data = Some(TaskData::Bytes(Arc::new(text.into_bytes()))));
    }

    /// `TaskHelpers.HandleExistsFile`: the path to write, or `None` to not write.
    async fn handle_exists_file(
        &self,
        path: &Path,
        settings: &TaskSettings,
        ctx: &TaskContext,
    ) -> Option<PathBuf> {
        use crate::tasks::settings::FileExistAction;
        if !path.exists() {
            return Some(path.to_path_buf());
        }
        match settings.image_settings.file_exist_action {
            FileExistAction::Ask => match ctx
                .channel
                .ask(
                    self.id,
                    TaskQuestion::FileExists {
                        path: path.to_path_buf(),
                    },
                )
                .await
            {
                Some(TaskAnswer::FileExists(answer)) => answer,
                _ => None,
            },
            FileExistAction::Overwrite => Some(path.to_path_buf()),
            FileExistAction::UniqueName => Some(naming::unique_file_path(path)),
            FileExistAction::Cancel => None,
        }
    }

    async fn download_from_url(&self, upload: bool, ctx: &TaskContext) -> bool {
        let (url, settings, mut file_name) = self.with(|s| {
            let url = s.info.result.url.trim().to_string();
            s.info.result.url.clear();
            (url, s.info.settings.clone(), s.info.file_name.clone())
        });
        if !settings.upload_settings.file_upload_use_name_pattern
            && let Some(name) = ctx.api.file_name_from_server(&url).await
        {
            file_name = naming::sanitize_file_name(&name);
            self.with(|s| s.info.file_name = file_name.clone());
        }
        let folder = ctx.screenshots_folder(&settings, None);
        let Some(path) = self
            .handle_exists_file(&folder.join(&file_name), &settings, ctx)
            .await
        else {
            // `Info.FilePath = null` clears the name as well.
            self.with(|s| s.info.set_file_path(""));
            return false;
        };
        self.with(|s| {
            s.info.set_file_path(&path.to_string_lossy());
            s.info.status = t(s::UploadTask_DownloadAndUpload_Downloading).to_string();
        });
        ctx.channel.event(self.id, TaskEvent::StatusChanged);
        match ctx.api.download_url_to_file(&url, &path).await {
            Ok(()) => {
                if upload {
                    self.with(|s| s.data = Some(TaskData::File(path.clone())));
                }
                true
            }
            Err(error) => {
                ctx.channel.effect(TaskEffect::MessageBox {
                    text: format(
                        t(s::UploadManager_DownloadAndUploadFile_Download_failed),
                        &[Arg::from(error)],
                    ),
                    title: "Modlight".into(),
                    icon: MessageIcon::Error,
                });
                false
            }
        }
    }

    async fn do_upload_job(&self, ctx: &TaskContext) {
        let app = ctx.app_settings();
        if app.show_large_file_size_warning > 0 {
            let (size, name) = self.with(|s| {
                (
                    s.data.as_ref().map(TaskData::len).unwrap_or(0),
                    s.info.file_name.clone(),
                )
            });
            if size > app.large_file_bytes() {
                let answer = ctx
                    .channel
                    .ask(
                        self.id,
                        TaskQuestion::LargeFile {
                            file_name: name,
                            size,
                        },
                    )
                    .await;
                let (should_continue, dont_show_again) = match answer {
                    Some(TaskAnswer::LargeFile {
                        should_continue,
                        dont_show_again,
                    }) => (should_continue, dont_show_again),
                    _ => (false, false),
                };
                if dont_show_again {
                    ctx.channel.effect(TaskEffect::DisableLargeFileWarning);
                }
                if !should_continue {
                    self.stop(ctx);
                }
            }
        }

        if self.stop_requested() {
            return;
        }
        self.with(|s| {
            s.status = TaskStatus::Working;
            s.info.status = t(s::UploadTask_DoUploadJob_Uploading).to_string();
        });
        ctx.channel.event(self.id, TaskEvent::StatusChanged);

        let mut cancel_upload = false;
        let info = self.info();
        if !gate::allow(&info, ctx, self.id).await {
            // Nothing to upload to yet: skipped, no URL expected, no error, and the file
            // is already on disk because the work before saw this coming.
            cancel_upload = true;
        } else if info
            .settings
            .after_capture_job
            .has(AfterCaptureTasks::ShowBeforeUploadWindow)
        {
            match ctx
                .channel
                .ask(self.id, TaskQuestion::BeforeUpload(Box::new(info)))
                .await
            {
                Some(TaskAnswer::BeforeUpload { accepted, settings }) => {
                    cancel_upload = !accepted;
                    self.with(|s| s.info.settings = *settings);
                }
                _ => cancel_upload = true,
            }
        }

        if cancel_upload {
            self.with(|s| s.info.result.is_url_expected = false);
            return;
        }

        ctx.channel.event(self.id, TaskEvent::UploadStarted);
        {
            let s = self.lock();
            let mut text = format!("Upload started. File name: {}", s.info.file_name);
            if !s.info.file_path().is_empty() {
                text.push_str(&format!(", File path: {}", s.info.file_path()));
            }
            log::info!("{text}");
        }

        let mut is_error = self.do_upload(ctx, 0).await;
        let retries = app.max_upload_fail_retry.max(0);
        let mut retry = 1;
        while !self.stop_requested() && is_error && retry <= retries {
            log::info!("Upload failed. Retrying upload.");
            is_error = self.do_upload(ctx, retry).await;
            retry += 1;
        }

        // Before the failure is reported, and only when nothing answered: an upload the
        // link could not carry is a postponement, not a failure.
        if is_error && !self.stop_requested() && self.hand_over_to_queue(ctx) {
            is_error = false;
        }
        // A session nobody is going to resume (stopped, or failed for good) is let go
        // of now rather than left on the server's disk for a day.
        self.abandon_live_session(ctx);

        if !is_error {
            ctx.channel.event(self.id, TaskEvent::UploadCompleted);
            let s = self.lock();
            if !s.info.result.is_error() {
                let url = s.info.result.to_string();
                if !url.is_empty() {
                    let mut text = format(t(s::TaskManager_UploadCompletedURL), &[Arg::from(url)]);
                    if let Some(duration) = s.info.upload_duration {
                        text.push_str(&format!(", Duration: {} ms", duration.as_millis()));
                    }
                    log::info!("{text}");
                }
            }
        }
    }

    /// Give this upload to the offline queue, if it will have it (`HandOverToQueue`).
    /// The task then ends exactly as a stopped one does: no error, no URL expected, no
    /// after-upload work. The queue tells the person, once, and again when it goes.
    fn hand_over_to_queue(&self, ctx: &TaskContext) -> bool {
        let (info, data, answer, session) = self.with(|s| {
            (
                s.info.clone(),
                s.data.clone(),
                s.last_answer,
                s.live_session.clone(),
            )
        });
        if !gate::uploads_to_modlight(&info) {
            return false;
        }
        let file_path = Some(PathBuf::from(info.file_path())).filter(|p| p.is_file());
        let bytes = if file_path.is_none() {
            data.and_then(|d| d.bytes().ok())
        } else {
            None
        };
        let request = TakeOver {
            file_name: naming::name_for_queue(&info),
            folder_id: info.modlight_folder_id.clone(),
            file_path,
            data: bytes,
            answer,
            session,
        };
        if !ctx.queue.try_take_over(request) {
            return false;
        }
        self.with(|s| {
            // The queue carries the session on now.
            s.live_session = None;
            // The file must still be there when the queue gets to it: "delete it once it
            // is uploaded" cannot be honoured by an upload that has not happened.
            s.info.settings.after_capture_job = s
                .info
                .settings
                .after_capture_job
                .remove(AfterCaptureTasks::DeleteFile);
            s.info.result = UploadResult {
                is_url_expected: false,
                ..UploadResult::default()
            };
            s.info.status = t(s::UploadQueue_StateWaiting).to_string();
            s.stop_requested = true;
        });
        true
    }

    /// Tell the server to forget the session a large upload left, if one is left.
    fn abandon_live_session(&self, ctx: &TaskContext) {
        let Some(url) = self.with(|s| s.live_session.take().and_then(|live| live.session.url))
        else {
            return;
        };
        if let Some(credential) = ctx.credential() {
            uploader::abandon_session(ctx, credential, url);
        }
    }

    /// One attempt (`DoUpload`). Returns whether it ended in an error.
    async fn do_upload(&self, ctx: &TaskContext, retry: i64) -> bool {
        if retry > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let (info, data) = self.with(|s| (s.info.clone(), s.data.clone()));
        // Only images, texts and files are sent; a URL task keeps the result it was made
        // with, and its after-upload jobs shorten or share it.
        if !matches!(
            info.upload_destination(),
            DataType::Image | DataType::Text | DataType::File
        ) {
            return info.result.is_error();
        }
        let started = Instant::now();
        let id = self.id;
        let task = self.clone();
        let channel = ctx.channel.clone();
        let progress = move |progress: crate::upload::progress::ProgressManager| {
            task.with(|s| s.info.progress = Some(progress));
            if task.status() == TaskStatus::Working {
                channel.event(id, TaskEvent::UploadProgressChanged);
            }
        };
        let mut live = self.with(|s| s.live_session.take());
        let outcome = uploader::upload(
            ctx,
            &info,
            data,
            self.upload_cancel.clone(),
            Box::new(progress),
            &mut live,
        )
        .await;
        self.with(|s| s.live_session = live);
        let elapsed = started.elapsed();
        self.with(|s| {
            s.last_answer = outcome.answer;
            s.info.upload_duration = Some(elapsed);
            if outcome.stopped {
                s.stop_requested = true;
            }
            let mut result = outcome.result.unwrap_or_default();
            if let Some(landed) = outcome.landed_folder {
                s.info.modlight_landed_folder_id = landed;
            }
            // `AddErrorMessage(uploader.Errors)`: everything the uploader recorded.
            result.errors.add_all(&outcome.errors);
            s.info.result = result;
            s.info.result.is_error()
        })
    }

    async fn do_after_upload_jobs(&self, ctx: &TaskContext) {
        let info = self.info();
        let settings = info.settings.clone();
        let mut result = info.result.clone();

        if settings.upload_settings.url_regex_replace {
            result.url = naming::regex_replace(
                &result.url,
                &settings.upload_settings.url_regex_replace_pattern,
                &settings.upload_settings.url_regex_replace_replacement,
            );
        }
        if settings.advanced_settings.result_force_https {
            result.force_https();
        }
        self.with(|s| s.info.result = result.clone());

        let job = info.job;
        let auto_length = settings.advanced_settings.auto_shorten_url_length;
        if job != TaskJob::ShareUrl
            && (settings
                .after_upload_job
                .has(AfterUploadTasks::UseURLShortener)
                || job == TaskJob::ShortenUrl
                || (auto_length > 0 && result.url.chars().count() as i64 > auto_length))
        {
            let shortened = uploader::shorten(ctx, &settings, &result.url).await;
            self.with(|s| {
                s.info.result.shortened_url = shortened.shortened_url.clone();
                s.info.result.errors.add_all(&shortened.errors);
            });
        }

        if job != TaskJob::ShortenUrl
            && (settings.after_upload_job.has(AfterUploadTasks::ShareURL)
                || job == TaskJob::ShareUrl)
        {
            let target = self.lock().info.result.to_string();
            if !target.is_empty() {
                let link = uploader::share_url(settings.url_sharing_service_destination, &target);
                ctx.channel.effect(TaskEffect::OpenUrl(link));
            }
            if job == TaskJob::ShareUrl {
                self.with(|s| s.info.result.is_url_expected = false);
            }
        }

        let info = self.info();
        if settings
            .after_upload_job
            .has(AfterUploadTasks::CopyURLToClipboard)
        {
            let format = &settings.advanced_settings.clipboard_content_format;
            let text = if format.is_empty() {
                info.result.to_string()
            } else {
                naming::parse_upload_info(&info, format, ctx)
            };
            if !text.is_empty() {
                ctx.channel.effect(TaskEffect::CopyText(text));
            }
        }
        if settings.after_upload_job.has(AfterUploadTasks::OpenURL) {
            let format = &settings.advanced_settings.open_url_format;
            let url = if format.is_empty() {
                info.result.to_string()
            } else {
                naming::parse_upload_info(&info, format, ctx)
            };
            ctx.channel.effect(TaskEffect::OpenUrl(url));
        }
        // Show QR code arrives with the QR tool (phase 5).
    }

    /// `OnTaskCompleted`: the final status, then the one event the task list acts on.
    pub fn on_task_completed(&self, ctx: &TaskContext) {
        self.with(|s| {
            s.info.task_end_time = Some(Utc::now());
            if s.stop_requested {
                s.status = TaskStatus::Stopped;
                s.info.status = t(s::UploadTask_OnUploadCompleted_Stopped).to_string();
            } else if s.info.result.is_error() {
                s.status = TaskStatus::Failed;
                s.info.status = t(s::TaskManager_task_UploadCompleted_Error).to_string();
            } else {
                s.status = TaskStatus::Completed;
                s.info.status = t(s::UploadTask_OnUploadCompleted_Done).to_string();
            }
        });
        ctx.channel.event(self.id, TaskEvent::TaskCompleted);
        self.with(|s| {
            s.data = None;
            if !s.keep_image {
                // `Dispose`: the picture goes, and with it the metadata's reference to
                // the same picture (upstream's `Metadata.Image` is that disposed bitmap).
                // The finished task stays on the list for the whole session.
                s.image = None;
                s.info.metadata.image = None;
            }
        });
    }

    /// Let go of the picture once the notice no longer needs it.
    pub fn release_image(&self) {
        self.with(|s| {
            s.keep_image = false;
            s.image = None;
            s.info.metadata.image = None;
        });
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use std::sync::RwLock;
    use std::sync::atomic::{AtomicBool, AtomicI64};
    use std::sync::mpsc::Receiver;

    use super::*;
    use crate::tasks::host::{TaskChannel, TaskMessage};
    use crate::tasks::settings::AppUploadSettings;
    use crate::upload::queue::{AccountSlot, UploadQueue};
    use crate::upload::store::QueueStore;
    use crate::upload::transport::ApiTransport;

    /// The task machinery over `profile` with nobody signed in, and the window's end of
    /// the channel. Nothing here talks to a server.
    fn context(
        runtime: &tokio::runtime::Runtime,
        profile: &Path,
    ) -> (TaskContext, Receiver<TaskMessage>) {
        let api = modlight_api::Api::new("0.0.0").unwrap();
        let tus = modlight_api::TusClient::new("0.0.0").unwrap();
        let account: AccountSlot = Arc::new(RwLock::new(None));
        let queue = UploadQueue::new(
            QueueStore::new(profile),
            account.clone(),
            Arc::new(ApiTransport::with_tus(api.clone(), tus.clone())),
            Arc::new(|| true),
        );
        let (channel, receiver) = TaskChannel::new(Arc::new(|| {}));
        let ctx = TaskContext {
            api,
            tus,
            account,
            queue,
            channel,
            runtime: runtime.handle().clone(),
            settings: Arc::new(RwLock::new(AppUploadSettings::default())),
            personal_folder: profile.to_path_buf(),
            auto_increment: Arc::new(AtomicI64::new(0)),
            strip_png_color_space: Arc::new(AtomicBool::new(false)),
        };
        (ctx, receiver)
    }

    fn wait_for_end(receiver: &Receiver<TaskMessage>, id: u64) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(TaskMessage::Event {
                    task,
                    event: TaskEvent::TaskCompleted,
                }) if task == id => return,
                Ok(_) => {}
                Err(error) => panic!("the task did not end: {error}"),
            }
        }
    }

    /// A finished task stays on the task list for the rest of the session, so it must
    /// not hold on to the decoded capture (tens of megabytes for a large screen) once
    /// nothing needs it: neither in `image` nor in `info.metadata.image`.
    #[test]
    fn a_finished_image_task_lets_go_of_its_picture() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let profile = tempfile::tempdir().unwrap();
        let (ctx, receiver) = context(&runtime, profile.path());
        for toast in [false, true] {
            let mut settings = TaskSettings {
                after_capture_job: AfterCaptureTasks::None,
                ..TaskSettings::default()
            };
            settings
                .general_settings
                .show_toast_notification_after_task_completed = toast;
            let picture = Arc::new(image::RgbaImage::new(64, 32));
            let metadata = TaskMetadata {
                image: Some(picture.clone()),
                ..TaskMetadata::default()
            };
            let task = WorkerTask::image_upload(metadata, settings, None, &ctx);
            task.start(&ctx);
            wait_for_end(&receiver, task.id);
            // The worker's last step runs just after the event; give it a moment.
            let deadline = Instant::now() + Duration::from_secs(5);
            while task.with(|s| s.data.is_some() || (!toast && s.image.is_some()))
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(task.status(), TaskStatus::Completed);
            if toast {
                // The notice is built from the task after it ended, so the picture is
                // still there until the task list lets go of it.
                assert!(task.with(|s| s.image.is_some()));
                task.release_image();
            }
            task.with(|s| {
                assert!(s.image.is_none(), "toast {toast}: the task's picture");
                assert!(
                    s.info.metadata.image.is_none(),
                    "toast {toast}: the picture kept in the task's metadata"
                );
            });
            assert_eq!(Arc::strong_count(&picture), 1, "toast {toast}");
        }
    }
}

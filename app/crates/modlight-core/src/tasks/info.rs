//! What one task is and what became of it: `TaskInfo`, `UploadResult`,
//! `UploaderErrorManager` and the enums around them.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::history::{HistoryItem, HistoryTags};
use crate::tasks::settings::{FileDestination, ImageDestination, TaskSettings, TextDestination};
use crate::upload::progress::ProgressManager;

/// `TaskJob`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskJob {
    Job,
    DataUpload,
    FileUpload,
    TextUpload,
    ShortenUrl,
    ShareUrl,
    Download,
    DownloadUpload,
}

/// `EDataType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DataType {
    #[default]
    Default,
    File,
    Image,
    Text,
    Url,
}

impl DataType {
    /// The name `HistoryItem.Type` stores (`DataType.ToString()`).
    pub fn name(self) -> &'static str {
        match self {
            DataType::Default => "Default",
            DataType::File => "File",
            DataType::Image => "Image",
            DataType::Text => "Text",
            DataType::Url => "URL",
        }
    }
}

/// `TaskStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TaskStatus {
    #[default]
    InQueue,
    Preparing,
    Working,
    Stopping,
    Stopped,
    Failed,
    Completed,
    History,
}

impl TaskStatus {
    /// `IsWorking`.
    pub fn is_working(self) -> bool {
        matches!(
            self,
            TaskStatus::Preparing | TaskStatus::Working | TaskStatus::Stopping
        )
    }

    /// `IsBusy`.
    pub fn is_busy(self) -> bool {
        self == TaskStatus::InQueue || self.is_working()
    }
}

/// One entry of `UploaderErrorManager`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploaderError {
    pub title: String,
    pub text: String,
}

/// `UploaderErrorManager`: what went wrong, the person-readable sentence first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploaderErrors {
    pub errors: Vec<UploaderError>,
    pub default_title: String,
}

impl Default for UploaderErrors {
    fn default() -> Self {
        UploaderErrors {
            errors: Vec::new(),
            default_title: crate::l10n::t(crate::l10n::uploaders::Common_Error).to_string(),
        }
    }
}

impl UploaderErrors {
    pub fn count(&self) -> usize {
        self.errors.len()
    }

    pub fn add(&mut self, text: &str) {
        self.errors.push(UploaderError {
            title: self.default_title.clone(),
            text: text.to_string(),
        });
    }

    pub fn add_first(&mut self, text: &str) {
        self.errors.insert(
            0,
            UploaderError {
                title: self.default_title.clone(),
                text: text.to_string(),
            },
        );
    }

    pub fn add_all(&mut self, other: &UploaderErrors) {
        self.errors.extend(other.errors.iter().cloned());
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl std::fmt::Display for UploaderErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let texts: Vec<&str> = self.errors.iter().map(|e| e.text.as_str()).collect();
        f.write_str(&texts.join("\n\n"))
    }
}

/// `ResponseInfo`: what the server answered, kept for the error text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResponseInfo {
    pub status: u16,
    pub status_text: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// `UploadResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct UploadResult {
    pub url: String,
    pub thumbnail_url: String,
    pub deletion_url: String,
    pub shortened_url: String,
    pub is_success: bool,
    pub response: String,
    pub errors: UploaderErrors,
    pub is_url_expected: bool,
    pub response_info: Option<ResponseInfo>,
}

impl Default for UploadResult {
    fn default() -> Self {
        UploadResult {
            url: String::new(),
            thumbnail_url: String::new(),
            deletion_url: String::new(),
            shortened_url: String::new(),
            is_success: false,
            response: String::new(),
            errors: UploaderErrors::default(),
            is_url_expected: true,
            response_info: None,
        }
    }
}

impl UploadResult {
    pub fn set_success(&mut self, value: bool) {
        self.is_success = value;
    }

    /// `IsSuccess`: an answer with a body.
    pub fn is_success(&self) -> bool {
        self.is_success && !self.response.is_empty()
    }

    /// `IsError`: errors were recorded and a URL was expected but did not come.
    pub fn is_error(&self) -> bool {
        self.errors.count() > 0 && (!self.is_url_expected || self.url.is_empty())
    }

    /// `ForceHTTPS`.
    pub fn force_https(&mut self) {
        for url in [
            &mut self.url,
            &mut self.thumbnail_url,
            &mut self.deletion_url,
            &mut self.shortened_url,
        ] {
            *url = crate::names::url_helpers::force_prefix(url);
        }
    }

    /// `ErrorsToString`.
    pub fn errors_to_string(&self) -> Option<String> {
        self.is_error().then(|| self.errors.to_string())
    }
}

impl std::fmt::Display for UploadResult {
    /// The shortened URL when there is one, else the URL.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.shortened_url.is_empty() {
            f.write_str(&self.shortened_url)
        } else {
            f.write_str(&self.url)
        }
    }
}

/// `TaskMetadata`: the picture and the window it came from (captures, phase 3).
#[derive(Clone, Debug, Default)]
pub struct TaskMetadata {
    pub image: Option<std::sync::Arc<image::RgbaImage>>,
    pub window_title: Option<String>,
    pub process_name: Option<String>,
}

/// `TaskInfo`.
#[derive(Clone, Debug)]
pub struct TaskInfo {
    pub settings: TaskSettings,
    /// What the task says it is doing, in the person's language.
    pub status: String,
    pub job: TaskJob,
    pub progress: Option<ProgressManager>,
    file_path: String,
    pub file_name: String,
    pub thumbnail_file_path: String,
    pub data_type: DataType,
    pub metadata: TaskMetadata,
    pub task_start_time: Option<DateTime<Utc>>,
    pub task_end_time: Option<DateTime<Utc>>,
    pub upload_duration: Option<Duration>,
    pub result: UploadResult,
    /// The Modlight folder this upload was aimed at, resolved; `None` = account default.
    pub modlight_folder_id: Option<String>,
    /// The folder the server says it landed in.
    pub modlight_landed_folder_id: Option<String>,
}

impl TaskInfo {
    pub fn new(settings: TaskSettings) -> TaskInfo {
        TaskInfo {
            settings,
            status: String::new(),
            job: TaskJob::Job,
            progress: None,
            file_path: String::new(),
            file_name: String::new(),
            thumbnail_file_path: String::new(),
            data_type: DataType::Default,
            metadata: TaskMetadata::default(),
            task_start_time: None,
            task_end_time: None,
            upload_duration: None,
            result: UploadResult::default(),
            modlight_folder_id: None,
            modlight_landed_folder_id: None,
        }
    }

    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    /// Setting the path also names the file after it, as upstream's setter does.
    pub fn set_file_path(&mut self, path: &str) {
        self.file_path = path.to_string();
        self.file_name = if path.is_empty() {
            String::new()
        } else {
            Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
    }

    /// `IsUploadJob`.
    pub fn is_upload_job(&self) -> bool {
        match self.job {
            TaskJob::Job => self
                .settings
                .after_capture_job
                .has(crate::tasks::settings::AfterCaptureTasks::UploadImageToHost),
            TaskJob::DataUpload
            | TaskJob::FileUpload
            | TaskJob::TextUpload
            | TaskJob::ShortenUrl
            | TaskJob::ShareUrl
            | TaskJob::DownloadUpload => true,
            TaskJob::Download => false,
        }
    }

    /// `UploadDestination`: an image or a text sent to the file uploader is a file.
    pub fn upload_destination(&self) -> DataType {
        if (self.data_type == DataType::Image
            && self.settings.image_destination == ImageDestination::FileUploader)
            || (self.data_type == DataType::Text
                && self.settings.text_destination == TextDestination::FileUploader)
        {
            return DataType::File;
        }
        self.data_type
    }

    /// `UploaderHost`: the destination's own name, which the history stores.
    pub fn uploader_host(&self) -> Option<String> {
        if !self.is_upload_job() {
            return None;
        }
        let name = match self.upload_destination() {
            DataType::Image => match self.settings.image_destination {
                ImageDestination::Modlight => "Modlight".to_string(),
                ImageDestination::FileUploader => file_uploader_name(),
            },
            DataType::Text => match self.settings.text_destination {
                TextDestination::Modlight => "Modlight".to_string(),
                TextDestination::FileUploader => file_uploader_name(),
            },
            DataType::File => match self.settings.file_destination_for(self.data_type) {
                FileDestination::Modlight => "Modlight".to_string(),
            },
            DataType::Url => {
                if self.job == TaskJob::ShareUrl {
                    sharing_service_name(self.settings.url_sharing_service_destination)
                } else {
                    "Modlight".to_string()
                }
            }
            DataType::Default => return None,
        };
        Some(name)
    }

    /// `TaskDuration`.
    pub fn task_duration(&self) -> Duration {
        match (self.task_start_time, self.task_end_time) {
            (Some(start), Some(end)) => (end - start).to_std().unwrap_or_default(),
            _ => Duration::ZERO,
        }
    }

    /// `ToString`: the result, or the file path when there is no result.
    pub fn summary(&self) -> String {
        let text = self.result.to_string();
        if text.is_empty() && !self.file_path.is_empty() {
            self.file_path.clone()
        } else {
            text
        }
    }

    /// `GetTags`.
    pub fn tags(&self) -> Option<HistoryTags> {
        let mut tags = HistoryTags::new();
        if let Some(title) = self
            .metadata
            .window_title
            .as_ref()
            .filter(|t| !t.is_empty())
        {
            tags.insert("WindowTitle".to_string(), Some(title.clone()));
        }
        if let Some(name) = self
            .metadata
            .process_name
            .as_ref()
            .filter(|t| !t.is_empty())
        {
            tags.insert("ProcessName".to_string(), Some(name.clone()));
        }
        (!tags.is_empty()).then_some(tags)
    }

    /// `GetHistoryItem`.
    pub fn history_item(&self) -> HistoryItem {
        HistoryItem {
            file_name: self.file_name.clone(),
            file_path: self.file_path.clone(),
            date_time: Some(self.task_end_time.unwrap_or_else(Utc::now)),
            kind: self.data_type.name().to_string(),
            host: self.uploader_host().unwrap_or_default(),
            url: self.result.url.clone(),
            thumbnail_url: self.result.thumbnail_url.clone(),
            deletion_url: self.result.deletion_url.clone(),
            shortened_url: self.result.shortened_url.clone(),
            tags: self.tags(),
            ..HistoryItem::default()
        }
    }
}

/// `FileUploader`'s localized description ("File uploader").
fn file_uploader_name() -> String {
    crate::l10n::t(crate::l10n::uploaders::ImageDestination_FileUploader).to_string()
}

fn sharing_service_name(service: crate::tasks::settings::UrlSharingServices) -> String {
    use crate::tasks::settings::UrlSharingServices;
    match service {
        UrlSharingServices::GoogleImageSearch => {
            crate::l10n::t(crate::l10n::uploaders::URLSharingServices_GoogleImageSearch).to_string()
        }
        UrlSharingServices::BingVisualSearch => {
            crate::l10n::t(crate::l10n::uploaders::URLSharingServices_BingVisualSearch).to_string()
        }
    }
}

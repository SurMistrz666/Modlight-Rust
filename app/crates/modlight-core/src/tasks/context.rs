//! What every task can reach: the server client, the account, the offline queue, the
//! channel to the window, the runtime, and the application settings and folders the
//! helpers in `TaskHelpers` read from `Program`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

use modlight_api::{Api, Credential};

use crate::tasks::host::{MessageIcon, TaskChannel, TaskEffect};
use crate::tasks::info::TaskMetadata;
use crate::tasks::naming;
use crate::tasks::settings::{AppUploadSettings, TaskSettings};
use crate::upload::queue::{AccountSlot, UploadQueue};

/// The name of the folder captures and saved texts go to inside the personal folder.
pub const SCREENSHOTS_FOLDER_NAME: &str = "Screenshots";

#[derive(Clone)]
pub struct TaskContext {
    pub api: Api,
    pub account: AccountSlot,
    pub queue: UploadQueue,
    pub channel: TaskChannel,
    pub runtime: tokio::runtime::Handle,
    /// The application settings as of the last save; the window refreshes them.
    pub settings: Arc<RwLock<AppUploadSettings>>,
    /// `Program.PersonalFolder`.
    pub personal_folder: PathBuf,
    /// `NameParserAutoIncrementNumber`, shared by every task and written back by the window.
    pub auto_increment: Arc<AtomicI64>,
    /// `PNGStripColorSpaceInformation`.
    pub strip_png_color_space: Arc<std::sync::atomic::AtomicBool>,
}

impl TaskContext {
    pub fn app_settings(&self) -> AppUploadSettings {
        self.settings
            .read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// The credential uploads carry, when somebody is signed in.
    pub fn credential(&self) -> Option<Credential> {
        self.account
            .read()
            .ok()
            .and_then(|a| a.as_ref().map(|a| Credential::new(&a.host, &a.token)))
    }

    pub fn png_strip_color_space(&self) -> bool {
        self.strip_png_color_space.load(Ordering::Relaxed)
    }

    /// `TaskHelpers.IsUploadAllowed`. The system-wide switch (`SystemOptions`, a
    /// Windows registry policy) has no macOS counterpart; the app's own one remains.
    pub fn is_upload_allowed(&self) -> bool {
        if self.app_settings().disable_upload {
            self.channel.effect(TaskEffect::MessageBox {
                text: crate::l10n::t(
                    crate::l10n::s::ThisFeatureWillNotWorkWhenDisableUploadOptionIsEnabled,
                )
                .to_string(),
                title: "Modlight".into(),
                icon: MessageIcon::Information,
            });
            return false;
        }
        true
    }

    /// `TaskHelpers.GetFileName`: a name from the naming pattern, with the counter
    /// carried across tasks.
    pub fn file_name(
        &self,
        settings: &TaskSettings,
        extension: Option<&str>,
        metadata: Option<&TaskMetadata>,
    ) -> String {
        let start = self.auto_increment.load(Ordering::SeqCst);
        let (name, counter) = naming::file_name(settings, extension, metadata, start);
        if counter != start {
            self.auto_increment.store(counter, Ordering::SeqCst);
            self.channel.effect(TaskEffect::SetAutoIncrement(counter));
        }
        name
    }

    /// `Program.ScreenshotsParentFolder`.
    pub fn screenshots_parent_folder(&self) -> PathBuf {
        let settings = self.app_settings();
        if settings.use_custom_screenshots_path {
            let first = settings
                .custom_screenshots_path
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(naming::expand_folder_variables);
            let second = settings
                .custom_screenshots_path2
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(naming::expand_folder_variables);
            if let Some(first) = &first
                && (second.is_none() || Path::new(first).is_dir())
            {
                return PathBuf::from(first);
            }
            if let Some(second) = second.filter(|p| Path::new(p).is_dir()) {
                return PathBuf::from(second);
            }
        }
        self.personal_folder.join(SCREENSHOTS_FOLDER_NAME)
    }

    /// `TaskHelpers.GetScreenshotsFolder`.
    pub fn screenshots_folder(
        &self,
        settings: &TaskSettings,
        metadata: Option<&TaskMetadata>,
    ) -> PathBuf {
        let app = self.app_settings();
        let folder = if settings.override_screenshots_folder
            && let Some(folder) = settings.screenshots_folder.as_deref().filter(|f| !f.is_empty())
        {
            PathBuf::from(naming::parse_path_pattern(folder, metadata))
        } else {
            let window = metadata.and_then(|m| m.window_title.as_deref()).is_some_and(|w| !w.is_empty());
            let pattern = match app.save_image_sub_folder_pattern_window.as_deref() {
                Some(window_pattern) if window && !window_pattern.is_empty() => window_pattern,
                _ => app.save_image_sub_folder_pattern.as_deref().unwrap_or(""),
            };
            let sub = naming::parse_path_pattern(pattern, metadata);
            self.screenshots_parent_folder().join(sub)
        };
        naming::absolute_path(&folder)
    }
}

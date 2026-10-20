//! The file panels and the picture plumbing the tool windows share: open and save panels
//! (QA and headless runs never open one: saves go to `<profile>/Saved`, opens take what a
//! test queued), loading a picture the way `ImageHelpers.LoadImage` does, and handing a
//! picture to the task flow (`MainFormUploadImage`, `UploadManager.RunImageTask`).

use std::path::{Path, PathBuf};

use image::RgbaImage;
use modlight_core::tasks::settings::{AfterCaptureTasks, TaskSettings};

use crate::state::AppState;

/// The picture extensions the open panels offer (`FilePickerFileTypes.ImageAll`).
pub const IMAGE_EXTENSIONS: [&str; 10] = [
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "bmp", "tif", "tiff", "webp",
];

impl AppState {
    fn panels_allowed(&self) -> bool {
        !self.env.qa && self.mode == crate::app::Mode::Normal
    }

    /// A save panel; QA and headless runs answer `<profile>/Saved/<file_name>`.
    pub fn tool_save_path(
        &mut self,
        title: &str,
        file_name: &str,
        filters: &[(&str, &[&str])],
    ) -> Option<PathBuf> {
        if !self.panels_allowed() {
            let folder = self.env.profile.join("Saved");
            let _ = std::fs::create_dir_all(&folder);
            return Some(folder.join(file_name));
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(file_name);
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, extensions);
        }
        dialog.save_file()
    }

    /// An open panel for one file; QA and headless runs take the next queued path.
    pub fn tool_open_file(&mut self, title: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
        if !self.panels_allowed() {
            return self.tools.qa_open.pop_front();
        }
        let mut dialog = rfd::FileDialog::new().set_title(title);
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, extensions);
        }
        dialog.pick_file()
    }

    /// An open panel for several files.
    pub fn tool_open_files(&mut self, title: &str, filters: &[(&str, &[&str])]) -> Vec<PathBuf> {
        if !self.panels_allowed() {
            return self.tools.qa_open.drain(..).collect();
        }
        let mut dialog = rfd::FileDialog::new().set_title(title);
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, extensions);
        }
        dialog.pick_files().unwrap_or_default()
    }

    /// A folder panel.
    pub fn tool_pick_folder(&mut self, title: &str, start: Option<&Path>) -> Option<PathBuf> {
        if !self.panels_allowed() {
            return self.tools.qa_open.pop_front();
        }
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(start) = start.filter(|p| p.is_dir()) {
            dialog = dialog.set_directory(start);
        }
        dialog.pick_folder()
    }

    /// `MainFormUploadImage`: the picture uploaded as it is.
    pub fn tool_upload_image(&mut self, image: RgbaImage, settings: Option<TaskSettings>) {
        let mut upload = settings.unwrap_or_else(|| self.default_task_settings());
        upload.after_capture_job = AfterCaptureTasks::UploadImageToHost;
        let ctx = self.uploads.ctx.clone();
        modlight_core::tasks::upload_manager::run_image_task(
            &mut self.uploads.tasks,
            image,
            &upload,
            &ctx,
        );
    }

    /// `UploadManager.RunImageTask(image, taskSettings)`: the after-capture tasks run on
    /// the picture (the image combiner's result, a pinned picture's "upload").
    pub fn tool_image_task(&mut self, image: RgbaImage, settings: TaskSettings) {
        self.run_image_task(crate::capture_flow::PendingImage {
            metadata: modlight_core::tasks::info::TaskMetadata {
                image: Some(std::sync::Arc::new(image)),
                ..Default::default()
            },
            settings,
            file: None,
            skip_quick_menu: false,
        });
    }
}

/// `ImageHelpers.LoadImage`: any picture the `image` crate reads, turned the way its
/// EXIF orientation says (`RotateImageByExifOrientationData`).
pub fn load_image(path: &Path) -> Option<RgbaImage> {
    modlight_tools::images::load(path).ok()
}

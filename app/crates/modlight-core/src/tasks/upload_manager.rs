//! The app's entry points for uploading (`UploadManager`): files and folders, text,
//! the clipboard's content, an address to shorten, share or fetch. Every one of them
//! makes a task and hands it to the task list, so they all share one upload path.
//!
//! The pieces that need a window first (the file picker, the multi-upload question, the
//! clipboard viewer) are the window's; it calls in here with the answers.

use std::path::{Path, PathBuf};

use crate::tasks::context::TaskContext;
use crate::tasks::info::{DataType, TaskMetadata};
use crate::tasks::manager::TaskManager;
use crate::tasks::naming;
use crate::tasks::settings::{AfterCaptureTasks, TaskSettings};
use crate::tasks::worker::WorkerTask;

/// More files than this at once asks first (`files.Length <= 10`).
pub const MULTI_UPLOAD_WITHOUT_ASKING: usize = 10;

/// What the clipboard held, read by the window (`ClipboardUpload`).
#[derive(Clone, Debug)]
pub enum ClipboardContent {
    Image(image::RgbaImage),
    Text(String),
    Files(Vec<PathBuf>),
}

/// Expand folders into the files inside them, every level down
/// (`Directory.GetFiles(path, "*.*", AllDirectories)`).
pub fn expand_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_file() {
            files.push(path.clone());
        } else if path.is_dir() {
            collect_files(path, &mut files);
        }
    }
    files
}

fn collect_files(folder: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.is_file() && !is_finder_metadata(&path) {
            files.push(path);
        }
    }
}

/// Finder's per-folder view settings (`.DS_Store`): a Mac writes one into almost every
/// folder it opens, where Windows has nothing comparable, so a folder upload leaves it
/// out rather than sending it as one of the person's files.
fn is_finder_metadata(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == ".DS_Store")
}

/// Whether uploading `count` files needs the multi-upload question first.
pub fn needs_confirmation(count: usize, show_multi_upload_warning: bool) -> bool {
    count > MULTI_UPLOAD_WITHOUT_ASKING && show_multi_upload_warning
}

/// `UploadFile(string)`: a file becomes a task; a folder becomes its files.
pub fn upload_file(
    manager: &mut TaskManager,
    path: &Path,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    if path.is_file() {
        if let Some(task) = WorkerTask::file_upload(path, settings.clone(), ctx) {
            manager.start(task, ctx);
        }
    } else if path.is_dir() {
        for file in expand_paths(&[path.to_path_buf()]) {
            upload_file(manager, &file, settings, ctx);
        }
    }
}

/// `UploadFile(string[])` after the window asked, when it had to.
pub fn upload_files(
    manager: &mut TaskManager,
    files: &[PathBuf],
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    for file in files {
        upload_file(manager, file, settings, ctx);
    }
}

/// `UploadText`, with the custom text template when it applies.
pub fn upload_text(
    manager: &mut TaskManager,
    text: &str,
    settings: &TaskSettings,
    allow_custom_text: bool,
    ctx: &TaskContext,
) {
    if text.is_empty() {
        return;
    }
    let mut text = text.to_string();
    if allow_custom_text {
        let template = &settings.advanced_settings.text_custom;
        if !template.is_empty() {
            if settings.advanced_settings.text_custom_encode_input {
                text = naming::html_encode(&text);
            }
            text = template.replace("%input", &text);
        }
    }
    let task = WorkerTask::text_upload(&text, settings.clone(), ctx);
    manager.start(task, ctx);
}

/// `ProcessTextUpload`: an address can be fetched, shortened or shared instead of sent
/// as text, when the clipboard settings say so.
pub fn process_text_upload(
    manager: &mut TaskManager,
    text: &str,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    if text.is_empty() {
        return;
    }
    let url = text.trim();
    if naming::is_valid_url(url) {
        let clipboard = &settings.upload_settings;
        if clipboard.clipboard_upload_url_contents {
            download_and_upload(manager, url, settings, ctx);
            return;
        }
        if clipboard.clipboard_upload_shorten_url {
            shorten_url(manager, url, settings, ctx);
            return;
        }
        if clipboard.clipboard_upload_share_url {
            share_url(manager, url, settings, ctx);
            return;
        }
    }
    // Indexing a folder (`ClipboardUploadAutoIndexFolder`) arrives with the indexer tool
    // (phase 5); until then the path is sent as the text it is.
    upload_text(manager, text, settings, true, ctx);
}

/// `ProcessImageUpload`: a clipboard picture is uploaded as it is unless the settings
/// ask for the after-capture work on it too.
pub fn process_image_upload(
    manager: &mut TaskManager,
    image: image::RgbaImage,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    let mut settings = settings.clone();
    if !settings
        .advanced_settings
        .process_images_during_clipboard_upload
    {
        settings.after_capture_job = AfterCaptureTasks::UploadImageToHost;
    }
    run_image_task(manager, image, &settings, ctx);
}

/// `RunImageTask`, without the quick task menu and the after-capture window, which
/// arrive with captures (phase 3).
pub fn run_image_task(
    manager: &mut TaskManager,
    image: image::RgbaImage,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    let metadata = TaskMetadata {
        image: Some(std::sync::Arc::new(image)),
        ..TaskMetadata::default()
    };
    let task = WorkerTask::image_upload(metadata, settings.clone(), None, ctx);
    manager.start(task, ctx);
}

/// `ClipboardUpload`: whatever the clipboard holds. Files go through the window first
/// when there are many (the window asks, then calls [`upload_files`]).
pub fn clipboard_upload(
    manager: &mut TaskManager,
    content: ClipboardContent,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    match content {
        ClipboardContent::Image(image) => process_image_upload(manager, image, settings, ctx),
        ClipboardContent::Text(text) => process_text_upload(manager, &text, settings, ctx),
        ClipboardContent::Files(files) => upload_files(manager, &files, settings, ctx),
    }
}

/// `UploadImageStream` / a data upload (the paste window's text as a named file).
pub fn upload_data(
    manager: &mut TaskManager,
    data_type: DataType,
    bytes: Vec<u8>,
    file_name: &str,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    if bytes.is_empty() || file_name.is_empty() {
        return;
    }
    let task = WorkerTask::data_upload(data_type, bytes, file_name, settings.clone());
    manager.start(task, ctx);
}

/// `ShortenURL`.
pub fn shorten_url(
    manager: &mut TaskManager,
    url: &str,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    if url.is_empty() {
        return;
    }
    manager.start(WorkerTask::url_shortener(url, settings.clone()), ctx);
}

/// `ShareURL`.
pub fn share_url(manager: &mut TaskManager, url: &str, settings: &TaskSettings, ctx: &TaskContext) {
    if url.is_empty() {
        return;
    }
    manager.start(WorkerTask::share_url(url, settings.clone()), ctx);
}

/// `DownloadAndUploadFile`.
pub fn download_and_upload(
    manager: &mut TaskManager,
    url: &str,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    download_file(manager, url, true, settings, ctx);
}

/// `DownloadFile`.
pub fn download_file(
    manager: &mut TaskManager,
    url: &str,
    upload: bool,
    settings: &TaskSettings,
    ctx: &TaskContext,
) {
    if url.is_empty() {
        return;
    }
    if let Some(task) = WorkerTask::download(url, upload, settings.clone(), ctx) {
        manager.start(task, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_expand_to_every_file_inside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("one.txt"), "1").unwrap();
        std::fs::write(dir.path().join("a/two.txt"), "2").unwrap();
        std::fs::write(dir.path().join("a/b/three.txt"), "3").unwrap();
        let files = expand_paths(&[dir.path().to_path_buf()]);
        assert_eq!(files.len(), 3);
        assert!(needs_confirmation(11, true));
        assert!(!needs_confirmation(11, false));
        assert!(!needs_confirmation(10, true));
    }
}

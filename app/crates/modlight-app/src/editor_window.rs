//! The image editor's window (`EditorWindow` through `ImageEditorIntegration`) and
//! `TaskHelpers`' ways into it: the Image editor hotkey and menu (`OpenImageEditor`, the
//! start screen), a file (`AnnotateImageFromFile`, the notice's "Annotate image"), and a
//! task's picture (`AnnotateImage` in task mode, `BeautifyImage` with the Background
//! panel open). What the editor asks for (`ImageEditorCallbacks`: copy, save, save as,
//! upload; open, insert, paste, the clipboard, a URL) is done here; closing hands the
//! result back (`EditorTaskResult`): to the waiting task, or to `RunImageTask`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{ViewportBuilder, ViewportCommand, ViewportId, vec2};
use image::RgbaImage;
use modlight_core::tasks::TaskAnswer;
use modlight_core::tasks::info::TaskMetadata;
use modlight_core::tasks::settings::ImageFormat;
use modlight_editor::editor::{Editor, EditorConfig, Request, TaskResult};
use modlight_editor::effects::ParamValue;
use modlight_editor::options::ImageEditorOptions;
use modlight_editor::ui::EditorView;
use tokio::sync::oneshot;

use crate::state::AppState;

/// Where the editor's result goes when its window closes.
pub enum Finish {
    /// A task waits for the picture (`TaskQuestion::Annotate`/`Beautify`).
    Task(oneshot::Sender<TaskAnswer>),
    /// `AnnotateImageAsync`: a picture that comes back runs as a new image task.
    RunImageTask,
}

/// One open editor window.
pub struct EditorSession {
    pub key: u64,
    pub editor: Editor,
    pub view: EditorView,
    finish: Option<Finish>,
    /// The window asked to close and the editor agreed (`CloseRequested`).
    pub closed: bool,
    /// The title last given to the window.
    title: String,
}

/// The editor windows and their counter.
#[derive(Default)]
pub struct EditorWindows {
    pub sessions: Vec<EditorSession>,
    counter: u64,
    /// Cmd+V from the AppKit key monitor while an editor window had the focus.
    pub paste: bool,
}

/// `ImageHelpers.SaveImage` by the file's extension (png, jpg, gif, bmp, tif).
pub fn encode_for_path(
    image: &RgbaImage,
    path: &Path,
    settings: &modlight_core::tasks::TaskSettings,
) -> Vec<u8> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let format = match ext.as_str() {
        "jpg" | "jpeg" => ImageFormat::JPEG,
        "gif" => ImageFormat::GIF,
        "bmp" => ImageFormat::BMP,
        "tif" | "tiff" => ImageFormat::TIFF,
        _ => ImageFormat::PNG,
    };
    let images = &settings.image_settings;
    modlight_core::tasks::image::save_image(
        image,
        format,
        images.image_png_bit_depth,
        images.image_jpeg_quality,
        images.image_gif_quality,
    )
}

/// The picture formats the open panels offer (`ImageFileFilter`).
const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp"];

fn load_image(path: &Path) -> Option<RgbaImage> {
    let image = modlight_editor::editor::background::open_image(path);
    if image.is_none() {
        log::warn!(
            "Modlight: the image editor could not open {}",
            path.display()
        );
    }
    image
}

/// The desktop picture for the Wallpaper background (`IDesktopWallpaperService`).
struct MacWallpaper;

impl modlight_editor::editor::background::WallpaperService for MacWallpaper {
    fn wallpaper(&self) -> Option<(String, modlight_editor::frame::ImageLayout)> {
        use modlight_editor::frame::ImageLayout;
        let (path, options) = modlight_macos::editor::desktop_wallpaper()?;
        // `MapDesktopWallpaperLayout` from `NSImageScaling` (and clipping).
        let layout = match options.scaling {
            2 => ImageLayout::Center,
            1 => ImageLayout::Stretch,
            0 => ImageLayout::Fit,
            _ if options.allow_clipping => ImageLayout::Fill,
            _ => ImageLayout::Fit,
        };
        Some((path.to_string_lossy().into_owned(), layout))
    }
}

/// The system's part of the editor: macOS cursors for cursor annotations, the desktop
/// picture, and `NSImage` for formats the image crate does not read. Real app only.
pub fn install_system_services() {
    modlight_editor::cursor::set_provider(Box::new(|cursor| {
        modlight_macos::editor::cursor_image(cursor.name())
    }));
    modlight_editor::editor::background::set_wallpaper_service(Some(Box::new(MacWallpaper)));
    modlight_editor::editor::background::set_image_loader(Some(Box::new(|path| {
        modlight_macos::editor::load_image(path)
    })));
}

impl AppState {
    /// `ToolsSettings.ImageEditorOptions` of the default task settings.
    pub fn editor_options(&self) -> ImageEditorOptions {
        let value = self
            .config
            .get("DefaultTaskSettings")
            .and_then(|d| d.get("ToolsSettings"))
            .and_then(|t| t.get("ImageEditorOptions"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        ImageEditorOptions::from_value(&value)
    }

    fn store_editor_options(&mut self, options: &ImageEditorOptions) {
        self.set_task_member(
            crate::task_pages::TaskTarget::Defaults,
            &["ToolsSettings", "ImageEditorOptions"],
            options.to_value(),
        );
    }

    /// `ShowEditorDialog`: a new editor window.
    pub fn open_editor(
        &mut self,
        image: Option<RgbaImage>,
        path: Option<PathBuf>,
        task_mode: bool,
        background_panel: bool,
        finish: Finish,
    ) {
        let config = EditorConfig {
            show_file_menu: true,
            show_options_button: true,
            show_task_buttons: true,
            use_continue_workflow: task_mode,
            show_bottom_toolbar: true,
            show_start_screen: !task_mode,
            open_background_panel: background_panel,
        };
        let mut editor = Editor::new(self.editor_options(), config);
        let path_text = path.as_ref().map(|p| p.to_string_lossy().into_owned());
        match image {
            Some(image) => editor.load_image(image, path_text),
            None => {
                editor.image_file_path = path_text;
                if !task_mode {
                    editor.show_start_screen();
                }
            }
        }
        let mut view = EditorView::new();
        view.host_watches_paste = modlight_macos::keys::monitoring();
        let windows = &mut self.windows.editors;
        windows.counter += 1;
        windows.sessions.push(EditorSession {
            key: windows.counter,
            editor,
            view,
            finish: Some(finish),
            closed: false,
            title: String::new(),
        });
    }

    /// `TaskHelpers.OpenImageEditor` after the selector: the start screen.
    pub fn open_image_editor_window(&mut self) {
        self.open_editor(None, None, false, false, Finish::RunImageTask);
    }

    /// `AnnotateImageFromFile`.
    pub fn annotate_file(&mut self, path: &Path) {
        if !path.is_file() {
            let text = modlight_core::l10n::tf(
                modlight_core::l10n::s::TaskHelpers_FileDoesNotExist,
                &[path.display().to_string().into()],
            );
            self.notice(modlight_core::icons::INFO, &text, "", true);
            return;
        }
        match load_image(path) {
            Some(image) => self.open_editor(
                Some(image),
                Some(path.to_path_buf()),
                false,
                false,
                Finish::RunImageTask,
            ),
            None => self.notice(
                modlight_core::icons::INFO,
                &path.display().to_string(),
                "",
                true,
            ),
        }
    }

    /// `TaskHelpers.AnnotateImage(…, taskMode: true)` (and `BeautifyImage` with the
    /// Background panel open): the task waits until the window closes.
    pub fn annotate_for_task(
        &mut self,
        image: Arc<RgbaImage>,
        background_panel: bool,
        reply: oneshot::Sender<TaskAnswer>,
    ) {
        self.open_editor(
            Some((*image).clone()),
            None,
            true,
            background_panel,
            Finish::Task(reply),
        );
    }

    /// What one editor asked for this frame.
    pub fn editor_requests(&mut self, key: u64) {
        let Some(index) = self
            .windows
            .editors
            .sessions
            .iter()
            .position(|s| s.key == key)
        else {
            return;
        };
        let requests = self.windows.editors.sessions[index].editor.take_requests();
        for request in requests {
            self.editor_request(key, request);
        }
    }

    fn session(&mut self, key: u64) -> Option<&mut EditorSession> {
        self.windows
            .editors
            .sessions
            .iter_mut()
            .find(|s| s.key == key)
    }

    fn editor_request(&mut self, key: u64, request: Request) {
        let headless = self.env.qa || self.mode != crate::app::Mode::Normal;
        let settings = self.default_task_settings();
        match request {
            Request::CopyImage => {
                let Some(image) = self.session(key).and_then(|s| s.editor.snapshot()) else {
                    return;
                };
                self.effects
                    .push(crate::state::Effect::CopyImage(Arc::new(image)));
                if let Some(s) = self.session(key) {
                    s.editor.show_task_action_notification(
                        modlight_core::l10n::t(
                            modlight_core::l10n::editor::MainViewModel_ImageCopiedToClipboard,
                        ),
                        modlight_editor::ui::icons::ACTION_COPY,
                    );
                    s.editor.close_after_task_action();
                }
            }
            Request::Save | Request::SaveAs => {
                let save_as = matches!(request, Request::SaveAs);
                let Some((image, current)) = self
                    .session(key)
                    .and_then(|s| Some((s.editor.snapshot()?, s.editor.image_file_path.clone())))
                else {
                    return;
                };
                let default_path = match current.filter(|p| !p.is_empty()) {
                    Some(p) => PathBuf::from(p),
                    None => {
                        let metadata = TaskMetadata {
                            image: Some(Arc::new(image.clone())),
                            ..TaskMetadata::default()
                        };
                        let folder = self
                            .uploads
                            .ctx
                            .screenshots_folder(&settings, Some(&metadata));
                        let counter = self
                            .uploads
                            .ctx
                            .auto_increment
                            .load(std::sync::atomic::Ordering::SeqCst);
                        let (name, next) = modlight_core::tasks::naming::file_name(
                            &settings,
                            Some(settings.image_settings.image_format.extension()),
                            Some(&metadata),
                            counter,
                        );
                        self.uploads
                            .ctx
                            .auto_increment
                            .store(next, std::sync::atomic::Ordering::SeqCst);
                        folder.join(name)
                    }
                };
                let path = if save_as && !headless {
                    let mut dialog = rfd::FileDialog::new()
                        .set_file_name(
                            default_path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        )
                        .add_filter("PNG", &["png"])
                        .add_filter("JPEG", &["jpg", "jpeg"])
                        .add_filter("GIF", &["gif"])
                        .add_filter("BMP", &["bmp"])
                        .add_filter("TIFF", &["tif", "tiff"]);
                    if let Some(parent) = default_path.parent() {
                        dialog = dialog.set_directory(parent);
                    }
                    match dialog.save_file() {
                        Some(p) => p,
                        None => return,
                    }
                } else {
                    default_path
                };
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let bytes = encode_for_path(&image, &path, &settings);
                if let Err(error) = std::fs::write(&path, bytes) {
                    log::warn!("Modlight: the edited image could not be saved: {error}");
                    return;
                }
                let text = path.to_string_lossy().into_owned();
                if let Some(s) = self.session(key) {
                    s.editor.image_file_path = Some(text.clone());
                    s.editor.is_dirty = false;
                    s.editor.options.add_recent_image_file(&text);
                    s.editor.saved_notification(&text, save_as);
                    s.editor.close_after_task_action();
                }
            }
            Request::Upload => {
                let Some(image) = self.session(key).and_then(|s| s.editor.snapshot()) else {
                    return;
                };
                // `MainFormUploadImage`: an upload of the picture as it is.
                let mut upload = settings.clone();
                upload.after_capture_job =
                    modlight_core::tasks::settings::AfterCaptureTasks::UploadImageToHost;
                let ctx = self.uploads.ctx.clone();
                modlight_core::tasks::upload_manager::run_image_task(
                    &mut self.uploads.tasks,
                    image,
                    &upload,
                    &ctx,
                );
                if let Some(s) = self.session(key) {
                    s.editor.close_after_task_action();
                }
            }
            // `TaskHelpers.PrintImage(image)` (phase 5, `tools::print`).
            Request::Print => {
                if let Some(image) = self.session(key).and_then(|s| s.editor.snapshot()) {
                    self.print_picture(image);
                }
            }
            // `TaskHelpers.PinToScreen(image)` (phase 5, `tools::pin`).
            Request::PinToScreen => {
                if let Some(image) = self.session(key).and_then(|s| s.editor.snapshot()) {
                    self.pin_picture(std::sync::Arc::new(image), &settings);
                }
            }
            // The screen colour picker (phase 5, `tools::editor_hooks`).
            Request::PickScreenColor(target) => self.editor_pick_screen_color(key, target),
            Request::OpenImage
            | Request::InsertImage
            | Request::ReplaceImage(_)
            | Request::BrowseBackgroundImage => {
                if headless {
                    return;
                }
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("Images", &IMAGE_EXTENSIONS)
                    .pick_file()
                else {
                    return;
                };
                self.editor_file_chosen(key, request, &path);
            }
            Request::OpenRecent(path) => {
                self.editor_file_chosen(key, Request::OpenImage, Path::new(&path));
            }
            Request::BrowseEffectFile(param) => {
                if headless {
                    return;
                }
                if let Some(path) = rfd::FileDialog::new().pick_file()
                    && let Some(s) = self.session(key)
                {
                    s.editor.set_effect_value(
                        &param,
                        ParamValue::Text(path.to_string_lossy().into_owned()),
                    );
                }
            }
            Request::LoadFromClipboard | Request::Paste => {
                let paste = matches!(request, Request::Paste);
                let content = self.read_clipboard();
                let Some(s) = self.session(key) else {
                    return;
                };
                // A picture, or the first picture file (`PasteImageFromClipboard`).
                let found = match content {
                    Some(modlight_core::tasks::upload_manager::ClipboardContent::Image(image)) => {
                        Some((image, None))
                    }
                    Some(modlight_core::tasks::upload_manager::ClipboardContent::Files(files)) => {
                        files
                            .iter()
                            .filter(|f| {
                                f.extension()
                                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                                    .is_some_and(|e| {
                                        IMAGE_EXTENSIONS.contains(&e.as_str()) || e == "ico"
                                    })
                            })
                            .find_map(|f| {
                                load_image(f).map(|i| (i, Some(f.to_string_lossy().into_owned())))
                            })
                    }
                    _ => None,
                };
                match (found, paste) {
                    (Some((image, path)), true) => s.editor.insert_external_image(image, path),
                    (Some((image, path)), false) => {
                        s.editor.modal = None;
                        s.editor.load_bitmap(image, path);
                    }
                    // Nothing outside: the editor's own copied annotation.
                    (None, true) => s.editor.paste_annotation(),
                    (None, false) => s.editor.start_screen_status(modlight_core::l10n::t(
                        modlight_core::l10n::editor::EditorView_ClipboardDoesNotContainImage,
                    )),
                }
            }
            Request::LoadFromUrl(url) => {
                self.rt.spawn(async move {
                    let result = download_image(&url).await;
                    move |state: &mut AppState, _: &egui::Context| {
                        let Some(s) = state.session(key) else {
                            return;
                        };
                        match result {
                            Ok(image) => {
                                s.editor.modal = None;
                                s.editor.load_bitmap(image, None);
                            }
                            Err(message) => s.editor.start_screen_status(&message),
                        }
                    }
                });
            }
            Request::ClearClipboard => self.effects.push(crate::state::Effect::ClearClipboard),
            Request::Close => {
                if let Some(s) = self.session(key) {
                    s.closed = true;
                }
            }
        }
    }

    fn editor_file_chosen(&mut self, key: u64, request: Request, path: &Path) {
        let text = path.to_string_lossy().into_owned();
        if let Request::BrowseBackgroundImage = request {
            if let Some(s) = self.session(key) {
                s.editor.set_background_image(&text);
            }
            return;
        }
        let Some(image) = load_image(path) else {
            if let Some(s) = self.session(key)
                && matches!(
                    s.editor.modal,
                    Some(modlight_editor::editor::Modal::StartScreen(_))
                )
            {
                // `OnLoadRecentFileRequested`: upstream's sentence with the path.
                s.editor.start_screen_status(&modlight_core::l10n::tf(
                    modlight_core::l10n::editor::EditorView_FailedToLoadImageFileFormat,
                    &[text.as_str().into()],
                ));
            }
            return;
        };
        let Some(s) = self.session(key) else {
            return;
        };
        match request {
            Request::InsertImage => s.editor.insert_external_image(image, Some(text)),
            Request::ReplaceImage(id) => s.editor.replace_image(id, image, Some(text)),
            _ => {
                s.editor.modal = None;
                s.editor.load_bitmap(image, Some(text.clone()));
                s.editor.opened_notification(&text);
            }
        }
    }

    /// The window closed: the options are kept and the result goes where it is owed.
    pub fn close_editor(&mut self, key: u64) {
        let Some(index) = self
            .windows
            .editors
            .sessions
            .iter()
            .position(|s| s.key == key)
        else {
            return;
        };
        let mut session = self.windows.editors.sessions.remove(index);
        let result = match session.editor.task_result {
            TaskResult::Continue => session.editor.snapshot(),
            TaskResult::ContinueNoSave => session.editor.source_image(),
            TaskResult::None | TaskResult::Cancel => None,
        };
        let options = session.editor.options.clone();
        self.store_editor_options(&options);
        match session.finish.take() {
            Some(Finish::Task(reply)) => {
                let _ = reply.send(TaskAnswer::Annotated(result.map(Arc::new)));
            }
            Some(Finish::RunImageTask) => {
                if let Some(image) = result {
                    let settings = self.default_task_settings();
                    self.run_image_task(crate::capture_flow::PendingImage {
                        metadata: TaskMetadata {
                            image: Some(Arc::new(image)),
                            ..TaskMetadata::default()
                        },
                        settings,
                        file: None,
                        skip_quick_menu: false,
                    });
                }
            }
            None => {}
        }
    }
}

/// The most an address opened in the editor may send: far more than any picture the
/// decoding limits let through, far less than would exhaust memory.
const MAX_URL_IMAGE_BYTES: usize = 256 * 1024 * 1024;

/// `RequestLoadFromUrl`: the picture behind an address, with upstream's messages
/// (`OnLoadFromUrlRequested`): a failed download says why, anything that does not decode
/// is "not a valid image".
async fn download_image(url: &str) -> Result<RgbaImage, String> {
    use modlight_core::l10n::{editor, tf};
    let failed = |message: String| {
        tf(
            editor::EditorView_FailedToDownloadImageFormat,
            &[message.into()],
        )
    };
    let client = modlight_core::components::net::file_client().map_err(failed)?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| failed(e.to_string()))?;
    if !response.status().is_success() {
        return Err(failed(response.status().to_string()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| failed(e.to_string()))? {
        if bytes.len() + chunk.len() > MAX_URL_IMAGE_BYTES {
            log::warn!("Modlight: the editor stopped a download past {MAX_URL_IMAGE_BYTES} bytes.");
            return Err(not_an_image());
        }
        bytes.extend_from_slice(&chunk);
    }
    decode_downloaded(&bytes)
}

fn not_an_image() -> String {
    modlight_core::l10n::t(modlight_core::l10n::editor::EditorView_UrlDoesNotPointToValidImage)
        .to_string()
}

/// The downloaded bytes as a picture, under the app's decoding limits.
fn decode_downloaded(bytes: &[u8]) -> Result<RgbaImage, String> {
    modlight_core::image_decode::from_bytes(bytes)
        .map(|i| i.into_rgba8())
        .map_err(|error| {
            log::warn!("Modlight: the editor could not open a downloaded picture: {error}");
            not_an_image()
        })
}

/// The window builder: the saved size when "Remember window state" is on, otherwise
/// maximised (`WindowState="Maximized"`).
fn builder(session: &EditorSession) -> ViewportBuilder {
    let o = &session.editor.options;
    let remember = o.remember_window_state && o.window_width > 0.0 && o.window_height > 0.0;
    let mut builder = ViewportBuilder::default()
        .with_title(session.editor.window_title())
        .with_min_inner_size(vec2(640.0, 420.0));
    if remember {
        builder = builder
            .with_inner_size(vec2(o.window_width as f32, o.window_height as f32))
            .with_maximized(o.is_window_maximized);
    } else {
        builder = builder
            .with_inner_size(vec2(1280.0, 720.0))
            .with_maximized(true);
    }
    builder
}

/// Draws one editor into `ui` (the window's content, and the QA shots).
pub fn editor_ui(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    colors: &modlight_editor::ui::Colors,
) {
    session.view.show(ui, &mut session.editor, colors);
}

/// Every editor window, each its own native window. Called from the app's viewports.
pub fn editor_viewports(state: &mut AppState, ctx: &egui::Context) {
    if state.windows.editors.sessions.is_empty() {
        return;
    }
    let colors = modlight_editor::ui::Colors::new(&state.theme);
    let keys: Vec<u64> = state
        .windows
        .editors
        .sessions
        .iter()
        .map(|s| s.key)
        .collect();
    for key in keys {
        let paste = state.windows.editors.paste;
        let Some(session) = state.session(key) else {
            continue;
        };
        let builder = builder(session);
        let title = session.editor.window_title();
        let focused_paste = ctx.show_viewport_immediate(
            ViewportId::from_hash_of(("image-editor", key)),
            builder,
            |ui, _| {
                let ctx = ui.ctx().clone();
                // Closing the window asks the editor first (`OnClosing` → `RequestClose`).
                if ctx.input(|i| i.viewport().close_requested()) && !session.closed {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    session.editor.request_close(true);
                }
                if session.title != title {
                    session.title = title.clone();
                    ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
                }
                // Picture files dropped from Finder (`OnDrop`): the picture when there is
                // none yet, an inserted image otherwise.
                let dropped: Vec<PathBuf> = ctx.input(|i| {
                    i.raw
                        .dropped_files
                        .iter()
                        .map(|f| f.path().to_path_buf())
                        .filter(|p| !p.as_os_str().is_empty())
                        .collect()
                });
                for path in dropped {
                    let is_image = path
                        .extension()
                        .map(|e| e.to_string_lossy().to_ascii_lowercase())
                        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()) || e == "ico");
                    if !is_image {
                        continue;
                    }
                    if let Some(image) = load_image(&path) {
                        let text = Some(path.to_string_lossy().into_owned());
                        if session.editor.has_image() {
                            session.editor.insert_external_image(image, text);
                        } else {
                            session.editor.modal = None;
                            session.editor.load_bitmap(image, text);
                        }
                        break;
                    }
                }
                let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
                if paste && focused {
                    session.view.paste_shortcut(&mut session.editor);
                }
                // "Remember window state": the size of the normal window and whether it is
                // maximised.
                let (inner, maximized) = ctx.input(|i| {
                    (
                        i.viewport().inner_rect,
                        i.viewport().maximized.unwrap_or(false),
                    )
                });
                if session.editor.options.remember_window_state {
                    session.editor.options.is_window_maximized = maximized;
                    if let Some(inner) = inner.filter(|_| !maximized) {
                        session.editor.options.window_width = inner.width() as f64;
                        session.editor.options.window_height = inner.height() as f64;
                    }
                }
                editor_ui(ui, session, &colors);
                paste && focused
            },
        );
        if focused_paste {
            state.windows.editors.paste = false;
        }
        state.editor_requests(key);
        if state.session(key).is_some_and(|s| s.closed) {
            state.close_editor(key);
        }
    }
    // A paste nobody took is dropped.
    state.windows.editors.paste = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S3: an address that answers with a small file claiming an enormous picture gets
    /// upstream's "not a valid image", and nothing is allocated for it.
    #[test]
    fn a_downloaded_picture_claiming_an_enormous_size_is_not_a_valid_image() {
        let mut png = Vec::new();
        RgbaImage::from_pixel(16, 16, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(decode_downloaded(&png).unwrap().dimensions(), (16, 16));

        let mut jpeg = Vec::new();
        image::RgbImage::from_pixel(16, 16, image::Rgb([1, 2, 3]))
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let sof = jpeg.windows(2).position(|w| w == [0xff, 0xc0]).unwrap();
        jpeg[sof + 5..sof + 7].copy_from_slice(&60_000u16.to_be_bytes());
        jpeg[sof + 7..sof + 9].copy_from_slice(&60_000u16.to_be_bytes());
        assert_eq!(decode_downloaded(&jpeg).unwrap_err(), not_an_image());
        assert_eq!(decode_downloaded(b"<html>").unwrap_err(), not_an_image());
    }
}

//! Everything a menu, a button or a key can ask the app to do, as data. Drawing code
//! returns a [`Command`]; [`AppState::run`] carries it out after the frame, where the
//! whole state is free to change.

use modlight_core::library::LinkFormat;

use crate::state::{AppState, Tab};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // The window.
    SelectTab(Tab),
    ShowSignIn,
    ShowSettings(Option<&'static str>),
    SignOut,
    ShowMainWindow,
    Quit,
    /// A tab or tool that arrives in a later phase was asked for.
    NotYet,

    // My space: where you are.
    Reload,
    GoUp,
    NavigateTo(Option<String>),
    SelectView(usize),
    ToggleLibraryPanel,
    ToggleFoldersSection,
    ToggleFilesSection,
    GoToPage(i64),
    SetPageSize(i64),
    SelectAll,

    // Folders.
    NewFolder(Option<String>),
    RenameFolder(String),
    SetFolderPublic(String, bool),
    OpenGallery(String),
    CopyGalleryLink(String),
    ShowInPanel(String),
    DeleteFolder(String),
    UploadTo(Option<String>),

    // Files.
    ShowDetails(String),
    CopyLinks(Vec<String>),
    CopyAs(Vec<String>, LinkFormat),
    OpenInBrowser(String),
    Download(String),
    DownloadAs(String, String),
    MoveTo(Vec<String>, Option<String>),
    MoveToNewFolder(Vec<String>),
    SetPublished(Vec<String>, bool),
    CustomUrl(String),
    LinkSettings(String),
    Trash(Vec<String>),
    Restore(Vec<String>),
    DeleteForever(Vec<String>),
    EmptyTrash,
    ScanDuplicates,

    // Uploading (`MainMenuBuilder.BuildUploadMenu` and the Upload tab).
    UploadFolder,
    ClipboardUpload,
    UploadText,
    PasteText,
    UploadUrl,
    DragDropUpload,
    ShortenUrl,
    CancelTask(u64),
    QueueSendNow(String),
    QueueCancel(String),
    RefreshUploadHistory,
    CopyHistoryLink(u64),
    OpenUrl(String),
    RetryUpload(std::path::PathBuf),
    ToggleAfterUploadTask(u32),
    ToggleAfterCaptureTask(u32),
    /// The tray's image effect preset (`SelectedImageEffectPreset` of the defaults).
    SelectImageEffectPreset(usize),
    OpenScreenshotsFolder,
    CopyText(String),
    /// Open a file with its app (`FileHelpers.OpenFile`).
    OpenFile(std::path::PathBuf),

    // Capturing (`MainMenuBuilder.BuildCaptureMenu`, the Capture tab, the hotkeys).
    /// A capture with the default task settings; `true` hides the main window first.
    Capture(modlight_core::capture::CaptureKind, bool),
    StartRecording(
        modlight_core::recording::ScreenRecordOutput,
        crate::capture_flow::RecordStartMethod,
    ),
    RunJob(modlight_core::hotkeys::HotkeyType),
    ToggleHotkeys,
    ToggleShowCursor,
    SetScreenshotDelay(i64),
    OpenAutoCapture,
    OpenScrollingCapture,
    /// An entry of the quick task menu, by position.
    QuickTask(usize),
    /// An entry of a menu whose owner reads the position itself; never run.
    Local(usize),
    /// `MainFormCommand.About`.
    ShowAbout,
}

impl AppState {
    /// Carry out one command.
    pub fn run(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::SelectTab(tab) => self.select_tab(tab),
            Command::ShowSignIn => self.show_sign_in(),
            Command::ShowSettings(section) => self.show_settings(section),
            Command::SignOut => {
                self.sign_out();
                self.refresh_account_strip();
            }
            Command::ShowMainWindow => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                // `ShowAndActivate`: the moment somebody looks again is when the storage
                // figure, the account and the folders (made and deleted in the web panel
                // meanwhile) have to be true.
                self.refresh_account_strip();
                self.space_refresh_account();
                self.refresh_folders();
            }
            Command::Quit => self.quit_requested = true,
            Command::NotYet => {
                let title = modlight_core::l10n::t(modlight_core::l10n::s::MainWindow_PlanLater)
                    .to_string();
                self.notice(modlight_core::icons::INFO, &title, "", false);
            }
            Command::Reload => self.space_reload(),
            Command::GoUp => self.space_go_up(),
            Command::NavigateTo(id) => self.space_navigate_to(id),
            Command::SelectView(index) => self.space_select_view(index),
            Command::ToggleLibraryPanel => self.toggle_library_panel(),
            Command::ToggleFoldersSection => {
                self.folders_section = !self.folders_section;
                self.save_config();
            }
            Command::ToggleFilesSection => {
                self.files_section = !self.files_section;
                self.save_config();
            }
            Command::GoToPage(page) => self.space_go_to_page(page),
            Command::SetPageSize(size) => self.space_set_page_size(size),
            Command::SelectAll => self.space.select_all_files(),
            Command::NewFolder(parent) => self.ask_new_folder(parent),
            Command::RenameFolder(id) => self.ask_rename_folder(&id),
            Command::SetFolderPublic(id, public) => self.set_folder_public(&id, public),
            Command::OpenGallery(id) => self.open_gallery(&id),
            Command::CopyGalleryLink(id) => self.copy_gallery_link(&id),
            Command::ShowInPanel(id) => self.show_in_panel(&id),
            Command::DeleteFolder(id) => self.ask_delete_folder(&id),
            Command::UploadTo(folder) => self.pick_and_upload(folder),
            Command::ShowDetails(id) => self.show_details(&id, ctx),
            Command::CopyLinks(ids) => self.copy_links(&ids),
            Command::CopyAs(ids, format) => self.copy_as(&ids, format),
            Command::OpenInBrowser(id) => {
                if let Some(file) = self.space.file(&id).filter(|f| f.can_open()) {
                    let link = file.open_link.clone();
                    self.open_url(&link);
                }
            }
            Command::Download(id) => self.download(&id, None),
            Command::DownloadAs(id, to) => self.download(&id, Some(to)),
            Command::MoveTo(ids, folder) => self.move_files(&ids, folder),
            Command::MoveToNewFolder(ids) => self.ask_move_to_new_folder(ids),
            Command::SetPublished(ids, published) => self.set_published(&ids, published),
            Command::CustomUrl(id) => self.ask_custom_url(&id),
            Command::LinkSettings(id) => self.ask_link_settings(&id),
            Command::Trash(ids) => self.ask_trash(&ids, false),
            Command::Restore(ids) => self.restore_files(&ids),
            Command::DeleteForever(ids) => self.ask_trash(&ids, true),
            Command::EmptyTrash => self.empty_trash(),
            Command::ScanDuplicates => self.scan_duplicates(),
            Command::UploadFolder => self.pick_folder_and_upload(),
            Command::ClipboardUpload => self.clipboard_upload_main_window(),
            Command::UploadText => self.open_text_upload(),
            Command::PasteText => self.open_paste_window(),
            Command::UploadUrl => self.open_url_upload(),
            Command::DragDropUpload => self.open_drop_window(),
            Command::ShortenUrl => self.open_shorten_url(),
            Command::CancelTask(id) => self.cancel_task(id),
            Command::QueueSendNow(id) => self.uploads.queue.send_now(&id),
            Command::QueueCancel(id) => self.uploads.queue.cancel(&id),
            Command::RefreshUploadHistory => self.refresh_upload_history(),
            Command::CopyHistoryLink(key) => self.copy_history_link(key),
            Command::OpenUrl(url) => self.open_url(&url),
            Command::RetryUpload(path) => self.retry_upload(path),
            Command::ToggleAfterUploadTask(bit) => self.toggle_after_upload_task(bit),
            Command::ToggleAfterCaptureTask(bit) => self.toggle_after_capture_task(bit),
            Command::SelectImageEffectPreset(index) => self.select_image_effect_preset(index),
            Command::OpenScreenshotsFolder => self.open_screenshots_folder(),
            Command::CopyText(text) => self.copy_text(&text),
            Command::OpenFile(path) => self.effects.push(crate::state::Effect::OpenFile(path)),
            Command::Capture(kind, auto_hide) => self.capture(kind, auto_hide),
            Command::StartRecording(output, method) => {
                let settings = self.default_task_settings();
                self.start_screen_recording(output, method, settings);
            }
            Command::RunJob(job) => {
                let settings = self.default_task_settings();
                self.execute_job(job, settings);
            }
            Command::ToggleHotkeys => self.toggle_hotkeys(),
            Command::ToggleShowCursor => self.toggle_show_cursor(),
            Command::SetScreenshotDelay(seconds) => self.set_screenshot_delay(seconds as f64),
            Command::OpenAutoCapture => {
                let settings = self.default_task_settings();
                self.open_auto_capture(settings);
            }
            Command::OpenScrollingCapture => {
                let settings = self.default_task_settings();
                self.open_scrolling_capture(settings);
            }
            Command::Local(_) => {}
            Command::ShowAbout => self.show_about(),
            Command::QuickTask(index) => {
                let choice = self
                    .captures
                    .quick_menu
                    .as_ref()
                    .map(|(menu, _)| menu.choice(index));
                match choice {
                    Some(Some(choice)) => self.quick_menu_finished(choice),
                    // "Edit this menu...": the editor window (`QuickTaskMenuEditor`).
                    Some(None) => self.open_quick_task_menu_editor(),
                    None => {}
                }
            }
        }
    }

    pub fn select_tab(&mut self, tab: Tab) {
        if self.tab == tab {
            return;
        }
        if tab == Tab::Admin && !self.is_admin() {
            return;
        }
        self.tab = tab;
        // `AdminView` reads the server only when its tab is opened.
        if tab == Tab::Admin {
            self.server_tab_show();
        }
    }
}

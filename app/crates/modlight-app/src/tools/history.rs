//! What the two history windows (`HistoryWindow`, `ImageHistoryWindow`) share: History.db
//! read off the UI thread, the context menu's actions, the services the windows call
//! (upload, edit, pin, analyze, the image viewer), and the dialogs drawn over the window
//! (statistics, folder import, edit item, the prompts). The windows are in
//! `history_list.rs` and `image_history.rs`; the logic without drawing is
//! `modlight_tools::history`.
//!
//! Files deleted from here are deleted for good, as upstream does (the dialog says the
//! action cannot be undone). Dragging rows out to other apps is not available: egui has no
//! drag source for other apps.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Align2, Id, Rect, Sense, Ui, pos2, vec2};
use modlight_core::history::{HistoryItem, HistoryStore};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, helpers as hp, history as hl, t};
use modlight_core::names::file_helpers;
use modlight_tools::history::{self as logic, CopyFormat, MenuState};

use super::kit::{self, Track};
use crate::commands::Command;
use crate::menus::Entry;
use crate::state::{AppState, Effect};
use crate::theme::{Colors, Weight};

/// One action of the context menus and the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    OpenUrl,
    OpenShortUrl,
    OpenThumbnailUrl,
    OpenDeletionUrl,
    OpenFile,
    OpenFolder,
    Copy(CopyFormat),
    CopyFile,
    CopyImage,
    CopyText,
    Favorite,
    EditTag,
    EditItem,
    Rename,
    Delete,
    DeleteFile,
    ShowImage,
    /// The image history's Enter: the picture, else open the file (asking for others).
    OpenItem,
    Upload,
    EditImage,
    Pin,
    Analyze,
}

/// A menu being built: entries and the action of each `Command::Local` position.
#[derive(Default)]
pub struct MenuBuilder {
    pub acts: Vec<Act>,
}

impl MenuBuilder {
    pub fn item(&mut self, label: &str, icon: Option<char>, act: Act, enabled: bool) -> Entry {
        self.acts.push(act);
        Entry::Item {
            label: label.to_string(),
            icon,
            enabled,
            toggle: crate::menus::Toggle::None,
            command: Some(Command::Local(self.acts.len() - 1)),
        }
    }
}

/// The dialogs over a history window (`ModalOverlay`).
#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Stats(String),
    Import(ImportForm),
    /// The window's own settings dialog.
    Settings,
    Edit(Box<EditForm>),
    Prompt(Prompt),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportForm {
    pub folder: String,
    pub images_only: bool,
    pub skip_duplicates: bool,
    pub status: String,
    pub busy: bool,
}

/// `EditDialog`'s fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditForm {
    pub id: i64,
    pub file_name: String,
    pub file_path: String,
    pub date: String,
    pub kind: String,
    pub host: String,
    pub url: String,
    pub thumbnail_url: String,
    pub deletion_url: String,
    pub shortened_url: String,
    pub tags: String,
}

/// What a prompt's confirm button does.
#[derive(Clone, Debug, PartialEq)]
pub enum PromptAction {
    Tag(i64),
    Rename(i64),
    Delete { ids: Vec<i64>, files: bool },
    OpenFile(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub title: String,
    pub message: String,
    pub confirm: String,
    pub input: Option<String>,
    pub action: PromptAction,
}

/// The rows and what both windows keep about them.
pub struct HistoryData {
    pub image_history: bool,
    pub store: Option<Arc<HistoryStore>>,
    /// Newest first (`items.Reverse()`).
    pub items: Vec<HistoryItem>,
    pub loaded: bool,
    pub load_error: Option<String>,
    pub modal: Option<Modal>,
    load_version: u64,
}

impl HistoryData {
    pub fn new(store: Option<Arc<HistoryStore>>, image_history: bool) -> HistoryData {
        HistoryData {
            image_history,
            store,
            items: Vec::new(),
            loaded: false,
            load_error: None,
            modal: None,
            load_version: 0,
        }
    }

    /// QA renders and tests: rows given instead of read.
    pub fn set_items(&mut self, mut items: Vec<HistoryItem>) {
        items.reverse();
        self.items = items;
        self.loaded = true;
    }

    pub fn find(&self, id: i64) -> Option<&HistoryItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn find_mut(&mut self, id: i64) -> Option<&mut HistoryItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    /// `_historyManager.Edit(item)`, the Upload tab told.
    pub fn save_item(&mut self, state: &mut AppState, id: i64) {
        let (Some(store), Some(item)) = (self.store.clone(), self.find(id).cloned()) else {
            return;
        };
        if let Err(error) = store.edit(&item) {
            log::warn!("The history item could not be saved: {error}");
        }
        state.refresh_upload_history();
    }
}

/// A window with history rows.
pub trait HistoryHost: super::ToolWindow {
    fn data(&mut self) -> &mut HistoryData;
    /// The rows changed (loaded, imported, deleted): filter again.
    fn refilter(&mut self, state: &mut AppState);
    /// Rows were edited in place (favourite, tag, item, rename: `RefreshVisibleItem`);
    /// `favorite` when it was the favourite that changed.
    fn edited(&mut self, state: &mut AppState, favorite: bool) {
        let _ = favorite;
        self.refilter(state);
    }
    /// The selected rows, in the window's order.
    fn selected_ids(&self) -> Vec<i64>;
    /// The rows as the window lists them now (for the viewer's neighbours).
    fn listed_ids(&self) -> Vec<i64>;
    fn key(&self) -> u64;
}

/// `RefreshHistoryAsync`: History.db read off the UI thread.
pub fn refresh<W: HistoryHost>(window: &mut W, state: &mut AppState) {
    let key = window.key();
    let data = window.data();
    data.load_version += 1;
    let version = data.load_version;
    let Some(store) = data.store.clone() else {
        data.loaded = true;
        window.refilter(state);
        return;
    };
    state.rt.spawn_blocking(move || {
        let result = store.load();
        move |state: &mut AppState, _ctx: &egui::Context| {
            super::with_window::<W, _>(state, key, |window, state| {
                let data = window.data();
                if data.load_version != version {
                    return;
                }
                match result {
                    Ok(mut items) => {
                        items.reverse();
                        data.items = items;
                        data.load_error = None;
                    }
                    Err(error) => {
                        log::warn!("The history could not be read: {error}");
                        data.load_error = Some(error.to_string());
                    }
                }
                data.loaded = true;
                window.refilter(state);
            });
        }
    });
}

fn exists(path: &str) -> bool {
    !path.trim().is_empty() && Path::new(path).is_file()
}

/// Run a menu entry or key on the window's selection.
pub fn run<W: HistoryHost>(window: &mut W, state: &mut AppState, act: Act) {
    let ids = window.selected_ids();
    let image_history = window.data().image_history;
    let items: Vec<HistoryItem> = ids
        .iter()
        .filter_map(|id| window.data().find(*id).cloned())
        .collect();
    let Some(primary) = items.first().cloned() else {
        return;
    };
    let path = primary.file_path.clone();
    let settings = state.default_task_settings();
    match act {
        Act::OpenUrl => state.open_url(primary.url.trim()),
        Act::OpenShortUrl => state.open_url(primary.shortened_url.trim()),
        Act::OpenThumbnailUrl => state.open_url(primary.thumbnail_url.trim()),
        Act::OpenDeletionUrl => state.open_url(primary.deletion_url.trim()),
        Act::OpenFile => {
            if exists(&path) {
                state.effects.push(Effect::OpenFile(PathBuf::from(&path)));
            }
        }
        Act::OpenFolder => {
            if exists(&path) {
                state.effects.push(Effect::Reveal(PathBuf::from(&path)));
            }
        }
        Act::Copy(format) => {
            let refs: Vec<&HistoryItem> = items.iter().collect();
            if let Some(text) = logic::copy_text(&refs, format, image_history) {
                state.effects.push(Effect::CopyText(text));
            }
        }
        Act::CopyFile => {
            let files: Vec<PathBuf> = items
                .iter()
                .filter(|i| exists(&i.file_path))
                .map(|i| PathBuf::from(&i.file_path))
                .collect();
            if !files.is_empty() {
                state.effects.push(Effect::CopyFiles(files));
            }
        }
        Act::CopyImage => {
            if exists(&path)
                && file_helpers::is_image_file(&path)
                && let Some(image) = super::files::load_image(Path::new(&path))
            {
                state.effects.push(Effect::CopyImage(Arc::new(image)));
            }
        }
        Act::CopyText => {
            if exists(&path)
                && file_helpers::is_text_file(&path)
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                state.effects.push(Effect::CopyText(text));
            }
        }
        Act::Favorite => {
            for id in &ids {
                if let Some(item) = window.data().find_mut(*id) {
                    let value = !item.favorite();
                    item.set_favorite(value);
                }
                window.data().save_item(state, *id);
            }
            window.edited(state, true);
        }
        Act::EditTag => {
            let (title, message) = if image_history {
                (
                    hl::ImageHistoryWindow_EditTag,
                    hl::ImageHistoryWindow_EnterTagForHistoryItem,
                )
            } else {
                (
                    hl::HistoryWindow_EditTag,
                    hl::HistoryWindow_EnterTagForHistoryItem,
                )
            };
            window.data().modal = Some(Modal::Prompt(Prompt {
                title: t(title).into(),
                message: t(message).into(),
                confirm: t(hl::HistoryWindows_Save).into(),
                input: Some(primary.tag().unwrap_or("").to_string()),
                action: PromptAction::Tag(primary.id),
            }));
        }
        Act::EditItem => {
            window.data().modal = Some(Modal::Edit(Box::new(EditForm {
                id: primary.id,
                file_name: primary.file_name.clone(),
                file_path: primary.file_path.clone(),
                date: logic::date_long_text(&primary),
                kind: primary.kind.clone(),
                host: primary.host.clone(),
                url: primary.url.clone(),
                thumbnail_url: primary.thumbnail_url.clone(),
                deletion_url: primary.deletion_url.clone(),
                shortened_url: primary.shortened_url.clone(),
                tags: logic::tags_text(&primary),
            })));
        }
        Act::Rename => {
            if path.trim().is_empty() {
                return;
            }
            let (title, message, confirm) = if image_history {
                (
                    hl::ImageHistoryWindow_RenameFile,
                    hl::ImageHistoryWindow_EnterNewFileName,
                    hl::ImageHistoryWindow_Rename,
                )
            } else {
                (
                    hl::HistoryWindow_RenameFile,
                    hl::HistoryWindow_EnterNewFileName,
                    hl::HistoryWindow_Rename,
                )
            };
            let old = Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            window.data().modal = Some(Modal::Prompt(Prompt {
                title: t(title).into(),
                message: t(message).into(),
                confirm: t(confirm).into(),
                input: Some(old),
                action: PromptAction::Rename(primary.id),
            }));
        }
        Act::Delete | Act::DeleteFile => {
            let files = act == Act::DeleteFile;
            let (title, message) = logic::delete_question(ids.len(), files, image_history);
            window.data().modal = Some(Modal::Prompt(Prompt {
                title,
                message,
                confirm: t(if image_history {
                    hl::ImageHistoryWindow_Delete
                } else {
                    hl::HistoryWindow_Delete
                })
                .into(),
                input: None,
                action: PromptAction::Delete { ids, files },
            }));
        }
        Act::ShowImage => show_image(window, state, &primary),
        Act::OpenItem => {
            if !exists(&path) {
                return;
            }
            if file_helpers::is_image_file(&path) {
                show_image(window, state, &primary);
            } else if file_helpers::is_text_file(&path) || file_helpers::is_video_file(&path) {
                state.effects.push(Effect::OpenFile(PathBuf::from(&path)));
            } else {
                window.data().modal = Some(Modal::Prompt(Prompt {
                    title: t(hl::ImageHistoryWindow_OpenFile).into(),
                    message: format(
                        t(hl::ImageHistoryWindow_ConfirmOpenFileFormat),
                        &[Arg::from(path.as_str())],
                    ),
                    confirm: t(hl::HistoryWindows_Open).into(),
                    input: None,
                    action: PromptAction::OpenFile(path.clone()),
                }));
            }
        }
        Act::Upload => {
            if exists(&path) {
                state.upload_files_with(vec![PathBuf::from(&path)], settings);
            }
        }
        Act::EditImage | Act::Pin | Act::Analyze => {
            if !exists(&path) || !file_helpers::is_image_file(&path) {
                return;
            }
            let file = PathBuf::from(&path);
            match act {
                Act::EditImage => state.annotate_file(&file),
                Act::Pin => state.pin_file(&file, &settings),
                _ => state.open_analyze_image(Some(file), settings),
            }
        }
    }
}

/// `ShowSelectedImage`: the picture and up to 100 neighbours each side in the viewer.
fn show_image<W: HistoryHost>(window: &mut W, state: &mut AppState, item: &HistoryItem) {
    if !exists(&item.file_path) || !file_helpers::is_image_file(&item.file_path) {
        return;
    }
    let listed: Vec<HistoryItem> = window
        .listed_ids()
        .iter()
        .filter_map(|id| window.data().find(*id).cloned())
        .collect();
    let refs: Vec<&HistoryItem> = listed.iter().collect();
    let current = listed.iter().position(|i| i.id == item.id).unwrap_or(0);
    let (files, selected) = logic::viewer_files(&refs, current);
    let files: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    if files.is_empty() {
        return;
    }
    state.open_image_viewer_files(&files, selected);
}

/// The prompt's confirm button.
fn confirm<W: HistoryHost>(window: &mut W, state: &mut AppState, prompt: Prompt) {
    match prompt.action {
        PromptAction::Tag(id) => {
            let value = prompt.input.unwrap_or_default();
            if let Some(item) = window.data().find_mut(id) {
                item.set_tag(&value);
            }
            window.data().save_item(state, id);
            window.edited(state, false);
        }
        PromptAction::Rename(id) => {
            let Some(item) = window.data().find(id).cloned() else {
                return;
            };
            let Some((name, new_path)) =
                logic::renamed(&item.file_path, &prompt.input.unwrap_or_default())
            else {
                return;
            };
            // `FileHelpers.RenameFile`: the path changes only when the file moved. Its
            // `File.Move` refuses a name another file has; `rename` would replace that
            // file, so a taken name is refused here first. A failure is said, as upstream.
            let moved = Path::new(&item.file_path).is_file() && {
                let result = if std::fs::symlink_metadata(&new_path).is_ok() {
                    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
                } else {
                    std::fs::rename(&item.file_path, &new_path)
                };
                if let Err(error) = &result {
                    state.uploads.windows.messages.push_back(
                        crate::dialogs::message::MessageBox::new(
                            &format!("Modlight - {}", t(hp::Error)),
                            &format!("{}\n{error}", t(hp::FileHelpers_Rename_file_error)),
                            modlight_core::tasks::host::MessageIcon::Error,
                        ),
                    );
                }
                result.is_ok()
            };
            if let Some(entry) = window.data().find_mut(id) {
                entry.file_name = name;
                if moved {
                    entry.file_path = new_path;
                }
            }
            window.data().save_item(state, id);
            window.edited(state, false);
        }
        PromptAction::Delete { ids, files } => {
            let items: Vec<HistoryItem> = ids
                .iter()
                .filter_map(|id| window.data().find(*id).cloned())
                .collect();
            if files {
                for item in &items {
                    if exists(&item.file_path)
                        && let Err(error) = std::fs::remove_file(&item.file_path)
                    {
                        log::warn!("The file could not be deleted: {error}");
                    }
                }
            }
            if let Some(store) = window.data().store.clone()
                && let Err(error) = store.delete(&items)
            {
                log::warn!("The history items could not be deleted: {error}");
            }
            window.data().items.retain(|i| !ids.contains(&i.id));
            state.refresh_upload_history();
            window.refilter(state);
        }
        PromptAction::OpenFile(path) => state.effects.push(Effect::OpenFile(PathBuf::from(path))),
    }
}

/// `OnSaveEditClick`.
fn save_edit<W: HistoryHost>(window: &mut W, state: &mut AppState, form: &EditForm) {
    if let Some(item) = window.data().find_mut(form.id) {
        item.file_name = form.file_name.clone();
        item.file_path = form.file_path.clone();
        item.kind = form.kind.clone();
        item.host = form.host.clone();
        item.url = form.url.clone();
        item.thumbnail_url = form.thumbnail_url.clone();
        item.deletion_url = form.deletion_url.clone();
        item.shortened_url = form.shortened_url.clone();
        item.tags = Some(logic::parse_tags(&form.tags));
    }
    window.data().save_item(state, form.id);
    window.edited(state, false);
}

/// `OnImportConfirmClick`: the files read and appended off the UI thread.
fn import<W: HistoryHost>(window: &mut W, state: &mut AppState) {
    let image_history = window.data().image_history;
    let Some(Modal::Import(form)) = window.data().modal.as_mut() else {
        return;
    };
    let folder = PathBuf::from(form.folder.trim());
    if !folder.is_dir() {
        form.status = t(if image_history {
            hl::ImageHistoryWindow_SelectedFolderDoesNotExist
        } else {
            hl::HistoryWindow_SelectedFolderDoesNotExist
        })
        .into();
        return;
    }
    form.busy = true;
    form.status = t(if image_history {
        hl::ImageHistoryWindow_ImportingFiles
    } else {
        hl::HistoryWindow_ImportingFiles
    })
    .into();
    let images_only = form.images_only;
    let existing = if form.skip_duplicates {
        logic::existing_paths(&window.data().items)
    } else {
        Default::default()
    };
    let store = window.data().store.clone();
    let key = window.key();
    state.rt.spawn_blocking(move || {
        let mut items = logic::import_folder(&folder, images_only, &existing);
        let result = match &store {
            Some(store) if !items.is_empty() => store.append(&mut items).map_err(|e| e.to_string()),
            _ => Ok(items.len()),
        };
        let count = items.len();
        move |state: &mut AppState, _ctx: &egui::Context| {
            super::with_window::<W, _>(state, key, |window, state| {
                if let Some(Modal::Import(form)) = window.data().modal.as_mut() {
                    form.busy = false;
                    form.status = match &result {
                        Ok(_) => format(
                            t(if image_history {
                                hl::ImageHistoryWindow_SuccessfullyImportedFilesFormat
                            } else {
                                hl::HistoryWindow_SuccessfullyImportedFilesFormat
                            }),
                            &[Arg::Int(count as i64)],
                        ),
                        Err(error) => error.clone(),
                    };
                }
                if result.is_ok() && count > 0 {
                    state.refresh_upload_history();
                    refresh(window, state);
                }
            });
        }
    });
}

/// The context menu (`ListBox.ContextMenu` of the list window).
pub fn list_menu(selected: &[&HistoryItem], services: bool) -> (Vec<Entry>, Vec<Act>) {
    let mut b = MenuBuilder::default();
    let Some(m) = MenuState::of(selected) else {
        return (Vec::new(), Vec::new());
    };
    let single = m.single;
    let copy = |b: &mut MenuBuilder, label: &str, format: CopyFormat| {
        b.item(label, None, Act::Copy(format), m.copy_enabled(format))
    };
    let open = vec![
        b.item(
            t(hl::HistoryWindows_URL),
            None,
            Act::OpenUrl,
            single && m.url,
        ),
        b.item(
            t(hl::HistoryWindows_Shortened_URL),
            None,
            Act::OpenShortUrl,
            single && m.short_url,
        ),
        b.item(
            t(hl::HistoryWindows_Thumbnail_URL),
            None,
            Act::OpenThumbnailUrl,
            single && m.thumbnail_url,
        ),
        b.item(
            t(hl::HistoryWindows_Deletion_URL),
            None,
            Act::OpenDeletionUrl,
            single && m.deletion_url,
        ),
        Entry::Separator,
        b.item(
            t(hl::HistoryWindows_File),
            None,
            Act::OpenFile,
            single && m.file,
        ),
        b.item(
            t(hl::HistoryWindows_Folder),
            None,
            Act::OpenFolder,
            single && m.file,
        ),
    ];
    let copies = vec![
        copy(&mut b, t(hl::HistoryWindows_URL), CopyFormat::Url),
        copy(
            &mut b,
            t(hl::HistoryWindows_Shortened_URL),
            CopyFormat::ShortUrl,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_Thumbnail_URL),
            CopyFormat::ThumbnailUrl,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_Deletion_URL),
            CopyFormat::DeletionUrl,
        ),
        Entry::Separator,
        b.item(
            t(hl::HistoryWindows_File),
            None,
            Act::CopyFile,
            !single || m.file,
        ),
        b.item(
            t(hl::HistoryWindows_Image),
            None,
            Act::CopyImage,
            single && m.image_file,
        ),
        b.item(
            t(hl::HistoryWindows_Text),
            None,
            Act::CopyText,
            single && m.text_file,
        ),
        Entry::Separator,
        copy(
            &mut b,
            t(hl::HistoryWindows_HTML_link),
            CopyFormat::HtmlLink,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_HTML_image),
            CopyFormat::HtmlImage,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_HTML_linked_image),
            CopyFormat::HtmlLinkedImage,
        ),
        Entry::Separator,
        copy(
            &mut b,
            t(hl::HistoryWindows_Forum_BBCode_link),
            CopyFormat::ForumLink,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_Forum_BBCode_image),
            CopyFormat::ForumImage,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_Forum_BBCode_linked_image),
            CopyFormat::ForumLinkedImage,
        ),
        Entry::Separator,
        copy(
            &mut b,
            t(hl::HistoryWindows_Markdown_link),
            CopyFormat::MarkdownLink,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_Markdown_image),
            CopyFormat::MarkdownImage,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_Markdown_linked_image),
            CopyFormat::MarkdownLinkedImage,
        ),
        Entry::Separator,
        copy(
            &mut b,
            t(hl::HistoryWindows_File_path),
            CopyFormat::FilePath,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_File_name),
            CopyFormat::FileName,
        ),
        copy(
            &mut b,
            t(hl::HistoryWindows_File_name_with_extension),
            CopyFormat::FileNameWithExtension,
        ),
        copy(&mut b, t(hl::HistoryWindows_Folder), CopyFormat::Folder),
    ];
    let mut entries = vec![
        Entry::submenu(
            t(hl::HistoryWindows_Open),
            icons::EXTERNAL_LINK,
            single,
            open,
        ),
        Entry::submenu(t(hl::HistoryWindows_Copy), icons::COPY, true, copies),
        Entry::Separator,
    ];
    let favorite = if m.favorite {
        t(hl::HistoryWindow_Unfavorite)
    } else {
        t(hl::HistoryWindows_Favorite)
    };
    entries.push(b.item(favorite, Some(icons::STAR), Act::Favorite, true));
    entries.push(b.item(
        t(hl::HistoryWindows_Edit_tag),
        Some(icons::TAG),
        Act::EditTag,
        single,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Edit_item),
        Some(icons::SQUARE_PEN),
        Act::EditItem,
        single,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Rename_file),
        Some(icons::FILE_PEN_LINE),
        Act::Rename,
        single && m.valid_path,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Delete_from_history),
        Some(icons::LIST_X),
        Act::Delete,
        true,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Delete_file_and_history_item),
        Some(icons::FILE_X),
        Act::DeleteFile,
        true,
    ));
    entries.push(Entry::Separator);
    entries.push(b.item(
        t(hl::HistoryWindows_Show_image_preview),
        Some(icons::EYE),
        Act::ShowImage,
        single && m.image_file,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Upload_file),
        Some(icons::UPLOAD),
        Act::Upload,
        single && m.file && services,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Edit_image),
        Some(icons::WAND_SPARKLES),
        Act::EditImage,
        single && m.image_file && services,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Pin_to_screen),
        Some(icons::PIN),
        Act::Pin,
        single && m.image_file && services,
    ));
    entries.push(b.item(
        t(hl::HistoryWindows_Analyze_image),
        Some(icons::SCAN_SEARCH),
        Act::Analyze,
        single && m.image_file && services,
    ));
    (entries, b.acts)
}

/// The image history's card menu (every entry always on, as upstream).
pub fn image_menu() -> (Vec<Entry>, Vec<Act>) {
    let mut b = MenuBuilder::default();
    let first = b.item(
        t(hl::HistoryWindows_Open_image),
        Some(icons::IMAGE),
        Act::ShowImage,
        true,
    );
    let open = vec![
        b.item(t(hl::HistoryWindows_URL), None, Act::OpenUrl, true),
        b.item(
            t(hl::HistoryWindows_Shortened_URL),
            None,
            Act::OpenShortUrl,
            true,
        ),
        b.item(
            t(hl::HistoryWindows_Thumbnail_URL),
            None,
            Act::OpenThumbnailUrl,
            true,
        ),
        b.item(
            t(hl::HistoryWindows_Deletion_URL),
            None,
            Act::OpenDeletionUrl,
            true,
        ),
        Entry::Separator,
        b.item(t(hl::HistoryWindows_File), None, Act::OpenFile, true),
        b.item(t(hl::HistoryWindows_Folder), None, Act::OpenFolder, true),
    ];
    let c =
        |b: &mut MenuBuilder, label: &str, f: CopyFormat| b.item(label, None, Act::Copy(f), true);
    let copies = vec![
        c(&mut b, t(hl::HistoryWindows_URL), CopyFormat::Url),
        c(
            &mut b,
            t(hl::HistoryWindows_Shortened_URL),
            CopyFormat::ShortUrl,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Thumbnail_URL),
            CopyFormat::ThumbnailUrl,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Deletion_URL),
            CopyFormat::DeletionUrl,
        ),
        Entry::Separator,
        b.item(t(hl::HistoryWindows_File), None, Act::CopyFile, true),
        b.item(t(hl::HistoryWindows_Image), None, Act::CopyImage, true),
        b.item(t(hl::HistoryWindows_Text), None, Act::CopyText, true),
        Entry::Separator,
        c(
            &mut b,
            t(hl::HistoryWindows_HTML_link),
            CopyFormat::HtmlLink,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_File_path),
            CopyFormat::FilePath,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_File_name),
            CopyFormat::FileName,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_File_name_with_extension),
            CopyFormat::FileNameWithExtension,
        ),
        c(&mut b, t(hl::HistoryWindows_Folder), CopyFormat::Folder),
        Entry::Separator,
        c(
            &mut b,
            t(hl::HistoryWindows_HTML_image),
            CopyFormat::HtmlImage,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_HTML_linked_image),
            CopyFormat::HtmlLinkedImage,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Forum_link),
            CopyFormat::ForumLink,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Forum_image),
            CopyFormat::ForumImage,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Forum_linked_image),
            CopyFormat::ForumLinkedImage,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Markdown_link),
            CopyFormat::MarkdownLink,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Markdown_image),
            CopyFormat::MarkdownImage,
        ),
        c(
            &mut b,
            t(hl::HistoryWindows_Markdown_linked_image),
            CopyFormat::MarkdownLinkedImage,
        ),
    ];
    let mut entries = vec![
        first,
        Entry::submenu(t(hl::HistoryWindows_Open), icons::EXTERNAL_LINK, true, open),
        Entry::Separator,
        Entry::submenu(t(hl::HistoryWindows_Copy), icons::COPY, true, copies),
        Entry::Separator,
    ];
    for (label, icon, act) in [
        (
            hl::HistoryWindows_Toggle_favorite,
            icons::STAR,
            Act::Favorite,
        ),
        (hl::HistoryWindows_Edit_tag, icons::TAG, Act::EditTag),
        (
            hl::HistoryWindows_Edit_item,
            icons::SQUARE_PEN,
            Act::EditItem,
        ),
        (
            hl::HistoryWindows_Rename_file,
            icons::FILE_PEN_LINE,
            Act::Rename,
        ),
        (
            hl::HistoryWindows_Delete_from_history,
            icons::LIST_X,
            Act::Delete,
        ),
        (
            hl::HistoryWindows_Delete_file_and_history_item,
            icons::FILE_X,
            Act::DeleteFile,
        ),
    ] {
        entries.push(b.item(t(label), Some(icon), act, true));
    }
    entries.push(Entry::Separator);
    for (label, icon, act) in [
        (hl::HistoryWindows_Upload_file, icons::UPLOAD, Act::Upload),
        (
            hl::HistoryWindows_Edit_image,
            icons::WAND_SPARKLES,
            Act::EditImage,
        ),
        (hl::HistoryWindows_Pin_to_screen, icons::PIN, Act::Pin),
        (
            hl::HistoryWindows_Analyze_image,
            icons::SCAN_SEARCH,
            Act::Analyze,
        ),
    ] {
        entries.push(b.item(t(label), Some(icon), act, true));
    }
    (entries, b.acts)
}

/// Show a context menu on `response`; the action picked.
pub fn context_menu(
    colors: &Colors,
    response: &egui::Response,
    build: impl FnOnce() -> (Vec<Entry>, Vec<Act>),
) -> Option<Act> {
    let mut acts = Vec::new();
    let picked = crate::menus::context(colors, response, || {
        let (entries, list) = build();
        acts = list;
        entries
    });
    match picked {
        Some(Command::Local(index)) => acts.get(index).copied(),
        _ => None,
    }
}

/// The keys both windows share (`OnHistoryListKeyDown`); Cmd stands for Ctrl.
pub fn key_act(ui: &Ui) -> Option<Act> {
    use egui::Key;
    ui.input(|i| {
        let m = i.modifiers;
        let pressed = |k: Key| i.key_pressed(k);
        // egui-winit hands Cmd+C (and Cmd+Shift+C) over as `Event::Copy`, without a key
        // press of C.
        let copy = pressed(Key::C) || i.events.iter().any(|e| matches!(e, egui::Event::Copy));
        if pressed(Key::Enter) && m.command {
            Some(Act::OpenFile)
        } else if pressed(Key::Enter) && m.shift {
            Some(Act::OpenFolder)
        } else if copy && m.command && m.shift {
            Some(Act::Copy(CopyFormat::FilePath))
        } else if copy && m.command && !m.shift && !m.alt {
            Some(Act::Copy(CopyFormat::Url))
        } else if pressed(Key::C) && m.shift && !m.command {
            Some(Act::CopyFile)
        } else if pressed(Key::C) && m.alt && !m.command {
            Some(Act::CopyImage)
        } else if (pressed(Key::Delete) || pressed(Key::Backspace)) && m.shift {
            Some(Act::DeleteFile)
        } else if pressed(Key::Delete) || (pressed(Key::Backspace) && m.command) {
            Some(Act::Delete)
        } else if pressed(Key::U) && m.command {
            Some(Act::Upload)
        } else if pressed(Key::E) && m.command {
            Some(Act::EditImage)
        } else if pressed(Key::P) && m.command {
            Some(Act::Pin)
        } else {
            None
        }
    })
}

/// Whether a text field has the keyboard (the window's keys stay out of its way).
pub fn typing(ui: &Ui) -> bool {
    ui.ctx().memory(|m| m.focused().is_some())
}

/// The toolbar's search box with its magnifier.
pub fn search_box(ui: &mut Ui, colors: &Colors, rect: Rect, id: Id, text: &mut String) -> bool {
    let changed = kit::text_box_at(
        ui,
        colors,
        Rect::from_min_max(rect.min, pos2(rect.right() - 30.0, rect.bottom())),
        id,
        text,
        t(hl::HistoryWindows_Search_file_names_and_tags),
        true,
        false,
    )
    .changed();
    crate::widgets::paint_icon(
        ui,
        ui.painter(),
        pos2(rect.right() - 12.0 - 7.0, rect.center().y),
        icons::SEARCH,
        14.0,
        colors.text_secondary(),
    );
    changed
}

/// An "active" toolbar toggle (Favorites, Advanced): the accent while on.
pub fn toggle_button(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    icon: char,
    label: &str,
    active: bool,
) -> bool {
    if active {
        kit::primary_at(ui, colors, rect, Some(icon), Some(label), true).clicked()
    } else {
        kit::button_at(ui, colors, rect, Some(icon), Some(label), true).clicked()
    }
}

/// The loading and empty states over the list.
pub fn state_message(ui: &Ui, colors: &Colors, rect: Rect, icon: char, text: &str, accent: bool) {
    let galley = crate::widgets::layout(
        ui,
        crate::widgets::job(text, 13.0, Weight::Normal, colors.text_secondary()),
    );
    let total = 44.0 + 8.0 + galley.size().y;
    let top = rect.center().y - total / 2.0;
    crate::widgets::paint_icon(
        ui,
        ui.painter(),
        pos2(rect.center().x, top + 22.0),
        icon,
        44.0,
        if accent {
            colors.accent_start()
        } else {
            colors.text_secondary()
        },
    );
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, top + 52.0),
        galley,
        colors.text_secondary(),
    );
}

/// The dialogs shared by both windows; the settings dialog is the window's own
/// (`draw_settings`). Returns true while a dialog is open.
pub fn modals<W: HistoryHost>(
    window: &mut W,
    ui: &mut Ui,
    state: &mut AppState,
    draw_settings: impl FnOnce(&mut W, &mut Ui, &mut AppState, Rect),
) -> bool {
    let Some(modal) = window.data().modal.clone() else {
        return false;
    };
    let colors = state.colors.clone();
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, colors.overlay_modal());
    let _ = ui.interact(
        full,
        Id::new(("history-modal", window.key())),
        Sense::click(),
    );
    let key = window.key();
    let image_history = window.data().image_history;
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    match modal {
        Modal::Stats(text) => {
            let card = Rect::from_center_size(
                full.center(),
                vec2(
                    640.0_f32.min(full.width() - 32.0),
                    540.0_f32.min(full.height() - 32.0),
                ),
            );
            kit::panel(ui, &colors, card);
            let rows = kit::rows(
                card.shrink(18.0),
                &[
                    Track::Px(kit::HEADING_HEIGHT),
                    Track::Star(1.0),
                    Track::Px(40.0),
                ],
                12.0,
            );
            kit::heading(
                ui,
                &colors,
                rows[0],
                t(hl::HistoryWindows_History_statistics),
            );
            let mut shown = text.clone();
            kit::text_area_at(
                ui,
                &colors,
                rows[1],
                Id::new(("history-stats", key)),
                &mut shown,
                "",
                true,
                true,
                false,
            );
            let r = kit::right_aligned(
                ui,
                &colors,
                rows[2],
                &[(None, Some(t(hl::HistoryWindows_Close)))],
                8.0,
            )[0];
            if kit::button_at(
                ui,
                &colors,
                r,
                None,
                Some(t(hl::HistoryWindows_Close)),
                true,
            )
            .clicked()
            {
                close = true;
            }
        }
        Modal::Import(mut form) => {
            let card = Rect::from_center_size(
                full.center(),
                vec2(
                    540.0,
                    18.0 * 2.0 + 20.0 + 40.0 * 2.0 + 24.0 * 2.0 + 8.0 + 20.0 + 12.0 * 4.0,
                ),
            );
            kit::panel(ui, &colors, card);
            let inner = card.shrink(18.0);
            let mut y = inner.top();
            kit::heading(
                ui,
                &colors,
                Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 20.0)),
                t(hl::HistoryWindows_Import_folder),
            );
            y += 20.0 + 12.0;
            let browse = t(hl::HistoryWindows_Browse);
            let c = kit::columns(
                Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 40.0)),
                &[
                    Track::Star(1.0),
                    Track::Px(kit::button_width(ui, &colors, None, Some(browse))),
                ],
                8.0,
            );
            let mut changed = kit::text_box_at(
                ui,
                &colors,
                c[0],
                Id::new(("history-import-folder", key)),
                &mut form.folder,
                t(hl::HistoryWindows_Folder_path),
                true,
                false,
            )
            .changed();
            if kit::button_at(ui, &colors, c[1], None, Some(browse), true).clicked()
                && let Some(folder) = state.tool_pick_folder(
                    t(if image_history {
                        hl::ImageHistoryWindow_SelectFolderToImport
                    } else {
                        hl::HistoryWindow_SelectFolderToImport
                    }),
                    None,
                )
            {
                form.folder = folder.to_string_lossy().into_owned();
                changed = true;
            }
            y += 40.0 + 12.0;
            changed |= kit::check_normal_at(
                ui,
                &colors,
                Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 24.0)),
                &mut form.images_only,
                t(hl::HistoryWindows_Only_import_image_files),
                true,
            );
            y += 24.0 + 8.0;
            changed |= kit::check_normal_at(
                ui,
                &colors,
                Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 24.0)),
                &mut form.skip_duplicates,
                t(hl::HistoryWindows_Skip_duplicate_files),
                true,
            );
            y += 24.0 + 12.0;
            kit::text_at(
                ui,
                Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 20.0)),
                Align2::LEFT_CENTER,
                &form.status,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
            );
            let items = [
                (None, Some(t(hl::HistoryWindows_Import))),
                (None, Some(t(hl::HistoryWindows_Cancel))),
            ];
            let rects = kit::right_aligned(
                ui,
                &colors,
                Rect::from_min_max(pos2(inner.left(), inner.bottom() - 40.0), inner.max),
                &items,
                8.0,
            );
            let can_import = !form.busy && !form.folder.trim().is_empty();
            let import_clicked =
                kit::button_at(ui, &colors, rects[0], None, items[0].1, can_import).clicked();
            if kit::button_at(ui, &colors, rects[1], None, items[1].1, true).clicked() {
                close = true;
            }
            if changed || import_clicked {
                window.data().modal = Some(Modal::Import(form));
            }
            if import_clicked {
                import(window, state);
            }
        }
        Modal::Settings => {
            let card = full;
            draw_settings(window, ui, state, card);
        }
        Modal::Edit(mut form) => {
            let height =
                (18.0_f32 * 2.0 + 20.0 + 12.0 * 2.0 + 40.0 + 9.0 * 40.0 + 110.0 + 9.0 * 8.0)
                    .min(650.0)
                    .min(full.height() - 32.0);
            let card = Rect::from_center_size(
                full.center(),
                vec2(700.0_f32.min(full.width() - 32.0), height),
            );
            kit::panel(ui, &colors, card);
            let rows = kit::rows(
                card.shrink(18.0),
                &[Track::Px(20.0), Track::Star(1.0), Track::Px(40.0)],
                12.0,
            );
            kit::heading(
                ui,
                &colors,
                rows[0],
                t(hl::HistoryWindows_Edit_history_item),
            );
            let mut changed = false;
            let mut area = kit::column(ui, rows[1]);
            area.set_clip_rect(rows[1].intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt(("history-edit", key))
                .auto_shrink([false, false])
                .show(&mut area, |ui| {
                    let width = rows[1].width() - 8.0;
                    let total = 9.0 * 40.0 + 110.0 + 9.0 * 8.0;
                    let (space, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
                    let fields: [(&str, &mut String, bool); 9] = [
                        (t(hl::HistoryWindows_File_name), &mut form.file_name, true),
                        (t(hl::HistoryWindows_File_path), &mut form.file_path, true),
                        (t(hl::HistoryWindows_Date), &mut form.date, false),
                        (t(hl::HistoryWindows_Type), &mut form.kind, true),
                        (t(hl::HistoryWindows_Host), &mut form.host, true),
                        (t(hl::HistoryWindows_URL), &mut form.url, true),
                        (
                            t(hl::HistoryWindows_Thumbnail_URL),
                            &mut form.thumbnail_url,
                            true,
                        ),
                        (
                            t(hl::HistoryWindows_Deletion_URL),
                            &mut form.deletion_url,
                            true,
                        ),
                        (
                            t(hl::HistoryWindows_Shortened_URL),
                            &mut form.shortened_url,
                            true,
                        ),
                    ];
                    let mut y = space.top();
                    for (index, (label, value, editable)) in fields.into_iter().enumerate() {
                        let c = kit::columns(
                            Rect::from_min_size(pos2(space.left(), y), vec2(width, 40.0)),
                            &[Track::Px(120.0), Track::Star(1.0)],
                            10.0,
                        );
                        kit::label(ui, &colors, c[0], label);
                        if editable {
                            changed |= kit::text_box_at(
                                ui,
                                &colors,
                                c[1],
                                Id::new(("history-edit-field", key, index)),
                                value,
                                "",
                                true,
                                false,
                            )
                            .changed();
                        } else {
                            kit::read_only_box_at(ui, &colors, c[1], value, false);
                        }
                        y += 48.0;
                    }
                    let c = kit::columns(
                        Rect::from_min_size(pos2(space.left(), y), vec2(width, 110.0)),
                        &[Track::Px(120.0), Track::Star(1.0)],
                        10.0,
                    );
                    kit::text_at(
                        ui,
                        Rect::from_min_size(c[0].min + vec2(0.0, 8.0), vec2(c[0].width(), 20.0)),
                        Align2::LEFT_CENTER,
                        t(hl::HistoryWindows_Tags),
                        13.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                    changed |= kit::text_area_at(
                        ui,
                        &colors,
                        c[1],
                        Id::new(("history-edit-tags", key)),
                        &mut form.tags,
                        t(hl::HistoryWindows_One_key_value_pair_per_line),
                        true,
                        false,
                        false,
                    )
                    .changed();
                });
            let items = [
                (None, Some(t(hl::HistoryWindows_Save))),
                (None, Some(t(hl::HistoryWindows_Cancel))),
            ];
            let rects = kit::right_aligned(ui, &colors, rows[2], &items, 8.0);
            if changed {
                window.data().modal = Some(Modal::Edit(form.clone()));
            }
            if kit::button_at(ui, &colors, rects[0], None, items[0].1, true).clicked() {
                save_edit(window, state, &form);
                close = true;
            }
            if kit::button_at(ui, &colors, rects[1], None, items[1].1, true).clicked() {
                close = true;
            }
        }
        Modal::Prompt(mut prompt) => {
            let message = crate::widgets::layout(
                ui,
                crate::widgets::wrapped(
                    &prompt.message,
                    13.0,
                    Weight::Normal,
                    colors.text(),
                    480.0 - 36.0,
                ),
            );
            let input_h = if prompt.input.is_some() {
                40.0 + 12.0
            } else {
                0.0
            };
            let height = 18.0 * 2.0 + 20.0 + 12.0 + message.size().y + 12.0 + input_h + 40.0;
            let card = Rect::from_center_size(full.center(), vec2(480.0, height));
            kit::panel(ui, &colors, card);
            let inner = card.shrink(18.0);
            kit::heading(
                ui,
                &colors,
                Rect::from_min_size(inner.min, vec2(inner.width(), 20.0)),
                &prompt.title,
            );
            let mut y = inner.top() + 20.0 + 12.0;
            ui.painter()
                .galley(pos2(inner.left(), y), message.clone(), colors.text());
            y += message.size().y + 12.0;
            let mut submit = false;
            if let Some(input) = prompt.input.as_mut() {
                let id = Id::new(("history-prompt", key));
                let response = kit::text_box_at(
                    ui,
                    &colors,
                    Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 40.0)),
                    id,
                    input,
                    "",
                    true,
                    false,
                );
                if !ui.memory(|m| m.has_focus(id)) && !response.lost_focus() {
                    response.request_focus();
                }
                submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if response.changed() {
                    window.data().modal = Some(Modal::Prompt(prompt.clone()));
                }
            }
            let items = [
                (None, Some(prompt.confirm.as_str())),
                (None, Some(t(hl::HistoryWindows_Cancel))),
            ];
            let rects = kit::right_aligned(
                ui,
                &colors,
                Rect::from_min_max(pos2(inner.left(), inner.bottom() - 40.0), inner.max),
                &items,
                8.0,
            );
            if kit::button_at(ui, &colors, rects[0], None, items[0].1, true).clicked() || submit {
                window.data().modal = None;
                confirm(window, state, prompt.clone());
                return true;
            }
            if kit::button_at(ui, &colors, rects[1], None, items[1].1, true).clicked() {
                close = true;
            }
        }
    }
    if close {
        window.data().modal = None;
    }
    true
}

/// `OnStatsClick`: the statistics worked out off the UI thread.
pub fn open_stats<W: HistoryHost>(window: &mut W, state: &mut AppState) {
    let image_history = window.data().image_history;
    window.data().modal = Some(Modal::Stats(
        t(if image_history {
            hl::ImageHistoryWindow_CalculatingStatistics
        } else {
            hl::HistoryWindow_CalculatingStatistics
        })
        .into(),
    ));
    let items = window.data().items.clone();
    let key = window.key();
    state.rt.spawn_blocking(move || {
        let text = logic::stats(&items);
        move |state: &mut AppState, _ctx: &egui::Context| {
            super::with_window::<W, _>(state, key, |window, _| {
                if let Some(Modal::Stats(shown)) = window.data().modal.as_mut() {
                    *shown = text;
                }
            });
        }
    });
}

/// `OnImportClick`.
pub fn open_import<W: HistoryHost>(window: &mut W, images_only: bool) {
    window.data().modal = Some(Modal::Import(ImportForm {
        images_only,
        skip_duplicates: true,
        ..ImportForm::default()
    }));
}

/// The remembered window size and place ("W, H" and "X, Y" as Newtonsoft writes them).
pub fn parse_pair(text: Option<&str>) -> Option<(f32, f32)> {
    let text = text?;
    let mut parts = text.split(',').map(|p| p.trim().parse::<f32>());
    let a = parts.next()?.ok()?;
    let b = parts.next()?.ok()?;
    (a != 0.0 || b != 0.0).then_some((a, b))
}

/// The window's spec with the remembered size and place.
pub fn remembered_spec(
    title: String,
    size: egui::Vec2,
    min: egui::Vec2,
    remember: bool,
    state: &modlight_tools::settings::WindowState,
) -> super::WindowSpec {
    let mut spec = super::WindowSpec::resizable(&title, size, min);
    if remember {
        if let Some((w, h)) = parse_pair(state.size.as_deref()) {
            spec.size = vec2(w.max(min.x), h.max(min.y));
        }
        if let Some((x, y)) = parse_pair(state.location.as_deref()) {
            spec.position = Some(pos2(x, y));
        }
    }
    spec
}

/// Keep the window's current size and place for the next time.
pub fn track_window(ui: &Ui, state: &mut modlight_tools::settings::WindowState, remember: bool) {
    if !remember {
        return;
    }
    let (outer, inner, maximized) = ui.ctx().input(|i| {
        (
            i.viewport().outer_rect,
            i.viewport().inner_rect,
            i.viewport().maximized.unwrap_or(false),
        )
    });
    state.is_maximized = maximized;
    if maximized {
        return;
    }
    if let Some(inner) = inner {
        state.size = Some(format!(
            "{}, {}",
            inner.width().round(),
            inner.height().round()
        ));
    }
    if let Some(outer) = outer {
        state.location = Some(format!("{}, {}", outer.left().round(), outer.top().round()));
    }
}

impl AppState {
    /// History.db, when the app opened it.
    pub fn history_store(&self) -> Option<Arc<HistoryStore>> {
        self.uploads.tasks.history.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs() {
        assert_eq!(parse_pair(Some("1200, 700")), Some((1200.0, 700.0)));
        assert_eq!(parse_pair(Some("0, 0")), None);
        assert_eq!(parse_pair(Some("x")), None);
        assert_eq!(parse_pair(None), None);
    }

    fn rename_prompt(id: i64, name: &str) -> Prompt {
        Prompt {
            title: String::new(),
            message: String::new(),
            confirm: String::new(),
            input: Some(name.into()),
            action: PromptAction::Rename(id),
        }
    }

    /// `FileHelpers.RenameFile` moves with `File.Move`, which refuses a name that is
    /// taken: the other file stays, the item keeps its path, and the error is said.
    #[test]
    fn renaming_never_replaces_another_file() {
        let mut state = crate::qa::state(&crate::qa::Look::default(), false);
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        std::fs::write(&a, "first").unwrap();
        std::fs::write(&b, "second").unwrap();
        let mut window = super::super::history_list::HistoryWindow::new(
            1,
            None,
            modlight_tools::settings::HistorySettings::default(),
        );
        window.qa_load(
            &mut state,
            vec![HistoryItem {
                id: 7,
                file_name: "a.png".into(),
                file_path: a.to_string_lossy().into_owned(),
                ..HistoryItem::default()
            }],
        );
        let messages = state.uploads.windows.messages.len();
        confirm(&mut window, &mut state, rename_prompt(7, "b"));
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "second");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "first");
        assert_eq!(
            window.data.find(7).unwrap().file_path,
            a.to_string_lossy().into_owned()
        );
        assert_eq!(state.uploads.windows.messages.len(), messages + 1);

        // A free name moves the file.
        confirm(&mut window, &mut state, rename_prompt(7, "c"));
        let c = dir.path().join("c.png");
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "first");
        assert!(!a.exists());
        let item = window.data.find(7).unwrap();
        assert_eq!(item.file_path, c.to_string_lossy().into_owned());
        assert_eq!(item.file_name, "c.png");
    }

    /// What `key_act` makes of one frame's events.
    fn act_for(modifiers: egui::Modifiers, event: egui::Event) -> Option<Act> {
        let ctx = egui::Context::default();
        let mut act = None;
        let input = egui::RawInput {
            events: vec![egui::Event::ModifiersChanged(modifiers), event],
            ..egui::RawInput::default()
        };
        ctx.run_ui(input, |ui| act = key_act(ui))
            .drop_without_applying_deltas();
        act
    }

    /// `OnHistoryListKeyDown`'s Ctrl+C and Ctrl+Shift+C (Cmd here). egui-winit hands
    /// both to the window as `Event::Copy`, never as a key press of C.
    #[test]
    fn the_copy_keys_arrive_as_copy_events() {
        use egui::Modifiers as M;
        assert_eq!(
            act_for(M::COMMAND, egui::Event::Copy),
            Some(Act::Copy(CopyFormat::Url))
        );
        assert_eq!(
            act_for(M::COMMAND | M::SHIFT, egui::Event::Copy),
            Some(Act::Copy(CopyFormat::FilePath))
        );
        let key = |modifiers| egui::Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        assert_eq!(act_for(M::SHIFT, key(M::SHIFT)), Some(Act::CopyFile));
        assert_eq!(act_for(M::ALT, key(M::ALT)), Some(Act::CopyImage));
    }

    #[test]
    fn menus_map_positions_to_actions() {
        let (entries, acts) = image_menu();
        assert!(!entries.is_empty());
        assert_eq!(acts[0], Act::ShowImage);
        assert!(acts.contains(&Act::Copy(CopyFormat::MarkdownLinkedImage)));
        let item = HistoryItem {
            file_name: "a.png".into(),
            url: "https://x/a.png".into(),
            ..HistoryItem::default()
        };
        let (_, acts) = list_menu(&[&item], true);
        assert!(acts.contains(&Act::Analyze));
        assert!(list_menu(&[], true).0.is_empty());
    }
}

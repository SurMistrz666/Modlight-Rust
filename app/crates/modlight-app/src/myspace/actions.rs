//! What My space does: listing, navigating, and every action on files and folders
//! (`MySpaceViewModel` loading, `MainWindow.MySpace.Actions.cs`, `.Input.cs`,
//! `.Duplicates.cs`). Each network call runs on the runtime and finishes with a
//! completion that checks it still belongs to the listing on screen.

use std::path::PathBuf;

use modlight_api::{
    ApiError, Credential, MAX_DELETE_IDS, UploadEdit, UploadQuery, escape_data_string,
};
use modlight_core::format::cloud_size;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::library::{LIBRARY_VIEWS, LinkFormat, convert, pager};
use modlight_core::messages;

use crate::dialogs::confirm::{Confirm, ConfirmAction, ConfirmOutcome};
use crate::dialogs::duplicates::{Duplicates, DuplicatesAction};
use crate::dialogs::folder_delete::{FolderDelete, FolderDeleteChoice};
use crate::dialogs::link_settings::{LinkSettings, LinkSettingsOutcome};
use crate::dialogs::prompt::{Prompt, PromptOutcome, PromptPurpose};
use crate::state::AppState;

/// Emptying the trash reads it in pages of this size...
const TRASH_SCAN_PAGE: i64 = 120;
/// ...deletes in batches of this size...
const TRASH_DELETE_BATCH: usize = 500;
/// ...and never reads more than this many ids.
const TRASH_SCAN_LIMIT: usize = 50_000;

fn identity_of(credential: &Credential) -> String {
    format!("{}\n{}", credential.host, credential.token)
}

/// What to do with the rows a PATCH answered with.
enum EditDone {
    Moved {
        ids: Vec<String>,
        target: Option<String>,
    },
    Published {
        published: bool,
        count: usize,
    },
    CustomUrl {
        id: String,
    },
    LinkSettings,
}

impl AppState {
    // ------------------------------------------------------------------ listing

    /// The account may have changed (`RefreshAccount`).
    pub fn space_refresh_account(&mut self) {
        let Some(credential) = self.credential() else {
            self.space_signed_out();
            return;
        };
        self.space.has_account = true;
        if identity_of(&credential) != self.space.identity {
            self.space.current_folder = None;
            self.space_reload();
            return;
        }
        if !self.space.loaded_once && !self.space.is_busy {
            self.space_reload();
        }
    }

    /// Start again from the folders down (`Reload`).
    pub fn space_reload(&mut self) {
        let Some(credential) = self.credential() else {
            self.space_signed_out();
            return;
        };
        if !self.space.restored_last_folder {
            self.space.restored_last_folder = true;
            let saved = self.config.last_folder_id();
            self.space.current_folder = (!saved.is_empty()).then_some(saved);
        }
        if !self.space.restored_page_size {
            self.space.restored_page_size = true;
            self.space.page_size = pager::clamp_page_size(self.config.page_size());
            let size = self.space.page_size;
            self.space.pager.update(1, size, 0);
        }
        self.start_listing(credential, true, 1);
    }

    fn space_signed_out(&mut self) {
        self.space.generation += 1;
        self.clear_listing();
        self.space.folders.clear();
        self.space.unfiled = None;
        self.space.selected_folder = None;
        self.space.current_kind = None;
        self.space.trashed = false;
        self.space.current_folder = None;
        self.space.identity.clear();
        self.space.loaded_once = false;
        self.space.total = 0;
        self.space.page = 1;
        self.space.pager.reset();
        self.space.count_text.clear();
        self.space.set_error(false, "");
        self.space.is_busy = false;
        self.space.has_account = false;
        self.details.retire();
    }

    fn clear_listing(&mut self) {
        self.media.clear_grid();
        self.space.clear_files();
        self.visible_range = None;
        self.scroll_to_top = true;
    }

    fn start_listing(&mut self, credential: Credential, reload_folders: bool, target: i64) {
        self.space.generation += 1;
        self.clear_listing();
        self.space.identity = identity_of(&credential);
        self.space.total = 0;
        self.space.page = target.max(1);
        self.space.loaded_once = false;
        self.space.count_text.clear();
        self.space.set_error(false, "");
        self.space.merging = false;
        self.space.merge_queued = false;
        let generation = self.space.generation;
        if reload_folders {
            self.load_folders(credential.clone(), generation);
        }
        let page = self.space.page;
        self.load_page(credential, page, generation);
    }

    fn load_folders(&mut self, credential: Credential, generation: u64) {
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_folders(&credential).await;
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                if state.space.generation != generation {
                    return;
                }
                match answer {
                    Ok(list) => {
                        if state.space.apply_folders(list) {
                            state.remember_folder();
                            if let Some(credential) = state.credential() {
                                state.start_listing(credential, false, 1);
                            }
                        }
                    }
                    Err(error) => log::debug!("Modlight: the folder list did not load: {error}"),
                }
            })
        });
    }

    fn load_page(&mut self, credential: Credential, target: i64, generation: u64) {
        self.space.is_busy = true;
        let query = self.query(target);
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_uploads(&credential, &query).await;
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                if state.space.generation != generation {
                    return;
                }
                match answer {
                    Ok(page) => {
                        let arrived = state.space.apply_page(page, target);
                        state.load_thumbnails(&arrived);
                    }
                    Err(ApiError::Unauthorized) => {
                        state.space.loaded_once = true;
                        state.space.set_error(true, t(s::MySpace_TokenRejected));
                    }
                    Err(error) => {
                        state.space.loaded_once = true;
                        let text = messages::unreachable(&credential.host, &error.to_string());
                        state.space.set_error(true, &text);
                    }
                }
                state.space.is_busy = false;
            })
        });
    }

    fn query(&self, page: i64) -> UploadQuery {
        UploadQuery {
            page,
            per_page: self.space.page_size,
            folder_id: self.space.current_folder.clone(),
            search: None,
            kind: self.space.current_kind.map(str::to_string),
            trashed: self.space.trashed,
        }
    }

    /// The folder tallies and names changed on the server (`RefreshFolders`).
    pub fn refresh_folders(&mut self) {
        let Some(credential) = self.credential() else {
            return;
        };
        if !self.space.has_account || !self.space.loaded_once {
            return;
        }
        let generation = self.space.generation;
        self.load_folders(credential, generation);
    }

    fn remember_folder(&mut self) {
        let current = self.space.current_folder.clone();
        self.config.set_last_folder_id(current.as_deref());
        self.save_config();
    }

    /// A file arrived in `folder_id` (`NoteUploadLanded`).
    pub fn note_upload_landed(&mut self, folder_id: Option<&str>, bytes: i64) {
        if !self.space.has_account {
            return;
        }
        self.space.adjust_folder_tally(folder_id, 1, bytes);
        let now = self.time;
        self.space.flash(folder_id, now);
        self.merge_newest();
    }

    fn merge_newest(&mut self) {
        let Some(credential) = self.credential() else {
            return;
        };
        if !self.space.loaded_once || self.space.trashed || self.space.page != 1 {
            return;
        }
        if self.space.merging {
            self.space.merge_queued = true;
            return;
        }
        self.space.merging = true;
        self.space.merge_queued = false;
        let generation = self.space.generation;
        let query = self.query(1);
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_uploads(&credential, &query).await;
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                if state.space.generation != generation {
                    return;
                }
                state.space.merging = false;
                if let Ok(page) = answer {
                    let arrived = state.space.merge_newest(page);
                    if !arrived.is_empty() {
                        state.load_thumbnails(&arrived);
                    }
                }
                if state.space.merge_queued {
                    state.merge_newest();
                }
            })
        });
    }

    // ------------------------------------------------------------------ getting about

    pub fn space_go_up(&mut self) {
        if self.space.current_folder.is_none() {
            return;
        }
        let parent = self
            .space
            .current_folder_view()
            .and_then(|f| f.parent_id.clone());
        self.space_navigate_to(parent);
    }

    pub fn space_navigate_to(&mut self, folder_id: Option<String>) {
        let folder_id = folder_id.filter(|id| self.space.find(id).is_some());
        if self.space.current_folder == folder_id && self.space.loaded_once && !self.space.has_error
        {
            return;
        }
        let Some(credential) = self.credential() else {
            self.space_signed_out();
            return;
        };
        self.space.current_kind = None;
        self.space.trashed = false;
        self.space.current_folder = folder_id;
        self.remember_folder();
        self.space.select_folder_tile(None);
        self.start_listing(credential, false, 1);
    }

    pub fn space_select_view(&mut self, index: usize) {
        let Some(view) = LIBRARY_VIEWS.get(index) else {
            return;
        };
        let Some(credential) = self.credential() else {
            self.space_signed_out();
            return;
        };
        if self.space.current_folder.is_none()
            && self.space.current_kind == view.kind
            && self.space.trashed == view.is_trash
            && self.space.loaded_once
            && !self.space.has_error
        {
            return;
        }
        self.space.current_kind = view.kind;
        self.space.trashed = view.is_trash;
        self.space.current_folder = None;
        self.remember_folder();
        self.space.select_folder_tile(None);
        self.start_listing(credential, false, 1);
    }

    pub fn space_go_to_page(&mut self, target: i64) {
        let Some(credential) = self.credential() else {
            self.space_signed_out();
            return;
        };
        self.start_listing(credential, false, target);
    }

    pub fn space_set_page_size(&mut self, size: i64) {
        let wanted = pager::clamp_page_size(size);
        if wanted == self.space.page_size {
            return;
        }
        let anchor = pager::anchor(pager::first(self.space.page, self.space.page_size), wanted);
        self.space.page_size = wanted;
        self.save_config();
        let Some(credential) = self.credential() else {
            self.space_signed_out();
            return;
        };
        self.start_listing(credential, false, anchor);
    }

    /// Open or close the library panel; it slides (`StartSidebarSlide`).
    pub fn toggle_library_panel(&mut self) {
        self.library_panel = !self.library_panel;
        self.panel_moved_at = self.time;
        self.save_config();
    }

    // ------------------------------------------------------------------ helpers

    /// The signed-in credential, or the sign-in window when there is none.
    fn credential_or_sign_in(&mut self) -> Option<Credential> {
        let credential = self.credential();
        if credential.is_none() {
            self.show_sign_in();
        }
        credential
    }

    /// A failed action, said in the corner (`ReportFailure`).
    pub fn report_failure(&mut self, error: &ApiError, host: &str) {
        let icon = match error {
            ApiError::Unauthorized => icons::LOG_IN,
            ApiError::Refused { .. } => icons::TRIANGLE_ALERT,
            _ => icons::CLOUD_OFF,
        };
        let (title, detail, _) = messages::library_failure(error, host);
        self.notice(icon, &title, &detail, true);
    }

    fn account_tag(&self) -> String {
        self.account.account().tag.clone()
    }

    fn open_on_server(&mut self, path: &str) {
        let host = self.account.host().to_string();
        if self.credential().is_none() || host.is_empty() {
            self.show_sign_in();
            return;
        }
        let url = format!("{}{}", host.trim_end_matches('/'), path);
        self.open_url(&url);
    }

    fn gallery_path(&self, slug: &str) -> String {
        format!(
            "/a/{}/{}",
            escape_data_string(&self.account_tag()),
            escape_data_string(slug)
        )
    }

    pub fn open_gallery(&mut self, id: &str) {
        let Some(slug) = self.space.find(id).map(|f| f.slug.clone()) else {
            return;
        };
        let path = self.gallery_path(&slug);
        self.open_on_server(&path);
    }

    pub fn copy_gallery_link(&mut self, id: &str) {
        let Some(slug) = self.space.find(id).map(|f| f.slug.clone()) else {
            return;
        };
        let host = self.account.host().to_string();
        if self.credential().is_none() || host.is_empty() {
            self.show_sign_in();
            return;
        }
        let link = format!("{}{}", host.trim_end_matches('/'), self.gallery_path(&slug));
        self.copy_text(&link);
    }

    pub fn show_in_panel(&mut self, id: &str) {
        let path = format!("/dashboard?folder={}", escape_data_string(id));
        self.open_on_server(&path);
    }

    pub fn copy_links(&mut self, ids: &[String]) {
        let text: Vec<String> = ids
            .iter()
            .filter_map(|id| self.space.file(id))
            .filter(|f| f.has_link())
            .map(|f| f.copy_link.clone())
            .collect();
        if !text.is_empty() {
            self.copy_text(&text.join("\n"));
        }
    }

    pub fn copy_as(&mut self, ids: &[String], format: LinkFormat) {
        let text: Vec<String> = ids
            .iter()
            .filter_map(|id| self.space.file(id))
            .filter(|f| f.has_link())
            .map(|f| f.link_format(format))
            .filter(|l| !l.is_empty())
            .collect();
        if !text.is_empty() {
            self.copy_text(&text.join("\n"));
        }
    }

    // ------------------------------------------------------------------ edits

    fn edit_files(&mut self, ids: &[String], edit: UploadEdit, done: EditDone) {
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        let ids: Vec<String> = ids
            .iter()
            .filter(|id| self.space.file(id).is_some_and(|f| f.can_trash()))
            .cloned()
            .collect();
        if ids.is_empty() || edit.is_empty() {
            return;
        }
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.edit_uploads(&credential, &ids, &edit).await;
            Box::new(
                move |state: &mut AppState, _ctx: &egui::Context| match answer {
                    Ok(result) => {
                        let rows = result.items.unwrap_or_default();
                        state.edit_done(done, &rows);
                    }
                    Err(error) => state.report_failure(&error, &credential.host),
                },
            )
        });
    }

    fn edit_done(&mut self, done: EditDone, rows: &[modlight_api::Upload]) {
        match done {
            EditDone::Moved { ids, target } => {
                let count = ids.len();
                let now = self.time;
                self.space
                    .note_files_moved(&ids, target.as_deref(), rows, now);
                let place = match &target {
                    None => t(s::MySpace_Unfiled).to_string(),
                    Some(id) => self.space.folder_name_for(Some(id)),
                };
                let text = format(
                    t(s::MySpace_MovedToast),
                    &[Arg::from(count), Arg::from(place)],
                );
                self.notice(icons::FOLDER_INPUT, &text, "", false);
            }
            EditDone::Published { published, count } => {
                self.space.apply_rows(rows);
                let (icon, key) = if published {
                    (icons::GLOBE, s::MySpace_PublishedToast)
                } else {
                    (icons::GLOBE_LOCK, s::MySpace_HiddenToast)
                };
                let text = format(t(key), &[Arg::from(count)]);
                self.notice(icon, &text, "", false);
            }
            EditDone::CustomUrl { id } => {
                self.space.apply_rows(rows);
                let code = self
                    .space
                    .file(&id)
                    .map(|f| f.code.clone())
                    .unwrap_or_default();
                let text = format(t(s::MySpace_CustomUrlToast), &[Arg::from(code)]);
                self.notice(icons::SQUARE_PEN, &text, "", false);
            }
            EditDone::LinkSettings => {
                self.space.apply_rows(rows);
                self.notice(
                    icons::SETTINGS_2,
                    t(s::MySpace_LinkSettingsToast),
                    "",
                    false,
                );
            }
        }
    }

    pub fn move_files(&mut self, ids: &[String], folder: Option<String>) {
        let targets: Vec<String> = ids
            .iter()
            .filter(|id| self.space.file(id).is_some_and(|f| f.folder_id != folder))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let edit = UploadEdit::new().move_to(folder.as_deref());
        self.edit_files(
            &targets.clone(),
            edit,
            EditDone::Moved {
                ids: targets,
                target: folder,
            },
        );
    }

    pub fn set_published(&mut self, ids: &[String], published: bool) {
        let edit = UploadEdit::new().set_published(published);
        self.edit_files(
            ids,
            edit,
            EditDone::Published {
                published,
                count: ids.len(),
            },
        );
    }

    pub fn ask_custom_url(&mut self, id: &str) {
        let Some(_) = self.credential_or_sign_in() else {
            return;
        };
        let Some(code) = self.space.file(id).map(|f| f.code.clone()) else {
            return;
        };
        let prefix = format!(
            "{}/{}/",
            self.account.host().trim_end_matches('/'),
            self.account_tag()
        );
        self.windows.prompt = Some(Prompt::new(
            t(s::MySpace_CustomUrlTitle),
            t(s::MySpace_CustomUrlDescription),
            t(s::MySpace_CustomUrlLabel),
            t(s::MySpace_Save),
            &code,
            &prefix,
            64,
            PromptPurpose::CustomUrl {
                file: id.to_string(),
                old: code.clone(),
            },
        ));
    }

    pub fn ask_link_settings(&mut self, id: &str) {
        if let Some(file) = self.space.file(id) {
            self.windows.link_settings = Some(LinkSettings::new(file));
        }
    }

    pub fn link_settings_finished(&mut self, outcome: LinkSettingsOutcome) {
        let Some(window) = self.windows.link_settings.take() else {
            return;
        };
        if let LinkSettingsOutcome::Save(edit) = outcome {
            self.edit_files(
                std::slice::from_ref(&window.file_id),
                edit,
                EditDone::LinkSettings,
            );
        }
    }

    // ------------------------------------------------------------------ trash

    /// Trash (or delete for good) after asking (`RemoveAsync`).
    pub fn ask_trash(&mut self, ids: &[String], forever: bool) {
        let Some(_) = self.credential_or_sign_in() else {
            return;
        };
        let targets: Vec<String> = ids
            .iter()
            .filter(|id| self.space.file(id).is_some_and(|f| f.can_trash()))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let question = if ids.len() == 1 {
            let name = self
                .space
                .file(&ids[0])
                .map(|f| f.name.clone())
                .unwrap_or_default();
            let key = if forever {
                s::MySpace_DeleteForeverConfirm
            } else {
                s::MySpace_DeleteConfirm
            };
            format(t(key), &[Arg::from(name)])
        } else {
            let key = if forever {
                s::MySpace_DeleteForeverManyConfirm
            } else {
                s::MySpace_DeleteManyConfirm
            };
            format(t(key), &[Arg::from(ids.len())])
        };
        let (title, action) = if forever {
            (
                t(s::MySpace_DeleteForeverTitle),
                ConfirmAction::DeleteForever(targets),
            )
        } else {
            (t(s::MySpace_DeleteTitle), ConfirmAction::Trash(targets))
        };
        self.windows.confirm = Some(Confirm::new(title, &question, action));
    }

    fn remove_files(&mut self, ids: Vec<String>, mode: &'static str) {
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.delete_uploads(&credential, &ids, mode).await;
            Box::new(
                move |state: &mut AppState, _ctx: &egui::Context| match answer {
                    Ok(result) if result.affected == 0 => {
                        state.notice(
                            icons::TRIANGLE_ALERT,
                            t(s::MySpace_DeleteNothingHappened),
                            "",
                            true,
                        );
                    }
                    Ok(result) => {
                        for id in &ids {
                            if mode == "restore" {
                                state.space.note_file_restored(id);
                            } else {
                                state.space.note_file_removed(id);
                            }
                            state.media.forget(id);
                        }
                        state.refresh_account_strip();
                        let (icon, key) = match mode {
                            "restore" => (icons::UNDO_2, s::MySpace_RestoredToast),
                            "permanent" => (icons::TRASH_2, s::MySpace_DeletedForeverToast),
                            _ => (icons::TRASH_2, s::MySpace_TrashedToast),
                        };
                        let text = format(t(key), &[Arg::Int(result.affected)]);
                        state.notice(icon, &text, "", false);
                    }
                    Err(error) => state.report_failure(&error, &credential.host),
                },
            )
        });
    }

    pub fn restore_files(&mut self, ids: &[String]) {
        let targets: Vec<String> = ids
            .iter()
            .filter(|id| self.space.file(id).is_some_and(|f| f.can_trash()))
            .cloned()
            .collect();
        if targets.is_empty() {
            if self.credential().is_none() {
                self.show_sign_in();
            }
            return;
        }
        self.remove_files(targets, "restore");
    }

    pub fn confirm_finished(&mut self, outcome: ConfirmOutcome) {
        let Some(confirm) = self.windows.confirm.take() else {
            return;
        };
        if outcome != ConfirmOutcome::Yes {
            if matches!(confirm.action, ConfirmAction::EmptyTrash { .. }) {
                self.emptying_trash = false;
            }
            return;
        }
        match confirm.action {
            ConfirmAction::Trash(ids) => self.remove_files(ids, "trash"),
            ConfirmAction::DeleteForever(ids) => self.remove_files(ids, "permanent"),
            ConfirmAction::EmptyTrash { ids, bytes } => self.delete_trash(ids, bytes),
            ConfirmAction::DuplicatesTrash => self.trash_duplicates(),
            ConfirmAction::DeleteLocalFile(path) => {
                if let Err(error) = modlight_macos::system::move_to_trash(&path) {
                    log::warn!("Modlight: {} could not be deleted: {error}", path.display());
                }
            }
        }
    }

    /// Count the trash, then ask (`EmptyTrashAsync`).
    pub fn empty_trash(&mut self) {
        if self.emptying_trash {
            return;
        }
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        self.emptying_trash = true;
        self.notice(icons::TRASH_2, t(s::MySpace_EmptyTrashCounting), "", false);
        let api = self.api.clone();
        self.rt.spawn(async move {
            let mut ids: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut bytes: i64 = 0;
            let mut failure = None;
            let mut page = 1;
            while ids.len() < TRASH_SCAN_LIMIT {
                let query = UploadQuery {
                    page,
                    per_page: TRASH_SCAN_PAGE,
                    folder_id: None,
                    search: None,
                    kind: None,
                    trashed: true,
                };
                let listing = match api.get_uploads(&credential, &query).await {
                    Ok(listing) => listing,
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                };
                let items = listing.items.clone().unwrap_or_default();
                if items.is_empty() {
                    break;
                }
                let mut added = 0;
                for row in &items {
                    let Some(id) = row.id.clone().filter(|id| !id.is_empty()) else {
                        continue;
                    };
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    ids.push(id);
                    bytes += row.size.max(0);
                    added += 1;
                }
                if added == 0 {
                    break;
                }
                let per_page = if listing.per_page > 0 {
                    listing.per_page
                } else {
                    TRASH_SCAN_PAGE
                };
                if (items.len() as i64) < per_page {
                    break;
                }
                page += 1;
            }
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                if let Some(error) = failure {
                    state.emptying_trash = false;
                    state.report_failure(&error, &credential.host);
                    return;
                }
                if ids.is_empty() {
                    state.emptying_trash = false;
                    state.notice(icons::TRASH_2, t(s::MySpace_EmptyTrashEmpty), "", false);
                    return;
                }
                let question = format(
                    t(s::MySpace_EmptyTrashConfirm),
                    &[Arg::from(ids.len()), Arg::from(cloud_size(bytes))],
                );
                state.windows.confirm = Some(Confirm::new(
                    t(s::MySpace_EmptyTrashTitle),
                    &question,
                    ConfirmAction::EmptyTrash { ids, bytes },
                ));
            })
        });
    }

    fn delete_trash(&mut self, ids: Vec<String>, bytes: i64) {
        let Some(credential) = self.credential() else {
            self.emptying_trash = false;
            return;
        };
        let api = self.api.clone();
        self.rt.spawn(async move {
            let mut affected = 0;
            let mut failure = None;
            for batch in ids.chunks(TRASH_DELETE_BATCH) {
                match api.delete_uploads(&credential, batch, "permanent").await {
                    Ok(result) => affected += result.affected,
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                state.emptying_trash = false;
                if let Some(error) = failure {
                    state.report_failure(&error, &credential.host);
                    return;
                }
                if affected == 0 {
                    state.notice(
                        icons::TRIANGLE_ALERT,
                        t(s::MySpace_DeleteNothingHappened),
                        "",
                        true,
                    );
                    return;
                }
                state.space_reload();
                state.refresh_account_strip();
                let text = format(
                    t(s::MySpace_EmptyTrashDone),
                    &[Arg::Int(affected), Arg::from(cloud_size(bytes))],
                );
                state.notice(icons::TRASH_2, &text, "", false);
            })
        });
    }

    // ------------------------------------------------------------------ folders

    pub fn ask_new_folder(&mut self, parent: Option<String>) {
        let Some(_) = self.credential_or_sign_in() else {
            return;
        };
        self.windows.prompt =
            Some(self.new_folder_prompt(parent.clone(), PromptPurpose::NewFolder { parent }));
    }

    fn new_folder_prompt(&self, parent: Option<String>, purpose: PromptPurpose) -> Prompt {
        let parent_name = self.space.folder_name_for(parent.as_deref());
        let nested = !parent_name.is_empty();
        let heading = if nested {
            t(s::MySpace_NewSubfolder)
        } else {
            t(s::MySpace_NewFolder)
        };
        let description = if nested {
            format(t(s::MySpace_NewFolderInside), &[Arg::from(parent_name)])
        } else {
            t(s::MySpace_NewFolderAtRoot).to_string()
        };
        Prompt::new(
            heading,
            &description,
            t(s::MySpace_FolderNameLabel),
            t(s::MySpace_Create),
            "",
            "",
            120,
            purpose,
        )
    }

    pub fn ask_move_to_new_folder(&mut self, files: Vec<String>) {
        let Some(_) = self.credential_or_sign_in() else {
            return;
        };
        let parent = self.space.current_folder.clone();
        self.windows.prompt = Some(self.new_folder_prompt(
            parent.clone(),
            PromptPurpose::MoveToNewFolder { files, parent },
        ));
    }

    pub fn ask_rename_folder(&mut self, id: &str) {
        let Some(_) = self.credential_or_sign_in() else {
            return;
        };
        let Some(name) = self.space.find(id).map(|f| f.name.clone()) else {
            return;
        };
        self.windows.prompt = Some(Prompt::new(
            t(s::MySpace_RenameFolder),
            t(s::MySpace_RenameFolderDescription),
            t(s::MySpace_FolderNameLabel),
            t(s::MySpace_Save),
            &name,
            "",
            120,
            PromptPurpose::RenameFolder {
                id: id.to_string(),
                old: name.clone(),
            },
        ));
    }

    pub fn prompt_finished(&mut self, outcome: PromptOutcome) {
        let Some(prompt) = self.windows.prompt.take() else {
            return;
        };
        let PromptOutcome::Confirmed(value) = outcome else {
            return;
        };
        match prompt.purpose {
            PromptPurpose::NewFolder { parent } => self.create_folder(value, parent, None),
            PromptPurpose::MoveToNewFolder { files, parent } => {
                self.create_folder(value, parent, Some(files))
            }
            PromptPurpose::RenameFolder { id, old } => {
                if value != old {
                    self.rename_folder(id, value);
                }
            }
            PromptPurpose::CustomUrl { file, old } => {
                if value != old {
                    let edit = UploadEdit::new().set_code(&value);
                    self.edit_files(
                        std::slice::from_ref(&file),
                        edit,
                        EditDone::CustomUrl { id: file.clone() },
                    );
                }
            }
        }
    }

    fn create_folder(
        &mut self,
        name: String,
        parent: Option<String>,
        then_move: Option<Vec<String>>,
    ) {
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api
                .create_folder(&credential, &name, parent.as_deref())
                .await;
            Box::new(
                move |state: &mut AppState, _ctx: &egui::Context| match answer {
                    Ok(folder) => {
                        state.refresh_folders();
                        let text = format(
                            t(s::MySpace_FolderCreatedToast),
                            &[Arg::from(name.as_str())],
                        );
                        state.notice(icons::FOLDER_PLUS, &text, "", false);
                        if let (Some(files), Some(id)) = (then_move, folder.and_then(|f| f.id)) {
                            state.move_files(&files, Some(id));
                        }
                    }
                    Err(error) => state.report_failure(&error, &credential.host),
                },
            )
        });
    }

    fn rename_folder(&mut self, id: String, name: String) {
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.rename_folder(&credential, &id, &name).await;
            Box::new(
                move |state: &mut AppState, _ctx: &egui::Context| match answer {
                    Ok(_) => {
                        state.refresh_folders();
                        let text = format(
                            t(s::MySpace_FolderRenamedToast),
                            &[Arg::from(name.as_str())],
                        );
                        state.notice(icons::FOLDER_PEN, &text, "", false);
                    }
                    Err(error) => state.report_failure(&error, &credential.host),
                },
            )
        });
    }

    pub fn set_folder_public(&mut self, id: &str, public: bool) {
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        let api = self.api.clone();
        let id = id.to_string();
        self.rt.spawn(async move {
            let answer = api.set_folder_visibility(&credential, &id, public).await;
            Box::new(
                move |state: &mut AppState, _ctx: &egui::Context| match answer {
                    Ok(_) => {
                        state.refresh_folders();
                        let (icon, key) = if public {
                            (icons::GLOBE, s::MySpace_FolderMadePublic)
                        } else {
                            (icons::GLOBE_LOCK, s::MySpace_FolderMadePrivate)
                        };
                        state.notice(icon, t(key), "", false);
                    }
                    Err(error) => state.report_failure(&error, &credential.host),
                },
            )
        });
    }

    pub fn ask_delete_folder(&mut self, id: &str) {
        let Some(_) = self.credential_or_sign_in() else {
            return;
        };
        let Some(folder) = self.space.find(id) else {
            return;
        };
        if !folder.is_real_folder() {
            return;
        }
        self.windows.folder_delete = Some(FolderDelete::new(
            &folder.id,
            &folder.name,
            folder.file_count,
            folder.descendant_count,
            self.time,
        ));
    }

    pub fn folder_delete_finished(&mut self, choice: FolderDeleteChoice) {
        let Some(window) = self.windows.folder_delete.take() else {
            return;
        };
        if choice == FolderDeleteChoice::Cancel {
            return;
        }
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        let with_contents = choice == FolderDeleteChoice::WithContents;
        let api = self.api.clone();
        let id = window.folder_id.clone();
        self.rt.spawn(async move {
            let answer = api.delete_folder(&credential, &id, with_contents).await;
            Box::new(
                move |state: &mut AppState, _ctx: &egui::Context| match answer {
                    Ok(result) => {
                        // Leave the folder first when it was the open one: a new listing
                        // drops the work of the old one, and the folder refresh must not be
                        // part of what is dropped (upstream refreshes first and loses it).
                        if state.space.current_folder.as_deref() == Some(id.as_str()) {
                            state.space_navigate_to(None);
                            state.space_reload();
                        } else {
                            state.refresh_folders();
                        }
                        state.refresh_account_strip();
                        let text = if result.trashed > 0 {
                            format(
                                t(s::MySpace_FolderDeletedWithContents),
                                &[Arg::Int(result.trashed)],
                            )
                        } else {
                            t(s::MySpace_FolderDeleted).to_string()
                        };
                        state.notice(icons::TRASH_2, &text, "", false);
                    }
                    Err(error) => state.report_failure(&error, &credential.host),
                },
            )
        });
    }

    // ------------------------------------------------------------------ downloads

    /// Save a file (or a conversion of it) where the person says (`SaveAsync`).
    pub fn download(&mut self, id: &str, convert_to: Option<String>) {
        let Some(credential) = self.credential_or_sign_in() else {
            return;
        };
        let Some(file) = self.space.file(id).cloned() else {
            return;
        };
        let (link, suggested) = match &convert_to {
            None => (file.download_link(), file.name.clone()),
            Some(to) => {
                let stem = file.stem();
                let stem = if stem.is_empty() {
                    file.code.clone()
                } else {
                    stem
                };
                (file.convert_link(to), format!("{stem}.{to}"))
            }
        };
        if link.is_empty() {
            return;
        }
        let Some(path) = self.choose_save_path(&suggested) else {
            return;
        };
        let check_first = convert_to.is_some();
        let api = self.api.clone();
        let name = file.name.clone();
        let extension = convert_to.clone().unwrap_or_default().to_uppercase();
        self.rt.spawn(async move {
            if check_first {
                let verdict = api.check_conversion(&credential, &link).await;
                if !verdict.ok {
                    return Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                        let title =
                            format(t(s::MySpace_ConvertRefusedTitle), &[Arg::from(extension)]);
                        state.notice(
                            icons::TRIANGLE_ALERT,
                            &title,
                            convert::refusal_text(&verdict.reason),
                            true,
                        );
                    }) as crate::runtime::Completion;
                }
            }
            let started = name.clone();
            let result = api.download_to_file(&credential, &link, &path).await;
            if result.is_err() {
                let _ = tokio::fs::remove_file(&path).await;
            }
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                let _ = started;
                match result {
                    Ok(()) => {
                        let saved = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let text = format(t(s::MySpace_DownloadSaved), &[Arg::from(saved)]);
                        state.notice(icons::CIRCLE_CHECK, &text, "", false);
                    }
                    Err(error) => state.report_failure(&error, &credential.host),
                }
            }) as crate::runtime::Completion
        });
        let text = format(
            t(s::MySpace_DownloadStarted),
            &[Arg::from(file.name.as_str())],
        );
        self.notice(icons::DOWNLOAD, &text, "", false);
    }

    /// Where a download goes: the save panel, or (QA) the profile's Downloads folder,
    /// so no automated run ever opens a panel on the screen.
    fn choose_save_path(&mut self, suggested: &str) -> Option<PathBuf> {
        if self.env.qa {
            let folder = self.env.profile.join("Downloads");
            std::fs::create_dir_all(&folder).ok()?;
            return Some(folder.join(sanitize_file_name(suggested)));
        }
        let extension = std::path::Path::new(suggested)
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut dialog = rfd::FileDialog::new()
            .set_title(t(s::MySpace_Download))
            .set_file_name(suggested);
        if !extension.is_empty() {
            dialog = dialog.add_filter(extension.to_uppercase(), &[extension.as_str()]);
        }
        dialog.save_file()
    }

    // ------------------------------------------------------------------ duplicates

    /// Open the duplicates window and scan (`ScanDuplicatesAsync`).
    pub fn scan_duplicates(&mut self) {
        if self.scanning_duplicates {
            return;
        }
        let Some(_) = self.credential_or_sign_in() else {
            return;
        };
        self.scanning_duplicates = true;
        self.windows.duplicates = Some(Duplicates::default());
        self.duplicates_scan(false);
    }

    fn duplicates_scan(&mut self, keep_status: bool) {
        let Some(credential) = self.credential() else {
            return;
        };
        let Some(window) = self.windows.duplicates.as_mut() else {
            return;
        };
        if window.busy {
            return;
        }
        let carried = if keep_status {
            window.status.clone()
        } else {
            String::new()
        };
        window.busy = true;
        window.failed = false;
        window.status = t(s::MySpace_DuplicatesScanning).to_string();
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = api.get_duplicates(&credential).await;
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                let Some(window) = state.windows.duplicates.as_mut() else {
                    return;
                };
                window.busy = false;
                match answer {
                    Ok(scan) => {
                        window.apply_scan(scan, true);
                        window.status = carried;
                    }
                    Err(ApiError::Unauthorized) => window.fail(t(s::MySpace_TokenRejected)),
                    Err(error) => window.fail(&error.to_string()),
                }
            })
        });
    }

    pub fn duplicates_action(&mut self, action: DuplicatesAction) {
        match action {
            DuplicatesAction::ToggleKeep(index) => self.toggle_keep(index),
            DuplicatesAction::Trash => {
                let Some(window) = self.windows.duplicates.as_ref() else {
                    return;
                };
                if !window.can_trash() {
                    return;
                }
                let question = format(
                    t(s::MySpace_DuplicatesConfirm),
                    &[
                        Arg::from(window.selected_count()),
                        Arg::from(cloud_size(window.selected_bytes())),
                    ],
                );
                self.windows.confirm = Some(Confirm::new(
                    t(s::MySpace_DuplicatesTitle),
                    &question,
                    ConfirmAction::DuplicatesTrash,
                ));
            }
            DuplicatesAction::Close => {
                let trashed = self
                    .windows
                    .duplicates
                    .take()
                    .map(|w| w.trashed)
                    .unwrap_or(0);
                self.scanning_duplicates = false;
                if trashed > 0 {
                    self.space_reload();
                    self.refresh_account_strip();
                    let text = format(t(s::MySpace_DuplicatesTrashed), &[Arg::Int(trashed)]);
                    self.notice(icons::COPY, &text, "", false);
                }
            }
        }
    }

    fn toggle_keep(&mut self, index: usize) {
        let Some(credential) = self.credential() else {
            return;
        };
        let Some(window) = self.windows.duplicates.as_mut() else {
            return;
        };
        if window.busy {
            return;
        }
        let Some(set) = window.sets.get_mut(index) else {
            return;
        };
        if set.checksum().is_empty() || set.busy {
            return;
        }
        set.busy = true;
        let dismissed = set.dismissed;
        let checksum = set.checksum().to_string();
        let group = set.group.clone();
        let api = self.api.clone();
        self.rt.spawn(async move {
            let answer = if dismissed {
                api.count_duplicate_sets_again(&credential, std::slice::from_ref(&checksum))
                    .await
                    .map(|_| ())
            } else {
                api.keep_duplicate_sets(&credential, std::slice::from_ref(&group))
                    .await
                    .map(|_| ())
            };
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                let Some(window) = state.windows.duplicates.as_mut() else {
                    return;
                };
                let Some(set) = window.sets.iter_mut().find(|s| s.checksum() == checksum) else {
                    return;
                };
                set.busy = false;
                match answer {
                    Ok(()) => {
                        if dismissed {
                            set.dismissed = false;
                            set.select_extras();
                            window.status = t(s::MySpace_DuplicatesCountingAgain).to_string();
                        } else {
                            set.dismissed = true;
                            set.clear_selection();
                            window.status = t(s::MySpace_DuplicatesKept).to_string();
                        }
                        window.failed = false;
                    }
                    Err(ApiError::Unauthorized) => window.fail(t(s::MySpace_TokenRejected)),
                    Err(error) => window.fail(&error.to_string()),
                }
            })
        });
    }

    fn trash_duplicates(&mut self) {
        let Some(credential) = self.credential() else {
            return;
        };
        let Some(window) = self.windows.duplicates.as_mut() else {
            return;
        };
        if window.busy {
            return;
        }
        let ids = window.selected_ids();
        if ids.is_empty() {
            return;
        }
        window.busy = true;
        window.failed = false;
        window.status = t(s::MySpace_DuplicatesTrashing).to_string();
        let api = self.api.clone();
        self.rt.spawn(async move {
            let mut affected = 0;
            let mut failure = None;
            for batch in ids.chunks(MAX_DELETE_IDS) {
                match api.delete_uploads(&credential, batch, "trash").await {
                    Ok(result) => affected += result.affected,
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                let Some(window) = state.windows.duplicates.as_mut() else {
                    return;
                };
                window.busy = false;
                window.trashed += affected;
                match failure {
                    Some(ApiError::Unauthorized) => window.fail(t(s::MySpace_TokenRejected)),
                    Some(error) => window.fail(&error.to_string()),
                    None if affected == 0 => window.fail(t(s::MySpace_DeleteNothingHappened)),
                    None => {
                        window.status =
                            format(t(s::MySpace_DuplicatesTrashed), &[Arg::Int(affected)]);
                        state.duplicates_scan(true);
                    }
                }
            })
        });
    }
}

/// A file name the file system will take: path separators and colons replaced.
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if cleaned.trim().is_empty() {
        "download".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_names_cannot_climb_out_of_the_folder() {
        assert_eq!(sanitize_file_name("../a/b:c.png"), ".._a_b_c.png");
        assert_eq!(sanitize_file_name("  "), "download");
    }
}

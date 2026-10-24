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
        // `RememberPageSize`, before the request: a preference, not a result.
        self.config.set_page_size(wanted);
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
        // A row whose code changed (a custom URL) takes the previews cached under its
        // old code along, before anything else can be fetched under either code.
        for row in rows {
            let (Some(id), Some(new)) = (row.id.as_deref(), row.code.as_deref()) else {
                continue;
            };
            if let Some(old) = self.space.file(id).map(|f| f.code.clone())
                && old != new
            {
                self.media.bytes.rename(&old, new);
                self.media.motion_bytes.rename(&old, new);
            }
        }
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
                self.refill_emptied_page();
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
                    Ok(result) => state.files_removed(&ids, mode, result.affected),
                    Err(error) => state.report_failure(&error, &credential.host),
                },
            )
        });
    }

    /// The server trashed, restored or deleted `ids` for good: off the grid, the folder
    /// tallies corrected, said in the corner.
    pub(crate) fn files_removed(&mut self, ids: &[String], mode: &str, affected: i64) {
        for id in ids {
            if mode == "restore" {
                self.space.note_file_restored(id);
            } else {
                self.space.note_file_removed(id);
            }
            self.media.forget(id);
        }
        self.refresh_account_strip();
        let (icon, key) = match mode {
            "restore" => (icons::UNDO_2, s::MySpace_RestoredToast),
            "permanent" => (icons::TRASH_2, s::MySpace_DeletedForeverToast),
            _ => (icons::TRASH_2, s::MySpace_TrashedToast),
        };
        let text = format(t(key), &[Arg::Int(affected)]);
        self.notice(icon, &text, "", false);
        self.refill_emptied_page();
    }

    /// Files left the page on screen and took all of it with them while the listing
    /// still has others: list the page that holds them now (`MySpace::refill_page`).
    fn refill_emptied_page(&mut self) {
        if let Some(page) = self.space.refill_page()
            && let Some(credential) = self.credential()
        {
            self.start_listing(credential, false, page);
        }
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
                if let Err(error) = crate::platform::system::move_to_trash(&path) {
                    log::warn!("Modlight: {} could not be deleted: {error}", path.display());
                }
            }
            ConfirmAction::RemoveComponent(id) => {
                crate::components_window::remove_confirmed(self, &id)
            }
            ConfirmAction::ResetSettings => self.reset_settings_confirmed(),
            ConfirmAction::EnableImageEffects => self.enable_image_effects(),
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
        let Some(to) = convert_to else {
            self.save_download(credential, file.name, link, suggested);
            return;
        };
        // The dry run first (`checkFirst`): a refusal arrives as a header on a response
        // already taken for a download, so it is asked for before anything else, and a
        // refused conversion never gets as far as the save panel.
        let api = self.api.clone();
        self.rt.spawn(async move {
            let verdict = api.check_conversion(&credential, &link).await;
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                if verdict.ok {
                    state.save_download(credential, file.name, link, suggested);
                    return;
                }
                let title = format(
                    t(s::MySpace_ConvertRefusedTitle),
                    &[Arg::from(to.to_uppercase())],
                );
                state.notice(
                    icons::TRIANGLE_ALERT,
                    &title,
                    convert::refusal_text(&verdict.reason),
                    true,
                );
            })
        });
    }

    /// Ask where, then pull `link` down there. The destination is replaced only by a
    /// whole download (`Api::download_to_file`): a failure leaves it as it was.
    fn save_download(&mut self, credential: Credential, name: String, link: String, suggested: String) {
        let Some(path) = self.choose_save_path(&suggested) else {
            return;
        };
        let text = format(t(s::MySpace_DownloadStarted), &[Arg::from(name.as_str())]);
        self.notice(icons::DOWNLOAD, &text, "", false);
        let api = self.api.clone();
        self.rt.spawn(async move {
            let result = api.download_to_file(&credential, &link, &path).await;
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| match result {
                Ok(()) => {
                    let saved = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let text = format(t(s::MySpace_DownloadSaved), &[Arg::from(saved)]);
                    state.notice(icons::CIRCLE_CHECK, &text, "", false);
                }
                Err(error) => state.report_failure(&error, &credential.host),
            })
        });
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
pub(crate) mod tests {
    use super::*;
    use modlight_api::{Upload, UploadPage};
    use modlight_core::account::{AccountStore, StoredAccount};

    #[test]
    fn saved_names_cannot_climb_out_of_the_folder() {
        assert_eq!(sanitize_file_name("../a/b:c.png"), ".._a_b_c.png");
        assert_eq!(sanitize_file_name("  "), "download");
    }

    /// A signed-in state whose server refuses connections at once (nothing listens on
    /// port 1), so a listing the test starts never reaches anything.
    pub(crate) fn offline_state() -> AppState {
        state_on("http://127.0.0.1:1")
    }

    /// A signed-in QA state whose account lives on `host`.
    pub(crate) fn state_on(host: &str) -> AppState {
        let mut state = crate::qa::state(&crate::qa::Look::default(), true);
        state.account = AccountStore::in_memory(
            StoredAccount {
                host: host.into(),
                tag: "alex".into(),
                ..StoredAccount::default()
            },
            "qa-token-not-a-secret",
        );
        state.space.has_account = true;
        state
    }

    /// A server on 127.0.0.1 answering every request with `reply` as is.
    fn server(reply: &'static [u8]) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut buffer = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buffer[..n]),
                    }
                }
                let _ = stream.write_all(reply);
            }
        });
        host
    }

    /// Run completions until `done` or ten seconds.
    fn settle(state: &mut AppState, done: impl Fn(&AppState) -> bool) {
        let ctx = egui::Context::default();
        let started = std::time::Instant::now();
        while !done(state) && started.elapsed().as_secs() < 10 {
            for completion in state.rt.drain() {
                completion(state, &ctx);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub(crate) fn row(id: &str, folder: Option<&str>) -> Upload {
        Upload {
            id: Some(id.into()),
            code: Some(format!("c{id}")),
            filename: Some(format!("{id}.png")),
            ext: Some("png".into()),
            kind: Some("image".into()),
            size: 100,
            folder_id: folder.map(str::to_string),
            url: Some(format!("http://127.0.0.1:1/alex/c{id}.png")),
            raw_url: Some(format!("http://127.0.0.1:1/alex/c{id}/raw.png")),
            ..Upload::default()
        }
    }

    /// Page `page` of a listing of `total` files, `per_page` a page, holding `ids`.
    pub(crate) fn listing(state: &mut AppState, page: i64, per_page: i64, total: i64, ids: &[&str]) {
        state.space.page_size = per_page;
        state.space.generation += 1;
        let items = ids.iter().map(|id| row(id, None)).collect();
        let page_rows = UploadPage {
            page,
            per_page,
            total,
            items: Some(items),
            ..UploadPage::default()
        };
        state.space.apply_page(page_rows, page);
    }

    /// Trashing the last file of the last page used to leave an empty grid under a bar
    /// saying "31-60 of 60", whose page 2 button did nothing (the bar had clamped itself
    /// to page 2 already): the emptied page is listed again, at the page that now exists.
    #[test]
    fn emptying_a_page_lists_the_page_that_now_holds_the_files() {
        let mut state = offline_state();
        listing(&mut state, 3, 30, 61, &["last"]);
        state.files_removed(&["last".into()], "trash", 1);
        assert!(state.space.is_busy, "the page is asked for again");
        assert_eq!(state.space.page, 2);
        assert_eq!(state.space.total, 0, "the new listing starts from nothing");

        // Selecting every file on page 1 of 2 and trashing them: page 1 again, which now
        // holds what was page 2.
        let mut state = offline_state();
        let ids: Vec<String> = (0..30).map(|i| format!("f{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        listing(&mut state, 1, 30, 40, &refs);
        state.files_removed(&ids, "trash", 30);
        assert!(state.space.is_busy);
        assert_eq!(state.space.page, 1);

        // A page that still shows files is left as it is (upstream does not refill it).
        let mut state = offline_state();
        listing(&mut state, 1, 30, 40, &["a", "b"]);
        state.files_removed(&["a".into()], "trash", 1);
        assert!(!state.space.is_busy);
        assert_eq!(state.space.files.len(), 1);

        // The last file of the whole listing: nothing to list again.
        let mut state = offline_state();
        listing(&mut state, 1, 30, 1, &["only"]);
        state.files_removed(&["only".into()], "permanent", 1);
        assert!(!state.space.is_busy);
        assert!(state.space.show_empty_panel());

        // Moving every file of the open folder's last page elsewhere empties it too.
        let mut state = offline_state();
        listing(&mut state, 2, 30, 31, &["moved"]);
        state.space.current_folder = Some("f1".into());
        state.space.files[0].file.folder_id = Some("f1".into());
        state.edit_done(
            EditDone::Moved {
                ids: vec!["moved".into()],
                target: None,
            },
            &[row("moved", None)],
        );
        assert!(state.space.is_busy, "the folder's page is asked for again");
        assert_eq!(state.space.page, 1);
    }

    /// A download the server refuses (404) never wrote a byte: the file the person
    /// agreed to replace is still theirs. It used to be deleted as a "part-written" file.
    #[test]
    fn a_refused_download_keeps_the_file_it_would_have_replaced() {
        let host = server(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let mut state = state_on(&host);
        let profile = tempfile::tempdir().unwrap();
        state.env.profile = profile.path().to_path_buf();
        listing(&mut state, 1, 30, 1, &["a"]);
        let downloads = profile.path().join("Downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("a.png"), b"my older a.png").unwrap();

        state.download("a", None);
        settle(&mut state, |s| s.toasts.iter().any(|t| t.failed));
        assert!(state.toasts.iter().any(|t| t.failed), "the refusal is said");
        assert_eq!(
            std::fs::read(downloads.join("a.png")).unwrap(),
            b"my older a.png"
        );
    }

    /// `SaveAsync(checkFirst: true)`: the conversion is asked about BEFORE the save
    /// panel, so a refusal never gets as far as asking where to put the file, and no
    /// "download started" is said for a download that does not start.
    #[test]
    fn a_refused_conversion_never_asks_where_to_save() {
        let host = server(
            b"HTTP/1.1 415 Unsupported Media Type\r\nX-Convert-Refused: no-audio\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let mut state = state_on(&host);
        let profile = tempfile::tempdir().unwrap();
        state.env.profile = profile.path().to_path_buf();
        listing(&mut state, 1, 30, 1, &["a"]);
        state.space.files[0].file.row.kind = Some("video".into());

        state.download("a", Some("mp3".into()));
        let started = format(t(s::MySpace_DownloadStarted), &[Arg::from("a.png")]);
        assert!(
            state.toasts.iter().all(|t| t.title != started),
            "nothing has started yet"
        );
        settle(&mut state, |s| s.toasts.iter().any(|t| t.failed));
        let refusal = state.toasts.iter().find(|t| t.failed).expect("the refusal");
        assert_eq!(
            refusal.title,
            format(t(s::MySpace_ConvertRefusedTitle), &[Arg::from("MP3")])
        );
        assert_eq!(refusal.detail, convert::refusal_text("no-audio"));
        assert!(state.toasts.iter().all(|t| t.title != started));
        assert!(
            !profile.path().join("Downloads").exists(),
            "no destination was asked for"
        );
    }

    /// `RememberPageSize` and `RememberFolder` write the two settings when the person
    /// changes them; every other save leaves them alone. A session that starts signed
    /// out never takes up the remembered folder, and saving anything (a theme, the
    /// panel, quitting) wrote "no folder" over it.
    #[test]
    fn an_unrelated_save_keeps_the_remembered_page_size_and_folder() {
        let mut state = crate::qa::state(&crate::qa::Look::default(), false);
        state.config.set_page_size(30);
        state.config.set_last_folder_id(Some("f-design"));
        // What `AppState::new` reads from the settings file.
        state.space.page_size = 30;
        state.space_refresh_account();
        state.toggle_library_panel();
        state.save_config();
        assert_eq!(state.config.page_size(), 30);
        assert_eq!(state.config.last_folder_id(), "f-design");

        // Signed in, the library writes them when they change.
        let mut state = offline_state();
        state.config.set_page_size(30);
        state.config.set_last_folder_id(Some("f-design"));
        state.space_refresh_account();
        assert_eq!(state.space.page_size, 30, "taken up at the first listing");
        assert_eq!(state.space.current_folder.as_deref(), Some("f-design"));
        state.space_set_page_size(60);
        assert_eq!(state.config.page_size(), 60);
        state.space.loaded_once = true;
        state.space_select_view(1);
        assert_eq!(state.config.last_folder_id(), "", "a kind is no folder");
    }

    /// The preview caches are keyed by code, and a custom URL frees the old code for any
    /// other file to take. What was cached under it must follow the file it belongs to,
    /// or the next file given that code is drawn with this one's picture.
    #[test]
    fn a_new_custom_url_takes_the_cached_previews_with_it() {
        let mut state = offline_state();
        listing(&mut state, 1, 30, 2, &["a", "b"]);
        let picture = std::sync::Arc::new(b"a's picture".to_vec());
        let reel = std::sync::Arc::new(b"a's reel".to_vec());
        state.media.bytes.put("ca", picture.clone());
        state.media.motion_bytes.put("ca", reel.clone());
        state.media.motion_bytes.refuse("cb");

        let mut renamed = row("a", None);
        renamed.code = Some("logo".into());
        state.edit_done(EditDone::CustomUrl { id: "a".into() }, &[renamed]);
        assert_eq!(state.space.file("a").unwrap().code, "logo");
        assert!(state.media.bytes.get("ca").is_none(), "the old code is free");
        assert!(state.media.motion_bytes.get("ca").is_none());
        assert_eq!(state.media.bytes.get("logo"), Some(picture));
        assert_eq!(state.media.motion_bytes.get("logo"), Some(reel));

        // b takes a's old code: nothing of a's is found under it, and b's own refusal
        // goes with b.
        let mut taken = row("b", None);
        taken.code = Some("ca".into());
        state.edit_done(EditDone::CustomUrl { id: "b".into() }, &[taken]);
        assert!(state.media.bytes.get("ca").is_none());
        assert!(!state.media.motion_bytes.is_refused("cb"));
        assert!(state.media.motion_bytes.is_refused("ca"));
    }

    /// The conversion the server would make is then saved under the stem and the new
    /// extension, and said so.
    #[test]
    fn an_accepted_conversion_is_saved() {
        let host = server(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nconverted");
        let mut state = state_on(&host);
        let profile = tempfile::tempdir().unwrap();
        state.env.profile = profile.path().to_path_buf();
        listing(&mut state, 1, 30, 1, &["a"]);

        state.download("a", Some("jpg".into()));
        let saved = format(t(s::MySpace_DownloadSaved), &[Arg::from("a.jpg")]);
        settle(&mut state, |s| s.toasts.iter().any(|t| t.title == saved));
        assert!(state.toasts.iter().all(|t| !t.failed));
        assert_eq!(
            std::fs::read(profile.path().join("Downloads").join("a.jpg")).unwrap(),
            b"converted"
        );
    }
}

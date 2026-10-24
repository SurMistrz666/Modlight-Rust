//! My space without drawing: where the library is standing, what it lists, what is
//! selected (`MySpaceViewModel.cs`).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use modlight_api::{FolderList, Upload, UploadPage};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::library::{
    CloudFile, CloudFolder, Crumb, FolderKind, LIBRARY_VIEWS, LibraryView, Pager, crumbs, pager,
    roll_up,
};

/// One card of the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub file: CloudFile,
    pub selected: bool,
}

/// The library's listing state.
#[derive(Debug)]
pub struct MySpace {
    pub has_account: bool,
    pub is_busy: bool,
    pub has_error: bool,
    pub loaded_once: bool,
    pub error_text: String,
    pub count_text: String,

    /// Which listing is current; completions for an older one are dropped (`Owns`).
    pub generation: u64,
    pub identity: String,

    pub page: i64,
    pub page_size: i64,
    pub total: i64,
    pub pager: Pager,

    pub current_kind: Option<&'static str>,
    pub trashed: bool,
    pub current_folder: Option<String>,
    pub root_value: String,

    pub folders: Vec<CloudFolder>,
    pub unfiled: Option<CloudFolder>,
    pub files: Vec<FileEntry>,

    pub selected_folder: Option<String>,
    pub selection_anchor: Option<String>,
    pub restored_last_folder: bool,
    pub restored_page_size: bool,

    /// The folder that just received a file, and until when it glows.
    pub flash: Option<(String, f64)>,
    pub merging: bool,
    pub merge_queued: bool,
}

impl Default for MySpace {
    fn default() -> Self {
        MySpace {
            has_account: false,
            is_busy: false,
            has_error: false,
            loaded_once: false,
            error_text: String::new(),
            count_text: String::new(),
            generation: 0,
            identity: String::new(),
            page: 1,
            page_size: pager::DEFAULT_PAGE_SIZE,
            total: 0,
            pager: Pager::default(),
            current_kind: None,
            trashed: false,
            current_folder: None,
            root_value: modlight_api::ROOT_FOLDER.to_string(),
            folders: Vec::new(),
            unfiled: None,
            files: Vec::new(),
            selected_folder: None,
            selection_anchor: None,
            restored_last_folder: false,
            restored_page_size: false,
            flash: None,
            merging: false,
            merge_queued: false,
        }
    }
}

impl MySpace {
    pub fn find(&self, id: &str) -> Option<&CloudFolder> {
        self.unfiled
            .as_ref()
            .filter(|u| u.id == id)
            .or_else(|| self.folders.iter().find(|f| f.id == id))
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut CloudFolder> {
        if self.unfiled.as_ref().is_some_and(|u| u.id == id) {
            return self.unfiled.as_mut();
        }
        self.folders.iter_mut().find(|f| f.id == id)
    }

    pub fn current_folder_view(&self) -> Option<&CloudFolder> {
        self.current_folder.as_deref().and_then(|id| self.find(id))
    }

    fn current_view(&self) -> Option<&'static LibraryView> {
        if self.current_folder.is_some() {
            return None;
        }
        LIBRARY_VIEWS
            .iter()
            .find(|v| v.kind == self.current_kind && v.is_trash == self.trashed)
    }

    pub fn current_folder_name(&self) -> String {
        if self.trashed {
            return t(s::MySpace_Trash).to_string();
        }
        if self.current_folder.is_some() {
            return self
                .current_folder_view()
                .map(|f| f.name.clone())
                .unwrap_or_else(|| t(s::MySpace_AllFiles).to_string());
        }
        self.current_view()
            .map(|v| t(v.name).to_string())
            .unwrap_or_else(|| t(s::MySpace_AllFiles).to_string())
    }

    pub fn current_folder_icon(&self) -> char {
        if self.current_folder.is_some() {
            return if self
                .current_folder_view()
                .is_some_and(|f| f.kind == FolderKind::Unfiled)
            {
                icons::INBOX
            } else {
                icons::FOLDER_OPEN
            };
        }
        self.current_view()
            .map(|v| v.icon)
            .unwrap_or(icons::LAYOUT_GRID)
    }

    pub fn is_view_current(&self, view: &LibraryView) -> bool {
        self.current_folder.is_none()
            && view.kind == self.current_kind
            && view.is_trash == self.trashed
    }

    pub fn can_upload(&self) -> bool {
        self.has_account && !self.trashed
    }

    pub fn can_go_up(&self) -> bool {
        self.current_folder.is_some()
    }

    /// The folders the shelf shows: the unfiled shelf at the very top, then the
    /// children of where you are. No shelf in a kind filter or the trash.
    pub fn visible_folders(&self) -> Vec<&CloudFolder> {
        let mut out = Vec::new();
        let show_shelf = self.current_kind.is_none() && !self.trashed;
        if !show_shelf {
            return out;
        }
        if self.current_folder.is_none()
            && let Some(unfiled) = &self.unfiled
        {
            out.push(unfiled);
        }
        let in_unfiled = self
            .current_folder_view()
            .is_some_and(|f| f.kind == FolderKind::Unfiled);
        if !in_unfiled {
            for folder in &self.folders {
                if folder.parent_id.as_deref() == self.current_folder.as_deref() {
                    out.push(folder);
                }
            }
        }
        out
    }

    /// The side panel's list: the unfiled shelf, then every folder in tree order.
    pub fn panel_folders(&self) -> Vec<&CloudFolder> {
        let mut out = Vec::new();
        if let Some(unfiled) = &self.unfiled {
            out.push(unfiled);
        }
        out.extend(self.folders.iter());
        out
    }

    pub fn crumbs(&self) -> Vec<Crumb> {
        crumbs(
            &self.folders,
            self.unfiled.as_ref(),
            self.current_folder.as_deref(),
        )
    }

    pub fn has_files(&self) -> bool {
        !self.files.is_empty()
    }

    pub fn has_folders(&self) -> bool {
        !self.visible_folders().is_empty()
    }

    pub fn has_content(&self) -> bool {
        self.has_folders() || self.has_files()
    }

    pub fn show_no_files_note(&self) -> bool {
        self.has_account
            && self.loaded_once
            && !self.is_busy
            && !self.has_error
            && self.has_folders()
            && !self.has_files()
    }

    pub fn show_loading_panel(&self) -> bool {
        self.has_account && self.is_busy && !self.has_error && !self.has_content()
    }

    pub fn show_empty_panel(&self) -> bool {
        self.has_account
            && self.loaded_once
            && !self.is_busy
            && !self.has_error
            && !self.has_content()
    }

    pub fn show_error_panel(&self) -> bool {
        self.has_account && self.has_error && !self.has_content()
    }

    pub fn show_error_footer(&self) -> bool {
        self.has_account && self.has_error && self.has_content()
    }

    pub fn folder_name_for(&self, id: Option<&str>) -> String {
        id.and_then(|id| self.find(id))
            .map(|f| f.name.clone())
            .unwrap_or_default()
    }

    /// Folder names by id, for the cards' folder labels.
    pub fn folder_names(&self) -> HashMap<String, String> {
        self.folders
            .iter()
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    // ------------------------------------------------------------- selection

    pub fn selected_count(&self) -> usize {
        self.files.iter().filter(|f| f.selected).count()
    }

    pub fn selected_ids(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.file.id.clone())
            .collect()
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.files.iter().position(|f| f.file.id == id)
    }

    pub fn file(&self, id: &str) -> Option<&CloudFile> {
        self.files.iter().find(|f| f.file.id == id).map(|f| &f.file)
    }

    pub fn select_folder_tile(&mut self, id: Option<&str>) {
        self.selected_folder = id.map(str::to_string);
        if id.is_some() {
            self.clear_file_selection();
        }
    }

    pub fn select_file(&mut self, id: &str) {
        self.selected_folder = None;
        for entry in &mut self.files {
            entry.selected = entry.file.id == id;
        }
        self.selection_anchor = Some(id.to_string());
    }

    pub fn toggle_file(&mut self, id: &str) {
        self.selected_folder = None;
        if let Some(entry) = self.files.iter_mut().find(|f| f.file.id == id) {
            entry.selected = !entry.selected;
        }
        self.selection_anchor = Some(id.to_string());
    }

    pub fn select_file_range(&mut self, id: &str) {
        let Some(to) = self.index_of(id) else { return };
        let from = self
            .selection_anchor
            .as_deref()
            .and_then(|a| self.index_of(a))
            .unwrap_or(to);
        self.selected_folder = None;
        let (first, last) = (from.min(to), from.max(to));
        for (index, entry) in self.files.iter_mut().enumerate() {
            entry.selected = index >= first && index <= last;
        }
    }

    pub fn select_all_files(&mut self) {
        if self.files.is_empty() {
            return;
        }
        self.selected_folder = None;
        for entry in &mut self.files {
            entry.selected = true;
        }
        self.selection_anchor = Some(self.files[0].file.id.clone());
    }

    /// Clear both selections; true when there was one.
    pub fn clear_selection(&mut self) -> bool {
        let had = self.selected_count() > 0 || self.selected_folder.is_some();
        self.selected_folder = None;
        self.clear_file_selection();
        had
    }

    fn clear_file_selection(&mut self) {
        for entry in &mut self.files {
            entry.selected = false;
        }
        self.selection_anchor = None;
    }

    pub fn selected_files(&self) -> Vec<CloudFile> {
        self.files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.file.clone())
            .collect()
    }

    // ------------------------------------------------------------- listings

    /// Forget the files (a new listing is coming).
    pub fn clear_files(&mut self) {
        self.files.clear();
        self.selection_anchor = None;
    }

    pub fn update_count(&mut self) {
        self.count_text = if self.total > 0 {
            format(t(s::MySpace_FileCount), &[Arg::Int(self.total)])
        } else {
            String::new()
        };
    }

    pub fn set_error(&mut self, failed: bool, text: &str) {
        self.has_error = failed;
        self.error_text = text.to_string();
    }

    /// Take a folder listing. Returns true when the folder that was open disappeared and
    /// the listing has to start again at the top.
    pub fn apply_folders(&mut self, list: FolderList) -> bool {
        self.root_value = list
            .root_value
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| modlight_api::ROOT_FOLDER.to_string());
        let mut folders: Vec<CloudFolder> = list
            .folders
            .unwrap_or_default()
            .iter()
            .filter(|f| f.id.as_deref().is_some_and(|id| !id.is_empty()))
            .map(CloudFolder::new)
            .collect();
        roll_up(&mut folders);
        self.folders = folders;
        self.unfiled = list
            .unfiled
            .filter(|u| u.file_count > 0)
            .map(|u| CloudFolder::unfiled(&self.root_value, u.file_count, u.size));

        let before = self.current_folder.clone();
        if let Some(current) = &self.current_folder
            && self.find(current).is_none()
        {
            self.current_folder = None;
        }
        self.current_folder != before
    }

    /// Take one page of files.
    pub fn apply_page(&mut self, result: UploadPage, target: i64) -> Vec<String> {
        let mut arrived = Vec::new();
        for upload in result.items.unwrap_or_default() {
            let file = CloudFile::new(upload);
            arrived.push(file.id.clone());
            self.files.push(FileEntry {
                file,
                selected: false,
            });
        }
        self.page = if result.page > 0 { result.page } else { target };
        self.total = result.total.max(self.files.len() as i64);
        if result.per_page > 0 {
            self.page_size = pager::clamp_page_size(result.per_page);
        }
        self.pager.update(self.page, self.page_size, self.total);
        self.loaded_once = true;
        self.set_error(false, "");
        self.update_count();
        arrived
    }

    /// New uploads at the top of page 1 (`Merge`): rows not seen yet, in order, until
    /// the first one already listed. Returns the ids that arrived.
    pub fn merge_newest(&mut self, result: UploadPage) -> Vec<String> {
        let known: std::collections::HashSet<String> =
            self.files.iter().map(|f| f.file.id.clone()).collect();
        let mut arrived = Vec::new();
        for upload in result.items.unwrap_or_default() {
            let Some(id) = upload.id.clone().filter(|id| !id.is_empty()) else {
                break;
            };
            if known.contains(&id) {
                break;
            }
            self.files.insert(
                arrived.len(),
                FileEntry {
                    file: CloudFile::new(upload),
                    selected: false,
                },
            );
            arrived.push(id);
        }
        if arrived.is_empty() {
            return arrived;
        }
        self.total = result.total.max(self.files.len() as i64);
        while self.files.len() as i64 > self.page_size {
            let dropped = self.files.pop();
            if let Some(dropped) = dropped
                && self.selection_anchor.as_deref() == Some(dropped.file.id.as_str())
            {
                self.selection_anchor = None;
            }
        }
        self.pager.update(self.page, self.page_size, self.total);
        self.update_count();
        arrived
    }

    /// Redraw cards from rows a PATCH answered with.
    pub fn apply_rows(&mut self, rows: &[Upload]) {
        for row in rows {
            let Some(id) = row.id.as_deref() else {
                continue;
            };
            if let Some(entry) = self.files.iter_mut().find(|f| f.file.id == id) {
                entry.file.apply(row.clone());
            }
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let Some(index) = self.index_of(id) else {
            return false;
        };
        self.files.remove(index);
        if self.selection_anchor.as_deref() == Some(id) {
            self.selection_anchor = None;
        }
        if self.total > 0 {
            self.total -= 1;
        }
        self.pager.update(self.page, self.page_size, self.total);
        self.update_count();
        true
    }

    /// The page to list again when removals emptied the one on screen while the listing
    /// still has files elsewhere: the same page, or the last one that still exists.
    ///
    /// Upstream leaves the empty grid under a bar that has already clamped itself to the
    /// new last page, so that page's button is a no-op until somebody presses Refresh.
    pub fn refill_page(&self) -> Option<i64> {
        if !self.loaded_once || !self.files.is_empty() || self.total <= 0 {
            return None;
        }
        Some(pager::clamp_page(
            self.page,
            pager::page_count(self.total, self.page_size),
        ))
    }

    /// Adjust a folder's direct tally and roll the tree up again.
    pub fn adjust_folder_tally(&mut self, folder_id: Option<&str>, files: i64, bytes: i64) {
        let Some(id) = folder_id else { return };
        let Some(folder) = self.find_mut(id) else {
            return;
        };
        folder.adjust_direct(files, bytes);
        if folder.kind == FolderKind::Folder {
            roll_up(&mut self.folders);
        }
    }

    /// A file was trashed or deleted for good.
    pub fn note_file_removed(&mut self, id: &str) {
        if let Some(file) = self.file(id).cloned() {
            if !file.is_trashed {
                self.adjust_folder_tally(file.folder_id.as_deref(), -1, -file.size);
            }
            self.remove(id);
        }
    }

    /// A file came back from the trash.
    pub fn note_file_restored(&mut self, id: &str) {
        if let Some(file) = self.file(id).cloned() {
            if file.is_trashed {
                self.adjust_folder_tally(file.folder_id.as_deref(), 1, file.size);
            }
            self.remove(id);
        }
    }

    /// Files moved to `target`: tallies, rows, and the ones that left the open folder.
    pub fn note_files_moved(
        &mut self,
        ids: &[String],
        target: Option<&str>,
        rows: &[Upload],
        now: f64,
    ) {
        for id in ids {
            let Some(file) = self.file(id).cloned() else {
                continue;
            };
            if file.folder_id.as_deref() == target {
                continue;
            }
            self.adjust_folder_tally(file.folder_id.as_deref(), -1, -file.size);
            self.adjust_folder_tally(target, 1, file.size);
        }
        self.apply_rows(rows);
        if let Some(current) = self.current_folder.clone() {
            let leaving: Vec<String> = ids
                .iter()
                .filter(|id| {
                    self.file(id)
                        .is_some_and(|f| f.folder_id.as_deref() != Some(current.as_str()))
                })
                .cloned()
                .collect();
            for id in leaving {
                self.remove(&id);
            }
        }
        self.flash(target, now);
    }

    pub fn flash(&mut self, folder_id: Option<&str>, now: f64) {
        self.flash = folder_id.map(|id| {
            (
                id.to_string(),
                now + modlight_core::library::FLASH_MILLIS as f64 / 1000.0,
            )
        });
    }

    pub fn is_flashing(&self, id: &str, now: f64) -> bool {
        self.flash
            .as_ref()
            .is_some_and(|(f, until)| f == id && now < *until)
    }

    pub fn now_ages(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

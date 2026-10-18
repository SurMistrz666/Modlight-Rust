//! The menus of My space (`BuildCloudBackgroundMenu`, `BuildFolderMenu`,
//! `BuildCloudFileMenu` and their submenus), in upstream's order and wording.

use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::library::{CloudFolder, LinkFormat, convert};

use crate::commands::Command;
use crate::menus::Entry;
use crate::state::AppState;

/// A right click on the empty part of the library.
pub fn background(state: &AppState) -> Vec<Entry> {
    let space = &state.space;
    let mut entries = Vec::new();
    if space.can_upload() {
        entries.push(Entry::item(
            t(s::MySpace_UploadHere),
            icons::UPLOAD,
            Command::UploadTo(space.current_folder.clone()),
        ));
        let label = if space.can_go_up() {
            t(s::MySpace_NewSubfolder)
        } else {
            t(s::MySpace_NewFolder)
        };
        entries.push(Entry::item(
            label,
            icons::FOLDER_PLUS,
            Command::NewFolder(space.current_folder.clone()),
        ));
    }
    if space.has_files() {
        entries.push(Entry::Separator);
        entries.push(Entry::item(
            t(s::MySpace_SelectAll),
            icons::CHECK,
            Command::SelectAll,
        ));
    }
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        t(s::MySpace_Refresh),
        icons::REFRESH_CW,
        Command::Reload,
    ));
    entries
}

/// A folder tile or a folder row.
pub fn folder(state: &AppState, folder: &CloudFolder) -> Vec<Entry> {
    let mut entries = vec![Entry::item(
        t(s::MySpace_OpenFolder),
        icons::FOLDER_OPEN,
        Command::NavigateTo(Some(folder.id.clone())),
    )];
    if !folder.is_real_folder() {
        return entries;
    }
    let id = folder.id.clone();
    entries.push(Entry::item(
        t(s::MySpace_UploadToFolder),
        icons::UPLOAD,
        Command::UploadTo(Some(id.clone())),
    ));
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        t(s::MySpace_NewSubfolder),
        icons::FOLDER_PLUS,
        Command::NewFolder(Some(id.clone())),
    ));
    entries.push(Entry::item(
        t(s::MySpace_RenameFolder),
        icons::FOLDER_PEN,
        Command::RenameFolder(id.clone()),
    ));
    entries.push(Entry::check(
        t(s::MySpace_FolderPublic),
        icons::GLOBE,
        folder.is_public,
        Command::SetFolderPublic(id.clone(), !folder.is_public),
    ));
    if folder.is_public && !folder.slug.is_empty() && !state.account.account().tag.is_empty() {
        entries.push(Entry::item(
            t(s::MySpace_OpenGallery),
            icons::EXTERNAL_LINK,
            Command::OpenGallery(id.clone()),
        ));
        entries.push(Entry::item(
            t(s::MySpace_CopyGalleryLink),
            icons::LINK,
            Command::CopyGalleryLink(id.clone()),
        ));
    }
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        t(s::MySpace_ShowInPanel),
        icons::EXTERNAL_LINK,
        Command::ShowInPanel(id.clone()),
    ));
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        t(s::MySpace_DeleteFolder),
        icons::TRASH_2,
        Command::DeleteFolder(id),
    ));
    entries
}

/// A card: the whole selection when the card is part of it, the card alone otherwise.
pub fn file(state: &AppState, id: &str) -> Vec<Entry> {
    let space = &state.space;
    let selected = space.selected_ids();
    let is_selected = selected.iter().any(|s| s == id);
    let many = selected.len() > 1 && is_selected;
    let targets: Vec<String> = if many { selected } else { vec![id.to_string()] };
    if space.trashed {
        trash_menu(state, &targets, many)
    } else {
        library_menu(state, id, &targets, many)
    }
}

fn library_menu(state: &AppState, id: &str, targets: &[String], many: bool) -> Vec<Entry> {
    let space = &state.space;
    let Some(file) = space.file(id) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    if many {
        entries.push(Entry::header(
            format(t(s::MySpace_SelectionCount), &[Arg::from(targets.len())]),
            Some(icons::CHECK_CHECK),
        ));
    } else {
        entries.push(Entry::item(
            t(s::MySpace_Details),
            icons::INFO,
            Command::ShowDetails(id.to_string()),
        ));
    }
    entries.push(Entry::Separator);
    let copyable = targets
        .iter()
        .filter(|t| space.file(t).is_some_and(|f| f.has_link()))
        .count();
    let copy_label = if many {
        format(t(s::MySpace_CopyLinkMany), &[Arg::from(copyable)])
    } else {
        t(s::MySpace_CopyLink).to_string()
    };
    entries.push(Entry::item_if(
        copy_label,
        icons::LINK,
        Command::CopyLinks(targets.to_vec()),
        copyable > 0,
    ));
    entries.push(Entry::submenu(
        t(s::MySpace_CopyAs),
        icons::COPY,
        copyable > 0,
        copy_as(targets),
    ));
    entries.push(Entry::Separator);
    if !many {
        entries.push(Entry::item_if(
            t(s::MySpace_OpenInBrowser),
            icons::EXTERNAL_LINK,
            Command::OpenInBrowser(id.to_string()),
            file.can_open(),
        ));
        entries.push(Entry::item_if(
            t(s::MySpace_Download),
            icons::DOWNLOAD,
            Command::Download(id.to_string()),
            file.has_media_base(),
        ));
        let formats = convert::options_for(
            file.row.kind.as_deref(),
            file.row.mime_type.as_deref(),
            file.row.ext.as_deref(),
        );
        let enabled = !formats.is_empty() && file.has_media_base();
        entries.push(Entry::submenu(
            t(s::MySpace_DownloadAs),
            icons::FILE_DOWN,
            enabled,
            download_as(id, &formats),
        ));
        entries.push(Entry::Separator);
    }
    let move_label = if many {
        format(t(s::MySpace_MoveManyToFolder), &[Arg::from(targets.len())])
    } else {
        t(s::MySpace_MoveToFolder).to_string()
    };
    entries.push(Entry::submenu(
        move_label,
        icons::FOLDER_INPUT,
        true,
        move_to(state, targets),
    ));
    if many {
        entries.push(Entry::item(
            format(t(s::MySpace_MakePublicMany), &[Arg::from(targets.len())]),
            icons::GLOBE,
            Command::SetPublished(targets.to_vec(), true),
        ));
        entries.push(Entry::item(
            format(t(s::MySpace_MakePrivateMany), &[Arg::from(targets.len())]),
            icons::GLOBE_LOCK,
            Command::SetPublished(targets.to_vec(), false),
        ));
    } else {
        let (label, icon) = if file.published {
            (t(s::MySpace_MakePrivate), icons::GLOBE_LOCK)
        } else {
            (t(s::MySpace_MakePublic), icons::GLOBE)
        };
        entries.push(Entry::item(
            label,
            icon,
            Command::SetPublished(targets.to_vec(), !file.published),
        ));
        entries.push(Entry::item(
            t(s::MySpace_CustomUrl),
            icons::SQUARE_PEN,
            Command::CustomUrl(id.to_string()),
        ));
        entries.push(Entry::item(
            t(s::MySpace_LinkSettingsMenu),
            icons::SETTINGS_2,
            Command::LinkSettings(id.to_string()),
        ));
    }
    entries.push(Entry::Separator);
    let deletable = targets
        .iter()
        .filter(|t| space.file(t).is_some_and(|f| f.can_trash()))
        .count();
    let delete_label = if many {
        format(t(s::MySpace_DeleteMany), &[Arg::from(deletable)])
    } else {
        t(s::MySpace_Delete).to_string()
    };
    entries.push(Entry::item_if(
        delete_label,
        icons::TRASH_2,
        Command::Trash(targets.to_vec()),
        deletable > 0,
    ));
    entries
}

fn trash_menu(state: &AppState, targets: &[String], many: bool) -> Vec<Entry> {
    let _ = state;
    let mut entries = Vec::new();
    if many {
        entries.push(Entry::header(
            format(t(s::MySpace_SelectionCount), &[Arg::from(targets.len())]),
            Some(icons::CHECK_CHECK),
        ));
    } else {
        entries.push(Entry::item(
            t(s::MySpace_Details),
            icons::INFO,
            Command::ShowDetails(targets[0].clone()),
        ));
    }
    entries.push(Entry::Separator);
    let restore = if many {
        format(t(s::MySpace_RestoreMany), &[Arg::from(targets.len())])
    } else {
        t(s::MySpace_Restore).to_string()
    };
    entries.push(Entry::item(
        restore,
        icons::UNDO_2,
        Command::Restore(targets.to_vec()),
    ));
    entries.push(Entry::Separator);
    let forever = if many {
        format(t(s::MySpace_DeleteForeverMany), &[Arg::from(targets.len())])
    } else {
        t(s::MySpace_DeleteForever).to_string()
    };
    entries.push(Entry::item(
        forever,
        icons::TRASH_2,
        Command::DeleteForever(targets.to_vec()),
    ));
    entries
}

fn copy_as(targets: &[String]) -> Vec<Entry> {
    LinkFormat::ALL
        .iter()
        .map(|format| {
            Entry::item(
                format.label(),
                icons::COPY,
                Command::CopyAs(targets.to_vec(), *format),
            )
        })
        .collect()
}

fn download_as(id: &str, formats: &[convert::DownloadOption]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut headed = false;
    for option in formats {
        if option.loses_motion && !headed {
            headed = true;
            entries.push(Entry::Separator);
            entries.push(Entry::header(t(s::MySpace_ConvertFrameSection), None));
        }
        entries.push(Entry::item(
            convert::option_header(option),
            icons::FILE_DOWN,
            Command::DownloadAs(id.to_string(), option.to.clone()),
        ));
    }
    entries
}

/// "Move to folder": unfiled, every folder indented by depth, then a new folder.
pub fn move_to(state: &AppState, targets: &[String]) -> Vec<Entry> {
    let space = &state.space;
    let folders: Vec<Option<String>> = targets
        .iter()
        .filter_map(|id| space.file(id))
        .map(|f| f.folder_id.clone())
        .collect();
    let shared = match folders.first() {
        Some(first) if folders.iter().all(|f| f == first) => first.clone(),
        _ => None,
    };
    let mut entries = vec![
        Entry::check(
            t(s::MySpace_Unfiled),
            icons::INBOX,
            shared.is_none(),
            Command::MoveTo(targets.to_vec(), None),
        ),
        Entry::Separator,
    ];
    for folder in space.panel_folders() {
        if !folder.is_real_folder() {
            continue;
        }
        let label = format!("{}{}", " ".repeat(folder.depth * 3), folder.name);
        entries.push(Entry::check(
            label,
            icons::FOLDER,
            shared.as_deref() == Some(folder.id.as_str()),
            Command::MoveTo(targets.to_vec(), Some(folder.id.clone())),
        ));
    }
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        t(s::MySpace_NewFolder),
        icons::FOLDER_PLUS,
        Command::MoveToNewFolder(targets.to_vec()),
    ));
    entries
}

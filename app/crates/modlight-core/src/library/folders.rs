//! Folders as the shelf, the side panel and the breadcrumb show them
//! (`CloudFolderViewModel`, `FolderCrumbViewModel`, `LibraryNavViewModel`).

use std::collections::{HashMap, HashSet};

use modlight_api::Folder;

use crate::format::cloud_size;
use crate::icons;
use crate::l10n::{Arg, Key, format, s, t};

/// A real folder, or the shelf standing for files in no folder at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderKind {
    Folder,
    Unfiled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CloudFolder {
    pub id: String,
    pub name: String,
    /// The slug its public gallery is addressed by, when it has one.
    pub slug: String,
    pub parent_id: Option<String>,
    pub is_public: bool,
    pub sort_order: i64,
    pub kind: FolderKind,
    /// Files and bytes sitting directly in this folder.
    pub direct_count: i64,
    pub direct_size: i64,
    /// The same, with everything nested underneath rolled up.
    pub file_count: i64,
    pub size: i64,
    pub subfolder_count: i64,
    pub descendant_count: i64,
    /// How many levels down the tree this folder sits.
    pub depth: usize,
}

impl CloudFolder {
    pub fn new(folder: &Folder) -> CloudFolder {
        let id = folder.id.clone().unwrap_or_default();
        let name = folder
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| folder.slug.clone())
            .unwrap_or_else(|| id.clone());
        let direct_count = folder.file_count.max(0);
        let direct_size = folder.size.max(0);
        CloudFolder {
            id,
            name,
            slug: folder.slug.clone().unwrap_or_default(),
            parent_id: folder.parent_id.clone().filter(|p| !p.is_empty()),
            is_public: folder.is_public,
            sort_order: folder.sort_order,
            kind: FolderKind::Folder,
            direct_count,
            direct_size,
            file_count: direct_count,
            size: direct_size,
            subfolder_count: 0,
            descendant_count: 0,
            depth: 0,
        }
    }

    /// The "Unfiled" shelf, addressed by the server's root sentinel.
    pub fn unfiled(root_value: &str, file_count: i64, size: i64) -> CloudFolder {
        CloudFolder {
            id: root_value.to_string(),
            name: t(s::MySpace_Unfiled).to_string(),
            slug: String::new(),
            parent_id: None,
            is_public: false,
            sort_order: 0,
            kind: FolderKind::Unfiled,
            direct_count: file_count.max(0),
            direct_size: size.max(0),
            file_count: file_count.max(0),
            size: size.max(0),
            subfolder_count: 0,
            descendant_count: 0,
            depth: 0,
        }
    }

    pub fn is_real_folder(&self) -> bool {
        self.kind == FolderKind::Folder
    }

    /// An empty folder shows the open folder glyph.
    pub fn icon(&self) -> char {
        if self.kind == FolderKind::Unfiled {
            return icons::INBOX;
        }
        if self.file_count == 0 && self.subfolder_count == 0 {
            icons::FOLDER_OPEN
        } else {
            icons::FOLDER
        }
    }

    /// "Files: 4  ·  478,5 KB  ·  2 folders", or "Empty".
    pub fn meta(&self) -> String {
        let mut text = String::new();
        if self.file_count > 0 {
            text.push_str(&format(
                t(s::MySpace_FileCount),
                &[Arg::Int(self.file_count)],
            ));
            text.push_str("  ·  ");
            text.push_str(&cloud_size(self.size));
        } else if self.subfolder_count == 0 {
            text.push_str(t(s::MySpace_FolderEmpty));
        }
        if self.subfolder_count > 0 {
            if !text.is_empty() {
                text.push_str("  ·  ");
            }
            text.push_str(&format(
                t(s::MySpace_SubfolderCount),
                &[Arg::Int(self.subfolder_count)],
            ));
        }
        text
    }

    pub fn tooltip(&self) -> String {
        format!("{}\n{}", self.name, self.meta())
    }

    /// An upload landed here or a file left: adjust the direct tally.
    pub fn adjust_direct(&mut self, files: i64, bytes: i64) {
        self.direct_count = (self.direct_count + files).max(0);
        self.direct_size = (self.direct_size + bytes).max(0);
        if self.kind == FolderKind::Unfiled {
            self.file_count = self.direct_count;
            self.size = self.direct_size;
        }
    }
}

/// Roll the direct counts up the tree: every folder's totals include its subtree, and
/// each folder learns its depth and how many folders sit directly inside it. A cycle in
/// the parent links (which the server should never produce) is cut, not followed.
pub fn roll_up(folders: &mut [CloudFolder]) {
    let index: HashMap<String, usize> = folders
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.clone(), i))
        .collect();

    let mut files: HashMap<String, i64> = HashMap::new();
    let mut bytes: HashMap<String, i64> = HashMap::new();
    let mut subfolders: HashMap<String, i64> = HashMap::new();
    let mut descendants: HashMap<String, i64> = HashMap::new();
    let mut depths = vec![0usize; folders.len()];

    for (i, folder) in folders.iter().enumerate() {
        *files.entry(folder.id.clone()).or_default() += folder.direct_count;
        *bytes.entry(folder.id.clone()).or_default() += folder.direct_size;

        let mut seen = HashSet::new();
        seen.insert(folder.id.clone());
        let mut direct = true;
        let mut depth = 0;
        let mut parent = folder.parent_id.clone();
        while let Some(parent_id) = parent {
            let Some(&p) = index.get(&parent_id) else {
                break;
            };
            if !seen.insert(parent_id.clone()) {
                break;
            }
            depth += 1;
            *files.entry(parent_id.clone()).or_default() += folder.direct_count;
            *bytes.entry(parent_id.clone()).or_default() += folder.direct_size;
            *descendants.entry(parent_id.clone()).or_default() += 1;
            if direct {
                *subfolders.entry(parent_id.clone()).or_default() += 1;
                direct = false;
            }
            parent = folders[p].parent_id.clone();
        }
        depths[i] = depth;
    }

    for (i, folder) in folders.iter_mut().enumerate() {
        folder.depth = depths[i];
        folder.file_count = files
            .get(&folder.id)
            .copied()
            .unwrap_or(folder.direct_count);
        folder.size = bytes.get(&folder.id).copied().unwrap_or(folder.direct_size);
        folder.subfolder_count = subfolders.get(&folder.id).copied().unwrap_or(0);
        folder.descendant_count = descendants.get(&folder.id).copied().unwrap_or(0);
    }
}

/// One segment of the breadcrumb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    /// The folder, or `None` for "All files".
    pub id: Option<String>,
    pub name: String,
    pub is_current: bool,
    pub show_separator: bool,
}

/// "All files › Design › Logos" for the folder `current`.
pub fn crumbs(
    folders: &[CloudFolder],
    unfiled: Option<&CloudFolder>,
    current: Option<&str>,
) -> Vec<Crumb> {
    let find = |id: &str| -> Option<&CloudFolder> {
        unfiled
            .filter(|u| u.id == id)
            .or_else(|| folders.iter().find(|f| f.id == id))
    };
    let mut path: Vec<&CloudFolder> = Vec::new();
    let mut seen = HashSet::new();
    let mut at = current.and_then(find);
    while let Some(folder) = at {
        if !seen.insert(folder.id.clone()) {
            break;
        }
        path.push(folder);
        at = folder.parent_id.as_deref().and_then(find);
    }
    path.reverse();

    let mut out = vec![Crumb {
        id: None,
        name: t(s::MySpace_AllFiles).to_string(),
        is_current: path.is_empty(),
        show_separator: false,
    }];
    let last = path.len().saturating_sub(1);
    for (i, folder) in path.iter().enumerate() {
        out.push(Crumb {
            id: Some(folder.id.clone()),
            name: folder.name.clone(),
            is_current: i == last,
            show_separator: true,
        });
    }
    out
}

/// One entry of the library panel's top list: a filter over the whole library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LibraryView {
    pub name: Key,
    pub icon: char,
    /// The server's kind, or `None` for every kind.
    pub kind: Option<&'static str>,
    pub is_trash: bool,
}

/// All files, the six classes of file, then the trash.
pub const LIBRARY_VIEWS: [LibraryView; 8] = [
    LibraryView {
        name: s::MySpace_AllFiles,
        icon: icons::LAYERS,
        kind: None,
        is_trash: false,
    },
    LibraryView {
        name: s::MySpace_KindImage,
        icon: icons::IMAGE,
        kind: Some("image"),
        is_trash: false,
    },
    LibraryView {
        name: s::MySpace_KindVideo,
        icon: icons::FILM,
        kind: Some("video"),
        is_trash: false,
    },
    LibraryView {
        name: s::MySpace_KindAudio,
        icon: icons::MUSIC,
        kind: Some("audio"),
        is_trash: false,
    },
    LibraryView {
        name: s::MySpace_KindDocument,
        icon: icons::FILE_TEXT,
        kind: Some("document"),
        is_trash: false,
    },
    LibraryView {
        name: s::MySpace_KindArchive,
        icon: icons::ARCHIVE,
        kind: Some("archive"),
        is_trash: false,
    },
    LibraryView {
        name: s::MySpace_KindPaste,
        icon: icons::SCROLL_TEXT,
        kind: Some("paste"),
        is_trash: false,
    },
    LibraryView {
        name: s::MySpace_Trash,
        icon: icons::TRASH_2,
        kind: None,
        is_trash: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::{Language, set_thread_language};

    fn folder(id: &str, parent: Option<&str>, files: i64, size: i64) -> CloudFolder {
        CloudFolder::new(&Folder {
            id: Some(id.into()),
            name: Some(id.to_uppercase()),
            parent_id: parent.map(str::to_string),
            file_count: files,
            size,
            ..Default::default()
        })
    }

    #[test]
    fn totals_roll_up_the_tree() {
        let mut folders = vec![
            folder("a", None, 1, 100),
            folder("b", Some("a"), 2, 200),
            folder("c", Some("b"), 3, 300),
            folder("d", None, 0, 0),
            folder("loop1", Some("loop2"), 1, 1),
            folder("loop2", Some("loop1"), 1, 1),
        ];
        roll_up(&mut folders);
        let a = &folders[0];
        assert_eq!(
            (
                a.file_count,
                a.size,
                a.subfolder_count,
                a.descendant_count,
                a.depth
            ),
            (6, 600, 1, 2, 0)
        );
        let b = &folders[1];
        assert_eq!((b.file_count, b.subfolder_count, b.depth), (5, 1, 1));
        assert_eq!(folders[2].depth, 2);
        assert_eq!(folders[3].icon(), icons::FOLDER_OPEN);
        assert_eq!(folders[0].icon(), icons::FOLDER);
        assert_eq!(
            folders[4].file_count, 2,
            "a cycle is cut, not followed forever"
        );
    }

    #[test]
    fn meta_and_crumbs_read_like_the_shelf() {
        set_thread_language(Some(Language::English));
        let mut folders = vec![folder("a", None, 0, 0), folder("b", Some("a"), 2, 2048)];
        roll_up(&mut folders);
        assert_eq!(
            folders[0].meta(),
            format(t(s::MySpace_FileCount), &[Arg::Int(2)])
                + "  ·  2.0 KB  ·  "
                + &format(t(s::MySpace_SubfolderCount), &[Arg::Int(1)])
        );
        let empty = folder("e", None, 0, 0);
        assert_eq!(empty.meta(), t(s::MySpace_FolderEmpty));

        let trail = crumbs(&folders, None, Some("b"));
        let names: Vec<&str> = trail.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec![t(s::MySpace_AllFiles), "A", "B"]);
        assert!(trail[2].is_current && !trail[1].is_current && !trail[0].show_separator);
        assert_eq!(crumbs(&folders, None, None).len(), 1);
        set_thread_language(None);
    }
}

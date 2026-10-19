//! The quick task menu (`QuickTaskMenu`): a menu at the pointer offering "Continue",
//! the presets of `QuickTaskPresets` (`QuickTaskInfo`), "Edit this menu..." and "Cancel".
//! Upstream opens a context menu on a one-pixel window at the pointer; here the menu is
//! its own small window there.

use egui::{Pos2, Ui};
use modlight_core::config::AppConfig;
use modlight_core::icons;
use modlight_core::l10n::{s, t};
use modlight_core::tasks::settings::{AfterCaptureTasks, AfterUploadTasks, FlagNames};
use serde_json::Value;

use crate::commands::Command;
use crate::menus::Entry;
use crate::theme::Colors;

/// `QuickTaskInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickTaskInfo {
    pub name: Option<String>,
    pub after_capture: AfterCaptureTasks,
    pub after_upload: AfterUploadTasks,
}

impl QuickTaskInfo {
    fn new(
        name: &str,
        after_capture: AfterCaptureTasks,
        after_upload: AfterUploadTasks,
    ) -> QuickTaskInfo {
        QuickTaskInfo {
            name: Some(name.to_string()),
            after_capture,
            after_upload,
        }
    }

    /// `IsValid`: an entry without tasks is a separator.
    pub fn is_valid(&self) -> bool {
        self.after_capture != AfterCaptureTasks::None
    }

    /// `ToString`: the name, or the tasks' texts.
    pub fn text(&self) -> String {
        if let Some(name) = self.name.as_deref().filter(|n| !n.is_empty()) {
            return name.to_string();
        }
        let capture: Vec<&str> = crate::dialogs::after_capture::after_capture_options()
            .into_iter()
            .filter(|(task, _)| self.after_capture.has(*task))
            .map(|(_, label)| label)
            .collect();
        let mut result = capture.join(", ");
        if self.after_capture.has(AfterCaptureTasks::UploadImageToHost) {
            let upload: Vec<&str> = crate::dialogs::after_capture::after_upload_options()
                .into_iter()
                .filter(|(task, _)| self.after_upload.has(*task))
                .map(|(_, label)| label)
                .collect();
            if !upload.is_empty() {
                result.push_str(", ");
                result.push_str(&upload.join(", "));
            }
        }
        result
    }

    /// `DefaultPresets` (upstream's names are English in every language).
    pub fn defaults() -> Vec<QuickTaskInfo> {
        use AfterCaptureTasks as C;
        use AfterUploadTasks as U;
        vec![
            QuickTaskInfo::new(
                "Save, Upload, Copy URL",
                C::SaveImageToFile.with(C::UploadImageToHost),
                U::CopyURLToClipboard,
            ),
            QuickTaskInfo::new(
                "Save, Copy image",
                C::SaveImageToFile.with(C::CopyImageToClipboard),
                U::None,
            ),
            QuickTaskInfo::new(
                "Save, Copy image file",
                C::SaveImageToFile.with(C::CopyFileToClipboard),
                U::None,
            ),
            QuickTaskInfo::new(
                "Annotate, Save, Upload, Copy URL",
                C::AnnotateImage
                    .with(C::SaveImageToFile)
                    .with(C::UploadImageToHost),
                U::CopyURLToClipboard,
            ),
            QuickTaskInfo {
                name: None,
                after_capture: C::None,
                after_upload: U::None,
            },
            QuickTaskInfo::new(
                "Upload, Copy URL",
                C::UploadImageToHost,
                U::CopyURLToClipboard,
            ),
            QuickTaskInfo::new("Save", C::SaveImageToFile, U::None),
            QuickTaskInfo::new("Copy image", C::CopyImageToClipboard, U::None),
            QuickTaskInfo::new("Annotate", C::AnnotateImage, U::None),
        ]
    }

    /// `Settings.QuickTaskPresets`, upstream's defaults when the file has none.
    pub fn presets(config: &AppConfig) -> Vec<QuickTaskInfo> {
        match config.get("QuickTaskPresets") {
            Some(Value::Array(list)) => list
                .iter()
                .map(|item| QuickTaskInfo {
                    name: item.get("Name").and_then(Value::as_str).map(str::to_string),
                    after_capture: item
                        .get("AfterCaptureTasks")
                        .and_then(AfterCaptureTasks::parse_value)
                        .unwrap_or_default(),
                    after_upload: item
                        .get("AfterUploadTasks")
                        .and_then(AfterUploadTasks::parse_value)
                        .unwrap_or_default(),
                })
                .collect(),
            _ => QuickTaskInfo::defaults(),
        }
    }
}

/// What the menu answered: `None` closed, `Some(None)` "Continue", `Some(Some(..))` a preset.
pub type QuickChoice = Option<Option<(AfterCaptureTasks, AfterUploadTasks)>>;

pub struct QuickTaskMenu {
    pub presets: Vec<QuickTaskInfo>,
    /// Where the pointer was when the menu opened (desktop points, `-10` each way).
    pub at: Option<Pos2>,
}

impl QuickTaskMenu {
    pub fn new(config: &AppConfig) -> QuickTaskMenu {
        QuickTaskMenu {
            presets: QuickTaskInfo::presets(config),
            at: None,
        }
    }

    /// The entries of `ShowMenuCore`. The commands are placeholders the window reads by
    /// position: index 0 continue, 1..=n presets, then edit and cancel.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = vec![
            Entry::item(
                t(s::QuickTaskMenu_ShowMenu_Continue),
                icons::CIRCLE_PLAY,
                Command::QuickTask(0),
            ),
            Entry::Separator,
        ];
        if !self.presets.is_empty() {
            for (index, preset) in self.presets.iter().enumerate() {
                if preset.is_valid() {
                    entries.push(Entry::Item {
                        label: preset.text(),
                        icon: None,
                        enabled: true,
                        toggle: crate::menus::Toggle::None,
                        command: Some(Command::QuickTask(index + 1)),
                    });
                } else {
                    entries.push(Entry::Separator);
                }
            }
            entries.push(Entry::Separator);
        }
        entries.push(Entry::item(
            t(s::QuickTaskMenu_ShowMenu_Edit_this_menu___),
            icons::PENCIL,
            Command::QuickTask(usize::MAX - 1),
        ));
        entries.push(Entry::Separator);
        entries.push(Entry::item(
            t(s::QuickTaskMenu_ShowMenu_Cancel),
            icons::X,
            Command::QuickTask(usize::MAX),
        ));
        entries
    }

    /// Draw the menu; returns the command of the entry clicked.
    pub fn ui(&self, ui: &mut Ui, colors: &Colors) -> (Option<Command>, egui::Rect) {
        let entries = self.entries();
        let response =
            crate::menus::frame(colors).show(ui, |ui| crate::menus::entries(ui, colors, &entries));
        (response.inner, response.response.rect)
    }

    /// What a `Command::QuickTask(index)` means.
    pub fn choice(&self, index: usize) -> Option<QuickChoice> {
        match index {
            0 => Some(Some(None)),
            i if i == usize::MAX => Some(None),
            i if i == usize::MAX - 1 => None,
            i => self
                .presets
                .get(i - 1)
                .map(|p| Some(Some((p.after_capture, p.after_upload)))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::l10n::{Language, helpers as h, set_thread_language};

    #[test]
    fn presets_read_from_the_settings_or_the_defaults() {
        set_thread_language(Some(Language::English));
        let mut config = AppConfig::in_memory();
        let presets = QuickTaskInfo::presets(&config);
        assert_eq!(presets.len(), 9);
        assert!(!presets[4].is_valid());
        config.set(
            "QuickTaskPresets",
            serde_json::json!([{ "AfterCaptureTasks": "SaveImageToFile, UploadImageToHost", "AfterUploadTasks": "CopyURLToClipboard" }]),
        );
        let presets = QuickTaskInfo::presets(&config);
        assert_eq!(presets.len(), 1);
        let text = presets[0].text();
        assert!(text.contains(t(h::AfterCaptureTasks_SaveImageToFile)));
        assert!(text.contains(t(h::AfterUploadTasks_CopyURLToClipboard)));
        let menu = QuickTaskMenu { presets, at: None };
        assert_eq!(menu.choice(0), Some(Some(None)));
        assert_eq!(menu.choice(usize::MAX), Some(None));
        assert_eq!(menu.choice(usize::MAX - 1), None);
        assert!(matches!(menu.choice(1), Some(Some(Some(_)))));
        set_thread_language(None);
    }
}

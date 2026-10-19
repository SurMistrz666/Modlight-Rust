//! My space: the account's files and folders (`Presentation/MySpace`).

pub mod actions;
pub mod details;
pub mod drag;
pub mod drawer;
pub mod menus;
pub mod model;
pub mod pager;
pub mod view;

use egui::{Key, Modifiers};

use crate::commands::Command;
use crate::state::AppState;

impl AppState {
    /// The library's keys (`HandleMySpaceKey`, `HandleLibraryDeleteKey`), with the
    /// Finder's own spellings added where a Mac keyboard has no such key.
    ///
    /// * Backspace (no modifier), Alt+Left or Cmd+Up: up one level.
    /// * Cmd+B: the library panel. Cmd+A: every file on the page.
    /// * Delete (Fn+Backspace) or Cmd+Backspace: to the trash; with Shift, or in the
    ///   trash, for good (Cmd+Option+Backspace too).
    /// * Left, Right, Escape: the drawer, when it has the keyboard.
    /// * Escape: clear the selection.
    pub fn my_space_keys(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || ctx.egui_wants_keyboard_input() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            self.my_space_key(ctx, key, modifiers);
        }
    }

    fn my_space_key(&mut self, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
        let command = modifiers.mac_cmd || modifiers.command;
        let alt = modifiers.alt;
        if self.details.present && self.drawer_has_keys && !command && !alt {
            match key {
                Key::ArrowLeft => return self.details_step(-1, ctx),
                Key::ArrowRight => return self.details_step(1, ctx),
                Key::Escape => {
                    self.details.close(self.time);
                    return;
                }
                _ => {}
            }
        }
        let up = (key == Key::Backspace && !command && !alt)
            || (key == Key::ArrowLeft && alt)
            || (key == Key::ArrowUp && command);
        if up {
            if self.space.can_go_up() {
                self.push(Command::GoUp);
            }
            return;
        }
        if key == Key::B && command {
            self.push(Command::ToggleLibraryPanel);
            return;
        }
        if key == Key::A && command {
            if self.space.has_files() {
                self.push(Command::SelectAll);
            }
            return;
        }
        let delete = (key == Key::Delete && !command && !alt) || (key == Key::Backspace && command);
        if delete {
            let forever = self.space.trashed || modifiers.shift || (key == Key::Backspace && alt);
            let files = self.space.selected_ids();
            if !files.is_empty() {
                self.push(if forever {
                    Command::DeleteForever(files)
                } else {
                    Command::Trash(files)
                });
                return;
            }
            if let Some(folder) = self.space.selected_folder.clone()
                && !self.space.trashed
            {
                self.push(Command::DeleteFolder(folder));
            }
            return;
        }
        if key == Key::Escape && !self.space.clear_selection() && self.details.present {
            self.details.close(self.time);
        }
    }
}

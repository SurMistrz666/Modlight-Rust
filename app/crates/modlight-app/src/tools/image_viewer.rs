//! `ImageViewerWindow` and `ImageViewerViewModel`: a picture over the whole display on
//! the checkered background, never enlarged; the other pictures of its folder (or of the
//! list it was given) one key, click or wheel step away; the status line on top; any
//! click, Escape, Enter, Space or leaving the window closes it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Align2, Id, Rect, Sense, TextureHandle, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::icons;
use modlight_core::l10n::{helpers as h, t};
use modlight_core::names::file_helpers::is_image_file;

use super::kit;
use super::{ToolWindow, WindowSpec};
use crate::state::AppState;
use crate::theme::Weight;

pub struct ImageViewerWindow {
    pub key: u64,
    images: Vec<PathBuf>,
    index: usize,
    current: Option<Arc<RgbaImage>>,
    texture: Option<TextureHandle>,
    /// `CurrentImageFilePath` (a display name for a picture without a file).
    name: Option<String>,
    pub status: String,
    /// `SupportWrap`.
    pub wrap: bool,
    close_on_deactivate: bool,
    was_focused: bool,
    /// Where the window sits (the display it covers).
    pub screen: Rect,
}

impl ImageViewerWindow {
    /// An empty viewer covering `screen` (tests and QA fill it).
    pub fn new_on(screen: Rect) -> ImageViewerWindow {
        ImageViewerWindow::empty(0, screen)
    }

    fn empty(key: u64, screen: Rect) -> ImageViewerWindow {
        ImageViewerWindow {
            key,
            images: Vec::new(),
            index: 0,
            current: None,
            texture: None,
            name: None,
            status: String::new(),
            wrap: false,
            close_on_deactivate: true,
            was_focused: false,
            screen,
        }
    }

    pub fn has_image(&self) -> bool {
        self.current.is_some()
    }

    pub fn can_navigate(&self) -> bool {
        self.images.len() > 1
    }

    pub fn can_navigate_left(&self) -> bool {
        self.can_navigate() && (self.wrap || self.index > 0)
    }

    pub fn can_navigate_right(&self) -> bool {
        self.can_navigate() && (self.wrap || self.index + 1 < self.images.len())
    }

    /// `LoadFile`: the file and the other pictures of its folder.
    pub fn load_file(&mut self, file: &Path) -> bool {
        let text = file.to_string_lossy();
        if !file.is_file() || !is_image_file(&text) {
            return false;
        }
        let mut images: Vec<PathBuf> = file
            .parent()
            .and_then(|folder| std::fs::read_dir(folder).ok())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_file() && is_image_file(&p.to_string_lossy()))
                    .collect()
            })
            .unwrap_or_default();
        // `Directory.GetFiles` order: by name, as NTFS lists them.
        images.sort_by_key(|p| p.to_string_lossy().to_lowercase());
        let found = images.iter().position(|p| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(&file.to_string_lossy())
        });
        match found {
            Some(index) => {
                self.images = images;
                self.index = index;
            }
            None => {
                self.images = vec![file.to_path_buf()];
                self.index = 0;
            }
        }
        self.load_current()
    }

    /// `LoadFiles`: the existing pictures of a list, starting at `selected`.
    pub fn load_files(&mut self, files: &[PathBuf], selected: usize) -> bool {
        let mut images = Vec::new();
        let mut filtered = None;
        for (index, path) in files.iter().enumerate() {
            if !path.is_file() || !is_image_file(&path.to_string_lossy()) {
                continue;
            }
            if index == selected {
                filtered = Some(images.len());
            }
            images.push(path.clone());
        }
        if images.is_empty() {
            return false;
        }
        self.index = filtered.unwrap_or_else(|| selected.min(images.len() - 1));
        self.images = images;
        self.load_current()
    }

    /// `LoadEncodedImage`: a picture without a file.
    pub fn load_picture(&mut self, image: RgbaImage, name: Option<&str>) -> bool {
        if image.width() == 0 || image.height() == 0 {
            return false;
        }
        self.current = Some(Arc::new(image));
        self.texture = None;
        self.images.clear();
        self.index = 0;
        self.name = name.map(str::to_string);
        self.update_status();
        true
    }

    fn load_current(&mut self) -> bool {
        let Some(path) = self.images.get(self.index).cloned() else {
            return false;
        };
        self.name = Some(path.to_string_lossy().into_owned());
        match super::files::load_image(&path) {
            Some(image) => {
                self.current = Some(Arc::new(image));
                self.texture = None;
                self.update_status();
                true
            }
            None => {
                log::warn!("Failed to load image '{}'.", path.display());
                false
            }
        }
    }

    /// `Navigate(offset)`.
    pub fn navigate(&mut self, offset: i64) {
        if !self.can_navigate() {
            return;
        }
        let len = self.images.len() as i64;
        let mut next = self.index as i64 + offset;
        next = if self.wrap {
            (next + len).rem_euclid(len)
        } else {
            next.clamp(0, len - 1)
        };
        if next as usize != self.index {
            self.index = next as usize;
            self.load_current();
        }
    }

    /// `UpdateStatus`: "2 / 7  |  name.png  |  1920 × 1080".
    fn update_status(&mut self) {
        let mut parts = Vec::new();
        if self.can_navigate() {
            parts.push(format!("{} / {}", self.index + 1, self.images.len()));
        }
        if let Some(name) = self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            let file_name = Path::new(name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string());
            if file_name.chars().count() > 128 {
                let short: String = file_name.chars().take(125).collect();
                parts.push(format!("{short}..."));
            } else {
                parts.push(file_name);
            }
        }
        if let Some(image) = &self.current {
            parts.push(format!("{} × {}", image.width(), image.height()));
        }
        self.status = parts.join("  |  ");
    }
}

impl ToolWindow for ImageViewerWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec {
            title: t(h::ImageViewerWindow_Title).to_string(),
            size: self.screen.size(),
            min_size: None,
            topmost: true,
            fit_height: false,
            screen: Some(self.screen),
            position: None,
        }
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = ui.max_rect();
        kit::paint_checkered_big(ui, &colors, full);
        let mut close = false;

        // The picture, never enlarged.
        let response = ui.interact(full, Id::new(("image-viewer", self.key)), Sense::click());
        if let Some(image) = self.current.clone() {
            if self.texture.is_none() {
                self.texture = Some(kit::texture(
                    ui.ctx(),
                    &format!("image-viewer-{}", self.key),
                    &image,
                    false,
                ));
            }
            if let Some(texture) = &self.texture {
                kit::image_uniform(ui, texture, full, false);
            }
        }
        if response.clicked() || response.secondary_clicked() {
            close = true;
        }
        // `PointerWheelChanged`: one step per wheel event.
        let steps: Vec<f32> = ui.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } if delta.y != 0.0 => Some(delta.y),
                    _ => None,
                })
                .collect()
        });
        if response.hovered() {
            for delta in steps {
                if delta > 0.0 && self.can_navigate_left() {
                    self.navigate(-1);
                } else if delta < 0.0 && self.can_navigate_right() {
                    self.navigate(1);
                }
            }
        }

        // The status line.
        if self.has_image() {
            let text_w = kit::text_width(ui, &self.status, 13.0, Weight::Normal);
            let panel = Rect::from_min_size(
                pos2(full.center().x - (text_w + 22.0) / 2.0, full.top() + 16.0),
                vec2(text_w + 22.0, 16.0 + 16.0),
            );
            kit::panel(ui, &colors, panel);
            kit::text_at(
                ui,
                panel,
                Align2::CENTER_CENTER,
                &self.status,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
            );
        }

        // Previous and next.
        let left = Rect::from_min_size(
            pos2(full.left() + 20.0, full.center().y - 48.0),
            vec2(72.0, 96.0),
        );
        let right = Rect::from_min_size(
            pos2(full.right() - 20.0 - 72.0, full.center().y - 48.0),
            vec2(72.0, 96.0),
        );
        let look = crate::widgets::Look::button(&colors).icon(38.0, None);
        if self.can_navigate_left() {
            let mut child = kit::child(ui, left);
            if crate::widgets::button_at(
                &mut child,
                &colors,
                &look.clone().width(72.0).height(96.0),
                crate::widgets::Content::icon(icons::CHEVRON_LEFT),
                true,
                left,
            )
            .clicked()
            {
                self.navigate(-1);
                close = false;
            }
        }
        if self.can_navigate_right() {
            let mut child = kit::child(ui, right);
            if crate::widgets::button_at(
                &mut child,
                &colors,
                &look.width(72.0).height(96.0),
                crate::widgets::Content::icon(icons::CHEVRON_RIGHT),
                true,
                right,
            )
            .clicked()
            {
                self.navigate(1);
                close = false;
            }
        }
        if (self.can_navigate_left()
            && left.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default()))
            || (self.can_navigate_right()
                && right.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default()))
        {
            close = false;
        }

        // Keys.
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowLeft) {
                self.navigate(-1);
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                self.navigate(1);
            }
            if i.key_pressed(egui::Key::Escape)
                || i.key_pressed(egui::Key::Enter)
                || i.key_pressed(egui::Key::Space)
            {
                close = true;
            }
        });
        // `Deactivated`: leaving the window closes it.
        let focused = ui.ctx().input(|i| i.viewport().focused);
        if focused == Some(true) {
            self.was_focused = true;
        } else if focused == Some(false) && self.was_focused && self.close_on_deactivate {
            close = true;
        }
        close
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl AppState {
    /// The display the viewer covers: the main one.
    fn viewer_screen(&self) -> Rect {
        let screens = self.captures.source.screens();
        let screen = screens
            .iter()
            .find(|s| s.primary)
            .or_else(|| screens.first());
        match screen {
            Some(s) => Rect::from_min_size(
                pos2(s.bounds.x as f32, s.bounds.y as f32),
                vec2(s.bounds.width as f32, s.bounds.height as f32),
            ),
            None => Rect::from_min_size(pos2(0.0, 0.0), vec2(1440.0, 900.0)),
        }
    }

    /// `ImageViewerWindowIntegration.ShowImage()`: pick a picture first.
    pub fn open_image_viewer(&mut self) {
        let Some(path) = self.tool_open_file(
            t(h::ImageViewerWindow_Open_image),
            &[("Images", &super::files::IMAGE_EXTENSIONS)],
        ) else {
            return;
        };
        self.open_image_viewer_file(&path);
    }

    /// `ShowImage(filePath)`.
    pub fn open_image_viewer_file(&mut self, path: &Path) {
        let screen = self.viewer_screen();
        let mut window = ImageViewerWindow::empty(0, screen);
        if window.load_file(path) {
            self.tools.open_keyed(|key| {
                window.key = key;
                Box::new(window)
            });
        }
    }

    /// `ShowImage(filePaths, selectedIndex)`.
    pub fn open_image_viewer_files(&mut self, files: &[PathBuf], selected: usize) {
        let screen = self.viewer_screen();
        let mut window = ImageViewerWindow::empty(0, screen);
        if window.load_files(files, selected) {
            self.tools.open_keyed(|key| {
                window.key = key;
                Box::new(window)
            });
        }
    }

    /// `ShowImage(imageData, displayName)`.
    pub fn open_image_viewer_picture(&mut self, image: RgbaImage, name: &str) {
        let screen = self.viewer_screen();
        let mut window = ImageViewerWindow::empty(0, screen);
        if window.load_picture(image, Some(name)) {
            self.tools.open_keyed(|key| {
                window.key = key;
                Box::new(window)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, w) in [("b.png", 3), ("a.png", 2), ("c.jpg", 4)] {
            modlight_tools::images::save(&RgbaImage::new(w, 1), &dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        dir
    }

    #[test]
    fn a_file_brings_its_folder() {
        let dir = folder();
        let mut viewer = ImageViewerWindow::empty(1, Rect::NOTHING);
        assert!(viewer.load_file(&dir.path().join("b.png")));
        assert_eq!(viewer.status, "2 / 3  |  b.png  |  3 × 1");
        assert!(viewer.can_navigate_left() && viewer.can_navigate_right());
        viewer.navigate(1);
        assert_eq!(viewer.status, "3 / 3  |  c.jpg  |  4 × 1");
        viewer.navigate(1);
        assert_eq!(viewer.status, "3 / 3  |  c.jpg  |  4 × 1");
        assert!(!viewer.can_navigate_right());
        assert!(!viewer.load_file(&dir.path().join("notes.txt")));
    }

    #[test]
    fn a_list_skips_what_is_not_a_picture() {
        let dir = folder();
        let files = vec![
            dir.path().join("notes.txt"),
            dir.path().join("missing.png"),
            dir.path().join("c.jpg"),
            dir.path().join("a.png"),
        ];
        let mut viewer = ImageViewerWindow::empty(1, Rect::NOTHING);
        assert!(viewer.load_files(&files, 3));
        assert_eq!(viewer.status, "2 / 2  |  a.png  |  2 × 1");
        let mut viewer = ImageViewerWindow::empty(1, Rect::NOTHING);
        assert!(!viewer.load_files(&files[..2], 0));
    }

    #[test]
    fn a_picture_without_a_file() {
        let mut viewer = ImageViewerWindow::empty(1, Rect::NOTHING);
        assert!(viewer.load_picture(RgbaImage::new(5, 6), Some("QR code")));
        assert_eq!(viewer.status, "QR code  |  5 × 6");
        assert!(!viewer.can_navigate());
    }
}

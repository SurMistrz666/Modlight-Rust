//! The customisable toolbar (`ToolbarCustomizationItemViewModel`,
//! `MainViewModel.ToolbarCustomization`, `ToolbarHotkeyHelper`).

use modlight_core::l10n::{Key, editor as s, t, tf};

use crate::model::EditorTool;
use crate::options::ToolbarItemOptions;
use crate::ui::icons;

use super::{Editor, SidePanel};

pub const FILE_ITEM: &str = "File";
pub const BACKGROUND_ITEM: &str = "Background";
pub const IMAGE_EFFECTS_ITEM: &str = "ImageEffects";

struct Definition {
    id: &'static str,
    tool: Option<EditorTool>,
    name: Key,
    icon: char,
    hotkey: &'static str,
    hotkey_editable: bool,
    visibility_editable: bool,
    begin_group: bool,
}

const fn tool(
    tool: EditorTool,
    id: &'static str,
    name: Key,
    icon: char,
    hotkey: &'static str,
    begin_group: bool,
) -> Definition {
    Definition {
        id,
        tool: Some(tool),
        name,
        icon,
        hotkey,
        hotkey_editable: true,
        visibility_editable: true,
        begin_group,
    }
}

fn definitions() -> [Definition; 23] {
    use EditorTool::*;
    [
        Definition {
            id: FILE_ITEM,
            tool: None,
            name: s::ToolbarCustomizationItemViewModel_File,
            icon: icons::FILE_MENU,
            hotkey: "",
            hotkey_editable: true,
            visibility_editable: false,
            begin_group: false,
        },
        tool(
            Select,
            "Select",
            s::ToolbarCustomizationItemViewModel_Select,
            icons::TOOL_SELECT,
            "V",
            true,
        ),
        tool(
            Rectangle,
            "Rectangle",
            s::ToolbarCustomizationItemViewModel_Rectangle,
            icons::TOOL_RECTANGLE,
            "R",
            false,
        ),
        tool(
            Ellipse,
            "Ellipse",
            s::ToolbarCustomizationItemViewModel_Ellipse,
            icons::TOOL_ELLIPSE,
            "E",
            false,
        ),
        tool(
            Line,
            "Line",
            s::ToolbarCustomizationItemViewModel_Line,
            icons::TOOL_LINE,
            "L",
            false,
        ),
        tool(
            Arrow,
            "Arrow",
            s::ToolbarCustomizationItemViewModel_Arrow,
            icons::TOOL_ARROW,
            "A",
            false,
        ),
        tool(
            Freehand,
            "Freehand",
            s::ToolbarCustomizationItemViewModel_Freehand,
            icons::TOOL_FREEHAND,
            "F",
            false,
        ),
        tool(
            Text,
            "Text",
            s::ToolbarCustomizationItemViewModel_Text,
            icons::TOOL_TEXT,
            "T",
            false,
        ),
        tool(
            SpeechBalloon,
            "SpeechBalloon",
            s::ToolbarCustomizationItemViewModel_SpeechBalloon,
            icons::TOOL_SPEECH_BALLOON,
            "O",
            false,
        ),
        tool(
            Step,
            "Step",
            s::ToolbarCustomizationItemViewModel_Step,
            icons::TOOL_STEP,
            "N",
            false,
        ),
        tool(
            Image,
            "Image",
            s::ToolbarCustomizationItemViewModel_Image,
            icons::TOOL_IMAGE,
            "I",
            false,
        ),
        tool(
            Emoji,
            "Emoji",
            s::ToolbarCustomizationItemViewModel_Emoji,
            icons::TOOL_EMOJI,
            "J",
            false,
        ),
        tool(
            Cursor,
            "Cursor",
            s::ToolbarCustomizationItemViewModel_Cursor,
            icons::TOOL_CURSOR,
            "K",
            false,
        ),
        tool(
            Highlight,
            "Highlight",
            s::ToolbarCustomizationItemViewModel_Highlight,
            icons::TOOL_HIGHLIGHT,
            "H",
            false,
        ),
        tool(
            SmartEraser,
            "SmartEraser",
            s::ToolbarCustomizationItemViewModel_SmartEraser,
            icons::TOOL_SMART_ERASER,
            "W",
            false,
        ),
        tool(
            Blur,
            "Blur",
            s::ToolbarCustomizationItemViewModel_Blur,
            icons::TOOL_BLUR,
            "B",
            false,
        ),
        tool(
            Pixelate,
            "Pixelate",
            s::ToolbarCustomizationItemViewModel_Pixelate,
            icons::TOOL_PIXELATE,
            "P",
            false,
        ),
        tool(
            Magnify,
            "Magnify",
            s::ToolbarCustomizationItemViewModel_Magnify,
            icons::TOOL_MAGNIFY,
            "M",
            false,
        ),
        tool(
            Spotlight,
            "Spotlight",
            s::ToolbarCustomizationItemViewModel_Spotlight,
            icons::TOOL_SPOTLIGHT,
            "S",
            false,
        ),
        tool(
            Crop,
            "Crop",
            s::ToolbarCustomizationItemViewModel_Crop,
            icons::TOOL_CROP,
            "C",
            true,
        ),
        tool(
            CutOut,
            "CutOut",
            s::ToolbarCustomizationItemViewModel_CutOut,
            icons::TOOL_CUT_OUT,
            "U",
            false,
        ),
        Definition {
            id: BACKGROUND_ITEM,
            tool: None,
            name: s::ToolbarCustomizationItemViewModel_Background,
            icon: icons::PANEL_BACKGROUND,
            hotkey: "",
            hotkey_editable: true,
            visibility_editable: true,
            begin_group: false,
        },
        Definition {
            id: IMAGE_EFFECTS_ITEM,
            tool: None,
            name: s::ToolbarCustomizationItemViewModel_ImageEffects,
            icon: icons::PANEL_EFFECTS,
            hotkey: "",
            hotkey_editable: true,
            visibility_editable: true,
            begin_group: false,
        },
    ]
}

/// One toolbar entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarItem {
    pub id: String,
    pub tool: Option<EditorTool>,
    pub name: Key,
    pub icon: char,
    pub hotkey: String,
    pub is_visible: bool,
    pub begin_group: bool,
    pub hotkey_editable: bool,
    pub visibility_editable: bool,
    pub is_active: bool,
}

impl ToolbarItem {
    pub fn is_file_menu(&self) -> bool {
        self.id.eq_ignore_ascii_case(FILE_ITEM)
    }

    pub fn name(&self) -> &'static str {
        t(self.name)
    }

    /// `ToolTip`: the name with its hotkey; the effects button adds its right-click hint.
    pub fn tooltip(&self) -> String {
        let hotkey = self.hotkey.trim();
        let header = if self.hotkey_editable && !hotkey.is_empty() {
            format!("{} ({})", self.name(), display_hotkey(hotkey))
        } else {
            self.name().to_string()
        };
        if self.id == IMAGE_EFFECTS_ITEM {
            return tf(
                s::ToolbarCustomizationItemViewModel_FavoriteImageEffectsRightClick,
                &[header.into()],
            );
        }
        header
    }

    /// `IsGroupSeparatorVisible`.
    pub fn separator_visible(&self) -> bool {
        self.is_visible && self.begin_group
    }

    /// `ToOptions`.
    pub fn to_options(&self) -> ToolbarItemOptions {
        ToolbarItemOptions {
            id: self.id.clone(),
            begin_group: Some(self.begin_group),
            is_visible: !self.visibility_editable || self.is_visible,
            hotkey: if self.hotkey_editable {
                self.hotkey.trim().to_string()
            } else {
                String::new()
            },
        }
    }
}

fn item(
    d: &Definition,
    visible: bool,
    hotkey: Option<&str>,
    begin_group: Option<bool>,
) -> ToolbarItem {
    ToolbarItem {
        id: d.id.to_string(),
        tool: d.tool,
        name: d.name,
        icon: d.icon,
        hotkey: if d.hotkey_editable {
            hotkey.unwrap_or(d.hotkey).to_string()
        } else {
            String::new()
        },
        is_visible: if d.visibility_editable { visible } else { true },
        begin_group: begin_group.unwrap_or(d.begin_group),
        hotkey_editable: d.hotkey_editable,
        visibility_editable: d.visibility_editable,
        is_active: false,
    }
}

/// `CreateDefaultItems`.
pub fn default_items() -> Vec<ToolbarItem> {
    definitions()
        .iter()
        .map(|d| item(d, true, None, None))
        .collect()
}

/// `CreateFromOptions`: the stored order, then any items it lacks.
pub fn items_from_options(options: &[ToolbarItemOptions]) -> Vec<ToolbarItem> {
    if options.is_empty() {
        return default_items();
    }
    let defs = definitions();
    let mut items = Vec::new();
    let mut used: Vec<&str> = Vec::new();
    for o in options {
        if let Some(d) = defs.iter().find(|d| d.id.eq_ignore_ascii_case(&o.id)) {
            items.push(item(d, o.is_visible, Some(&o.hotkey), o.begin_group));
            used.push(d.id);
        }
    }
    for d in defs.iter().filter(|d| !used.contains(&d.id)) {
        items.push(item(d, true, None, None));
    }
    if items.is_empty() {
        default_items()
    } else {
        items
    }
}

/// `UpdateToolbarActiveStates`.
pub fn update_active_states(editor: &mut Editor) {
    let tool = editor.tools.active_tool;
    let settings = editor.is_settings_panel_open;
    let effects = editor.is_effects_panel_open && editor.effects_panel != Some(SidePanel::Options);
    for item in &mut editor.toolbar_items {
        item.is_active = match item.tool {
            Some(t) => t == tool,
            None => match item.id.as_str() {
                BACKGROUND_ITEM => settings,
                IMAGE_EFFECTS_ITEM => effects,
                _ => false,
            },
        };
    }
}

/// The key of a hotkey, as egui names keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub key: egui::Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// `ToolbarHotkeyHelper.TryParse`: `Ctrl+Shift+R` style, the key last.
pub fn parse_hotkey(text: &str) -> Option<Hotkey> {
    let parts: Vec<&str> = text
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let (last, mods) = parts.split_last()?;
    let mut hotkey = Hotkey {
        key: key_from_name(last)?,
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
    };
    for m in mods {
        match m.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => hotkey.ctrl = true,
            "SHIFT" => hotkey.shift = true,
            "ALT" => hotkey.alt = true,
            "WIN" | "META" => hotkey.meta = true,
            _ => return None,
        }
    }
    Some(hotkey)
}

/// Avalonia `Key` names for the keys a toolbar hotkey may use.
pub fn key_from_name(name: &str) -> Option<egui::Key> {
    let upper = name.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.chars().next()?;
        if c.is_ascii_alphabetic() {
            return egui::Key::from_name(&c.to_string());
        }
    }
    if let Some(digit) = upper.strip_prefix('D').filter(|d| d.len() == 1) {
        return egui::Key::from_name(digit);
    }
    if let Some(n) = upper.strip_prefix('F')
        && let Ok(n) = n.parse::<u8>()
        && (1..=20).contains(&n)
    {
        return egui::Key::from_name(&format!("F{n}"));
    }
    match upper.as_str() {
        "SPACE" => Some(egui::Key::Space),
        "TAB" => Some(egui::Key::Tab),
        "INSERT" => Some(egui::Key::Insert),
        "HOME" => Some(egui::Key::Home),
        "END" => Some(egui::Key::End),
        "PAGEUP" => Some(egui::Key::PageUp),
        "PAGEDOWN" | "NEXT" => Some(egui::Key::PageDown),
        "OEMMINUS" => Some(egui::Key::Minus),
        "OEMPLUS" => Some(egui::Key::Plus),
        "OEMCOMMA" => Some(egui::Key::Comma),
        "OEMPERIOD" => Some(egui::Key::Period),
        _ => None,
    }
}

/// `ToolbarHotkeyHelper.Format`.
pub fn format_hotkey(h: &Hotkey) -> String {
    let mut parts = Vec::new();
    if h.ctrl {
        parts.push("Ctrl".to_string());
    }
    if h.shift {
        parts.push("Shift".to_string());
    }
    if h.alt {
        parts.push("Alt".to_string());
    }
    if h.meta {
        parts.push("Meta".to_string());
    }
    parts.push(key_display_name(h.key));
    parts.join("+")
}

fn key_display_name(key: egui::Key) -> String {
    let name = key.name();
    if name.len() == 1 && name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return format!("D{name}");
    }
    name.to_string()
}

/// How a stored hotkey is shown: as upstream writes it on Windows; on a Mac `Ctrl` reads
/// as `Cmd` and `Alt` as `Option`, the keys they stand for.
pub fn display_hotkey(text: &str) -> String {
    if cfg!(windows) {
        return text.to_string();
    }
    text.split('+')
        .map(|p| match p.trim() {
            "Ctrl" | "Control" | "CTRL" => "Cmd",
            "Alt" => "Option",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// `ToolbarHotkeyHelper.Matches` against the pressed key and modifiers (Cmd counts as
/// Ctrl, as everywhere in the port).
pub fn matches(text: &str, key: egui::Key, modifiers: egui::Modifiers) -> bool {
    let Some(h) = parse_hotkey(text) else {
        return false;
    };
    h.key == key
        && h.ctrl == (modifiers.command || modifiers.ctrl)
        && h.shift == modifiers.shift
        && h.alt == modifiers.alt
        && !h.meta
}

impl Editor {
    /// `TrySelectToolForToolbarHotkey`.
    pub fn try_toolbar_hotkey(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        let found = self
            .toolbar_items
            .iter()
            .find(|i| i.is_visible && i.hotkey_editable && matches(&i.hotkey, key, modifiers))
            .cloned();
        match found {
            Some(item) => {
                self.execute_toolbar_item(&item);
                true
            }
            None => false,
        }
    }

    /// `ExecuteToolbarItem`.
    pub fn execute_toolbar_item(&mut self, item: &ToolbarItem) {
        if let Some(tool) = item.tool {
            self.select_tool(tool);
            return;
        }
        match item.id.as_str() {
            FILE_ITEM => self.input.file_menu_requested = true,
            BACKGROUND_ITEM => {
                let open = !self.is_settings_panel_open;
                self.set_settings_panel_open(open);
            }
            IMAGE_EFFECTS_ITEM => self.toggle_effects_panel(),
            _ => {}
        }
    }

    /// `OpenToolbarCustomizationDialog`.
    pub fn open_toolbar_customization(&mut self) {
        if self.modal.is_none() {
            self.modal = Some(super::Modal::ToolbarCustomization(
                self.toolbar_items.clone(),
            ));
        }
    }

    /// `ApplyToolbarCustomizationItems(items, persist: true)`.
    pub fn apply_toolbar_items(&mut self, items: Vec<ToolbarItem>) {
        self.toolbar_items = items;
        self.options.toolbar_items = self.toolbar_items.iter().map(|i| i.to_options()).collect();
        update_active_states(self);
        self.modal = None;
    }

    pub fn visible_toolbar_items(&self) -> impl Iterator<Item = &ToolbarItem> {
        self.toolbar_items.iter().filter(|i| i.is_visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_upstream_order_and_hotkeys() {
        let items = default_items();
        assert_eq!(items.len(), 23);
        assert_eq!(items[0].id, "File");
        assert_eq!(items[1].hotkey, "V");
        assert!(items[1].begin_group);
        let crop = items.iter().find(|i| i.id == "Crop").unwrap();
        assert!(crop.begin_group);
        assert_eq!(crop.hotkey, "C");
    }

    #[test]
    fn options_keep_their_order_and_add_missing_items() {
        let options = vec![
            ToolbarItemOptions {
                id: "Arrow".into(),
                begin_group: Some(true),
                is_visible: false,
                hotkey: "Ctrl+Shift+A".into(),
            },
            ToolbarItemOptions {
                id: "Unknown".into(),
                ..ToolbarItemOptions::default()
            },
        ];
        let items = items_from_options(&options);
        assert_eq!(items[0].id, "Arrow");
        assert!(!items[0].is_visible);
        assert_eq!(items.len(), 23);
        assert!(matches(
            "Ctrl+Shift+A",
            egui::Key::A,
            egui::Modifiers {
                command: true,
                mac_cmd: true,
                shift: true,
                ..egui::Modifiers::NONE
            }
        ));
        assert!(!matches("A", egui::Key::A, egui::Modifiers::SHIFT));
    }

    #[test]
    fn hotkeys_show_as_the_system_names_them() {
        if cfg!(windows) {
            assert_eq!(display_hotkey("Ctrl+Shift+S"), "Ctrl+Shift+S");
            assert_eq!(display_hotkey("Alt+D2"), "Alt+D2");
        } else {
            assert_eq!(display_hotkey("Ctrl+Shift+S"), "Cmd+Shift+S");
            assert_eq!(display_hotkey("Alt+D2"), "Option+D2");
        }
    }
}

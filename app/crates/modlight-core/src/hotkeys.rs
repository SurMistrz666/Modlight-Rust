//! Hotkeys (`HotkeyType`, `HotkeyInfo`, `HotkeySettings`, `HotkeysConfig`,
//! `HotkeyManager.GetDefaultHotkeyList`) without the system registration, which is the
//! platform's (`RegisterHotKey` in `modlight-windows`, `modlight-macos::hotkeys`).
//!
//! Bindings are stored exactly as the Windows app stores them, as WinForms `Keys` values
//! (`"D2, Shift, Control"`), so the synced hotkeys document stays one document for every
//! installation of an account. On Windows everything reads as upstream: Ctrl, Shift, Alt
//! and Win, Print Screen, Scroll Lock and Pause, and upstream's Print Screen defaults. A
//! Mac reads the modifiers the way its keyboard has them (MACOS.md): Control is Command,
//! Alt is Option, Win is Control; Print Screen, Scroll Lock and Pause are F13, F14 and
//! F15, the keys in their place on an Apple keyboard.

use serde_json::{Map, Value, json};

use crate::config::AppConfig;
use crate::l10n::{self, Key, helpers, t};
use crate::tasks::settings::named_enum;

named_enum! {
    /// `HotkeyType`.
    HotkeyType {
        None,
        FileUpload, FolderUpload, ClipboardUpload, ClipboardUploadWithContentViewer, UploadText,
        UploadURL, DragDropUpload, ShortenURL, StopUploads,
        PrintScreen, ActiveWindow, CustomWindow, ActiveMonitor, RectangleRegion, RectangleLight,
        RectangleTransparent, CustomRegion, LastRegion, ScrollingCapture, AutoCapture,
        StartAutoCapture, StopAutoCapture,
        ScreenRecorder, ScreenRecorderActiveWindow, ScreenRecorderCustomRegion, StartScreenRecorder,
        ScreenRecorderGIF, ScreenRecorderGIFActiveWindow, ScreenRecorderGIFCustomRegion,
        StartScreenRecorderGIF, StopScreenRecording, PauseScreenRecording, AbortScreenRecording,
        ColorPicker, ScreenColorPicker, PinToScreen, PinToScreenFromScreen, PinToScreenFromClipboard,
        PinToScreenFromFile, PinToScreenCloseAll, ImageEditor, ImageEffects, ImageViewer,
        BackgroundRemover, ImageCombiner, VideoConverter, AnalyzeImage, OCR, QRCode,
        QRCodeDecodeFromScreen, QRCodeScanRegion, Metadata, StripMetadata, IndexFolder,
        ActiveWindowBorderless, ActiveWindowTopMost,
        DisableHotkeys, OpenMainWindow, OpenScreenshotsFolder, OpenHistory, OpenImageHistory,
        ToggleTrayMenu, ExitShareX
    } default None
}

/// `[Category]` of a hotkey type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyCategory {
    None,
    Upload,
    ScreenCapture,
    ScreenRecord,
    Tools,
    Other,
}

impl HotkeyCategory {
    pub fn text(self) -> &'static str {
        match self {
            HotkeyCategory::None => "",
            HotkeyCategory::Upload => t(helpers::HotkeyType_Category_Upload),
            HotkeyCategory::ScreenCapture => t(helpers::HotkeyType_Category_ScreenCapture),
            HotkeyCategory::ScreenRecord => t(helpers::HotkeyType_Category_ScreenRecord),
            HotkeyCategory::Tools => t(helpers::HotkeyType_Category_Tools),
            HotkeyCategory::Other => t(helpers::HotkeyType_Category_Other),
        }
    }
}

impl HotkeyType {
    pub fn category(self) -> HotkeyCategory {
        use HotkeyType::*;
        match self {
            None => HotkeyCategory::None,
            FileUpload
            | FolderUpload
            | ClipboardUpload
            | ClipboardUploadWithContentViewer
            | UploadText
            | UploadURL
            | DragDropUpload
            | ShortenURL
            | StopUploads => HotkeyCategory::Upload,
            PrintScreen | ActiveWindow | CustomWindow | ActiveMonitor | RectangleRegion
            | RectangleLight | RectangleTransparent | CustomRegion | LastRegion
            | ScrollingCapture | AutoCapture | StartAutoCapture | StopAutoCapture => {
                HotkeyCategory::ScreenCapture
            }
            ScreenRecorder
            | ScreenRecorderActiveWindow
            | ScreenRecorderCustomRegion
            | StartScreenRecorder
            | ScreenRecorderGIF
            | ScreenRecorderGIFActiveWindow
            | ScreenRecorderGIFCustomRegion
            | StartScreenRecorderGIF
            | StopScreenRecording
            | PauseScreenRecording
            | AbortScreenRecording => HotkeyCategory::ScreenRecord,
            DisableHotkeys
            | OpenMainWindow
            | OpenScreenshotsFolder
            | OpenHistory
            | OpenImageHistory
            | ToggleTrayMenu
            | ExitShareX => HotkeyCategory::Other,
            _ => HotkeyCategory::Tools,
        }
    }

    /// The localized name (`GetLocalizedDescription`).
    pub fn text(self) -> String {
        l10n::enum_text("HotkeyType", self.name(), None)
    }

    /// `TaskHelpers.FindMenuLucideIcon`.
    pub fn icon(self) -> char {
        use crate::icons as i;
        use HotkeyType::*;
        match self {
            None => i::CIRCLE_DASHED,
            FileUpload => i::FILE_UP,
            FolderUpload => i::FOLDER_UP,
            ClipboardUpload => i::CLIPBOARD,
            ClipboardUploadWithContentViewer => i::CLIPBOARD_LIST,
            UploadText => i::FILE_TEXT,
            UploadURL => i::LINK,
            DragDropUpload => i::MOUSE_POINTER_2,
            ShortenURL => i::LINK_2,
            StopUploads => i::CIRCLE_STOP,
            PrintScreen => i::MONITOR,
            ActiveWindow => i::APP_WINDOW,
            CustomWindow => i::SCAN,
            ActiveMonitor => i::MONITOR,
            RectangleRegion => i::SCAN,
            RectangleLight => i::SQUARE,
            RectangleTransparent => i::SQUARE_DASHED,
            CustomRegion => i::SCAN_LINE,
            LastRegion => i::LAYERS,
            ScrollingCapture => i::SCROLL_TEXT,
            AutoCapture => i::CLOCK,
            StartAutoCapture => i::CIRCLE_PLAY,
            StopAutoCapture => i::TIMER_OFF,
            ScreenRecorder => i::VIDEO,
            ScreenRecorderActiveWindow => i::APP_WINDOW,
            ScreenRecorderCustomRegion => i::CROP,
            StartScreenRecorder => i::CIRCLE_PLAY,
            ScreenRecorderGIF => i::FILM,
            ScreenRecorderGIFActiveWindow => i::FILM,
            ScreenRecorderGIFCustomRegion => i::CROP,
            StartScreenRecorderGIF => i::CIRCLE_PLAY,
            StopScreenRecording => i::SQUARE_STOP,
            PauseScreenRecording => i::CIRCLE_PAUSE,
            AbortScreenRecording => i::CIRCLE_X,
            ColorPicker => i::PALETTE,
            ScreenColorPicker => i::PIPETTE,
            PinToScreen => i::PIN,
            PinToScreenFromScreen => i::PICTURE_IN_PICTURE,
            PinToScreenFromClipboard => i::CLIPBOARD,
            PinToScreenFromFile => i::FILE_IMAGE,
            PinToScreenCloseAll => i::PIN_OFF,
            ImageEditor => i::IMAGE,
            ImageEffects => i::WAND_SPARKLES,
            ImageViewer => i::EYE,
            BackgroundRemover => i::ERASER,
            ImageCombiner => i::COMBINE,
            VideoConverter => i::FILE_VIDEO,
            AnalyzeImage => i::BOT,
            OCR => i::SCAN_TEXT,
            QRCode => i::QR_CODE,
            QRCodeDecodeFromScreen => i::SCAN_EYE,
            QRCodeScanRegion => i::SCAN_LINE,
            Metadata => i::TAGS,
            StripMetadata => i::FILE_X,
            IndexFolder => i::FOLDER_TREE,
            ActiveWindowBorderless => i::MAXIMIZE,
            ActiveWindowTopMost => i::PANEL_TOP,
            DisableHotkeys => i::KEYBOARD_OFF,
            OpenMainWindow => i::PANEL_TOP_OPEN,
            OpenScreenshotsFolder => i::FOLDER_OPEN,
            OpenHistory => i::HISTORY,
            OpenImageHistory => i::IMAGES,
            ToggleTrayMenu => i::MENU,
            ExitShareX => i::LOG_OUT,
        }
    }
}

/// WinForms `Keys`: a key code in the low 16 bits and the modifier bits above.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Keys(pub u32);

impl Keys {
    pub const NONE: Keys = Keys(0);
    pub const KEY_CODE: u32 = 0xFFFF;
    pub const SHIFT: u32 = 0x1_0000;
    pub const CONTROL: u32 = 0x2_0000;
    pub const ALT: u32 = 0x4_0000;

    pub fn key_code(self) -> u32 {
        self.0 & Self::KEY_CODE
    }

    pub fn has(self, modifier: u32) -> bool {
        self.0 & modifier != 0
    }

    pub fn with(self, modifier: u32) -> Keys {
        Keys(self.0 | modifier)
    }
}

/// Key names and codes of WinForms `Keys`. The first name of a code is the one written.
pub const KEY_NAMES: &[(&str, u32)] = &[
    ("None", 0),
    ("LButton", 1),
    ("RButton", 2),
    ("Cancel", 3),
    ("MButton", 4),
    ("XButton1", 5),
    ("XButton2", 6),
    ("Back", 8),
    ("Tab", 9),
    ("LineFeed", 10),
    ("Clear", 12),
    ("Return", 13),
    ("Enter", 13),
    ("ShiftKey", 16),
    ("ControlKey", 17),
    ("Menu", 18),
    ("Pause", 19),
    ("Capital", 20),
    ("CapsLock", 20),
    ("Escape", 27),
    ("Space", 32),
    ("PageUp", 33),
    ("Prior", 33),
    ("Next", 34),
    ("PageDown", 34),
    ("End", 35),
    ("Home", 36),
    ("Left", 37),
    ("Up", 38),
    ("Right", 39),
    ("Down", 40),
    ("Select", 41),
    ("Print", 42),
    ("Execute", 43),
    ("PrintScreen", 44),
    ("Snapshot", 44),
    ("Insert", 45),
    ("Delete", 46),
    ("Help", 47),
    ("D0", 48),
    ("D1", 49),
    ("D2", 50),
    ("D3", 51),
    ("D4", 52),
    ("D5", 53),
    ("D6", 54),
    ("D7", 55),
    ("D8", 56),
    ("D9", 57),
    ("A", 65),
    ("B", 66),
    ("C", 67),
    ("D", 68),
    ("E", 69),
    ("F", 70),
    ("G", 71),
    ("H", 72),
    ("I", 73),
    ("J", 74),
    ("K", 75),
    ("L", 76),
    ("M", 77),
    ("N", 78),
    ("O", 79),
    ("P", 80),
    ("Q", 81),
    ("R", 82),
    ("S", 83),
    ("T", 84),
    ("U", 85),
    ("V", 86),
    ("W", 87),
    ("X", 88),
    ("Y", 89),
    ("Z", 90),
    ("LWin", 91),
    ("RWin", 92),
    ("Apps", 93),
    ("Sleep", 95),
    ("NumPad0", 96),
    ("NumPad1", 97),
    ("NumPad2", 98),
    ("NumPad3", 99),
    ("NumPad4", 100),
    ("NumPad5", 101),
    ("NumPad6", 102),
    ("NumPad7", 103),
    ("NumPad8", 104),
    ("NumPad9", 105),
    ("Multiply", 106),
    ("Add", 107),
    ("Separator", 108),
    ("Subtract", 109),
    ("Decimal", 110),
    ("Divide", 111),
    ("F1", 112),
    ("F2", 113),
    ("F3", 114),
    ("F4", 115),
    ("F5", 116),
    ("F6", 117),
    ("F7", 118),
    ("F8", 119),
    ("F9", 120),
    ("F10", 121),
    ("F11", 122),
    ("F12", 123),
    ("F13", 124),
    ("F14", 125),
    ("F15", 126),
    ("F16", 127),
    ("F17", 128),
    ("F18", 129),
    ("F19", 130),
    ("F20", 131),
    ("F21", 132),
    ("F22", 133),
    ("F23", 134),
    ("F24", 135),
    ("NumLock", 144),
    ("Scroll", 145),
    ("LShiftKey", 160),
    ("RShiftKey", 161),
    ("LControlKey", 162),
    ("RControlKey", 163),
    ("LMenu", 164),
    ("RMenu", 165),
    ("VolumeMute", 173),
    ("VolumeDown", 174),
    ("VolumeUp", 175),
    ("MediaNextTrack", 176),
    ("MediaPreviousTrack", 177),
    ("MediaStop", 178),
    ("MediaPlayPause", 179),
    ("OemSemicolon", 186),
    ("Oem1", 186),
    ("Oemplus", 187),
    ("Oemcomma", 188),
    ("OemMinus", 189),
    ("OemPeriod", 190),
    ("OemQuestion", 191),
    ("Oem2", 191),
    ("Oemtilde", 192),
    ("Oem3", 192),
    ("OemOpenBrackets", 219),
    ("Oem4", 219),
    ("OemPipe", 220),
    ("Oem5", 220),
    ("OemCloseBrackets", 221),
    ("Oem6", 221),
    ("OemQuotes", 222),
    ("Oem7", 222),
    ("Oem8", 223),
    ("OemBackslash", 226),
    ("Oem102", 226),
    ("Shift", Keys::SHIFT),
    ("Control", Keys::CONTROL),
    ("Alt", Keys::ALT),
];

pub fn key_name(code: u32) -> Option<&'static str> {
    KEY_NAMES.iter().find(|(_, c)| *c == code).map(|(n, _)| *n)
}

pub fn key_code(name: &str) -> Option<u32> {
    KEY_NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
        .map(|(_, c)| *c)
}

impl Keys {
    /// Read Newtonsoft's form: `"D2, Shift, Control"` or a number.
    pub fn parse_value(value: &Value) -> Keys {
        match value {
            Value::Number(n) => Keys(n.as_u64().unwrap_or(0) as u32),
            Value::String(text) => {
                let mut bits = 0;
                for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    if let Ok(number) = part.parse::<u32>() {
                        bits |= number;
                    } else if let Some(code) = key_code(part) {
                        bits |= code;
                    }
                }
                Keys(bits)
            }
            _ => Keys::NONE,
        }
    }

    /// Newtonsoft's form: the key, then the modifiers in ascending order of value.
    pub fn to_value(self) -> Value {
        if self.0 == 0 {
            return json!("None");
        }
        let mut parts = Vec::new();
        let code = self.key_code();
        if code != 0 {
            match key_name(code) {
                Some(name) => parts.push(name.to_string()),
                None => parts.push(code.to_string()),
            }
        }
        for (name, bit) in [
            ("Shift", Keys::SHIFT),
            ("Control", Keys::CONTROL),
            ("Alt", Keys::ALT),
        ] {
            if self.has(bit) {
                parts.push(name.to_string());
            }
        }
        json!(parts.join(", "))
    }
}

/// `HotkeyStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotkeyStatus {
    #[default]
    NotConfigured,
    Registered,
    Failed,
}

/// `HotkeyInfo`: the binding (`Hotkey` + `Win`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HotkeyInfo {
    pub hotkey: Keys,
    pub win: bool,
}

/// The names of the four modifiers (`Keys.Control`, `Keys.Shift`, `Keys.Alt`, Win), in
/// the order upstream prints them: upstream's own on Windows (Ctrl, Shift, Alt, Win),
/// the Mac's keys on a Mac (Cmd, Shift, Option, Control).
pub fn modifier_names() -> [&'static str; 4] {
    if cfg!(windows) {
        [
            t(helpers::HotkeyInfo_Ctrl),
            t(helpers::HotkeyInfo_Shift),
            t(helpers::HotkeyInfo_Alt),
            t(helpers::HotkeyInfo_Win),
        ]
    } else {
        ["Cmd", t(helpers::HotkeyInfo_Shift), "Option", "Control"]
    }
}

impl HotkeyInfo {
    pub fn new(hotkey: u32) -> HotkeyInfo {
        HotkeyInfo {
            hotkey: Keys(hotkey),
            win: false,
        }
    }

    pub fn key_code(&self) -> u32 {
        self.hotkey.key_code()
    }

    /// `Keys.Control`: Ctrl on Windows, Command on a Mac.
    pub fn control(&self) -> bool {
        self.hotkey.has(Keys::CONTROL)
    }

    pub fn shift(&self) -> bool {
        self.hotkey.has(Keys::SHIFT)
    }

    /// `Keys.Alt`: Alt on Windows, Option on a Mac.
    pub fn alt(&self) -> bool {
        self.hotkey.has(Keys::ALT)
    }

    /// `IsOnlyModifiers`.
    pub fn is_only_modifiers(&self) -> bool {
        let code = self.key_code();
        code == 17 || code == 16 || code == 18 || (code == 0 && self.win)
    }

    /// `IsValidHotkey`.
    pub fn is_valid(&self) -> bool {
        self.key_code() != 0 && !self.is_only_modifiers()
    }

    /// `HotkeyInfo.ToString`, with this system's modifier names ([`modifier_names`]).
    pub fn text(&self) -> String {
        let names = modifier_names();
        let mut text = String::new();
        if self.control() {
            text.push_str(names[0]);
            text.push_str(" + ");
        }
        if self.shift() {
            text.push_str(names[1]);
            text.push_str(" + ");
        }
        if self.alt() {
            text.push_str(names[2]);
            text.push_str(" + ");
        }
        if self.win {
            text.push_str(names[3]);
            text.push_str(" + ");
        }
        let code = self.key_code();
        if self.is_only_modifiers() {
            text.push_str("...");
        } else if code == 8 {
            text.push_str(t(helpers::HotkeyInfo_Backspace));
        } else if code == 13 {
            text.push_str(t(helpers::HotkeyInfo_Enter));
        } else if code == 20 {
            text.push_str(t(helpers::HotkeyInfo_Caps_lock));
        } else if code == 34 {
            text.push_str(t(helpers::HotkeyInfo_Page_down));
        } else if code == 145 && cfg!(windows) {
            text.push_str(t(helpers::HotkeyInfo_Scroll_lock));
        } else if !cfg!(windows) && code == 44 {
            // Print Screen, Scroll Lock and Pause are F13, F14 and F15 on a Mac
            // (`code_for` in modlight-macos), and the label says so.
            text.push_str("F13");
        } else if !cfg!(windows) && code == 145 {
            text.push_str("F14");
        } else if !cfg!(windows) && code == 19 {
            text.push_str("F15");
        } else if (48..=57).contains(&code) {
            text.push_str(&(code - 48).to_string());
        } else if (96..=105).contains(&code) {
            text.push_str(t(helpers::HotkeyInfo_Numpad));
            text.push(' ');
            text.push_str(&(code - 96).to_string());
        } else if code == 0 {
            text.push_str("None");
        } else {
            text.push_str(&with_spaces(key_name(code).unwrap_or("?")));
        }
        text
    }

    fn from_value(value: &Value) -> HotkeyInfo {
        HotkeyInfo {
            hotkey: value
                .get("Hotkey")
                .map(Keys::parse_value)
                .unwrap_or_default(),
            win: value.get("Win").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    fn write_into(&self, object: &mut Map<String, Value>) {
        object.insert("Hotkey".into(), self.hotkey.to_value());
        object.insert("Win".into(), json!(self.win));
    }
}

/// `HotkeySettingsAvaloniaService.ConvertGesture`: the key's WinForms name (`None` while
/// only modifiers are down), then the modifiers by their `Keys` meaning: `control`
/// (`Keys.Control`: Ctrl on Windows, Command on a Mac), `shift`, `alt` (Alt, Option) and
/// `win` (Win, the Mac's Control). On a Mac F13, F14 and F15 are stored as Print Screen,
/// Scroll Lock and Pause, the keys in their place, so a binding means the same key on
/// Windows; on Windows every key is itself.
pub fn gesture(key: Option<&str>, control: bool, shift: bool, alt: bool, win: bool) -> HotkeyInfo {
    let name = match key {
        Some("F13") if !cfg!(windows) => Some("PrintScreen"),
        Some("F14") if !cfg!(windows) => Some("Scroll"),
        Some("F15") if !cfg!(windows) => Some("Pause"),
        other => other,
    };
    let mut bits = name.and_then(key_code).unwrap_or(0);
    if control {
        bits |= Keys::CONTROL;
    }
    if shift {
        bits |= Keys::SHIFT;
    }
    if alt {
        bits |= Keys::ALT;
    }
    HotkeyInfo {
        hotkey: Keys(bits),
        win,
    }
}

/// `ToStringWithSpaces`.
fn with_spaces(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// One hotkey (`HotkeySettings`): the binding and the task settings it runs with, kept
/// as the file has them so members this build does not read survive.
#[derive(Clone, Debug, PartialEq)]
pub struct HotkeySettings {
    pub info: HotkeyInfo,
    /// The `TaskSettings` object, as JSON.
    pub task: Value,
    /// Not saved: whether the system took the key.
    pub status: HotkeyStatus,
}

impl HotkeySettings {
    /// `new HotkeySettings(job, keys)`.
    pub fn new(job: HotkeyType, hotkey: u32) -> HotkeySettings {
        HotkeySettings {
            info: HotkeyInfo::new(hotkey),
            task: json!({ "Job": job.name() }),
            status: HotkeyStatus::NotConfigured,
        }
    }

    pub fn job(&self) -> HotkeyType {
        self.task
            .get("Job")
            .and_then(Value::as_str)
            .and_then(HotkeyType::parse)
            .or_else(|| {
                self.task
                    .get("Job")
                    .and_then(Value::as_u64)
                    .and_then(|i| HotkeyType::ALL.get(i as usize).copied())
            })
            .unwrap_or(HotkeyType::None)
    }

    pub fn set_job(&mut self, job: HotkeyType) {
        if !self.task.is_object() {
            self.task = json!({});
        }
        self.task["Job"] = json!(job.name());
    }

    pub fn description(&self) -> String {
        self.task
            .get("Description")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.job().text())
    }

    /// `TaskSettings.IsUsingDefaultSettings`, read from the JSON.
    pub fn is_using_default_settings(&self) -> bool {
        let flag = |name: &str, default: bool| {
            self.task
                .get(name)
                .and_then(Value::as_bool)
                .unwrap_or(default)
        };
        flag("UseDefaultAfterCaptureJob", true)
            && flag("UseDefaultAfterUploadJob", true)
            && flag("UseDefaultDestinations", true)
            && !flag("OverrideScreenshotsFolder", false)
            && flag("UseDefaultGeneralSettings", true)
            && flag("UseDefaultImageSettings", true)
            && flag("UseDefaultCaptureSettings", true)
            && flag("UseDefaultUploadSettings", true)
            && flag("UseDefaultActions", true)
            && flag("UseDefaultToolsSettings", true)
            && flag("UseDefaultAdvancedSettings", true)
            && !flag("WatchFolderEnabled", false)
    }

    /// `GetSafeTaskSettings`: the defaults, with whatever this hotkey overrides.
    pub fn task_settings(&self, defaults: &Value) -> crate::tasks::TaskSettings {
        resolve_task_settings(&self.task, defaults)
    }

    fn from_value(value: &Value) -> HotkeySettings {
        HotkeySettings {
            info: value
                .get("HotkeyInfo")
                .map(HotkeyInfo::from_value)
                .unwrap_or_default(),
            task: value
                .get("TaskSettings")
                .cloned()
                .unwrap_or_else(|| json!({})),
            status: HotkeyStatus::NotConfigured,
        }
    }

    fn to_value(&self, original: Option<&Value>) -> Value {
        let mut info = original
            .and_then(|o| o.get("HotkeyInfo"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        self.info.write_into(&mut info);
        let mut object = original
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        object.insert("HotkeyInfo".into(), Value::Object(info));
        object.insert("TaskSettings".into(), self.task.clone());
        Value::Object(object)
    }
}

/// `TaskSettings.SetDefaultSettings` on JSON: every group the hotkey does not override
/// comes from the defaults.
pub fn resolve_task_settings(task: &Value, defaults: &Value) -> crate::tasks::TaskSettings {
    let flag = |name: &str| task.get(name).and_then(Value::as_bool).unwrap_or(true);
    let mut merged = match defaults {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let own = |name: &str| task.get(name).cloned();
    if let Some(job) = own("Job") {
        merged.insert("Job".into(), job);
    }
    if let Some(description) = own("Description") {
        merged.insert("Description".into(), description);
    }
    let groups: &[(&str, &[&str])] = &[
        ("UseDefaultAfterCaptureJob", &["AfterCaptureJob"]),
        ("UseDefaultAfterUploadJob", &["AfterUploadJob"]),
        (
            "UseDefaultDestinations",
            &[
                "ImageDestination",
                "ImageFileDestination",
                "TextDestination",
                "TextFileDestination",
                "FileDestination",
                "URLShortenerDestination",
                "URLSharingServiceDestination",
            ],
        ),
        ("UseDefaultGeneralSettings", &["GeneralSettings"]),
        ("UseDefaultImageSettings", &["ImageSettings"]),
        ("UseDefaultCaptureSettings", &["CaptureSettings"]),
        ("UseDefaultUploadSettings", &["UploadSettings"]),
        ("UseDefaultActions", &["ExternalPrograms"]),
        ("UseDefaultToolsSettings", &["ToolsSettings"]),
        ("UseDefaultAdvancedSettings", &["AdvancedSettings"]),
    ];
    for (use_default, members) in groups {
        if !flag(use_default) {
            for member in *members {
                if let Some(value) = own(member).filter(|v| !v.is_null()) {
                    merged.insert((*member).into(), value);
                }
            }
        }
    }
    if task
        .get("OverrideScreenshotsFolder")
        .and_then(Value::as_bool)
        == Some(true)
    {
        merged.insert("OverrideScreenshotsFolder".into(), json!(true));
        if let Some(folder) = own("ScreenshotsFolder") {
            merged.insert("ScreenshotsFolder".into(), folder);
        }
    }
    if task.get("WatchFolderEnabled").and_then(Value::as_bool) == Some(true) {
        merged.insert("WatchFolderEnabled".into(), json!(true));
        if let Some(list) = own("WatchFolderList") {
            merged.insert("WatchFolderList".into(), list);
        }
    }
    serde_json::from_value(Value::Object(merged)).unwrap_or_default()
}

/// What registers a key with the system (`HotkeyForm.RegisterHotkey`): the Mac's
/// global-hotkey registration, or a fake in QA and tests.
pub trait HotkeyBackend: Send {
    /// Take the key; the id is what [`HotkeyManager::triggered`] will be called with.
    fn register(&mut self, info: HotkeyInfo) -> Result<u32, String>;
    fn unregister(&mut self, id: u32);
}

/// `HotkeyManager`: the list, what the system took, and the switches.
pub struct HotkeyManager {
    pub hotkeys: Vec<HotkeySettings>,
    ids: Vec<Option<u32>>,
    backend: Box<dyn HotkeyBackend>,
    /// `IgnoreHotkeys`: while a hotkey is being typed into the settings page.
    pub ignore: bool,
    /// `Settings.DisableHotkeys`.
    pub disabled: bool,
}

impl HotkeyManager {
    pub fn new(
        backend: Box<dyn HotkeyBackend>,
        hotkeys: Vec<HotkeySettings>,
        disabled: bool,
    ) -> HotkeyManager {
        let ids = vec![None; hotkeys.len()];
        HotkeyManager {
            hotkeys,
            ids,
            backend,
            ignore: false,
            disabled,
        }
    }

    fn sync_ids(&mut self) {
        self.ids.resize(self.hotkeys.len(), None);
    }

    /// `RegisterHotkey`.
    pub fn register(&mut self, index: usize) {
        self.sync_ids();
        let Some(hotkey) = self.hotkeys.get(index) else {
            return;
        };
        if self.disabled && hotkey.job() != HotkeyType::DisableHotkeys {
            return;
        }
        self.unregister(index);
        let info = self.hotkeys[index].info;
        if info.is_valid() {
            match self.backend.register(info) {
                Ok(id) => {
                    self.ids[index] = Some(id);
                    self.hotkeys[index].status = HotkeyStatus::Registered;
                    log::info!("Hotkey registered: {}", info.text());
                }
                Err(error) => {
                    self.hotkeys[index].status = HotkeyStatus::Failed;
                    log::info!("Hotkey register failed: {} ({error})", info.text());
                }
            }
        } else {
            self.hotkeys[index].status = HotkeyStatus::NotConfigured;
        }
    }

    /// `UnregisterHotkey` (without removing it from the list).
    pub fn unregister(&mut self, index: usize) {
        self.sync_ids();
        if let Some(Some(id)) = self.ids.get(index).copied() {
            self.backend.unregister(id);
            self.ids[index] = None;
        }
        if let Some(hotkey) = self.hotkeys.get_mut(index) {
            hotkey.status = HotkeyStatus::NotConfigured;
        }
    }

    /// `RegisterAllHotkeys`.
    pub fn register_all(&mut self) {
        for index in 0..self.hotkeys.len() {
            self.register(index);
        }
    }

    /// `RegisterFailedHotkeys`.
    pub fn register_failed(&mut self) {
        for index in 0..self.hotkeys.len() {
            if self.hotkeys[index].status == HotkeyStatus::Failed {
                self.register(index);
            }
        }
    }

    /// `UnregisterAllHotkeys(false, temporary)`.
    pub fn unregister_all(&mut self, temporary: bool) {
        for index in 0..self.hotkeys.len() {
            if !temporary || self.hotkeys[index].job() != HotkeyType::DisableHotkeys {
                self.unregister(index);
            }
        }
    }

    /// Replace the list (`UpdateHotkeys`).
    pub fn update(&mut self, hotkeys: Vec<HotkeySettings>) {
        self.unregister_all(false);
        self.hotkeys = hotkeys;
        self.ids = vec![None; self.hotkeys.len()];
        self.register_all();
    }

    /// Remove one (`UnregisterHotkey(…, removeFromList)`).
    pub fn remove(&mut self, index: usize) {
        if index < self.hotkeys.len() {
            self.unregister(index);
            self.hotkeys.remove(index);
            self.ids.remove(index);
        }
    }

    /// Add one and register it.
    pub fn add(&mut self, hotkey: HotkeySettings) -> usize {
        self.hotkeys.push(hotkey);
        self.ids.push(None);
        let index = self.hotkeys.len() - 1;
        self.register(index);
        index
    }

    /// Move one (`Move(item, offset)`, wrapping around).
    pub fn move_by(&mut self, index: usize, offset: i64) -> usize {
        let count = self.hotkeys.len() as i64;
        if count < 2 || index as i64 >= count {
            return index;
        }
        let new = ((index as i64 + offset) % count + count) % count;
        let hotkey = self.hotkeys.remove(index);
        let id = self.ids.remove(index);
        self.hotkeys.insert(new as usize, hotkey);
        self.ids.insert(new as usize, id);
        new as usize
    }

    /// `ToggleHotkeys(hotkeysDisabled)`.
    pub fn toggle(&mut self, disabled: bool) {
        self.disabled = disabled;
        if disabled {
            self.unregister_all(true);
        } else {
            self.register_all();
        }
    }

    /// `ResetHotkeys`.
    pub fn reset(&mut self) {
        self.unregister_all(false);
        self.hotkeys = default_hotkeys();
        self.ids = vec![None; self.hotkeys.len()];
        self.register_all();
    }

    /// `FailedHotkeys`.
    pub fn failed(&self) -> Vec<&HotkeySettings> {
        self.hotkeys
            .iter()
            .filter(|h| h.status == HotkeyStatus::Failed)
            .collect()
    }

    /// The hotkey a system id belongs to, unless hotkeys are being ignored
    /// (`HotkeyForm_HotkeyPress`; the full-screen check is the caller's).
    pub fn triggered(&self, id: u32) -> Option<&HotkeySettings> {
        if self.ignore {
            return None;
        }
        self.ids
            .iter()
            .position(|i| *i == Some(id))
            .and_then(|index| self.hotkeys.get(index))
    }

    /// The registered shortcut of a job, for the Capture tab's tiles (`HotkeyFor`).
    pub fn shortcut_for(&self, job: HotkeyType) -> Option<String> {
        self.hotkeys
            .iter()
            .find(|h| h.job() == job && h.status == HotkeyStatus::Registered && h.info.is_valid())
            .map(|h| h.info.text())
    }

    pub fn id_of(&self, index: usize) -> Option<u32> {
        self.ids.get(index).copied().flatten()
    }
}

/// A backend that takes every key except those listed as taken (QA, tests).
#[derive(Default)]
pub struct FakeBackend {
    pub taken: Vec<HotkeyInfo>,
    pub next: u32,
    pub registered: std::sync::Arc<std::sync::Mutex<Vec<(u32, HotkeyInfo)>>>,
}

impl HotkeyBackend for FakeBackend {
    fn register(&mut self, info: HotkeyInfo) -> Result<u32, String> {
        let mut registered = self.registered.lock().unwrap();
        if self.taken.contains(&info) || registered.iter().any(|(_, i)| *i == info) {
            return Err("taken by another app".into());
        }
        self.next += 1;
        registered.push((self.next, info));
        Ok(self.next)
    }

    fn unregister(&mut self, id: u32) {
        self.registered.lock().unwrap().retain(|(i, _)| *i != id);
    }
}

/// Upstream's defaults on Windows (`HotkeyManager.GetDefaultHotkeyList`): Print Screen
/// alone and with modifiers.
#[cfg(windows)]
pub fn default_hotkeys() -> Vec<HotkeySettings> {
    const PRINT_SCREEN: u32 = 44;
    vec![
        HotkeySettings::new(HotkeyType::RectangleRegion, Keys::CONTROL | PRINT_SCREEN),
        HotkeySettings::new(HotkeyType::PrintScreen, PRINT_SCREEN),
        HotkeySettings::new(HotkeyType::ActiveWindow, Keys::ALT | PRINT_SCREEN),
        HotkeySettings::new(HotkeyType::ScreenRecorder, Keys::SHIFT | PRINT_SCREEN),
        HotkeySettings::new(
            HotkeyType::ScreenRecorderGIF,
            Keys::CONTROL | Keys::SHIFT | PRINT_SCREEN,
        ),
    ]
}

/// The Mac's defaults (`HotkeyManager.GetDefaultHotkeyList`).
///
/// Upstream binds Print Screen with and without modifiers. A Mac keyboard has no Print
/// Screen, and Command+Shift+3, 4, 5 and 6 are the system's own screenshot keys, so the
/// same five jobs go to the free digits around them:
///
/// | Job | Windows | Mac |
/// |---|---|---|
/// | Region | Ctrl + Print Screen | Cmd + Shift + 2 |
/// | Fullscreen | Print Screen | Cmd + Shift + 1 |
/// | Active window | Alt + Print Screen | Cmd + Shift + 7 |
/// | Screen recording | Shift + Print Screen | Cmd + Shift + 8 |
/// | GIF recording | Ctrl + Shift + Print Screen | Cmd + Shift + 9 |
#[cfg(not(windows))]
pub fn default_hotkeys() -> Vec<HotkeySettings> {
    let cmd_shift = Keys::CONTROL | Keys::SHIFT;
    vec![
        HotkeySettings::new(HotkeyType::RectangleRegion, cmd_shift | 50),
        HotkeySettings::new(HotkeyType::PrintScreen, cmd_shift | 49),
        HotkeySettings::new(HotkeyType::ActiveWindow, cmd_shift | 55),
        HotkeySettings::new(HotkeyType::ScreenRecorder, cmd_shift | 56),
        HotkeySettings::new(HotkeyType::ScreenRecorderGIF, cmd_shift | 57),
    ]
}

/// `HotkeysConfig`: read from and written into the settings' hotkeys document.
pub fn load(config: &AppConfig) -> Vec<HotkeySettings> {
    match config.hotkeys().and_then(|h| h.get("Hotkeys")) {
        Some(Value::Array(list)) => list.iter().map(HotkeySettings::from_value).collect(),
        _ => default_hotkeys(),
    }
}

/// The groups of a `TaskSettings` and the flag that says the defaults stand in for each.
const TASK_GROUPS: [(&str, &str); 7] = [
    ("UseDefaultGeneralSettings", "GeneralSettings"),
    ("UseDefaultImageSettings", "ImageSettings"),
    ("UseDefaultCaptureSettings", "CaptureSettings"),
    ("UseDefaultUploadSettings", "UploadSettings"),
    ("UseDefaultActions", "ExternalPrograms"),
    ("UseDefaultToolsSettings", "ToolsSettings"),
    ("UseDefaultAdvancedSettings", "AdvancedSettings"),
];

fn uses_default(task: &Value, flag: &str) -> bool {
    task.get(flag).and_then(Value::as_bool).unwrap_or(true)
}

/// `TaskSettings.SetDefaultSettings`: every part a hotkey takes from the defaults gets a
/// copy of them, so its settings window shows what it runs with.
pub fn set_default_settings(task: &mut Value, defaults: &Value) {
    if !task.is_object() {
        *task = json!({});
    }
    let copy = |name: &str| defaults.get(name).cloned().unwrap_or(Value::Null);
    if uses_default(task, "UseDefaultAfterCaptureJob") {
        task["AfterCaptureJob"] = copy("AfterCaptureJob");
    }
    if uses_default(task, "UseDefaultAfterUploadJob") {
        task["AfterUploadJob"] = copy("AfterUploadJob");
    }
    if uses_default(task, "UseDefaultDestinations") {
        for name in [
            "ImageDestination",
            "ImageFileDestination",
            "TextDestination",
            "TextFileDestination",
            "FileDestination",
            "URLShortenerDestination",
            "URLSharingServiceDestination",
        ] {
            task[name] = copy(name);
        }
    }
    for (flag, group) in TASK_GROUPS {
        if uses_default(task, flag) {
            task[group] = copy(group);
        }
    }
    // Members the defaults never had stay out rather than becoming null.
    if let Value::Object(map) = task {
        map.retain(|_, v| !v.is_null());
    }
}

/// `TaskSettings.Cleanup` (`SettingManager.CleanupHotkeysConfig`): the parts a hotkey
/// takes from the defaults are written as `null`.
pub fn cleanup(task: &mut Value) {
    if !task.is_object() {
        return;
    }
    for (flag, group) in TASK_GROUPS {
        if uses_default(task, flag) {
            task[group] = Value::Null;
        }
    }
}

/// Write the list back, keeping every member of each entry this build does not know.
pub fn save(config: &mut AppConfig, hotkeys: &[HotkeySettings]) {
    let mut document = config.hotkeys().cloned().unwrap_or_default();
    let original: Vec<Value> = match document.get("Hotkeys") {
        Some(Value::Array(list)) => list.clone(),
        _ => Vec::new(),
    };
    let list: Vec<Value> = hotkeys
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let mut value = h.to_value(original.get(i));
            if let Some(task) = value.get_mut("TaskSettings") {
                cleanup(task);
            }
            value
        })
        .collect();
    document.insert("Hotkeys".into(), Value::Array(list));
    document
        .entry("ApplicationVersion")
        .or_insert_with(|| json!(env!("CARGO_PKG_VERSION")));
    config.set_hotkeys(document);
}

/// The localized key of a modifier for the hotkey page's legend (unused names kept so
/// a translation check sees them).
pub fn windows_modifier_keys() -> [Key; 4] {
    [
        helpers::HotkeyInfo_Ctrl,
        helpers::HotkeyInfo_Shift,
        helpers::HotkeyInfo_Alt,
        helpers::HotkeyInfo_Win,
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_hotkey_shows_the_defaults_and_saves_them_as_null() {
        let defaults = serde_json::json!({
            "AfterCaptureJob": "CopyImageToClipboard, SaveImageToFile",
            "GeneralSettings": {"PlaySoundAfterCapture": false},
            "CaptureSettings": {"ShowCursor": false}
        });
        let mut task = serde_json::json!({"Job": "RectangleRegion", "UseDefaultCaptureSettings": false, "CaptureSettings": {"ShowCursor": true}});
        super::set_default_settings(&mut task, &defaults);
        assert_eq!(
            task["GeneralSettings"]["PlaySoundAfterCapture"],
            serde_json::json!(false)
        );
        assert_eq!(
            task["CaptureSettings"]["ShowCursor"],
            serde_json::json!(true)
        );
        assert_eq!(task["AfterCaptureJob"], defaults["AfterCaptureJob"]);
        super::cleanup(&mut task);
        assert!(task["GeneralSettings"].is_null());
        assert_eq!(
            task["CaptureSettings"]["ShowCursor"],
            serde_json::json!(true)
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn a_mac_gesture_becomes_a_windows_binding() {
        let info = super::gesture(Some("D2"), true, true, false, false);
        assert_eq!(
            info.hotkey.to_value(),
            serde_json::json!("D2, Shift, Control")
        );
        assert!(!info.win);
        assert_eq!(
            info.text(),
            format!("Cmd + {} + 2", super::modifier_names()[1])
        );
        let print = super::gesture(Some("F13"), false, false, false, true);
        assert_eq!(print.key_code(), 44);
        assert!(print.win);
        assert!(print.text().ends_with("F13"));
        let modifiers = super::gesture(None, true, false, true, false);
        assert!(modifiers.is_only_modifiers() || modifiers.key_code() == 0);
        assert!(!modifiers.is_valid());
    }

    use super::*;
    use crate::l10n::{Language, set_thread_language};

    #[test]
    fn keys_read_and_write_like_newtonsoft() {
        let keys = Keys::parse_value(&json!("PrintScreen, Control"));
        assert_eq!(keys.key_code(), 44);
        assert!(keys.has(Keys::CONTROL));
        assert_eq!(keys.to_value(), json!("PrintScreen, Control"));
        let keys = Keys::parse_value(&json!("D2, Shift, Control"));
        assert_eq!(keys, Keys(50 | Keys::SHIFT | Keys::CONTROL));
        assert_eq!(keys.to_value(), json!("D2, Shift, Control"));
        assert_eq!(Keys::parse_value(&json!(131116)), Keys(44 | Keys::CONTROL));
        assert_eq!(
            Keys::parse_value(&json!("Snapshot")).to_value(),
            json!("PrintScreen")
        );
        assert_eq!(Keys::NONE.to_value(), json!("None"));
        assert_eq!(
            Keys::parse_value(&json!("Enter, Alt")).to_value(),
            json!("Return, Alt")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn hotkeys_print_with_the_macs_modifier_names() {
        set_thread_language(Some(Language::English));
        let info = HotkeyInfo::new(Keys::CONTROL | Keys::SHIFT | 50);
        assert_eq!(info.text(), "Cmd + Shift + 2");
        assert_eq!(HotkeyInfo::new(Keys::ALT | 44).text(), "Option + F13");
        assert_eq!(HotkeyInfo::new(34).text(), t(helpers::HotkeyInfo_Page_down));
        assert_eq!(HotkeyInfo::new(Keys::CONTROL | 99).text(), "Cmd + Numpad 3");
        let only = HotkeyInfo::new(Keys::CONTROL | 17);
        assert!(only.is_only_modifiers());
        assert!(!only.is_valid());
        assert_eq!(only.text(), "Cmd + ...");
        let win = HotkeyInfo {
            hotkey: Keys(Keys::SHIFT | 65),
            win: true,
        };
        assert_eq!(win.text(), "Shift + Control + A");
        assert!(!HotkeyInfo::default().is_valid());
        set_thread_language(None);
    }

    #[cfg(not(windows))]
    #[test]
    fn the_defaults_avoid_the_system_screenshot_keys() {
        let defaults = default_hotkeys();
        assert_eq!(defaults.len(), 5);
        for hotkey in &defaults {
            assert!(hotkey.info.is_valid());
            assert!(hotkey.info.control() && hotkey.info.shift());
            assert!(
                ![51, 52, 53, 54].contains(&hotkey.info.key_code()),
                "Cmd+Shift+3..6 are the system's"
            );
        }
        assert_eq!(defaults[0].job(), HotkeyType::RectangleRegion);
        assert_eq!(defaults[4].job(), HotkeyType::ScreenRecorderGIF);
    }

    #[test]
    fn a_hotkeys_document_round_trips_without_losing_members() {
        let mut config = AppConfig::in_memory();
        assert_eq!(load(&config), default_hotkeys());
        config.set_hotkeys(
            json!({
                "Hotkeys": [{
                    "HotkeyInfo": { "Hotkey": "PrintScreen, Control", "Win": false, "Extra": 1 },
                    "TaskSettings": { "Job": "RectangleRegion", "Description": "Mine", "UseDefaultAfterCaptureJob": false, "AfterCaptureJob": "SaveImageToFile" }
                }],
                "ApplicationVersion": "0.7.7"
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        let mut list = load(&config);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].job(), HotkeyType::RectangleRegion);
        assert_eq!(list[0].description(), "Mine");
        assert!(!list[0].is_using_default_settings());
        list[0].info = HotkeyInfo::new(Keys::CONTROL | Keys::SHIFT | 50);
        list.push(HotkeySettings::new(HotkeyType::ActiveMonitor, 0));
        save(&mut config, &list);
        let written = Value::Object(config.hotkeys().unwrap().clone());
        assert_eq!(
            written["Hotkeys"][0]["HotkeyInfo"]["Hotkey"],
            "D2, Shift, Control"
        );
        assert_eq!(written["Hotkeys"][0]["HotkeyInfo"]["Extra"], 1);
        assert_eq!(written["Hotkeys"][0]["TaskSettings"]["Description"], "Mine");
        assert_eq!(
            written["Hotkeys"][1]["TaskSettings"]["Job"],
            "ActiveMonitor"
        );
        assert_eq!(written["ApplicationVersion"], "0.7.7");
    }

    #[test]
    fn a_hotkey_overrides_only_what_it_says() {
        let defaults = json!({
            "AfterCaptureJob": "CopyImageToClipboard, UploadImageToHost",
            "CaptureSettings": { "ShowCursor": false },
            "ImageSettings": { "ImageFormat": "JPEG" }
        });
        let hotkey = HotkeySettings {
            info: HotkeyInfo::default(),
            task: json!({
                "Job": "PrintScreen",
                "UseDefaultAfterCaptureJob": false,
                "AfterCaptureJob": "SaveImageToFile",
                "UseDefaultCaptureSettings": true,
                "CaptureSettings": null,
                "ImageSettings": { "ImageFormat": "BMP" }
            }),
            status: HotkeyStatus::NotConfigured,
        };
        let settings = hotkey.task_settings(&defaults);
        assert_eq!(settings.job, HotkeyType::PrintScreen);
        assert_eq!(
            settings.after_capture_job,
            crate::tasks::settings::AfterCaptureTasks::SaveImageToFile
        );
        assert!(
            !settings.capture_settings.show_cursor,
            "the defaults' capture settings"
        );
        assert_eq!(
            settings.image_settings.image_format,
            crate::tasks::settings::ImageFormat::JPEG,
            "image settings are not overridden"
        );
    }

    #[test]
    fn the_manager_registers_reports_failures_and_toggles() {
        set_thread_language(Some(Language::English));
        // The default PrintScreen job's key is taken by another app.
        let taken = default_hotkeys()[1].info;
        let backend = FakeBackend {
            taken: vec![taken],
            ..FakeBackend::default()
        };
        let registered = backend.registered.clone();
        let mut manager = HotkeyManager::new(Box::new(backend), default_hotkeys(), false);
        manager.register_all();
        assert_eq!(manager.failed().len(), 1);
        assert_eq!(manager.failed()[0].job(), HotkeyType::PrintScreen);
        assert_eq!(registered.lock().unwrap().len(), 4);
        let id = manager.id_of(0).unwrap();
        assert_eq!(
            manager.triggered(id).unwrap().job(),
            HotkeyType::RectangleRegion
        );
        manager.ignore = true;
        assert!(manager.triggered(id).is_none());
        manager.ignore = false;
        let region = if cfg!(windows) {
            "Ctrl + Print Screen"
        } else {
            "Cmd + Shift + 2"
        };
        assert_eq!(
            manager.shortcut_for(HotkeyType::RectangleRegion).as_deref(),
            Some(region)
        );
        assert_eq!(
            manager.shortcut_for(HotkeyType::PrintScreen),
            None,
            "a failed key is not shown"
        );
        manager.toggle(true);
        assert!(registered.lock().unwrap().is_empty());
        manager.toggle(false);
        assert_eq!(registered.lock().unwrap().len(), 4);
        let index = manager.add(HotkeySettings::new(
            HotkeyType::ActiveMonitor,
            Keys::CONTROL | 77,
        ));
        assert_eq!(manager.hotkeys[index].status, HotkeyStatus::Registered);
        assert_eq!(manager.move_by(index, 1), 0);
        assert_eq!(manager.hotkeys[0].job(), HotkeyType::ActiveMonitor);
        manager.remove(0);
        assert_eq!(registered.lock().unwrap().len(), 4);
        manager.reset();
        assert_eq!(manager.hotkeys.len(), 5);
        set_thread_language(None);
    }

    #[test]
    fn categories_and_names() {
        set_thread_language(Some(Language::English));
        assert_eq!(
            HotkeyType::RectangleRegion.category(),
            HotkeyCategory::ScreenCapture
        );
        assert_eq!(
            HotkeyType::ScreenRecorderGIF.category(),
            HotkeyCategory::ScreenRecord
        );
        assert_eq!(HotkeyType::ExitShareX.category(), HotkeyCategory::Other);
        assert_eq!(HotkeyType::OCR.category(), HotkeyCategory::Tools);
        assert!(!HotkeyType::RectangleRegion.text().is_empty());
        set_thread_language(None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_names_keys_and_defaults_are_upstreams() {
        set_thread_language(Some(Language::English));
        assert_eq!(modifier_names(), ["Ctrl", "Shift", "Alt", "Win"]);
        assert_eq!(
            HotkeyInfo::new(Keys::CONTROL | 44).text(),
            "Ctrl + Print Screen"
        );
        assert_eq!(HotkeyInfo::new(Keys::ALT | 44).text(), "Alt + Print Screen");
        assert_eq!(
            HotkeyInfo::new(145).text(),
            t(helpers::HotkeyInfo_Scroll_lock)
        );
        assert_eq!(HotkeyInfo::new(19).text(), "Pause");
        assert_eq!(HotkeyInfo::new(124).text(), "F13");
        assert_eq!(
            HotkeyInfo::new(Keys::CONTROL | Keys::SHIFT | 50).text(),
            "Ctrl + Shift + 2"
        );
        assert_eq!(
            HotkeyInfo::new(Keys::CONTROL | 99).text(),
            "Ctrl + Numpad 3"
        );
        let only = HotkeyInfo::new(Keys::CONTROL | 17);
        assert_eq!(only.text(), "Ctrl + ...");
        let win = HotkeyInfo {
            hotkey: Keys(Keys::SHIFT | 65),
            win: true,
        };
        assert_eq!(win.text(), "Shift + Win + A");
        // Keys are themselves on Windows: F13 is F13.
        let typed = gesture(Some("F13"), true, false, false, false);
        assert_eq!(typed.hotkey.to_value(), json!("F13, Control"));
        let print = gesture(Some("PrintScreen"), false, true, false, false);
        assert_eq!(print.hotkey.to_value(), json!("PrintScreen, Shift"));
        // Upstream's defaults.
        let defaults = default_hotkeys();
        let bindings: Vec<(HotkeyType, Value)> = defaults
            .iter()
            .map(|h| (h.job(), h.info.hotkey.to_value()))
            .collect();
        assert_eq!(
            bindings,
            [
                (HotkeyType::RectangleRegion, json!("PrintScreen, Control")),
                (HotkeyType::PrintScreen, json!("PrintScreen")),
                (HotkeyType::ActiveWindow, json!("PrintScreen, Alt")),
                (HotkeyType::ScreenRecorder, json!("PrintScreen, Shift")),
                (
                    HotkeyType::ScreenRecorderGIF,
                    json!("PrintScreen, Shift, Control")
                ),
            ]
        );
        assert!(defaults.iter().all(|h| h.info.is_valid() && !h.info.win));
        set_thread_language(None);
    }
}

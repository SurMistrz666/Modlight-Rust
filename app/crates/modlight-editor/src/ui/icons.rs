//! `EditorIcons`: the Lucide glyphs the editor uses, by upstream's names.

use modlight_core::icons as l;

use crate::model::EditorTool;

pub const TOOL_SELECT: char = l::MOUSE_POINTER_2;
pub const TOOL_RECTANGLE: char = l::SQUARE;
pub const TOOL_ELLIPSE: char = l::CIRCLE;
pub const TOOL_LINE: char = l::MINUS;
pub const TOOL_ARROW: char = l::ARROW_RIGHT;
pub const TOOL_FREEHAND: char = l::PENCIL;
pub const TOOL_HIGHLIGHT: char = l::HIGHLIGHTER;
pub const TOOL_TEXT: char = l::TYPE;
pub const TOOL_SPEECH_BALLOON: char = l::MESSAGE_SQUARE;
pub const TOOL_STEP: char = l::HASH;
pub const TOOL_CURSOR: char = l::MOUSE_POINTER;
pub const TOOL_BLUR: char = l::DROPLET;
pub const TOOL_PIXELATE: char = l::GRID_2X2;
pub const TOOL_MAGNIFY: char = l::SEARCH;
pub const TOOL_SPOTLIGHT: char = l::LIGHTBULB;
pub const TOOL_SMART_ERASER: char = l::ERASER;
pub const TOOL_CROP: char = l::CROP;
pub const TOOL_CUT_OUT: char = l::SCISSORS;
pub const TOOL_IMAGE: char = l::IMAGE;
pub const TOOL_EMOJI: char = l::SMILE;
pub const ACTION_UNDO: char = l::UNDO_2;
pub const ACTION_REDO: char = l::REDO_2;
pub const ACTION_DELETE: char = l::TRASH_2;
pub const ACTION_CLEAR_ALL: char = l::BRUSH_CLEANING;
pub const ACTION_COPY: char = l::CLIPBOARD;
pub const ACTION_PASTE: char = l::CLIPBOARD_PASTE;
pub const ACTION_DUPLICATE: char = l::COPY_PLUS;
pub const ACTION_BRING_TO_FRONT: char = l::BRING_TO_FRONT;
pub const ACTION_BRING_FORWARD: char = l::MOVE_UP;
pub const ACTION_SEND_BACKWARD: char = l::MOVE_DOWN;
pub const ACTION_SEND_TO_BACK: char = l::SEND_TO_BACK;
pub const ACTION_SAVE: char = l::SAVE;
pub const ACTION_SAVE_AS: char = l::SAVE_ALL;
pub const ACTION_PRINT: char = l::PRINTER;
pub const ACTION_DOWNLOAD: char = l::DOWNLOAD;
pub const ACTION_PIN_TO_SCREEN: char = l::PIN;
pub const ACTION_UPLOAD: char = l::CLOUD_UPLOAD;
pub const ACTION_CANCEL: char = l::X;
pub const ACTION_CONTINUE: char = l::PLAY;
pub const ACTION_RESET: char = l::REFRESH_CW;
pub const ACTION_ROTATE_LEFT: char = l::ROTATE_CCW_SQUARE;
pub const ACTION_ROTATE_RIGHT: char = l::ROTATE_CW_SQUARE;
pub const ACTION_TOGGLE_TOOLBARS: char = l::CHEVRONS_UP_DOWN;
pub const ACHIEVEMENT: char = l::TROPHY;
pub const FORMAT_BOLD: char = l::BOLD;
pub const FORMAT_ITALIC: char = l::ITALIC;
pub const MENU_THEME: char = l::MOON_STAR;
pub const TOGGLE_SPEECH_BALLOON_TAIL: char = l::MESSAGE_SQUARE_CHECK;
pub const TOGGLE_ELLIPSE_SHAPE: char = TOOL_ELLIPSE;
pub const PANEL_BACKGROUND: char = l::WALLPAPER;
pub const PANEL_EFFECTS: char = l::SPARKLES;
pub const LAYER_FLATTEN: char = l::LAYERS_2;
pub const CHEVRON_DOWN: char = l::CHEVRON_DOWN;
pub const ZOOM: char = l::ZOOM_IN;
pub const SCREEN_COLOR_PICKER: char = l::PIPETTE;
pub const FILE_MENU: char = l::FILE;
pub const FILE_NEW: char = l::FILE_PLUS;
pub const FILE_OPEN: char = l::FOLDER_OPEN;
pub const FILE_SAVE: char = l::SAVE;
pub const FILE_SAVE_AS: char = l::SAVE_ALL;
pub const FILE_EXIT: char = l::LOG_OUT;
pub const HISTORY: char = l::HISTORY;
pub const SETTINGS: char = l::SETTINGS;
pub const LIGHTBULB: char = l::LIGHTBULB;

/// `EditorIcons.ForTool`.
pub fn for_tool(tool: EditorTool) -> char {
    match tool {
        EditorTool::Select => TOOL_SELECT,
        EditorTool::Rectangle => TOOL_RECTANGLE,
        EditorTool::Ellipse => TOOL_ELLIPSE,
        EditorTool::Line => TOOL_LINE,
        EditorTool::Arrow => TOOL_ARROW,
        EditorTool::Freehand => TOOL_FREEHAND,
        EditorTool::Text => TOOL_TEXT,
        EditorTool::SpeechBalloon => TOOL_SPEECH_BALLOON,
        EditorTool::Step => TOOL_STEP,
        EditorTool::Cursor => TOOL_CURSOR,
        EditorTool::Blur => TOOL_BLUR,
        EditorTool::Pixelate => TOOL_PIXELATE,
        EditorTool::Magnify => TOOL_MAGNIFY,
        EditorTool::Spotlight => TOOL_SPOTLIGHT,
        EditorTool::SmartEraser => TOOL_SMART_ERASER,
        EditorTool::Highlight => TOOL_HIGHLIGHT,
        EditorTool::Crop => TOOL_CROP,
        EditorTool::CutOut => TOOL_CUT_OUT,
        EditorTool::Image => TOOL_IMAGE,
        EditorTool::Emoji => TOOL_EMOJI,
    }
}

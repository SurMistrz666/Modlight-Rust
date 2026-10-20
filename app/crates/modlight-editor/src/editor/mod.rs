//! The editor without a window: `MainViewModel` and the parts of `EditorView` and its
//! controllers that are not drawing. Input arrives as pointer and key events in image
//! coordinates; what the host must do (file pickers, clipboard, save, upload, close)
//! goes out as `Request`s. The egui view in `ui` draws this state and feeds it events,
//! and the scripted tests drive it the same way.

pub mod background;
pub mod effects_flow;
pub mod input;
pub mod toolbar;
pub mod tools;

use std::sync::Arc;
use std::time::{Duration, Instant};

use image::RgbaImage;

use crate::annot_effects::{self, EffectCache};
use crate::core::{CoreEvents, EditorCore};
use crate::frame;
use crate::geom::{Point, Rect};
use crate::model::{Annotation, AnnotationId, EditorTool, Kind};
use crate::options::ImageEditorOptions;

pub use background::BackgroundSettings;
pub use input::{Button, InputState, Modifiers};
pub use toolbar::ToolbarItem;
pub use tools::{ToolState, Visibility};

/// `EditorTaskResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TaskResult {
    #[default]
    None,
    Continue,
    ContinueNoSave,
    Cancel,
}

/// What the host has to do for the editor.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Copy the result image (`CopyRequested`).
    CopyImage,
    /// Save to `ImageFilePath`, or ask the host where (`SaveRequested`).
    Save,
    SaveAs,
    Print,
    PinToScreen,
    Upload,
    /// Pick an image to open (`OpenImageRequested`).
    OpenImage,
    /// Pick an image to insert (`ImageInsertionRequested`).
    InsertImage,
    /// Pick an image to replace an image annotation's picture.
    ReplaceImage(AnnotationId),
    /// Load the image the clipboard holds (`LoadFromClipboardRequested`).
    LoadFromClipboard,
    /// Download and open (`LoadFromUrlRequested`).
    LoadFromUrl(String),
    /// Pick a background image.
    BrowseBackgroundImage,
    /// Paste: the host looks at the system clipboard first (`OnPasteRequested`).
    Paste,
    /// Clear the system clipboard (copy/cut of an annotation does).
    ClearClipboard,
    /// Pick a colour from the screen (`ScreenColorPickerWindow`).
    PickScreenColor(ColorTarget),
    /// The window should close (`CloseRequested`).
    Close,
}

/// Which colour a screen pick goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorTarget {
    Stroke,
    Fill,
    Text,
    Shadow,
    Background,
    Gradient1,
    Gradient2,
}

/// The modal dialogs (`ModalContent`).
#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Confirmation,
    NewImage(NewImageState),
    InsertImage { width: u32, height: u32 },
    EmojiPicker(EmojiPickerState),
    StartScreen(StartScreenState),
    ToolbarCustomization(Vec<ToolbarItem>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewImageState {
    pub width: i32,
    pub height: i32,
    pub transparent: bool,
    pub background: crate::color::Color,
}

impl Default for NewImageState {
    fn default() -> Self {
        NewImageState {
            width: 800,
            height: 600,
            transparent: true,
            background: crate::color::Color::WHITE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StartScreenState {
    pub recent_files: Vec<String>,
    pub url: String,
    pub url_input_visible: bool,
    pub url_loading: bool,
    pub status: Option<String>,
}

/// Who receives the emoji picked in the picker.
#[derive(Clone, Debug, PartialEq)]
pub enum EmojiTarget {
    Insert,
    Replace(AnnotationId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmojiPickerState {
    pub search: String,
    pub group: Option<String>,
    pub target: EmojiTarget,
}

/// The toast at the bottom (`MainViewModel.Notifications`).
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub message: String,
    pub icon: Option<char>,
    pub shown_at: Instant,
    pub duration: Duration,
    pub hovered: bool,
    pub closing_since: Option<Instant>,
}

pub const NOTIFICATION_DURATION: Duration = Duration::from_millis(2400);
pub const NOTIFICATION_HIDE: Duration = Duration::from_millis(220);

/// How the host opened the editor (`ImageEditorIntegration.ShowEditorDialog`).
#[derive(Clone, Debug)]
pub struct EditorConfig {
    pub show_file_menu: bool,
    pub show_options_button: bool,
    pub show_task_buttons: bool,
    pub use_continue_workflow: bool,
    pub show_bottom_toolbar: bool,
    pub show_start_screen: bool,
    pub open_background_panel: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig {
            show_file_menu: true,
            show_options_button: true,
            show_task_buttons: true,
            use_continue_workflow: false,
            show_bottom_toolbar: true,
            show_start_screen: true,
            open_background_panel: false,
        }
    }
}

/// The text box over an annotation being written (`HandleTextTool`, `ShowTextEditor`,
/// `ShowSpeechBalloonTextEditor`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextEditing {
    pub id: AnnotationId,
    pub text: String,
    /// A new text annotation (its box has a white border and is dropped when empty).
    pub creating: bool,
    pub balloon: bool,
}

pub const ZOOM_LEVELS: [f64; 9] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
pub const MIN_ZOOM: f64 = 0.25;
pub const MAX_ZOOM: f64 = 4.0;

pub struct Editor {
    pub core: EditorCore,
    pub options: ImageEditorOptions,
    pub config: EditorConfig,
    pub tools: ToolState,
    pub(crate) loading_options: bool,
    /// `SelectedAnnotation` of the view model (drives the options shown in Select mode).
    pub vm_selected: Option<AnnotationId>,
    pub input: InputState,
    pub text_editing: Option<TextEditing>,
    pub toolbar_items: Vec<ToolbarItem>,
    pub background: BackgroundSettings,
    pub effects: EffectCache,
    spotlight_cache: Option<(u64, f32, Arc<RgbaImage>)>,
    clipboard_annotation: Option<Annotation>,
    requests: Vec<Request>,
    pub modal: Option<Modal>,
    pub notification: Option<Notification>,
    pub image_file_path: Option<String>,
    pub is_dirty: bool,
    pub task_result: TaskResult,
    pub zoom: f64,
    /// Pixels per point of the window (`DpiScale`); 100 % shows one image pixel per
    /// physical pixel (`EffectiveZoom = Zoom / DpiScale`).
    pub dpi_scale: f64,
    pub show_toolbars: bool,
    pub is_settings_panel_open: bool,
    pub is_effects_panel_open: bool,
    /// The right sidebar's content: `None` is the effect browser (centre panel),
    /// `Some` a dialog or the editor options.
    pub effects_panel: Option<SidePanel>,
    pub can_paste: bool,
    zoom_to_fit_on_next_load: bool,
    /// Set when the view should fit the image (zoom to fit, on open).
    pub zoom_to_fit_requested: bool,
    pub preview: Option<crate::effects::PreviewState>,
    /// Bumped on every visual change, for the view's texture caches.
    pub revision: u64,
}

/// What the right sidebar holds.
#[derive(Clone, Debug, PartialEq)]
pub enum SidePanel {
    Options,
    Effect(crate::effects::DialogState),
}

impl Editor {
    pub fn new(options: ImageEditorOptions, config: EditorConfig) -> Editor {
        let mut editor = Editor {
            core: EditorCore::new(),
            options,
            config,
            tools: ToolState::default(),
            loading_options: false,
            vm_selected: None,
            input: InputState::default(),
            text_editing: None,
            toolbar_items: Vec::new(),
            background: BackgroundSettings::default(),
            effects: EffectCache::default(),
            spotlight_cache: None,
            clipboard_annotation: None,
            requests: Vec::new(),
            modal: None,
            notification: None,
            image_file_path: None,
            is_dirty: false,
            task_result: TaskResult::None,
            zoom: 1.0,
            dpi_scale: 1.0,
            show_toolbars: true,
            is_settings_panel_open: false,
            is_effects_panel_open: false,
            effects_panel: None,
            can_paste: false,
            zoom_to_fit_on_next_load: false,
            zoom_to_fit_requested: false,
            preview: None,
            revision: 1,
        };
        editor.toolbar_items = toolbar::items_from_options(&editor.options.toolbar_items);
        if editor.options.toolbar_items.is_empty() {
            editor.options.toolbar_items = editor
                .toolbar_items
                .iter()
                .map(|i| i.to_options())
                .collect();
        }
        // `_activeTool = GetInitialAnnotationTool()`, then the initial values from the
        // options (the constructor's field initialisers), then the tool's options.
        let initial = if tools::is_rememberable(editor.options.last_used_annotation_tool) {
            editor.options.last_used_annotation_tool
        } else {
            EditorTool::Rectangle
        };
        let o = editor.options.clone();
        editor.tools.active_tool = initial;
        editor.core.active_tool = initial;
        editor.tools.text_color = o.text_text_color().to_hex_argb();
        editor.tools.fill_color = o.fill_color().to_hex_argb();
        editor.tools.selected_color = o.border_color().to_hex_argb();
        editor.tools.stroke_width = o.thickness;
        editor.tools.corner_radius = o.corner_radius;
        editor.tools.font_size = o.text_font_size;
        editor.tools.font_family = if o.text_font_family.trim().is_empty() {
            "Segoe UI".into()
        } else {
            o.text_font_family.clone()
        };
        editor.tools.border_style = o.border_style;
        editor.tools.arrow_style = o.arrow_style;
        editor.tools.shadow_enabled = o.shadow;
        editor.tools.shadow_color = o.shadow_color_hex.clone();
        editor.tools.shadow_blur_radius = o.shadow_blur_radius;
        editor.tools.shadow_opacity = o.shadow_opacity;
        editor.tools.shadow_offset_x = o.shadow_offset_x;
        editor.tools.shadow_offset_y = o.shadow_offset_y;
        editor.tools.text_bold = o.text_bold;
        editor.tools.text_italic = o.text_italic;
        editor.loading_options = true;
        editor.load_options_for_tool(initial);
        editor.loading_options = false;
        editor.background = BackgroundSettings::from_options(&editor.options);
        if editor.config.open_background_panel {
            editor.is_settings_panel_open = true;
        }
        toolbar::update_active_states(&mut editor);
        editor
    }

    pub fn annotation(&self, id: AnnotationId) -> Option<&Annotation> {
        self.core
            .annotation(id)
            .or_else(|| self.input.pending.as_ref().filter(|p| p.id == id))
    }

    pub fn has_image(&self) -> bool {
        self.core.source().is_some()
    }

    pub fn image_size(&self) -> (u32, u32) {
        self.core.source().map(|s| s.dimensions()).unwrap_or((0, 0))
    }

    pub fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn request(&mut self, request: Request) {
        self.requests.push(request);
    }

    pub fn has_annotations(&self) -> bool {
        !self.core.annotations().is_empty()
    }

    /// `HasSelectedAnnotation`.
    pub fn has_selection(&self) -> bool {
        self.input.sel.selected.is_some()
    }

    pub fn selected(&self) -> Option<AnnotationId> {
        self.input.sel.selected
    }

    /// Applies a change to the selected annotation (the view's `Apply*`).
    pub(crate) fn with_selected(&mut self, f: impl FnOnce(&mut Annotation)) {
        if let Some(id) = self.input.sel.selected {
            if let Some(a) = self.core.annotation_mut(id) {
                f(a);
            } else if let Some(p) = self.input.pending.as_mut().filter(|p| p.id == id) {
                f(p);
            }
            self.invalidate();
        }
    }

    /// Regenerates the selected effect annotation's bitmap (`OnRequestUpdateEffect`).
    pub(crate) fn refresh_selected_effect(&mut self) {
        if let Some(id) = self.input.sel.selected {
            self.refresh_effect(id);
        }
    }

    pub(crate) fn refresh_effect(&mut self, id: AnnotationId) {
        let Some(source) = self.core.source().cloned() else {
            return;
        };
        let generation = self.core.generation();
        if let Some(a) = self.core.annotation_mut(id) {
            annot_effects::update_effect(a, &source, generation, &mut self.effects);
        } else if let Some(p) = self.input.pending.as_mut().filter(|p| p.id == id) {
            annot_effects::update_effect(p, &source, generation, &mut self.effects);
        }
        self.invalidate();
    }

    fn refresh_all_effects(&mut self) {
        let ids: Vec<AnnotationId> = self
            .core
            .annotations()
            .iter()
            .filter(|a| a.is_effect())
            .map(|a| a.id)
            .collect();
        for id in ids {
            self.refresh_effect(id);
        }
    }

    /// The blurred image behind spotlights (`SpotlightOverlayControl.GetBlurredSource`).
    pub fn spotlight_blur_source(&mut self) -> Option<Arc<RgbaImage>> {
        let amount = crate::render::spotlight_blur_amount(self.core.annotations());
        let source = self.core.source()?.clone();
        if amount <= 0.0 {
            self.spotlight_cache = None;
            return None;
        }
        let generation = self.core.generation();
        if let Some((g, a, image)) = &self.spotlight_cache
            && *g == generation
            && (*a - amount).abs() < f32::EPSILON
        {
            return Some(image.clone());
        }
        let blurred = Arc::new(annot_effects::blurred_source(&source, amount));
        self.spotlight_cache = Some((generation, amount, blurred.clone()));
        Some(blurred)
    }

    /// Reacts to what the core reported (`ImageChanged`, `AnnotationsRestored`,
    /// `HistoryChanged` handlers of the view). Called after every operation.
    pub fn pump(&mut self) {
        let events = self.core.take_events();
        if events == CoreEvents::default() {
            return;
        }
        if events.image_changed || events.annotations_restored {
            self.effects.clear();
            self.refresh_all_effects();
            self.background.invalidate_smart_padding();
            if let Some(source) = self.core.source().cloned() {
                self.background
                    .ensure_smart_padding(&source, self.is_settings_panel_open);
            }
        }
        if events.annotations_restored {
            // `OnAnnotationsRestored`: the canvas is rebuilt and the selection cleared.
            input::clear_selection(self);
            if let Some(core_selected) = self.core.selected()
                && self.core.annotation(core_selected).is_some()
            {
                // The core keeps its own selection for z-order; the view's is cleared.
            }
        }
        if events.history_changed {
            self.recalculate_number_counter();
            if !self.input.suppress_dirty {
                self.is_dirty = true;
            }
            self.input.suppress_dirty = false;
            if self.options.auto_copy_image_to_clipboard && self.has_image() {
                self.requests.push(Request::CopyImage);
            }
        }
        self.invalidate();
    }

    /// `RecalculateNumberCounter`.
    pub fn recalculate_number_counter(&mut self) {
        let max = self
            .core
            .annotations()
            .iter()
            .filter_map(|a| match &a.kind {
                Kind::Number(n) => Some(n.number),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        self.tools.number_counter = (max + 1).max(self.tools.step_start_number);
    }

    // ------------------------------------------------------------------ images

    /// `UpdatePreview`: a new image from the host (clears annotations and history).
    pub fn load_image(&mut self, image: RgbaImage, path: Option<String>) {
        self.cancel_effect_preview();
        input::clear_selection(self);
        input::cancel_crop(self);
        self.input.pending = None;
        self.text_editing = None;
        self.input.suppress_dirty = true;
        self.core.load_image(image);
        self.image_file_path = path;
        self.zoom = 1.0;
        self.zoom_to_fit_on_next_load = self.options.zoom_to_fit_on_open;
        if self.zoom_to_fit_on_next_load {
            self.zoom_to_fit_requested = true;
            self.zoom_to_fit_on_next_load = false;
        }
        self.tools.number_counter = self.tools.step_start_number;
        self.pump();
        self.is_dirty = false;
        if self.options.auto_copy_image_to_clipboard && self.has_image() {
            self.requests.push(Request::CopyImage);
        }
    }

    /// `LoadBitmapIntoEditor` (open, recent file, clipboard, URL).
    pub fn load_bitmap(&mut self, image: RgbaImage, path: Option<String>) {
        self.load_image(image, path.clone());
        if let Some(path) = path.filter(|p| !p.is_empty()) {
            self.options.add_recent_image_file(&path);
        }
    }

    /// `OnNewImageRequested`'s OK: a blank image.
    pub fn create_new_image(&mut self, state: &NewImageState) {
        let w = state.width.max(1) as u32;
        let h = state.height.max(1) as u32;
        let fill = if state.transparent {
            image::Rgba([0, 0, 0, 0])
        } else {
            let c = state.background;
            image::Rgba([c.r, c.g, c.b, c.a])
        };
        self.modal = None;
        self.load_image(RgbaImage::from_pixel(w, h, fill), None);
        self.show_notification(
            &image_size_notification(
                modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_NewImageCreated),
                w,
                h,
            ),
            Some(crate::ui::icons::FILE_NEW),
        );
    }

    /// `GetSnapshot`: the image as the frame shows it, annotations flattened.
    pub fn snapshot(&mut self) -> Option<RgbaImage> {
        let source = self.core.source()?.clone();
        let blur = self.spotlight_blur_source();
        let state = self.background_state();
        Some(frame::snapshot(
            &state,
            &source,
            self.core.annotations(),
            blur.as_deref(),
        ))
    }

    /// `GetSource`: the image without annotations.
    pub fn source_image(&self) -> Option<RgbaImage> {
        self.core.snapshot()
    }

    pub fn background_state(&self) -> frame::BackgroundState {
        self.background
            .state(self.are_background_effects_active(), &self.options)
    }

    /// `AreBackgroundEffectsActive`.
    pub fn are_background_effects_active(&self) -> bool {
        self.is_settings_panel_open && self.preview.is_none()
    }

    // ------------------------------------------------------------------ commands

    pub fn undo(&mut self) {
        if self.core.can_undo() {
            self.core.undo();
            self.pump();
        }
    }

    pub fn redo(&mut self) {
        if self.core.can_redo() {
            self.core.redo();
            self.pump();
        }
    }

    /// `PerformDelete`.
    pub fn delete_selected(&mut self) {
        let Some(id) = self.input.sel.selected else {
            return;
        };
        if self.core.annotation(id).is_none() {
            return;
        }
        self.core.select(Some(id));
        self.core.delete_selected();
        input::clear_selection(self);
        self.pump();
    }

    /// `ClearAnnotationsCommand`: everything removed in one undoable step.
    pub fn clear_annotations(&mut self) {
        if !self.has_annotations() {
            return;
        }
        input::clear_selection(self);
        self.core.clear_all(false);
        self.tools.number_counter = self.tools.step_start_number;
        self.pump();
    }

    /// `FlattenImageCommand`.
    pub fn flatten(&mut self) {
        if !self.has_annotations() {
            return;
        }
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        if self.core.flatten_image(snapshot) {
            input::clear_selection(self);
        }
        self.pump();
    }

    pub fn bring_to_front(&mut self) {
        self.sync_core_selection();
        self.core.bring_to_front();
        self.pump();
    }

    pub fn send_to_back(&mut self) {
        self.sync_core_selection();
        self.core.send_to_back();
        self.pump();
    }

    pub fn bring_forward(&mut self) {
        self.sync_core_selection();
        self.core.bring_forward();
        self.pump();
    }

    pub fn send_backward(&mut self) {
        self.sync_core_selection();
        self.core.send_backward();
        self.pump();
    }

    fn sync_core_selection(&mut self) {
        if let Some(id) = self.input.sel.selected {
            self.core.select(Some(id));
        }
    }

    /// `OnCopyRequested`: the annotation goes to the editor's own clipboard; the system
    /// clipboard is cleared so a later paste takes the annotation.
    pub fn copy_annotation(&mut self) {
        if let Some(a) = self
            .input
            .sel
            .selected
            .and_then(|id| self.annotation(id))
            .cloned()
        {
            self.clipboard_annotation = Some(a.clone_for_history());
            self.can_paste = true;
            self.requests.push(Request::ClearClipboard);
        }
    }

    /// `OnCutRequested`.
    pub fn cut_annotation(&mut self) {
        if self.input.sel.selected.is_some() {
            self.copy_annotation();
            self.delete_selected();
        }
    }

    pub fn has_clipboard_annotation(&self) -> bool {
        self.clipboard_annotation.is_some()
    }

    /// `PasteInternalShape`: a copy 20 px down and right, selected.
    pub fn paste_annotation(&mut self) {
        let Some(mut clone) = self.clipboard_annotation.clone() else {
            return;
        };
        clone.id = crate::model::new_id();
        offset_clone(&mut clone, 20.0);
        let id = clone.id;
        self.core.add_annotation(clone);
        self.refresh_effect(id);
        input::set_selected_shape(self, id);
        self.pump();
    }

    /// `DuplicateSelectedAnnotation` (Cmd+D).
    pub fn duplicate_selected(&mut self) {
        let Some(original) = self
            .input
            .sel
            .selected
            .and_then(|id| self.core.annotation(id))
            .cloned()
        else {
            return;
        };
        let mut clone = original.clone_for_history();
        clone.id = crate::model::new_id();
        offset_clone(&mut clone, 20.0);
        let id = clone.id;
        self.core.add_annotation(clone);
        self.refresh_effect(id);
        input::set_selected_shape(self, id);
        self.pump();
    }

    /// `SelectToolCommand`.
    pub fn select_tool(&mut self, tool: EditorTool) {
        match tool {
            EditorTool::Image => {
                self.deselect();
                if self.modal.is_none() {
                    self.requests.push(Request::InsertImage);
                }
                return;
            }
            EditorTool::Emoji => {
                self.deselect();
                self.show_emoji_picker(EmojiTarget::Insert);
                return;
            }
            _ => {}
        }
        if self.tools.active_tool == tool && matches!(tool, EditorTool::Crop | EditorTool::CutOut) {
            return;
        }
        self.deselect();
        if matches!(tool, EditorTool::Crop | EditorTool::CutOut) {
            self.set_settings_panel_open(false);
        }
        self.set_active_tool(tool);
    }

    /// `DeselectRequested`: cancel a crop, clear the selection.
    pub fn deselect(&mut self) {
        input::cancel_crop(self);
        input::clear_selection(self);
    }

    /// The `ActiveTool` property change with the view's reaction (crop activation).
    pub fn set_active_tool(&mut self, tool: EditorTool) {
        let changed = self.tools.active_tool != tool;
        self.set_active_tool_property(tool);
        toolbar::update_active_states(self);
        if changed || tool == EditorTool::Crop {
            if tool == EditorTool::Crop {
                if self.options.quick_crop {
                    input::cancel_crop(self);
                } else {
                    input::activate_crop_to_full_image(self);
                }
            } else {
                input::cancel_crop(self);
            }
            input::clear_selection(self);
        }
        self.invalidate();
    }

    pub fn show_emoji_picker(&mut self, target: EmojiTarget) {
        if self.modal.is_some() {
            return;
        }
        self.modal = Some(Modal::EmojiPicker(EmojiPickerState {
            search: String::new(),
            group: None,
            target,
        }));
    }

    /// The picker's choice (`EmojiInsertionRequested` or the replacement callback).
    pub fn emoji_picked(&mut self, unicode: &str, name: &str) {
        let target = match &self.modal {
            Some(Modal::EmojiPicker(state)) => state.target.clone(),
            _ => EmojiTarget::Insert,
        };
        self.modal = None;
        match target {
            EmojiTarget::Insert => self.insert_emoji(unicode, name, None),
            EmojiTarget::Replace(id) => {
                if let Some(a) = self.core.annotation_mut(id)
                    && let Kind::Emoji {
                        unicode: u,
                        display_name,
                        bitmap,
                    } = &mut a.kind
                {
                    *u = unicode.to_string();
                    *display_name = name.to_string();
                    *bitmap = None;
                }
                self.is_dirty = true;
                self.invalidate();
            }
        }
    }

    /// `InsertEmojiAnnotation`: 160 px in the middle, selected, Select tool.
    pub fn insert_emoji(&mut self, unicode: &str, name: &str, at: Option<Point>) {
        const SIZE: f32 = 160.0;
        let (w, h) = self.core.canvas_size();
        let pos = at.unwrap_or(Point::new(w / 2.0 - SIZE / 2.0, h / 2.0 - SIZE / 2.0));
        let mut a = Annotation::emoji(unicode, name);
        a.start = pos;
        a.end = Point::new(pos.x + SIZE, pos.y + SIZE);
        let id = a.id;
        self.core.add_annotation(a);
        self.set_active_tool(EditorTool::Select);
        input::set_selected_shape(self, id);
        self.pump();
    }

    /// `InsertExternalImageAsync`: a picked, pasted or dropped picture. Without an image
    /// it becomes the image; otherwise the insert dialog asks where (or it goes to the
    /// middle when that dialog is turned off).
    pub fn insert_external_image(&mut self, image: RgbaImage, path: Option<String>) {
        if !self.has_image() {
            self.load_bitmap(image, path);
            return;
        }
        if self.options.show_insert_image_dialog {
            if self.modal.is_some() {
                return;
            }
            self.input.pending_insert = Some(Arc::new(image));
            let (w, h) = self
                .input
                .pending_insert
                .as_ref()
                .map(|i| i.dimensions())
                .unwrap_or((0, 0));
            self.modal = Some(Modal::InsertImage {
                width: w,
                height: h,
            });
        } else {
            self.insert_image_with_placement(Arc::new(image), InsertPlacement::Center);
        }
    }

    /// The insert dialog's answer.
    pub fn insert_image_choice(&mut self, placement: Option<InsertPlacement>) {
        self.modal = None;
        let Some(image) = self.input.pending_insert.take() else {
            return;
        };
        if let Some(placement) = placement {
            self.insert_image_with_placement(image, placement);
        }
    }

    fn insert_image_with_placement(&mut self, image: Arc<RgbaImage>, placement: InsertPlacement) {
        let (cw, ch) = self.core.canvas_size();
        let (cw, ch) = (cw.round() as i32, ch.round() as i32);
        let (iw, ih) = (image.width() as i32, image.height() as i32);
        let position = match placement {
            InsertPlacement::Center => None,
            InsertPlacement::CanvasExpandDown => {
                let right = (iw - cw).max(0);
                self.core
                    .resize_canvas(0, right, ih, 0, crate::color::Color::TRANSPARENT);
                Some(Point::new(0.0, ch as f32))
            }
            InsertPlacement::CanvasExpandRight => {
                let bottom = (ih - ch).max(0);
                self.core
                    .resize_canvas(0, iw, bottom, 0, crate::color::Color::TRANSPARENT);
                Some(Point::new(cw as f32, 0.0))
            }
        };
        self.pump();
        self.insert_image_annotation(image, position, true);
    }

    /// `InsertImageAnnotationCore`.
    pub fn insert_image_annotation(
        &mut self,
        image: Arc<RgbaImage>,
        at: Option<Point>,
        notify: bool,
    ) {
        let (w, h) = (image.width() as f32, image.height() as f32);
        let pos = at.unwrap_or_else(|| {
            let c = self.input.visible_center.unwrap_or_else(|| {
                let (cw, ch) = self.core.canvas_size();
                Point::new(cw / 2.0, ch / 2.0)
            });
            Point::new(c.x - w / 2.0, c.y - h / 2.0)
        });
        let mut a = Annotation::image(Some(image));
        a.start = pos;
        a.end = Point::new(pos.x + w, pos.y + h);
        let id = a.id;
        self.core.add_annotation(a);
        self.set_active_tool(EditorTool::Select);
        input::set_selected_shape(self, id);
        self.pump();
        if notify {
            self.show_notification(
                modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_ImageInserted),
                Some(crate::ui::icons::TOOL_IMAGE),
            );
        }
    }

    /// `ReplaceImageAnnotationFromFilePickerAsync`'s result: same centre, new size.
    pub fn replace_image(&mut self, id: AnnotationId, image: RgbaImage, path: Option<String>) {
        if let Some(a) = self.core.annotation_mut(id) {
            let b = a.bounds();
            let (cx, cy) = (b.mid_x(), b.mid_y());
            let (w, h) = (image.width() as f32, image.height() as f32);
            a.start = Point::new(cx - w / 2.0, cy - h / 2.0);
            a.end = Point::new(cx + w / 2.0, cy + h / 2.0);
            if let Kind::Image { path: p, .. } = &mut a.kind {
                *p = path.unwrap_or_default();
            }
            a.set_image(Some(Arc::new(image)));
            self.is_dirty = true;
            self.invalidate();
        }
    }

    // ------------------------------------------------------------------ panels

    /// `ToggleSettingsPanelCommand` (Background).
    pub fn set_settings_panel_open(&mut self, open: bool) {
        self.is_settings_panel_open = open;
        if let Some(source) = self.core.source().cloned() {
            if open {
                self.background.invalidate_smart_padding();
            }
            self.background.ensure_smart_padding(&source, open);
            self.background
                .disable_smart_padding_if_unavailable(&mut self.options);
        }
        toolbar::update_active_states(self);
        self.invalidate();
    }

    /// `ToggleEffectsPanelCommand`.
    pub fn toggle_effects_panel(&mut self) {
        if !self.has_image() {
            return;
        }
        if self.is_effects_panel_open && self.effects_panel.is_none() {
            self.is_effects_panel_open = false;
        } else if self.is_effects_panel_open && self.effects_panel.is_some() {
            self.cancel_effect_preview();
            self.effects_panel = None;
            self.is_effects_panel_open = true;
        } else {
            self.effects_panel = None;
            self.is_effects_panel_open = true;
        }
        toolbar::update_active_states(self);
    }

    /// `CloseEffectsPanelCommand`.
    pub fn close_effects_panel(&mut self) {
        self.cancel_effect_preview();
        self.is_effects_panel_open = false;
        self.effects_panel = None;
        toolbar::update_active_states(self);
    }

    /// `OnOpenOptionsPanelRequested`: the options in the right sidebar (a toggle).
    pub fn toggle_options_panel(&mut self) {
        if self.is_effects_panel_open && self.effects_panel == Some(SidePanel::Options) {
            self.close_effects_panel();
            return;
        }
        if self.is_effects_panel_open {
            self.close_effects_panel();
        }
        self.effects_panel = Some(SidePanel::Options);
        self.is_effects_panel_open = true;
        toolbar::update_active_states(self);
    }

    pub fn is_effect_browser_visible(&self) -> bool {
        self.is_effects_panel_open && self.effects_panel.is_none()
    }

    pub fn is_right_sidebar_visible(&self) -> bool {
        self.is_effects_panel_open && self.effects_panel.is_some()
    }

    // ------------------------------------------------------------------ zoom

    /// `Zoom` with its clamp.
    pub fn set_zoom(&mut self, zoom: f64) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.invalidate();
    }

    pub fn zoom_in(&mut self) {
        self.set_zoom(((self.zoom + 0.1) * 100.0).round() / 100.0);
    }

    pub fn zoom_out(&mut self) {
        self.set_zoom(((self.zoom - 0.1) * 100.0).round() / 100.0);
    }

    /// The scale from image pixels to points (`EffectiveZoom`).
    pub fn effective_zoom(&self) -> f64 {
        self.zoom / self.dpi_scale.max(0.01)
    }

    /// One step of `ZoomLevels` (wheel with Cmd, or a pinch).
    pub fn zoom_step(&mut self, direction: i32) -> bool {
        let current = ZOOM_LEVELS
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1 - self.zoom)
                    .abs()
                    .partial_cmp(&(b.1 - self.zoom).abs())
                    .unwrap()
            })
            .map(|(i, _)| i)
            .unwrap_or(3) as i32;
        let next = (current + direction.signum()).clamp(0, ZOOM_LEVELS.len() as i32 - 1);
        if next == current {
            return false;
        }
        let zoom = ZOOM_LEVELS[next as usize];
        if (zoom - self.zoom).abs() < 0.0001 {
            return false;
        }
        self.set_zoom(zoom);
        true
    }

    /// `ZoomToFit`: the frame inside the viewport less a 24-point margin.
    pub fn zoom_to_fit(&mut self, viewport: (f64, f64)) {
        let layout = frame::layout(&self.background_state(), self.core.canvas_size());
        let (cw, ch) = (layout.width as f64, layout.height as f64);
        if cw <= 0.0 || ch <= 0.0 || viewport.0 <= 0.0 || viewport.1 <= 0.0 {
            return;
        }
        let aw = (viewport.0 - 24.0).max(1.0);
        let ah = (viewport.1 - 24.0).max(1.0);
        let fit = (aw / cw).min(ah / ch) * self.dpi_scale;
        self.set_zoom(fit);
        self.input.center_on_zoom = true;
    }

    // ------------------------------------------------------------------ task flow

    /// `ContinueCommand`.
    pub fn continue_task(&mut self) {
        self.task_result = TaskResult::Continue;
        self.requests.push(Request::Close);
    }

    /// `CancelCommand` / `ExitEditorCommand` / closing the window (`RequestClose`).
    pub fn request_close(&mut self, ignore_modal: bool) {
        if self.modal.is_some() && !ignore_modal {
            return;
        }
        if self.modal.is_some() && ignore_modal {
            self.modal = None;
        }
        self.task_result = TaskResult::Cancel;
        if self.options.show_exit_confirmation && self.is_dirty {
            self.modal = Some(Modal::Confirmation);
        } else {
            self.requests.push(Request::Close);
        }
    }

    /// The confirmation's answers: Yes saves then closes, No closes, Cancel stays.
    pub fn confirmation_answer(&mut self, answer: Option<bool>) {
        self.modal = None;
        match answer {
            Some(true) => {
                self.requests.push(Request::Save);
                self.requests.push(Request::Close);
            }
            Some(false) => self.requests.push(Request::Close),
            None => {}
        }
    }

    /// `CloseAfterTaskActionIfEnabled`.
    pub fn close_after_task_action(&mut self) {
        if self.options.auto_close_editor_on_task {
            self.task_result = TaskResult::Cancel;
            self.requests.push(Request::Close);
        }
    }

    // ------------------------------------------------------------------ notifications

    /// `ShowNotification`.
    pub fn show_notification(&mut self, message: &str, icon: Option<char>) {
        if !self.options.show_notifications || message.trim().is_empty() {
            return;
        }
        self.notification = Some(Notification {
            message: message.to_string(),
            icon,
            shown_at: Instant::now(),
            duration: NOTIFICATION_DURATION,
            hovered: false,
            closing_since: None,
        });
    }

    /// `ShowTaskActionNotification`: not when the editor closes after task actions.
    pub fn show_task_action_notification(&mut self, message: &str, icon: char) {
        if self.options.auto_close_editor_on_task {
            return;
        }
        self.show_notification(message, Some(icon));
    }

    pub fn dismiss_notification(&mut self) {
        self.notification = None;
    }

    /// Advances the toast's timers; true while it is on screen.
    pub fn tick_notification(&mut self, now: Instant) -> bool {
        let Some(n) = &mut self.notification else {
            return false;
        };
        if n.hovered {
            n.shown_at = now - n.duration.min(now - n.shown_at);
            return true;
        }
        if n.closing_since.is_none() && now.duration_since(n.shown_at) >= n.duration {
            n.closing_since = Some(now);
        }
        if let Some(since) = n.closing_since
            && now.duration_since(since) >= NOTIFICATION_HIDE
        {
            self.notification = None;
            return false;
        }
        true
    }

    pub fn image_cropped_notification(&mut self) {
        let (w, h) = self.image_size();
        let text = image_size_notification(
            modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_ImageCropped),
            w,
            h,
        );
        self.show_notification(&text, Some(crate::ui::icons::TOOL_CROP));
    }

    pub fn image_cut_out_notification(&mut self) {
        let (w, h) = self.image_size();
        let text = image_size_notification(
            modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_ImageCutOut),
            w,
            h,
        );
        self.show_notification(&text, Some(crate::ui::icons::TOOL_CUT_OUT));
    }

    /// The file-path toast after saving (`ShowSaveNotification`).
    pub fn saved_notification(&mut self, path: &str, save_as: bool) {
        if path.trim().is_empty() {
            return;
        }
        let text = file_path_notification(
            modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_ImageSavedToFile),
            path,
        );
        let icon = if save_as {
            crate::ui::icons::ACTION_SAVE_AS
        } else {
            crate::ui::icons::ACTION_SAVE
        };
        self.show_task_action_notification(&text, icon);
    }

    pub fn opened_notification(&mut self, path: &str) {
        if path.trim().is_empty() {
            return;
        }
        let text = file_path_notification(
            modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_ImageOpened),
            path,
        );
        self.show_notification(&text, Some(crate::ui::icons::FILE_OPEN));
    }

    /// The window title (`BuildWindowTitle`).
    pub fn window_title(&self) -> String {
        let mut title =
            modlight_core::l10n::t(modlight_core::l10n::editor::MainViewModel_WindowTitle)
                .to_string();
        let (w, h) = self.image_size();
        if w > 0 && h > 0 {
            title.push_str(&format!(" - {w}x{h}"));
        }
        if let Some(name) = self
            .image_file_path
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .and_then(|n| n.to_str())
        {
            title.push_str(&format!(" - {name}"));
        }
        title
    }

    /// The visible region of the image in the view, for inserts (`GetVisibleCanvasCenter`).
    pub fn set_visible_center(&mut self, center: Option<Point>) {
        self.input.visible_center = center.map(|c| {
            let (w, h) = self.core.canvas_size();
            Point::new(c.x.clamp(0.0, w), c.y.clamp(0.0, h))
        });
    }

    /// The bounds of the drawing surface (the image) in image pixels.
    pub fn canvas_rect(&self) -> Rect {
        let (w, h) = self.core.canvas_size();
        Rect::new(0.0, 0.0, w, h)
    }
}

/// `InsertImagePlacement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertPlacement {
    Center,
    CanvasExpandDown,
    CanvasExpandRight,
}

/// Paste and duplicate move only the start and end points and freehand points.
fn offset_clone(a: &mut Annotation, offset: f32) {
    a.start = a.start.offset(offset, offset);
    a.end = a.end.offset(offset, offset);
    if let Some(points) = a.freehand_points_mut() {
        for p in points.iter_mut() {
            *p = p.offset(offset, offset);
        }
    }
}

/// `BuildImageSizeNotification`.
pub fn image_size_notification(headline: &str, width: u32, height: u32) -> String {
    if width > 0 && height > 0 {
        format!(
            "{headline}\n{}",
            modlight_core::l10n::tf(
                modlight_core::l10n::editor::MainViewModel_SizeFormat,
                &[(width as i64).into(), (height as i64).into()]
            )
        )
    } else {
        headline.to_string()
    }
}

/// `BuildFilePathNotification`.
pub fn file_path_notification(headline: &str, path: &str) -> String {
    format!(
        "{headline}\n{}",
        modlight_core::l10n::tf(
            modlight_core::l10n::editor::MainViewModel_FilePathFormat,
            &[path.into()]
        )
    )
}

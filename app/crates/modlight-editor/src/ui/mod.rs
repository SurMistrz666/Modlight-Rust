//! The editor drawn with egui (`Presentation/Views`, `Presentation/Controls`).

pub mod canvas;
pub mod dialogs;
pub mod effects;
pub mod icons;
pub mod menu;
pub mod pickers;
pub mod sidebar;
pub mod style;
pub mod toolbars;
pub mod view;

pub use style::Colors;
pub use view::EditorView;

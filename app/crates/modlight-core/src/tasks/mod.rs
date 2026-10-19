//! The task flow of upstream's `ShareX` project, for uploads: `TaskManager`,
//! `WorkerTask`, `UploadManager`, `TaskInfo`, `TaskSettings`, the upload gate and the
//! helpers they lean on. Captures (phase 3) start tasks through the same entry points.
//!
//! Nothing here touches a window: a task asks its questions and hands its side
//! effects (clipboard, sounds, toasts) to the window through a [`host::TaskChannel`].

pub mod actions;
pub mod context;
pub mod gate;
pub mod host;
pub mod image;
pub mod info;
pub mod manager;
pub mod naming;
pub mod paste;
pub mod recent;
pub mod settings;
pub mod upload_manager;
pub mod uploader;
pub mod worker;

pub use context::TaskContext;
pub use host::{TaskAnswer, TaskChannel, TaskEffect, TaskEvent, TaskMessage, TaskQuestion};
pub use info::{DataType, TaskInfo, TaskJob, TaskStatus, UploadResult, UploaderErrors};
pub use manager::TaskManager;
pub use settings::{AppUploadSettings, TaskSettings};
pub use worker::WorkerTask;

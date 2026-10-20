//! How a running task talks to the window: questions it waits on (the windows upstream
//! shows from a worker thread and blocks on), things it asks the platform to do, and
//! the events the task list follows.
//!
//! A task never touches the clipboard, a window or a sound itself. It sends a message
//! and, for a question, awaits the answer, exactly where upstream blocks its worker
//! thread on a dialog. The app answers from its frame loop; tests answer from a script.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::oneshot;

use crate::tasks::info::TaskInfo;
use crate::tasks::settings::{
    ContentAlignment, NotificationActionButton, Size, TaskSettings, ToastClickAction,
};

/// `NotificationSound`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationSound {
    Capture,
    TaskCompleted,
    ActionCompleted,
    Error,
}

/// `NotificationWindowConfig`: everything the desktop notice after a task needs.
#[derive(Clone, Debug)]
pub struct NotificationConfig {
    /// Milliseconds on screen before fading.
    pub duration_ms: f64,
    pub fade_ms: f64,
    pub placement: ContentAlignment,
    pub size: Size,
    pub left_click: ToastClickAction,
    pub right_click: ToastClickAction,
    pub middle_click: ToastClickAction,
    pub button_size: i64,
    pub buttons: Vec<NotificationActionButton>,
    pub auto_hide: bool,
    pub file_path: String,
    pub image: Option<Arc<image::RgbaImage>>,
    pub title: String,
    pub text: String,
    pub url: String,
}

/// A message-box icon (`MessageBoxIcon`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageIcon {
    Information,
    Warning,
    Error,
}

/// Something the platform does for a task.
#[derive(Clone, Debug)]
pub enum TaskEffect {
    CopyText(String),
    CopyImage(Arc<image::RgbaImage>),
    CopyFile(PathBuf),
    ClearClipboard,
    OpenUrl(String),
    /// Reveal the file in Finder (`FileHelpers.OpenFolderWithFile`).
    RevealFile(PathBuf),
    PlaySound {
        sound: NotificationSound,
        custom_path: Option<String>,
    },
    /// The desktop notice after a task (`NotificationWindow.Show`).
    ShowNotification(Box<NotificationConfig>),
    /// A plain desktop notice (`TaskHelpers.ShowNotificationTip`).
    ShowNotificationTip {
        text: String,
        title: String,
        duration_ms: Option<f64>,
    },
    /// The after-upload window over this task's result.
    ShowAfterUploadWindow(Box<TaskInfo>),
    MessageBox {
        text: String,
        title: String,
        icon: MessageIcon,
    },
    /// Open the sign-in window without waiting for it (`GetInvalidConfigResult`).
    ShowSignIn,
    /// `PinToScreen`: the picture pinned to the screen (phase 5).
    PinToScreen(Arc<image::RgbaImage>),
    /// `TaskHelpers.OCRImage(image, taskSettings)` (`DoOCR`, phase 5).
    Ocr(
        Arc<image::RgbaImage>,
        Box<crate::tasks::settings::TaskSettings>,
    ),
    /// `TaskHelpers.AnalyzeImage(filePath, taskSettings)` (phase 5).
    AnalyzeImage(PathBuf, Box<crate::tasks::settings::TaskSettings>),
    /// `TaskHelpers.OpenQRCodeScanFromImageFile(filePath)` (phase 5).
    ScanQrCode(PathBuf),
    /// `TaskHelpers.PrintImage(image)` (`SendImageToPrinter`, phase 5).
    PrintImage(Arc<image::RgbaImage>),
    /// The large-file warning's "don't show again".
    DisableLargeFileWarning,
    /// The name parser's counter moved (`NameParserAutoIncrementNumber`).
    SetAutoIncrement(i64),
    /// The multi-upload warning's "don't show again".
    DisableMultiUploadWarning,
    /// The folder the file picker last used (`FileUploadDefaultDirectory`).
    SetFileUploadDefaultDirectory(String),
}

/// A question a task waits on.
#[derive(Clone, Debug)]
pub enum TaskQuestion {
    /// `LargeFileUploadWarningWindow`: the file is over the configured size.
    LargeFile { file_name: String, size: u64 },
    /// `SignInWindowIntegration.ShowAsync`: the upload needs an account.
    SignIn,
    /// `BeforeUploadWindow`: confirm (and possibly redirect) this upload.
    BeforeUpload(Box<TaskInfo>),
    /// `FileExistWindow`: the file to write already exists.
    FileExists { path: PathBuf },
    /// `SaveImageToFileWithDialog`: where to save the picture (a save panel).
    SaveAs { folder: PathBuf, file_name: String },
    /// `AnnotateImage`: the picture through the image editor in task mode; the answer
    /// comes when its window closes.
    Annotate(Arc<image::RgbaImage>),
    /// `BeautifyImage`: the editor in task mode with its Background panel open.
    Beautify(Arc<image::RgbaImage>),
    /// `AddImageEffects` (`TaskHelpers.ApplyImageEffects`): the task's image effect
    /// preset applied, after the Image effects window when the task asks for it. The
    /// answer is `Annotated`; `None` (an effect left no picture) ends the task.
    ImageEffects(Arc<image::RgbaImage>, Box<TaskSettings>),
}

/// The answer to a [`TaskQuestion`].
#[derive(Clone, Debug)]
pub enum TaskAnswer {
    LargeFile {
        should_continue: bool,
        dont_show_again: bool,
    },
    SignIn(bool),
    BeforeUpload {
        accepted: bool,
        settings: Box<TaskSettings>,
    },
    /// The path to write to, or `None` to not write the file.
    FileExists(Option<PathBuf>),
    /// The chosen path, or `None` when the save panel was cancelled.
    SaveAs(Option<PathBuf>),
    /// The annotated picture (or the one image effects made), or `None` when the editor
    /// was closed without it.
    Annotated(Option<Arc<image::RgbaImage>>),
}

/// What happened to a task (the `WorkerTask` events the task list follows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskEvent {
    StatusChanged,
    ImageReady,
    UploadStarted,
    UploadProgressChanged,
    UploadCompleted,
    /// The task ended, whatever became of it. Sent once.
    TaskCompleted,
}

/// One message from a task to the window.
pub enum TaskMessage {
    Event {
        task: u64,
        event: TaskEvent,
    },
    Ask {
        task: u64,
        question: TaskQuestion,
        reply: oneshot::Sender<TaskAnswer>,
    },
    Effect(TaskEffect),
}

impl std::fmt::Debug for TaskMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskMessage::Event { task, event } => write!(f, "Event({task}, {event:?})"),
            TaskMessage::Ask { task, question, .. } => write!(f, "Ask({task}, {question:?})"),
            TaskMessage::Effect(effect) => write!(f, "Effect({effect:?})"),
        }
    }
}

/// The sending half tasks hold, with the waker that gets the window to look.
#[derive(Clone)]
pub struct TaskChannel {
    sender: std::sync::mpsc::Sender<TaskMessage>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl TaskChannel {
    pub fn new(
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> (TaskChannel, std::sync::mpsc::Receiver<TaskMessage>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (TaskChannel { sender, waker }, receiver)
    }

    pub fn send(&self, message: TaskMessage) {
        let _ = self.sender.send(message);
        (self.waker)();
    }

    /// Wake the window so it reads what arrived some other way (watch folders).
    pub fn wake(&self) {
        (self.waker)();
    }

    pub fn event(&self, task: u64, event: TaskEvent) {
        self.send(TaskMessage::Event { task, event });
    }

    pub fn effect(&self, effect: TaskEffect) {
        self.send(TaskMessage::Effect(effect));
    }

    /// Ask and wait. A window that went away without answering reads as "no".
    pub async fn ask(&self, task: u64, question: TaskQuestion) -> Option<TaskAnswer> {
        let (reply, answer) = oneshot::channel();
        self.send(TaskMessage::Ask {
            task,
            question,
            reply,
        });
        answer.await.ok()
    }
}

//! Screen recording to video and GIF through ffmpeg (`ScreenRecordManager`,
//! `ScreenRecorder`, `ScreenRecordingOptions`, `FFmpegOptions`, `FFmpegCLIManager`).

pub mod args;
pub mod ffmpeg;
pub mod options;
pub mod session;

pub use args::ScreenRecordingOptions;
pub use options::{FFmpegOptions, FFmpegVideoCodec, ScreenRecordOutput};
pub use session::{RecordState, RecorderControl, RecordingStatus};

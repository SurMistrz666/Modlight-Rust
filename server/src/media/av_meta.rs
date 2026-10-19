//! PLACEHOLDER for `lib/media/av-meta.ts` until the media helpers branch
//! (srv-library-media: ffmpeg runner, ffprobe) is merged; that branch's module
//! replaces this file. Same public signature.

use crate::storage::StorageDriver;

/// The columns an audio/video row can learn from one probe.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AvMeta {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub frames: Option<i64>,
    pub duration_seconds: Option<f64>,
    pub fps: Option<f64>,
}

/// Probe a stored file; `None` for anything that cannot be probed. Never fails.
pub async fn probe_stored_av_meta(_storage: &dyn StorageDriver, _key: &str) -> Option<AvMeta> {
    None
}

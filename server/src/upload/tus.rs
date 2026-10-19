//! Port of `lib/upload/tus.ts`: resumable web uploads on `/api/files/*`.
//! The constants and the sweep hook the job runner calls are here; the tus
//! server itself is NOT PORTED YET (the uploads agent owns it).

use std::time::Duration;

use crate::state::AppState;

/// Uploads not finished within this window are expired (`TUS_EXPIRATION_MS`).
pub const TUS_EXPIRATION: Duration = Duration::from_secs(24 * 60 * 60);

/// How often stale uploads are swept (`TUS_CLEANUP_INTERVAL_MS`).
pub const TUS_CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// `sweepTusUploads(server)`: abandoned uploads plus records whose payload was
/// already adopted into storage. Returns how many were removed.
/// NOT PORTED YET: removes nothing.
pub async fn sweep_tus_uploads(_state: &AppState) -> anyhow::Result<u64> {
    tracing::debug!("[tus] sweep is not ported yet (lib/upload/tus.ts)");
    Ok(0)
}

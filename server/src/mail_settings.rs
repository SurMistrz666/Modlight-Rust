//! Port of `lib/mail-settings.ts`: the SMTP configuration, stored in
//! `settings` rows and overlaid on the SMTP_* environment, read through a
//! process-wide snapshot so `requireEmailVerification` can be answered
//! synchronously. NOT PORTED YET apart from the boot hook; the mail agent
//! implements `load_mail_settings`, `mail_snapshot`, `mail_ready`,
//! `verification_required_sync`, `save_mail_settings`, ... here.

use crate::state::AppState;

/// `warmMailSettings()`: populate the snapshot at boot so the first request is
/// not answered from the environment. A database that is not up yet must not
/// stop the server from starting (warn and carry on).
pub async fn warm_mail_settings(_state: &AppState) {
    tracing::debug!("[mail] settings snapshot is not ported yet (lib/mail-settings.ts)");
}

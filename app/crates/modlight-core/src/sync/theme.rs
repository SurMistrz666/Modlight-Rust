//! The account's skin, kept in step with the website (`ModlightTheme.SyncAsync`,
//! `PushAsync`).
//!
//! Which side wins is settled by `SkinConfirmed`: if this installation holds a skin the
//! server never confirmed (picked offline), it is pushed; otherwise the server's skin is
//! adopted. Failures are quiet and remembered, so an unreachable server becomes a push
//! on the next start rather than a choice that evaporates.

use modlight_api::{Api, Credential};

use crate::theme::{ThemeOptions, is_known_skin};

/// What a sync changed, to be written back on the thread that owns the settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeChange {
    pub skin: Option<String>,
    pub skin_confirmed: Option<String>,
}

impl ThemeChange {
    pub fn is_empty(&self) -> bool {
        self.skin.is_none() && self.skin_confirmed.is_none()
    }

    pub fn apply(&self, options: &mut ThemeOptions) {
        if let Some(skin) = &self.skin {
            options.set_skin(skin);
        }
        if let Some(confirmed) = &self.skin_confirmed {
            options.skin_confirmed = confirmed.clone();
        }
    }
}

/// Bring this installation's skin and the account's into agreement, once (at sign-in
/// and when the app starts already signed in; never on a timer).
pub async fn sync_skin(api: &Api, credential: &Credential, options: &ThemeOptions) -> ThemeChange {
    let mut change = ThemeChange {
        skin: None,
        skin_confirmed: None,
    };

    if !options.skin.eq_ignore_ascii_case(&options.skin_confirmed) {
        if let Ok(Some(saved)) = api.set_theme(credential, &options.skin).await {
            change.skin_confirmed = Some(saved);
        }
        return change;
    }

    let Ok(Some(remote)) = api.get_theme(credential).await else {
        return change;
    };

    // Recorded even when this build cannot paint it: an older client's inability to
    // draw a skin is no reason to push the old value back over a choice made online.
    change.skin_confirmed = Some(remote.clone());
    if is_known_skin(&remote) {
        change.skin = Some(remote);
    }
    change
}

/// Push the skin just picked here. Only `skin_confirmed` can change.
pub async fn push_skin(api: &Api, credential: &Credential, options: &ThemeOptions) -> ThemeChange {
    let mut change = ThemeChange {
        skin: None,
        skin_confirmed: None,
    };
    if options.skin.eq_ignore_ascii_case(&options.skin_confirmed) {
        return change;
    }
    if let Ok(Some(saved)) = api.set_theme(credential, &options.skin).await {
        change.skin_confirmed = Some(saved);
    }
    change
}

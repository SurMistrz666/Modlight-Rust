//! Loader of `app/download/page.tsx`: the public landing page for the
//! companion app (CONTRACT 6.3).
//!
//! Public because the point is to be linkable: somebody sharing "get the app"
//! should not be handing out a page that first demands a login. The download
//! itself is a normal published upload, so no account is needed to fetch it
//! either; an account is only needed to configure it, which is what the panel
//! link at the bottom is for.
//!
//! Props: `{release: currentRelease()}`. Metadata (`generateMetadata`, which
//! upstream computed with a second `currentRelease()` call; one read serves
//! both here): `{title: APP_NAME, description: release ? "Download <APP_NAME>
//! <version> for <appName>." : "<APP_NAME> for <appName>."}`.

use serde::Serialize;

use crate::pages::meta::{Field, PageMeta, Title};
use crate::pages::{PageCtx, PageData, PageResult};
use crate::releases::{APP_NAME, ReleaseManifest, current_release};

/// `DownloadPageProps`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DownloadProps {
    pub release: Option<ReleaseManifest>,
}

/// `generateMetadata()`
pub fn metadata(release: Option<&ReleaseManifest>, app_name: &str) -> PageMeta {
    let description = match release {
        Some(release) => format!("Download {APP_NAME} {} for {app_name}.", release.version),
        None => format!("{APP_NAME} for {app_name}."),
    };
    PageMeta {
        title: Title::Templated(APP_NAME.to_string()),
        description: Field::Set(description),
        ..Default::default()
    }
}

pub async fn load(ctx: PageCtx) -> PageResult {
    let release = current_release(&ctx.state.db, &ctx.state.config).await?;
    let meta = metadata(release.as_ref(), &ctx.state.config.app_name);
    PageData::new(&DownloadProps { release }, meta)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;
    use crate::pages::meta::resolve;

    #[test]
    fn metadata_names_the_version_when_there_is_one() {
        let config = Config::from_env_map(&HashMap::new()).unwrap();
        let none = resolve(&config, &metadata(None, &config.app_name), false);
        // Verified on the reference (no release published).
        assert_eq!(none.title, "Modlight App - Modlight");
        assert_eq!(none.description.as_deref(), Some("Modlight App for Modlight."));
        assert_eq!(none.application_name.as_deref(), Some("Modlight"));
        let release = ReleaseManifest {
            name: APP_NAME.into(),
            version: "1.4.2".into(),
            url: "https://x/u/c/raw.exe".into(),
            filename: "x.exe".into(),
            size: 1,
            sha256: None,
            notes: None,
            published_at: String::new(),
        };
        let some = resolve(&config, &metadata(Some(&release), &config.app_name), false);
        assert_eq!(some.description.as_deref(), Some("Download Modlight App 1.4.2 for Modlight."));
    }
}

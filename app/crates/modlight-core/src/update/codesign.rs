//! Who signed an app bundle, and whether a new bundle may replace the running one: macOS
//! code signatures read through `/usr/bin/codesign` (part of every macOS, like the
//! `/usr/bin/security` the Keychain goes through), so nothing here links Security.framework.
//!
//! The rule ([`same_developer`]): the running app must be signed by a developer team (an
//! ad hoc signature names nobody, so it cannot vouch for anything), and the new bundle must
//! carry the same team identifier and bundle identifier, be intact (`codesign --verify
//! --deep --strict`), and satisfy the running app's designated requirement (`-R`): for an
//! Apple Development or Developer ID build that pins Apple's anchor and the same
//! certificate or team. The parsing and the rule are plain functions, tested with
//! `codesign`'s own output; [`read`] and [`verify`] run the tool.

#[cfg(target_os = "macos")]
use std::path::Path;

/// The tool. Absolute: an app opened from Finder has only the system's PATH anyway.
pub const CODESIGN: &str = "/usr/bin/codesign";

/// What `codesign --display` tells about a bundle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signing {
    /// `Identifier=`: the bundle identifier the signature seals.
    pub identifier: String,
    /// `TeamIdentifier=`; `None` for "not set" (ad hoc, or no Apple certificate).
    pub team: Option<String>,
    /// `Signature=adhoc`.
    pub ad_hoc: bool,
    /// The `Authority=` chain, leaf first.
    pub authority: Vec<String>,
    /// The designated requirement, without `designated =>`; empty when unknown.
    pub requirement: String,
}

/// Why the new bundle may not replace the running one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureProblem {
    /// The running app is ad hoc signed, or signed without a team: nobody to compare with.
    Unsigned,
    /// The new bundle is not the same developer's (or not intact).
    Differs,
}

/// Read `codesign --display --verbose=2` (its standard error).
pub fn parse_display(text: &str) -> Signing {
    let mut signing = Signing::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Identifier" => signing.identifier = value.to_string(),
            "TeamIdentifier" => {
                signing.team = (!value.is_empty() && value != "not set").then(|| value.to_string())
            }
            "Signature" => signing.ad_hoc |= value.eq_ignore_ascii_case("adhoc"),
            "Authority" => signing.authority.push(value.to_string()),
            // `flags=0x2(adhoc)` on the CodeDirectory line.
            "CodeDirectory v" => signing.ad_hoc |= value.contains("(adhoc)"),
            _ => {}
        }
    }
    signing
}

/// The designated requirement from `codesign --display --requirements -` (standard
/// output): `designated => ...`, written as a comment (`# designated => ...`) when it is
/// the implicit one.
pub fn parse_requirement(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim_start().trim_start_matches('#').trim_start();
        line.strip_prefix("designated =>")
            .map(|rest| rest.trim().to_string())
            .filter(|rest| !rest.is_empty())
    })
}

/// The rule, without running anything: whether `new` comes from the developer who signed
/// `running`. [`verify`] adds what only `codesign` can tell (intact, and satisfying the
/// running app's designated requirement).
pub fn same_developer(running: &Signing, new: &Signing) -> Result<(), SignatureProblem> {
    let Some(team) = running.team.as_deref() else {
        return Err(SignatureProblem::Unsigned);
    };
    // An ad hoc signature's requirement is its own code hash: nothing else satisfies it.
    if running.ad_hoc
        || running.identifier.is_empty()
        || running.requirement.is_empty()
        || running.requirement.starts_with("cdhash")
    {
        return Err(SignatureProblem::Unsigned);
    }
    if new.ad_hoc || new.team.as_deref() != Some(team) || new.identifier != running.identifier {
        return Err(SignatureProblem::Differs);
    }
    Ok(())
}

/// The signature of a bundle (two runs of `codesign --display`). An unsigned bundle is an
/// error.
#[cfg(target_os = "macos")]
pub fn read(bundle: &Path) -> Result<Signing, String> {
    let display = std::process::Command::new(CODESIGN)
        .args(["--display", "--verbose=2"])
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{CODESIGN}: {e}"))?;
    if !display.status.success() {
        return Err(first_line(&display.stderr));
    }
    let mut signing = parse_display(&String::from_utf8_lossy(&display.stderr));
    let requirements = std::process::Command::new(CODESIGN)
        .args(["--display", "--requirements", "-"])
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{CODESIGN}: {e}"))?;
    if requirements.status.success() {
        signing.requirement =
            parse_requirement(&String::from_utf8_lossy(&requirements.stdout)).unwrap_or_default();
    }
    Ok(signing)
}

/// Whether `bundle` is intact (every file sealed, nested code included) and satisfies
/// `requirement` (another bundle's designated requirement).
#[cfg(target_os = "macos")]
pub fn verify(bundle: &Path, requirement: &str) -> Result<(), String> {
    let output = std::process::Command::new(CODESIGN)
        .args(["--verify", "--deep", "--strict", "--test-requirement"])
        .arg(format!("={requirement}"))
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{CODESIGN}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(first_line(&output.stderr))
    }
}

#[cfg(target_os = "macos")]
fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("codesign failed")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `codesign --display --verbose=2` of a build signed with an Apple Development
    /// certificate (as `build-app.sh` signs when the Keychain has one).
    const DEVELOPMENT: &str = "Executable=/Applications/Modlight.app/Contents/MacOS/Modlight
Identifier=org.modlight.Modlight
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20500 size=1234 flags=0x0(none) hashes=30+7 location=embedded
Signature size=4790
Authority=Apple Development: Alex Example (AB12CD34EF)
Authority=Apple Worldwide Developer Relations Certification Authority
Authority=Apple Root CA
Signed Time=27 Oct 2026 at 14:03:05
Info.plist entries=24
TeamIdentifier=TEAM123456
Sealed Resources version=2 rules=13 files=9
Internal requirements count=1 size=212
";
    const DEVELOPMENT_REQUIREMENT: &str = r#"Executable=/Applications/Modlight.app/Contents/MacOS/Modlight
# designated => identifier "org.modlight.Modlight" and anchor apple generic and certificate leaf[subject.CN] = "Apple Development: Alex Example (AB12CD34EF)" and certificate 1[field.1.2.840.113635.100.6.2.1] /* exists */
"#;
    /// The same of an ad hoc signature (`codesign --sign -`).
    const AD_HOC: &str = "Executable=/tmp/Modlight.app/Contents/MacOS/Modlight
Identifier=org.modlight.Modlight
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20400 size=294 flags=0x2(adhoc) hashes=3+3 location=embedded
Signature=adhoc
Info.plist entries=24
TeamIdentifier=not set
Sealed Resources version=2 rules=13 files=9
";
    const AD_HOC_REQUIREMENT: &str =
        r#"# designated => cdhash H"ae70deb59b770d9289f3d5bbef8ee3cc5c10819c""#;

    fn signed(team: &str) -> Signing {
        let mut signing = parse_display(&DEVELOPMENT.replace("TEAM123456", team));
        signing.requirement = parse_requirement(DEVELOPMENT_REQUIREMENT).unwrap();
        signing
    }

    #[test]
    fn codesigns_words_are_read() {
        let signing = signed("TEAM123456");
        assert_eq!(signing.identifier, "org.modlight.Modlight");
        assert_eq!(signing.team.as_deref(), Some("TEAM123456"));
        assert!(!signing.ad_hoc);
        assert_eq!(signing.authority.len(), 3);
        assert!(
            signing
                .requirement
                .starts_with("identifier \"org.modlight.Modlight\" and anchor apple generic")
        );
        let ad_hoc = parse_display(AD_HOC);
        assert!(ad_hoc.ad_hoc);
        assert_eq!(ad_hoc.team, None);
        assert_eq!(
            parse_requirement(AD_HOC_REQUIREMENT).as_deref(),
            Some(r#"cdhash H"ae70deb59b770d9289f3d5bbef8ee3cc5c10819c""#)
        );
        assert_eq!(
            parse_requirement("designated => anchor apple").as_deref(),
            Some("anchor apple")
        );
        assert_eq!(parse_requirement("Executable=/x\n"), None);
    }

    #[test]
    fn only_the_same_developers_build_replaces_a_signed_app() {
        let running = signed("TEAM123456");
        assert_eq!(same_developer(&running, &signed("TEAM123456")), Ok(()));
        // Another team, another app, an ad hoc or unsigned new build.
        assert_eq!(
            same_developer(&running, &signed("OTHER99999")),
            Err(SignatureProblem::Differs)
        );
        let mut other_app = signed("TEAM123456");
        other_app.identifier = "org.example.Other".into();
        assert_eq!(
            same_developer(&running, &other_app),
            Err(SignatureProblem::Differs)
        );
        let mut ad_hoc_new = parse_display(AD_HOC);
        ad_hoc_new.team = Some("TEAM123456".into());
        assert_eq!(
            same_developer(&running, &ad_hoc_new),
            Err(SignatureProblem::Differs)
        );
        assert_eq!(
            same_developer(&running, &Signing::default()),
            Err(SignatureProblem::Differs)
        );
    }

    #[test]
    fn an_ad_hoc_or_teamless_app_replaces_itself_with_nothing() {
        let mut ad_hoc = parse_display(AD_HOC);
        ad_hoc.requirement = parse_requirement(AD_HOC_REQUIREMENT).unwrap();
        for new in [signed("TEAM123456"), ad_hoc.clone()] {
            assert_eq!(
                same_developer(&ad_hoc, &new),
                Err(SignatureProblem::Unsigned)
            );
        }
        // Signed with a certificate of no Apple team (a self-made one): no team to compare.
        let mut teamless = signed("TEAM123456");
        teamless.team = None;
        assert_eq!(
            same_developer(&teamless, &signed("TEAM123456")),
            Err(SignatureProblem::Unsigned)
        );
        // A team, but an implicit cdhash requirement (somebody re-signed it oddly).
        let mut hashed = signed("TEAM123456");
        hashed.requirement = r#"cdhash H"00""#.into();
        assert_eq!(
            same_developer(&hashed, &signed("TEAM123456")),
            Err(SignatureProblem::Unsigned)
        );
        let mut nothing = signed("TEAM123456");
        nothing.requirement.clear();
        assert_eq!(
            same_developer(&nothing, &signed("TEAM123456")),
            Err(SignatureProblem::Unsigned)
        );
    }

    /// The real tool on throwaway bundles signed ad hoc (no identity needed, no Keychain).
    #[cfg(target_os = "macos")]
    #[test]
    fn codesign_reads_and_checks_real_bundles() {
        let folder = tempfile::tempdir().unwrap();
        let one = super::super::install::tests::fake_bundle(folder.path(), "One.app", "one");
        let two = super::super::install::tests::fake_bundle(folder.path(), "Two.app", "two");
        for bundle in [&one, &two] {
            let status = std::process::Command::new(CODESIGN)
                .args(["--force", "--sign", "-", "--timestamp=none"])
                .arg(bundle)
                .output()
                .unwrap();
            assert!(status.status.success(), "{status:?}");
        }
        let signing = read(&one).unwrap();
        assert!(signing.ad_hoc);
        assert_eq!(signing.team, None);
        assert_eq!(signing.identifier, "org.modlight.Modlight");
        assert!(signing.requirement.starts_with("cdhash"), "{signing:?}");
        assert_eq!(
            same_developer(&signing, &read(&two).unwrap()),
            Err(SignatureProblem::Unsigned)
        );
        // Its own requirement is satisfied; the other bundle's code hash is not.
        assert_eq!(verify(&one, &signing.requirement), Ok(()));
        assert!(verify(&two, &signing.requirement).is_err());
        // A sealed file changed after signing: not intact.
        std::fs::write(one.join("Contents/Resources/data.txt"), "changed").unwrap();
        assert!(verify(&one, &signing.requirement).is_err());
        // Never signed as a bundle (its program is a copy of Apple's `echo`, which carries
        // Apple's own signature): no developer team to replace anything with.
        let three = super::super::install::tests::fake_bundle(folder.path(), "Three.app", "3");
        assert!(read(&three).map_or(true, |s| s.team.is_none()
            && same_developer(&s, &s).is_err()));
    }
}

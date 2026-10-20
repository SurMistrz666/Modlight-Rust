//! Port of `lib/media/download-menu.ts`: the single "Download as..." list,
//! for every kind of file.
//!
//! There are two conversion engines behind this app - libvips for pictures,
//! ffmpeg for anything with a timeline - and each already owns the question of
//! what it can produce (`convert_targets`, `av_targets`). What was missing was
//! one place that asks BOTH, because the dashboard menu, the viewer bar and the
//! route each had to know which engine claimed a file, and three copies of that
//! knowledge is how a menu entry ends up answering 415.
//!
//! The list is also ORDERED and GROUPED here, and that is the part that makes
//! it readable rather than merely correct. A menu that puts "one frame as a
//! picture" next to "the whole animation" and lets the reader work out which is
//! which is a menu that gets a file converted wrongly; anything that throws the
//! motion away is separated out under its own heading, and never appears above
//! something that keeps it.
//!
//! Pure functions over a database row's own columns (the SPA carries the
//! upstream module itself; the server needs it for the convert actions and
//! `convert_plan`).

use serde::Serialize;
use serde::ser::SerializeMap;

use crate::media::av_targets::{AvCost, AvSourceSize, AvYield, av_targets_for};
use crate::media::convert_targets::{Motion, convert_targets_for};

/// Which engine will run, which decides how the entry is labelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Engine {
    Image,
    Av,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Image => "image",
            Engine::Av => "av",
        }
    }
}

impl Serialize for Engine {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// Where an entry sits in the menu.
///
///  - `main`  : what the reader came for. Keeps whatever the file has.
///  - `frame` : produces one frame of something that moves. Real - taking a
///    still out of a GIF is a thing people do - but it must never be reachable
///    by accident, so it lives below a heading of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadSection {
    Main,
    Frame,
}

impl DownloadSection {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadSection::Main => "main",
            DownloadSection::Frame => "frame",
        }
    }
}

/// `DownloadOption`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOption {
    /// Value for `?to=`.
    pub to: &'static str,
    pub engine: Engine,
    pub section: DownloadSection,
    /// Pictures only: what happens to the animation on the way out
    /// (`motion: null` for a picture with nothing to lose).
    pub motion: Option<Motion>,
    /// Media only: what comes out, and who may ask for it.
    pub yields: Option<AvYield>,
    pub cost: Option<AvCost>,
}

/// Serialises exactly like the upstream object literals: an ffmpeg entry is
/// `{to, engine, section, yields, cost}`, a libvips entry is
/// `{to, engine, section, motion}` with `motion: null` when there is none.
impl Serialize for DownloadOption {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        map.serialize_entry("to", self.to)?;
        map.serialize_entry("engine", &self.engine)?;
        map.serialize_entry("section", self.section.as_str())?;
        match self.engine {
            Engine::Av => {
                map.serialize_entry("yields", &self.yields.map(AvYield::as_str))?;
                map.serialize_entry("cost", &self.cost.map(AvCost::as_str))?;
            }
            Engine::Image => {
                map.serialize_entry("motion", &self.motion.map(Motion::as_str))?;
            }
        }
        map.end()
    }
}

/// `DownloadMenuOptions`
#[derive(Debug, Clone, Copy, Default)]
pub struct DownloadMenuOptions {
    /// May this viewer ask for something that runs an encoder?
    ///
    /// True for the owner and for admins. A public visitor is shown only the
    /// stream copies, because the rest would be a menu entry that answers 403 -
    /// see the authorization note in av-targets. This is a DISPLAY rule; the
    /// route enforces the same thing independently.
    pub can_encode: bool,
    pub size: AvSourceSize,
}

/// `downloadOptionsFor(mime, ext, frames, { canEncode, width, height })`
pub fn download_options_for(
    mime: Option<&str>,
    ext: Option<&str>,
    frames: Option<f64>,
    opts: DownloadMenuOptions,
) -> Vec<DownloadOption> {
    let claimed = av_targets_for(mime, ext, frames, Some(&opts.size));
    let av = claimed.iter().filter(|o| opts.can_encode || o.cost == AvCost::Copy).map(|o| DownloadOption {
        to: o.to.as_str(),
        engine: Engine::Av,
        // Nothing ffmpeg offers throws motion away any more: the frame grabs
        // were removed from the menu outright (av-targets), so every entry here
        // is something the reader asked for whole.
        section: DownloadSection::Main,
        motion: None,
        yields: Some(o.yields),
        cost: Some(o.cost),
    });

    // Video and audio belong to ffmpeg alone; libvips returns nothing for them.
    // An ANIMATION, though, is genuinely claimed by both - sharp keeps a GIF's
    // motion in WebP, ffmpeg turns the same GIF into an MP4 seventy times
    // smaller, and both are worth having. Where the two name the same target,
    // ffmpeg wins: it is the one that reaches every frame of an AVIF sequence,
    // where libvips only ever sees the cover.
    //
    // Note the dedupe runs against everything ffmpeg CLAIMS, not against what
    // this viewer is allowed to run. The route dispatches on `avOptionFor`
    // before it checks permissions, so a name ffmpeg claims never reaches
    // libvips.
    let image: Vec<_> = convert_targets_for(mime, ext, frames)
        .into_iter()
        .filter(|o| !claimed.iter().any(|c| c.to.as_str() == o.to.as_str()))
        .collect();

    // Does anything reach this file's motion? That decides what to do with the
    // uncounted rows - every imported row has no frame count, so "one frame
    // comes out" is a guess there. Where the motion is reachable the guess is
    // worth acting on and the still targets are demoted; with nothing to demote
    // them below (a lone WebP that may or may not move) they stay where they
    // are and say "if" instead. Read from `claimed` rather than from the
    // visible list, because whether the file moves is not a fact about who is
    // looking at it.
    let keeps_motion =
        claimed.iter().any(|o| o.yields == AvYield::Animation) || image.iter().any(|o| o.motion == Some(Motion::Keeps));

    let all: Vec<DownloadOption> = av
        .chain(image.iter().map(|o| DownloadOption {
            to: o.to.as_str(),
            engine: Engine::Image,
            section: if o.motion == Some(Motion::Single) || (o.motion == Some(Motion::SingleIfAnimated) && keeps_motion)
            {
                DownloadSection::Frame
            } else {
                DownloadSection::Main
            },
            motion: o.motion,
            yields: None,
            cost: None,
        }))
        .collect();

    let (main, frame): (Vec<_>, Vec<_>) = all.into_iter().partition(|o| o.section == DownloadSection::Main);
    main.into_iter().chain(frame).collect()
}

/// `downloadHintKey(option)`: the i18n key for the small grey line under a
/// target's name. Every one of these has to survive being read by somebody
/// who does not know what a container is.
pub fn download_hint_key(option: &DownloadOption) -> Option<&'static str> {
    if option.engine == Engine::Image {
        return match option.motion {
            Some(Motion::Keeps) => Some("convert.keepsAnimation"),
            Some(Motion::Single) => Some("convert.singleFrame"),
            Some(Motion::SingleIfAnimated) => Some("convert.frameIfAnimated"),
            // WebP and AVIF used to carry "usually about half the size, same
            // quality". Measured over 308 production conversions that is wrong
            // in both directions, and the menu now prints the predicted size
            // beside the format name instead - see convert_size.
            None => None,
        };
    }
    match option.yields {
        Some(AvYield::Video) => Some("convert.containerCopy"),
        Some(AvYield::WebVideo) => Some("convert.webPlayable"),
        Some(AvYield::Clip) => Some("convert.animationClip"),
        Some(AvYield::Animation) => Some("convert.keepsMotion"),
        Some(AvYield::Audio) => Some("convert.audioTrack"),
        Some(AvYield::AudioFormat) => Some("convert.audioFormat"),
        Some(AvYield::AudioVideo) => Some("convert.audioAsVideo"),
        None => None,
    }
}

/// `downloadLabelKey(to)`: the name to print for a target when its own letters
/// do not say it. `webmp4` writes an ordinary .mp4 and would otherwise print
/// as "WEBMP4".
pub fn download_label_key(to: &str) -> Option<&'static str> {
    (to == "webmp4").then_some("convert.webPlayableLabel")
}

/// `sectionHeadingKey(option, previous)`: only the first frame target gets a
/// heading.
pub fn section_heading_key(option: &DownloadOption, previous: Option<&DownloadOption>) -> Option<&'static str> {
    if option.section != DownloadSection::Frame {
        return None;
    }
    match previous {
        Some(p) if p.section == DownloadSection::Frame => None,
        _ => Some("convert.frameSection"),
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/download-menu.test.ts
    use super::*;
    use crate::media::av_targets::av_option_for;
    use crate::media::convert_targets::convert_option_for;

    const OWNER: DownloadMenuOptions =
        DownloadMenuOptions { can_encode: true, size: AvSourceSize { width: None, height: None } };
    const STRANGER: DownloadMenuOptions =
        DownloadMenuOptions { can_encode: false, size: AvSourceSize { width: None, height: None } };

    fn menu(mime: &str, ext: &str, frames: Option<f64>, o: DownloadMenuOptions) -> Vec<DownloadOption> {
        download_options_for(Some(mime), Some(ext), frames, o)
    }
    fn names(o: &[DownloadOption]) -> Vec<&'static str> {
        o.iter().map(|x| x.to).collect()
    }
    fn main(o: &[DownloadOption]) -> Vec<&'static str> {
        o.iter().filter(|x| x.section == DownloadSection::Main).map(|x| x.to).collect()
    }
    fn frame(o: &[DownloadOption]) -> Vec<&'static str> {
        o.iter().filter(|x| x.section == DownloadSection::Frame).map(|x| x.to).collect()
    }
    fn find<'a>(o: &'a [DownloadOption], to: &str) -> &'a DownloadOption {
        o.iter().find(|x| x.to == to).unwrap()
    }
    fn headings(o: &[DownloadOption]) -> Vec<&'static str> {
        o.iter().enumerate().filter_map(|(i, x)| section_heading_key(x, i.checked_sub(1).map(|j| &o[j]))).collect()
    }

    #[test]
    fn what_each_kind_of_file_is_offered() {
        assert_eq!(names(&menu("video/mp4", "mp4", None, OWNER)), ["mov", "mkv", "gif", "mp3"]);
        assert_eq!(names(&menu("audio/mpeg", "mp3", None, OWNER)), ["ogg", "wav", "mp4"]);
        for (mime, ext, own) in [
            ("image/png", "png", "png"),
            ("image/jpeg", "jpg", "jpg"),
            ("image/webp", "webp", "webp"),
            ("image/avif", "avif", "avif"),
            ("video/mp4", "mp4", "mp4"),
            ("audio/mpeg", "mp3", "mp3"),
        ] {
            assert!(!names(&menu(mime, ext, Some(1.0), OWNER)).contains(&own), "{mime} -> {own}");
        }
        let png = menu("image/png", "png", Some(1.0), OWNER);
        assert_eq!(names(&png), ["pdf", "jpg", "webp", "avif"]);
        assert_eq!(download_hint_key(find(&png, "avif")), None);
        assert_eq!(download_hint_key(find(&png, "webp")), None);
        assert!(png.iter().all(|o| o.engine == Engine::Image));
        assert!(menu("video/mp4", "mp4", None, OWNER).iter().all(|o| o.engine == Engine::Av));
        assert!(menu("audio/mpeg", "mp3", None, OWNER).iter().all(|o| o.engine == Engine::Av));

        let gif = menu("image/gif", "gif", Some(60.0), OWNER);
        assert_eq!(main(&gif), ["mp4", "webp"]);
        assert_eq!(find(&gif, "mp4").engine, Engine::Av);
        assert_eq!(find(&gif, "webp").engine, Engine::Image);
        let avif = menu("image/avif", "avif", None, OWNER);
        assert_eq!(main(&avif), ["gif", "webp"]);
        assert_eq!(find(&avif, "webp").engine, Engine::Av);
        assert_eq!(names(&avif).iter().filter(|n| **n == "webp").count(), 1);
    }

    #[test]
    fn what_keeps_the_motion_and_what_throws_it_away() {
        let gif = menu("image/gif", "gif", Some(60.0), OWNER);
        assert_eq!(main(&gif), ["mp4", "webp"]);
        assert_eq!(frame(&gif), ["pdf", "png", "jpg", "avif"]);
        let first = gif.iter().position(|o| o.section == DownloadSection::Frame).unwrap();
        assert!(gif[first..].iter().all(|o| o.section == DownloadSection::Frame));
        assert_eq!(headings(&gif), ["convert.frameSection"]);
        assert!(headings(&menu("image/png", "png", Some(1.0), OWNER)).is_empty());

        let still = menu("image/webp", "webp", Some(1.0), OWNER);
        assert!(
            still
                .iter()
                .all(|o| o.section == DownloadSection::Main && download_hint_key(o) != Some("convert.singleFrame"))
        );
        let unknown = menu("image/webp", "webp", None, OWNER);
        assert_eq!(download_hint_key(find(&unknown, "png")), Some("convert.frameIfAnimated"));
        assert!(frame(&unknown).is_empty());
        let animated = menu("image/webp", "webp", Some(40.0), OWNER);
        assert_eq!(download_hint_key(find(&animated, "png")), Some("convert.singleFrame"));
    }

    #[test]
    fn who_is_shown_what() {
        let public_view = menu("video/x-matroska", "mkv", None, STRANGER);
        assert_eq!(names(&public_view), ["mp4", "mov"]);
        assert!(public_view.iter().all(|o| o.cost == Some(AvCost::Copy)));
        assert_eq!(names(&menu("video/x-matroska", "mkv", None, OWNER)), ["mp4", "mov", "gif", "mp3"]);

        let public_avif = menu("image/avif", "avif", None, STRANGER);
        assert!(!names(&public_avif).contains(&"webp"));
        assert!(public_avif.iter().all(|o| o.engine == Engine::Image));
        for option in &public_avif {
            assert!(av_option_for(Some("image/avif"), Some("avif"), None, Some(option.to), None).is_none());
        }
        assert!(main(&public_avif).is_empty());
        assert!(menu("audio/mpeg", "mp3", None, STRANGER).is_empty());
        assert_eq!(names(&menu("image/png", "png", Some(1.0), STRANGER)), ["pdf", "jpg", "webp", "avif"]);
    }

    #[test]
    fn the_menu_and_the_route_agree() {
        for (mime, ext) in [
            ("video/x-matroska", "mkv"),
            ("video/mp4", "mp4"),
            ("video/quicktime", "mov"),
            ("video/webm", "webm"),
            ("audio/mpeg", "mp3"),
            ("audio/wav", "wav"),
            ("image/avif", "avif"),
            ("image/png", "png"),
            ("image/gif", "gif"),
            ("application/zip", "zip"),
        ] {
            for option in menu(mime, ext, None, OWNER) {
                let admitted = match option.engine {
                    Engine::Av => av_option_for(Some(mime), Some(ext), None, Some(option.to), None).is_some(),
                    Engine::Image => convert_option_for(Some(mime), Some(ext), None, Some(option.to)).is_some(),
                };
                assert!(admitted, "{mime} -> {} ({:?})", option.to, option.engine);
            }
        }
        let option = menu("image/avif", "avif", None, OWNER).into_iter().find(|o| o.to == "webp").unwrap();
        assert_eq!(option.engine, Engine::Av);
        assert!(av_option_for(Some("image/avif"), Some("avif"), None, Some("webp"), None).is_some());
    }

    #[test]
    fn what_the_menu_says_an_entry_does() {
        let mkv = menu("video/x-matroska", "mkv", None, OWNER);
        assert_eq!(download_hint_key(find(&mkv, "mp4")), Some("convert.containerCopy"));
        assert_eq!(download_hint_key(find(&menu("video/mp4", "mp4", None, OWNER), "mp3")), Some("convert.audioTrack"));
        assert_eq!(download_hint_key(find(&menu("audio/wav", "wav", None, OWNER), "mp3")), Some("convert.audioFormat"));
        assert_eq!(
            download_hint_key(find(&menu("video/mp4", "mp4", None, OWNER), "gif")),
            Some("convert.animationClip")
        );
        let avif = menu("image/avif", "avif", None, OWNER);
        assert_eq!(download_hint_key(find(&avif, "gif")), Some("convert.keepsMotion"));
        let gif = menu("image/gif", "gif", Some(60.0), OWNER);
        assert_eq!(download_hint_key(find(&gif, "mp4")), Some("convert.keepsMotion"));
        assert_eq!(download_hint_key(find(&gif, "webp")), Some("convert.keepsAnimation"));
        let audio = menu("audio/mpeg", "mp3", None, OWNER);
        assert_eq!(download_hint_key(find(&audio, "mp4")), Some("convert.audioAsVideo"));
    }

    #[test]
    fn names_a_key_that_exists_in_both_locales() {
        let read = |l: &str| -> serde_json::Value {
            let p = format!("{}/messages/{l}/files.json", env!("CARGO_MANIFEST_DIR"));
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
        };
        let (en, pl) = (read("en"), read("pl"));
        let mut keys = std::collections::BTreeSet::new();
        for (mime, ext) in [
            ("video/x-matroska", "mkv"),
            ("video/mp4", "mp4"),
            ("audio/mpeg", "mp3"),
            ("image/avif", "avif"),
            ("image/gif", "gif"),
            ("image/png", "png"),
            ("image/webp", "webp"),
        ] {
            for frames in [None, Some(1.0), Some(40.0)] {
                let options = menu(mime, ext, frames, OWNER);
                for (i, o) in options.iter().enumerate() {
                    if let Some(h) = download_hint_key(o) {
                        keys.insert(h);
                    }
                    if let Some(h) = section_heading_key(o, i.checked_sub(1).map(|j| &options[j])) {
                        keys.insert(h);
                    }
                }
            }
        }
        assert!(keys.len() > 6);
        for key in keys {
            let (group, leaf) = key.split_once('.').unwrap();
            assert!(en[group][leaf].as_str().is_some_and(|s| !s.is_empty()), "en {key}");
            assert!(pl[group][leaf].as_str().is_some_and(|s| !s.is_empty()), "pl {key}");
        }
    }

    #[test]
    fn serialises_like_the_upstream_literals() {
        let gif = menu("image/gif", "gif", Some(60.0), OWNER);
        let v = serde_json::to_string(&gif[..2]).unwrap();
        assert_eq!(
            v,
            r#"[{"to":"mp4","engine":"av","section":"main","yields":"animation","cost":"encode"},{"to":"webp","engine":"image","section":"main","motion":"keeps"}]"#
        );
        let png = menu("image/png", "png", Some(1.0), OWNER);
        assert_eq!(
            serde_json::to_string(&png[0]).unwrap(),
            r#"{"to":"pdf","engine":"image","section":"main","motion":null}"#
        );
    }
}

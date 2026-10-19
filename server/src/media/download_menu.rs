//! Port of `lib/media/download-menu.ts`: the single "Download as…" list, for
//! every kind of file.
//!
//! There are two conversion engines behind this app - libvips for pictures,
//! ffmpeg for anything with a timeline - and each already owns the question of
//! what it can produce (`convert_targets`, `av_targets`). This is the one place
//! that asks BOTH, so the dashboard menu, the viewer bar and the route cannot
//! disagree about which engine claimed a file. The browser renders the menu from
//! upstream's module (carried verbatim by the SPA); the server keeps the same
//! function for the conversion action and for tests that pin the menu to the
//! route's admission.
//!
//! The list is ORDERED and GROUPED: anything that throws the motion away is
//! separated out under its own heading, and never appears above something that
//! keeps it.

use crate::media::av_targets::{AvCost, AvSourceSize, AvYield, av_targets_for};
use crate::media::convert_targets::{Motion, convert_targets_for};

/// Where an entry sits in the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadSection {
    /// What the reader came for. Keeps whatever the file has.
    Main,
    /// Produces one frame of something that moves; below a heading of its own.
    Frame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Image,
    Av,
}

/// `DownloadOption`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOption {
    /// Value for `?to=`.
    pub to: &'static str,
    pub engine: Engine,
    pub section: DownloadSection,
    /// Pictures only: what happens to the animation on the way out.
    pub motion: Option<Motion>,
    /// Media only: what comes out, and who may ask for it.
    pub yields: Option<AvYield>,
    pub cost: Option<AvCost>,
}

/// `downloadOptionsFor(mime, ext, frames, { canEncode, width, height })`.
///
/// `can_encode` is a DISPLAY rule (true for the owner and admins): a public
/// visitor is shown only the stream copies. The route enforces the same thing
/// independently.
pub fn download_options_for(
    mime: Option<&str>,
    ext: Option<&str>,
    frames: Option<f64>,
    can_encode: bool,
    size: Option<&AvSourceSize>,
) -> Vec<DownloadOption> {
    let claimed = av_targets_for(mime, ext, frames, size);
    let av = claimed.iter().filter(|o| can_encode || o.cost == AvCost::Copy).map(|o| DownloadOption {
        to: o.to.as_str(),
        engine: Engine::Av,
        // Nothing ffmpeg offers throws motion away any more.
        section: DownloadSection::Main,
        motion: None,
        yields: Some(o.yields),
        cost: Some(o.cost),
    });

    // Where the two engines name the same target, ffmpeg wins: it is the one that
    // reaches every frame of an AVIF sequence. The dedupe runs against everything
    // ffmpeg CLAIMS, not against what this viewer is allowed to run, because the
    // route dispatches on `avOptionFor` before it checks permissions.
    let taken: Vec<&str> = claimed.iter().map(|o| o.to.as_str()).collect();
    let image: Vec<_> =
        convert_targets_for(mime, ext, frames).into_iter().filter(|o| !taken.contains(&o.to.as_str())).collect();

    // Does anything reach this file's motion? That decides what to do with the
    // uncounted rows ("one frame comes out" is a guess there).
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
/// target's name.
pub fn download_hint_key(option: &DownloadOption) -> Option<&'static str> {
    if option.engine == Engine::Image {
        return match option.motion {
            Some(Motion::Keeps) => Some("convert.keepsAnimation"),
            Some(Motion::Single) => Some("convert.singleFrame"),
            Some(Motion::SingleIfAnimated) => Some("convert.frameIfAnimated"),
            // The predicted size beside the format name says the rest.
            None => None,
        };
    }
    option.yields.map(|y| match y {
        AvYield::Video => "convert.containerCopy",
        AvYield::WebVideo => "convert.webPlayable",
        AvYield::Clip => "convert.animationClip",
        AvYield::Animation => "convert.keepsMotion",
        AvYield::Audio => "convert.audioTrack",
        AvYield::AudioFormat => "convert.audioFormat",
        AvYield::AudioVideo => "convert.audioAsVideo",
    })
}

/// `downloadLabelKey(to)`: `webmp4` writes an ordinary .mp4 and would otherwise
/// print as "WEBMP4".
pub fn download_label_key(to: &str) -> Option<&'static str> {
    (to == "webmp4").then_some("convert.webPlayableLabel")
}

/// `sectionHeadingKey(option, previous)`: a heading only before the first
/// frame target.
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

    const OWNER: bool = true;
    const STRANGER: bool = false;

    fn opts(mime: &str, ext: &str, frames: Option<f64>, can_encode: bool) -> Vec<DownloadOption> {
        download_options_for(Some(mime), Some(ext), frames, can_encode, None)
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

    #[test]
    fn what_each_kind_of_file_is_offered() {
        assert_eq!(names(&opts("video/mp4", "mp4", None, OWNER)), ["mov", "mkv", "gif", "mp3"]);
        assert_eq!(names(&opts("audio/mpeg", "mp3", None, OWNER)), ["ogg", "wav", "mp4"]);
        for (mime, ext, own) in [
            ("image/png", "png", "png"),
            ("image/jpeg", "jpg", "jpg"),
            ("image/webp", "webp", "webp"),
            ("image/avif", "avif", "avif"),
            ("video/mp4", "mp4", "mp4"),
            ("audio/mpeg", "mp3", "mp3"),
        ] {
            assert!(!names(&opts(mime, ext, Some(1.0), OWNER)).contains(&own), "{mime}");
        }
        let png = opts("image/png", "png", Some(1.0), OWNER);
        assert_eq!(names(&png), ["pdf", "jpg", "webp", "avif"]);
        assert_eq!(download_hint_key(find(&png, "avif")), None);
        assert_eq!(download_hint_key(find(&png, "webp")), None);
        assert!(png.iter().all(|o| o.engine == Engine::Image));
        assert!(opts("video/mp4", "mp4", None, OWNER).iter().all(|o| o.engine == Engine::Av));
        assert!(opts("audio/mpeg", "mp3", None, OWNER).iter().all(|o| o.engine == Engine::Av));
    }

    #[test]
    fn both_engines_serve_an_animation_never_twice_under_one_name() {
        let gif = opts("image/gif", "gif", Some(60.0), OWNER);
        assert_eq!(main(&gif), ["mp4", "webp"]);
        assert_eq!(find(&gif, "mp4").engine, Engine::Av);
        assert_eq!(find(&gif, "webp").engine, Engine::Image);
        let avif = opts("image/avif", "avif", None, OWNER);
        assert_eq!(main(&avif), ["gif", "webp"]);
        assert_eq!(find(&avif, "webp").engine, Engine::Av);
        assert_eq!(names(&avif).iter().filter(|n| **n == "webp").count(), 1);
    }

    #[test]
    fn what_keeps_the_motion_and_what_throws_it_away() {
        let gif = opts("image/gif", "gif", Some(60.0), OWNER);
        assert_eq!(frame(&gif), ["pdf", "png", "jpg", "avif"]);
        let first = gif.iter().position(|o| o.section == DownloadSection::Frame).unwrap();
        assert!(gif[first..].iter().all(|o| o.section == DownloadSection::Frame));
        let headings: Vec<_> = gif
            .iter()
            .enumerate()
            .filter_map(|(i, o)| section_heading_key(o, i.checked_sub(1).map(|p| &gif[p])))
            .collect();
        assert_eq!(headings, ["convert.frameSection"]);
        let png = opts("image/png", "png", Some(1.0), OWNER);
        assert!(
            png.iter().enumerate().all(|(i, o)| section_heading_key(o, i.checked_sub(1).map(|p| &png[p])).is_none())
        );

        let still = opts("image/webp", "webp", Some(1.0), OWNER);
        assert!(
            still
                .iter()
                .all(|o| o.section == DownloadSection::Main && download_hint_key(o) != Some("convert.singleFrame"))
        );
        let unknown = opts("image/webp", "webp", None, OWNER);
        assert_eq!(download_hint_key(find(&unknown, "png")), Some("convert.frameIfAnimated"));
        assert!(frame(&unknown).is_empty());
        let animated = opts("image/webp", "webp", Some(40.0), OWNER);
        assert_eq!(download_hint_key(find(&animated, "png")), Some("convert.singleFrame"));
    }

    #[test]
    fn who_is_shown_what() {
        let public_view = opts("video/x-matroska", "mkv", None, STRANGER);
        assert_eq!(names(&public_view), ["mp4", "mov"]);
        assert!(public_view.iter().all(|o| o.cost == Some(AvCost::Copy)));
        assert_eq!(names(&opts("video/x-matroska", "mkv", None, OWNER)), ["mp4", "mov", "gif", "mp3"]);

        let public_avif = opts("image/avif", "avif", None, STRANGER);
        assert!(!names(&public_avif).contains(&"webp"));
        assert!(public_avif.iter().all(|o| o.engine == Engine::Image));
        for o in &public_avif {
            assert!(av_option_for(Some("image/avif"), Some("avif"), None, Some(o.to), None).is_none(), "{}", o.to);
        }
        assert!(main(&public_avif).is_empty());
        assert!(opts("audio/mpeg", "mp3", None, STRANGER).is_empty());
        assert_eq!(names(&opts("image/png", "png", Some(1.0), STRANGER)), ["pdf", "jpg", "webp", "avif"]);
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
            for o in opts(mime, ext, None, OWNER) {
                let admitted = match o.engine {
                    Engine::Av => av_option_for(Some(mime), Some(ext), None, Some(o.to), None).is_some(),
                    Engine::Image => convert_option_for(Some(mime), Some(ext), None, Some(o.to)).is_some(),
                };
                assert!(admitted, "{mime} -> {}", o.to);
            }
        }
        let avif = opts("image/avif", "avif", None, OWNER);
        assert_eq!(find(&avif, "webp").engine, Engine::Av);
    }

    #[test]
    fn what_the_menu_says_an_entry_does() {
        let mkv = opts("video/x-matroska", "mkv", None, OWNER);
        assert_eq!(download_hint_key(find(&mkv, "mp4")), Some("convert.containerCopy"));
        assert_eq!(download_hint_key(find(&opts("video/mp4", "mp4", None, OWNER), "mp3")), Some("convert.audioTrack"));
        assert_eq!(download_hint_key(find(&opts("audio/wav", "wav", None, OWNER), "mp3")), Some("convert.audioFormat"));
        assert_eq!(
            download_hint_key(find(&opts("video/mp4", "mp4", None, OWNER), "gif")),
            Some("convert.animationClip")
        );
        assert_eq!(
            download_hint_key(find(&opts("image/avif", "avif", None, OWNER), "gif")),
            Some("convert.keepsMotion")
        );
        let gif = opts("image/gif", "gif", Some(60.0), OWNER);
        assert_eq!(download_hint_key(find(&gif, "mp4")), Some("convert.keepsMotion"));
        assert_eq!(download_hint_key(find(&gif, "webp")), Some("convert.keepsAnimation"));
        assert_eq!(
            download_hint_key(find(&opts("audio/mpeg", "mp3", None, OWNER), "mp4")),
            Some("convert.audioAsVideo")
        );
        assert_eq!(download_label_key("webmp4"), Some("convert.webPlayableLabel"));
        assert_eq!(download_label_key("mp4"), None);
    }

    #[test]
    fn names_a_key_that_exists_in_both_locales() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("messages");
        let load = |locale: &str| -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(root.join(locale).join("files.json")).unwrap()).unwrap()
        };
        let (en, pl) = (load("en"), load("pl"));
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
                let o = opts(mime, ext, frames, OWNER);
                for (i, x) in o.iter().enumerate() {
                    keys.extend(download_hint_key(x));
                    keys.extend(section_heading_key(x, i.checked_sub(1).map(|p| &o[p])));
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
}

//! Port of `lib/media/convert-targets.ts`: which image formats a file can be
//! turned into, and what that does to its animation. The menu, the serving
//! route's admission test and `/api/link-info` all answer from
//! [`convert_targets_for`], so "offered" and "accepted" cannot drift apart.

use crate::media::mime::{MediaKind, effective_type, media_kind};

/// `ConvertTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConvertTarget {
    Pdf,
    Png,
    Jpg,
    Webp,
    Avif,
}

impl ConvertTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            ConvertTarget::Pdf => "pdf",
            ConvertTarget::Png => "png",
            ConvertTarget::Jpg => "jpg",
            ConvertTarget::Webp => "webp",
            ConvertTarget::Avif => "avif",
        }
    }
}

/// Every target, in menu order. There is no "jpeg" alias: it was in the type but
/// never in this list, so `isConvertTarget("jpeg")` was false and the encoder's
/// `case "jpeg"` was unreachable code pretending to be a supported format.
pub const IMAGE_TARGETS: [ConvertTarget; 5] =
    [ConvertTarget::Pdf, ConvertTarget::Png, ConvertTarget::Jpg, ConvertTarget::Webp, ConvertTarget::Avif];

impl ConvertTarget {
    /// `isConvertTarget(v)`
    pub fn parse(v: Option<&str>) -> Option<ConvertTarget> {
        IMAGE_TARGETS.into_iter().find(|t| Some(t.as_str()) == v)
    }
}

/// `isConvertTarget(v)`
pub fn is_convert_target(v: Option<&str>) -> bool {
    ConvertTarget::parse(v).is_some()
}

/// `Motion`: what happens to motion on the way to a target (`None` = nothing
/// to say: the format cannot animate, or this file holds one frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Motion {
    #[serde(rename = "keeps")]
    Keeps,
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "single-if-animated")]
    SingleIfAnimated,
}

impl Motion {
    pub fn as_str(self) -> &'static str {
        match self {
            Motion::Keeps => "keeps",
            Motion::Single => "single",
            Motion::SingleIfAnimated => "single-if-animated",
        }
    }
}

/// `ConvertOption`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertOption {
    pub to: ConvertTarget,
    pub motion: Option<Motion>,
}

/// Containers that can hold an animation.
const ANIMATION_CAPABLE: &[&str] = &["image/gif", "image/apng", "image/webp", "image/avif"];

/// ...and the subset libvips can re-encode AS an animation (not AVIF: libheif
/// exposes only the cover of a sequence, and animated AVIF is not written).
const ANIMATION_READABLE: &[&str] = &["image/gif", "image/apng", "image/webp"];

/// The target a file already IS, which is therefore not a conversion.
fn self_target(m: &str) -> Option<ConvertTarget> {
    match m {
        "image/png" | "image/apng" => Some(ConvertTarget::Png),
        "image/jpeg" => Some(ConvertTarget::Jpg),
        "image/webp" => Some(ConvertTarget::Webp),
        "image/avif" => Some(ConvertTarget::Avif),
        _ => None,
    }
}

/// WebP is the only animated container libvips writes.
pub fn keeps_animation(to: ConvertTarget) -> bool {
    to == ConvertTarget::Webp
}

/// `convertTargetsFor(mime, ext, frames)`: the single source of truth for
/// "download as". `frames` is what the row knows about THIS file (1 = still,
/// >1 = animated, `None` = nobody counted).
pub fn convert_targets_for(mime: Option<&str>, ext: Option<&str>, frames: Option<f64>) -> Vec<ConvertOption> {
    if media_kind(mime, ext) != MediaKind::Image {
        return Vec::new();
    }
    let m = effective_type(mime, ext);
    let known = frames.filter(|f| *f > 0.0);
    let own = self_target(&m);
    let targets: Vec<ConvertTarget> = IMAGE_TARGETS.into_iter().filter(|t| Some(*t) != own).collect();

    if known == Some(1.0) || !ANIMATION_CAPABLE.contains(&m.as_str()) {
        return targets.into_iter().map(|to| ConvertOption { to, motion: None }).collect();
    }
    let loss = if known.is_some() { Motion::Single } else { Motion::SingleIfAnimated };
    let keeps_motion = ANIMATION_READABLE.contains(&m.as_str());
    let mut options: Vec<ConvertOption> = targets
        .into_iter()
        .map(|to| ConvertOption {
            to,
            motion: Some(if to == ConvertTarget::Webp && keeps_motion { Motion::Keeps } else { loss }),
        })
        .collect();
    // The one that survives intact goes first; the rest keep their order (a
    // stable sort, as Array.prototype.sort is).
    options.sort_by_key(|o| o.motion != Some(Motion::Keeps));
    options
}

/// Does this file offer any conversion at all?
pub fn can_convert(mime: Option<&str>, ext: Option<&str>, frames: Option<f64>) -> bool {
    !convert_targets_for(mime, ext, frames).is_empty()
}

/// `convertOptionFor(...)`: is `to` something this file really offers?
pub fn convert_option_for(
    mime: Option<&str>,
    ext: Option<&str>,
    frames: Option<f64>,
    to: Option<&str>,
) -> Option<ConvertOption> {
    let target = ConvertTarget::parse(to)?;
    convert_targets_for(mime, ext, frames).into_iter().find(|o| o.to == target)
}

#[cfg(test)]
mod tests {
    //! tests/unit/convert-targets.test.ts (the menu and admission parts; the
    //! budget parts belong to convert.ts)
    use super::*;

    fn names(options: &[ConvertOption]) -> Vec<&'static str> {
        options.iter().map(|o| o.to.as_str()).collect()
    }

    fn motion_of(options: &[ConvertOption], to: &str) -> Option<Option<Motion>> {
        options.iter().find(|o| o.to.as_str() == to).map(|o| o.motion)
    }

    #[test]
    fn still_list_for_an_ordinary_image() {
        let options = convert_targets_for(Some("image/png"), Some("png"), None);
        assert_eq!(names(&options), ["pdf", "jpg", "webp", "avif"]);
        assert!(options.iter().all(|o| o.motion.is_none()));
    }

    #[test]
    fn never_offers_the_format_the_file_already_is() {
        for (mime, ext, own) in [
            ("image/png", "png", "png"),
            ("image/jpeg", "jpg", "jpg"),
            ("image/webp", "webp", "webp"),
            ("image/avif", "avif", "avif"),
            ("application/octet-stream", "avif", "avif"),
        ] {
            assert!(!names(&convert_targets_for(Some(mime), Some(ext), Some(1.0))).contains(&own), "{mime}");
        }
        assert_eq!(
            names(&convert_targets_for(Some("image/gif"), Some("gif"), Some(1.0))),
            IMAGE_TARGETS.map(|t| t.as_str())
        );
    }

    #[test]
    fn nothing_for_av_archives_and_unopenable_images() {
        assert!(convert_targets_for(Some("video/mp4"), Some("mp4"), None).is_empty());
        assert!(convert_targets_for(Some("audio/mpeg"), Some("mp3"), None).is_empty());
        assert!(convert_targets_for(Some("application/zip"), Some("zip"), None).is_empty());
        assert!(!can_convert(Some("video/quicktime"), Some("mov"), None));
        for (mime, ext) in [
            ("image/svg+xml", "svg"),
            ("image/heic", "heic"),
            ("image/heif", "heif"),
            ("image/jxl", "jxl"),
            ("image/vnd.adobe.photoshop", "psd"),
        ] {
            assert!(convert_targets_for(Some(mime), Some(ext), None).is_empty(), "{mime}");
        }
    }

    #[test]
    fn animation_rules() {
        let options = convert_targets_for(Some("image/gif"), Some("gif"), Some(60.0));
        assert_eq!(names(&options)[0], "webp");
        assert_eq!(motion_of(&options, "webp"), Some(Some(Motion::Keeps)));
        for to in ["png", "jpg", "pdf", "avif"] {
            assert_eq!(motion_of(&options, to), Some(Some(Motion::Single)), "{to}");
        }
        assert_eq!(
            motion_of(&convert_targets_for(Some("image/avif"), Some("avif"), Some(43.0)), "webp"),
            Some(Some(Motion::Single))
        );
        let still = convert_targets_for(Some("image/webp"), Some("webp"), Some(1.0));
        assert_eq!(names(&still), ["pdf", "png", "jpg", "avif"]);
        assert!(still.iter().all(|o| o.motion.is_none()));
        let unknown = convert_targets_for(Some("image/avif"), Some("avif"), None);
        assert!(unknown.iter().all(|o| o.motion == Some(Motion::SingleIfAnimated)));
        assert_eq!(
            motion_of(&convert_targets_for(Some("image/gif"), Some("gif"), None), "webp"),
            Some(Some(Motion::Keeps))
        );
    }

    #[test]
    fn octet_stream_judged_by_what_it_is() {
        let options = convert_targets_for(Some("application/octet-stream"), Some("avif"), None);
        assert!(names(&options).contains(&"png"));
        assert!(convert_targets_for(Some("application/octet-stream"), Some("gr2"), None).is_empty());
    }

    #[test]
    fn the_route_admits_exactly_what_the_menu_shows() {
        let cases = [
            ("image/png", "png", None),
            ("image/gif", "gif", None),
            ("image/webp", "webp", Some(1.0)),
            ("image/webp", "webp", Some(40.0)),
            ("application/octet-stream", "avif", None),
            ("image/svg+xml", "svg", None),
            ("video/mp4", "mp4", None),
        ];
        for (mime, ext, frames) in cases {
            let offered = names(&convert_targets_for(Some(mime), Some(ext), frames));
            for to in ["pdf", "png", "jpg", "webp", "avif", "jpeg", "gif", "", "../etc"] {
                let admitted = convert_option_for(Some(mime), Some(ext), frames, Some(to)).is_some();
                assert_eq!(admitted, offered.contains(&to), "{mime} {ext} {frames:?} -> {to}");
            }
        }
        assert!(convert_option_for(Some("image/png"), Some("png"), None, None).is_none());
        assert!(!is_convert_target(Some("jpeg")));
        assert!(keeps_animation(ConvertTarget::Webp));
        assert!(!keeps_animation(ConvertTarget::Png));
    }
}

//! Port of `lib/media/avatar.ts`: turn whatever the user handed us into the
//! square we are willing to serve.
//!
//! One function, two entry points: the raw-body route (a file from their disk)
//! and the server action (one of their own uploads). Both arrive as bytes and
//! both leave as a small WebP, so there is exactly one set of budgets, one
//! encoder and one place where "this is not an image" is decided.
//!
//! The bytes the user supplied are NEVER stored. Re-encoding is what makes an
//! avatar safe to serve publicly: libvips strips EXIF (including the GPS tag on
//! a photo somebody is about to put next to their name), drops any payload
//! smuggled after the image data, and guarantees the served type matches the
//! served bytes.

use bytes::Bytes;

use crate::avatar::{
    AVATAR_MAX_FRAME_PIXELS, AVATAR_MAX_OUTPUT_BYTES, AVATAR_MAX_TOTAL_PIXELS, AVATAR_SIZE, AvatarPlan,
    AvatarPlanReason, AvatarSource,
};
use crate::media::probe::probe_image_blocking;
use crate::media::transform_cost::THUMBNAIL_TIMEOUT_MS;
use crate::media::vips::{self, FailOn, Fit, Position, Resize, Sharp, VipsError, WebpOptions};

/// `planAvatar` (lib/avatar.ts) from the probe's whole-number shape.
fn plan_avatar(width: u32, height: u32, pages: u32) -> AvatarPlan {
    crate::avatar::plan_avatar(AvatarSource {
        width: f64::from(width),
        height: f64::from(height),
        pages: f64::from(pages),
    })
}

// ---------------------------------------------------------------------------
// lib/media/avatar.ts
// ---------------------------------------------------------------------------

/// Why an animated source came out still. None when nothing was lost - either
/// the source was already a still, or the animation survived.
///
/// Reported all the way to the toast on purpose. Round 12's lesson was that a
/// refusal the user cannot see reads as a bug in the app: "your GIF was saved as
/// a still because it has 400 frames" is actionable, a silently frozen avatar is
/// a mystery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarDegradation {
    Frames,
    Pixels,
    OutputSize,
}

impl AvatarDegradation {
    /// The wire spelling (`"output-size"`).
    pub fn as_str(self) -> &'static str {
        match self {
            AvatarDegradation::Frames => "frames",
            AvatarDegradation::Pixels => "pixels",
            AvatarDegradation::OutputSize => "output-size",
        }
    }
}

/// The two refusals the caller has a specific answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarEncodeError {
    Decode,
    TooManyPixels,
}

impl AvatarEncodeError {
    /// The wire spelling (`"too-many-pixels"`).
    pub fn as_str(self) -> &'static str {
        match self {
            AvatarEncodeError::Decode => "decode",
            AvatarEncodeError::TooManyPixels => "too-many-pixels",
        }
    }
}

/// `AvatarEncodeResult`: `{ ok: true, ... }` or `{ ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvatarEncodeResult {
    Ok { bytes: Bytes, animated: bool, frames: u32, degraded: Option<AvatarDegradation> },
    Failed { error: AvatarEncodeError },
}

/// Encode one avatar. Runs inside the caller's limiter slot; never acquires one
/// itself, so the two entry points can price this alongside their own work.
///
/// Returns a result rather than an error for the two conditions the caller has a
/// specific answer for. Anything else (libvips falling over mid-encode) is an
/// error and the route answers 500, which is honest: we do not know what
/// happened.
pub async fn encode_avatar(input: Bytes) -> anyhow::Result<AvatarEncodeResult> {
    Ok(vips::run_blocking(move || encode_avatar_blocking(&input)).await?)
}

/// [`encode_avatar`] for code already on the blocking pool.
pub fn encode_avatar_blocking(input: &Bytes) -> Result<AvatarEncodeResult, VipsError> {
    // Metadata only - no decode. `probe_image` is the shared reader, so the `avis`
    // brand workaround and the frame-versus-stacked-strip distinction apply here
    // for free rather than being reimplemented slightly differently.
    let Some(probe) = probe_image_blocking(input) else {
        return Ok(AvatarEncodeResult::Failed { error: AvatarEncodeError::Decode });
    };

    let plan = plan_avatar(probe.width, probe.height, probe.pages);
    if !plan.ok {
        return Ok(AvatarEncodeResult::Failed { error: AvatarEncodeError::TooManyPixels });
    }

    let seconds = THUMBNAIL_TIMEOUT_MS.div_ceil(1000).max(1) as u32;

    // Edge of the square we produce: the stored size, or the source's own short
    // side when that is smaller.
    //
    // Derived rather than passed straight to `resize(AVATAR_SIZE, AVATAR_SIZE)`
    // with `withoutEnlargement`, because MEASURED: that combination abandons the
    // crop entirely once both source dimensions are under the target - a 60x40 GIF
    // came out 60x40 rather than square. Choosing the edge from the source keeps
    // the cover crop honest at every size and still never upscales, since the edge
    // can never exceed either dimension.
    let edge = AVATAR_SIZE.min(probe.width).min(probe.height).max(1);

    let encode = |animated: bool| -> Result<Vec<u8>, VipsError> {
        let mut pipe = Sharp::new(probe.input.clone())
            .fail_on(FailOn::None)
            .animated(animated)
            // Measured: with `animated: true` libvips counts the WHOLE strip against
            // this, so the frame-aware budget is enforced by the decoder as well as by
            // plan_avatar. On the still path only page 0 is loaded, so the per-frame
            // bomb guard is the right number there.
            .limit_input_pixels(Some(if animated { AVATAR_MAX_TOTAL_PIXELS } else { AVATAR_MAX_FRAME_PIXELS }))
            .timeout(seconds);

        // EXIF orientation exists on stills (phone photos) and is meaningless on the
        // animated formats. Kept off the animated pipeline anyway: a rotation there
        // would have to be applied to a multi-page strip, and there is no real file
        // that needs it.
        if !animated {
            pipe = pipe.rotate();
        }

        pipe.resize(Resize::to(Some(edge), Some(edge)).fit(Fit::Cover).position(
            // Measured: "attention" fails with `Resize strategy is not supported for
            // multi-page images`. A face is centred in an avatar anyway; the smart
            // crop is only worth having on the still path.
            if animated { Position::Centre } else { Position::Attention },
        ))
        .webp(WebpOptions { quality: if animated { 60 } else { 82 }, effort: 4, ..WebpOptions::default() })
        .to_buffer_blocking()
    };

    let mut bytes = encode(plan.animate)?;
    let mut animated = plan.animate;
    let mut degraded = match plan.reason {
        AvatarPlanReason::Frames => Some(AvatarDegradation::Frames),
        AvatarPlanReason::Pixels => Some(AvatarDegradation::Pixels),
        _ => None,
    };

    // The frame budget bounds the ENCODE; this bounds the RESULT, which is the
    // only number the viewer feels. A busy 120-frame sequence can clear every
    // pixel ceiling and still come out fat, and this file is fetched on every page
    // of the panel plus every public page its owner owns.
    if animated && bytes.len() > AVATAR_MAX_OUTPUT_BYTES {
        bytes = encode(false)?;
        animated = false;
        degraded = Some(AvatarDegradation::OutputSize);
    }

    Ok(AvatarEncodeResult::Ok {
        bytes: Bytes::from(bytes),
        animated,
        frames: if animated { plan.frames as u32 } else { 1 },
        degraded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The planAvatar cases of tests/unit/avatar.test.ts that encode_avatar
    // relies on (the full suite is ported with lib/avatar.ts).

    #[test]
    fn keeps_the_animation_of_an_ordinary_gif() {
        let plan = plan_avatar(200, 200, 30);
        assert_eq!(plan, AvatarPlan { ok: true, animate: true, frames: 30, reason: AvatarPlanReason::Animate });
    }

    #[test]
    fn charges_every_frame() {
        // 1654 x 762 x 256 = 322 Mpx: past the frame cap anyway.
        assert_eq!(plan_avatar(1654, 762, 256).reason, AvatarPlanReason::Frames);
        // 1000 x 1000 x 50 = 50 Mpx: within the cap, over the total.
        assert_eq!(plan_avatar(1000, 1000, 50).reason, AvatarPlanReason::Pixels);
    }

    #[test]
    fn refuses_a_single_frame_over_the_bomb_ceiling() {
        assert!(!plan_avatar(10_000, 10_000, 1).ok);
        assert_eq!(plan_avatar(10_000, 10_000, 1).reason, AvatarPlanReason::FrameTooLarge);
    }

    #[test]
    fn accepts_exactly_the_cap_and_one_frame_past_it_degrades() {
        assert!(plan_avatar(8, 8, 120).animate);
        assert_eq!(plan_avatar(8, 8, 121).reason, AvatarPlanReason::Frames);
    }
}

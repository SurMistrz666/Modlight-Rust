//! Pictures: card thumbnails (`MySpaceViewModel` thumbnail pipeline), moving previews on
//! hover (`CloudMotionPlayer`, `AnimatedThumbnail`), the account picture
//! (`AccountAvatar`) and the details drawer's larger preview.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions};
use image::{AnimationDecoder, DynamicImage, ImageFormat, imageops::FilterType};
use modlight_api::{Credential, MotionPreview};
use modlight_core::library::{
    MotionCache, THUMBNAIL_BUDGET_BYTES, THUMBNAIL_DECODE_WIDTH, THUMBNAIL_SLOTS, ThumbnailBudget,
    ThumbnailCache, ThumbnailQueue, clamp_frame_delay, enlarge_thumbnail_url, motion, visible_band,
};

use crate::state::AppState;

/// A decoded animation: frames and how long each one stays.
#[derive(Clone)]
pub struct Animation {
    pub frames: Vec<ColorImage>,
    pub delays_ms: Vec<u32>,
}

/// Animation frames already on the GPU, and where playback is.
pub struct Reel {
    pub frames: Vec<TextureHandle>,
    pub delays_ms: Vec<u32>,
    pub index: usize,
    pub next_at: f64,
}

impl Reel {
    pub fn new(ctx: &egui::Context, name: &str, animation: Animation, now: f64) -> Reel {
        let frames: Vec<TextureHandle> = animation
            .frames
            .into_iter()
            .enumerate()
            .map(|(i, frame)| {
                ctx.load_texture(format!("{name}-{i}"), frame, TextureOptions::LINEAR)
            })
            .collect();
        let first = animation.delays_ms.first().copied().unwrap_or(120) as f64 / 1000.0;
        Reel {
            frames,
            delays_ms: animation.delays_ms,
            index: 0,
            next_at: now + first,
        }
    }

    pub fn frame(&self) -> &TextureHandle {
        &self.frames[self.index % self.frames.len()]
    }

    /// Advance to the frame for `now`; returns when the next one is due.
    pub fn tick(&mut self, now: f64) -> f64 {
        let mut guard = 0;
        while now >= self.next_at && guard < self.frames.len() * 2 {
            self.index = (self.index + 1) % self.frames.len();
            let delay = self.delays_ms[self.index % self.delays_ms.len()] as f64 / 1000.0;
            self.next_at += delay.max(0.01);
            guard += 1;
        }
        if now >= self.next_at {
            self.next_at = now + 0.04;
        }
        self.next_at
    }
}

/// Decode a still picture, shrunk to `max_width` when wider (never enlarged).
pub fn decode_still(bytes: &[u8], max_width: u32) -> Option<ColorImage> {
    let image = modlight_core::image_decode::from_bytes(bytes).ok()?;
    Some(to_color_image(shrink(image, max_width)))
}

fn shrink(image: DynamicImage, max_width: u32) -> DynamicImage {
    if image.width() <= max_width || image.width() == 0 {
        return image;
    }
    let height =
        ((image.height() as f64 * max_width as f64 / image.width() as f64).round() as u32).max(1);
    image.resize_exact(max_width, height, FilterType::CatmullRom)
}

pub fn to_color_image(image: DynamicImage) -> ColorImage {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
}

/// Decode an animated WebP or GIF within the motion budget; `None` for fewer than two
/// frames (a still is not a moving preview).
///
/// Every frame is decoded under the app's animation limits and shrunk to the plan's size
/// as soon as it is out of the decoder, so no more than one full-size frame is held at a
/// time, whatever the server sends.
pub fn decode_animation(bytes: &[u8]) -> Option<Animation> {
    use image::ImageDecoder;
    let format = image::guess_format(bytes).ok()?;
    let limits = modlight_core::image_decode::animation_limits();
    let ((canvas_w, canvas_h), frames) = match format {
        ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            decoder.set_limits(limits).ok()?;
            (decoder.dimensions(), decoder.into_frames())
        }
        ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
            decoder.set_limits(limits).ok()?;
            (decoder.dimensions(), decoder.into_frames())
        }
        _ => return None,
    };
    // The plan for as many frames as the ceiling allows; fewer arrive when the artefact
    // has fewer, which is what planning with the count would have taken.
    let plan = motion::plan(canvas_w as i64, canvas_h as i64, motion::MAX_FRAMES as i64);
    if plan.frames < motion::MIN_FRAMES {
        return None;
    }
    let mut out = Animation {
        frames: Vec::new(),
        delays_ms: Vec::new(),
    };
    for frame in frames.take(plan.frames as usize) {
        // The first frame that fails ends the animation.
        let Ok(frame) = frame else {
            break;
        };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = numerator.checked_div(denominator).unwrap_or(0) as i64;
        let buffer = frame.into_buffer();
        let image = DynamicImage::ImageRgba8(buffer);
        let scaled = if image.width() == plan.width && image.height() == plan.height {
            image
        } else {
            image.resize_exact(plan.width, plan.height, FilterType::Triangle)
        };
        out.frames.push(to_color_image(scaled));
        out.delays_ms.push(clamp_frame_delay(delay));
    }
    (out.frames.len() >= motion::MIN_FRAMES as usize).then_some(out)
}

/// The moving preview of the card under the pointer.
#[derive(Default)]
pub struct MotionPlayer {
    /// The card the pointer rests on, and since when.
    pub hovered: Option<(String, f64)>,
    pub playing: Option<(String, Reel)>,
    pub fetching: Option<String>,
    pub decoding: Option<String>,
    /// The hovered card this hover has already made its one attempt for (a fetch or a
    /// decode started): `BeginAsync` runs once per dwell, and an answer that is not a
    /// preview (503, a dropped connection) is not asked again until the next hover.
    pub attempted: Option<String>,
}

/// The account's picture: a still, frames of an animation, or nothing yet.
#[derive(Default)]
pub struct Avatar {
    pub url: String,
    pub still: Option<TextureHandle>,
    pub reel: Option<Reel>,
    pub generation: u64,
}

impl Avatar {
    pub fn clear(&mut self) {
        self.url.clear();
        self.still = None;
        self.reel = None;
        self.generation += 1;
    }

    pub fn texture(&self) -> Option<&TextureHandle> {
        self.reel.as_ref().map(Reel::frame).or(self.still.as_ref())
    }
}

/// Initials for the avatar: the first two characters of the name, upper case.
pub fn initials_of(name: &str) -> String {
    name.trim()
        .chars()
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

pub struct Media {
    pub thumbs: HashMap<String, TextureHandle>,
    pub budget: ThumbnailBudget<String>,
    pub queue: ThumbnailQueue<String>,
    pub bytes: ThumbnailCache,
    pub motion_bytes: MotionCache,
    pub motion: MotionPlayer,
    pub avatar: Avatar,
    /// The details drawer's preview: which file, and the picture.
    pub preview: Option<(String, TextureHandle)>,
    pub preview_generation: u64,
}

impl Default for Media {
    fn default() -> Self {
        Media {
            thumbs: HashMap::new(),
            budget: ThumbnailBudget::new(THUMBNAIL_BUDGET_BYTES),
            queue: ThumbnailQueue::new(THUMBNAIL_SLOTS),
            bytes: ThumbnailCache::default(),
            motion_bytes: MotionCache::default(),
            motion: MotionPlayer::default(),
            avatar: Avatar::default(),
            preview: None,
            preview_generation: 0,
        }
    }
}

impl Media {
    /// Forget the pictures of the grid (a new listing replaces it).
    pub fn clear_grid(&mut self) {
        self.thumbs.clear();
        self.budget.clear();
        self.queue.reset();
        self.motion = MotionPlayer::default();
    }

    pub fn forget(&mut self, id: &str) {
        self.thumbs.remove(id);
        self.budget.forget(&id.to_string());
        if self.motion.playing.as_ref().is_some_and(|(p, _)| p == id) {
            self.motion.playing = None;
        }
        if self.motion.hovered.as_ref().is_some_and(|(h, _)| h == id) {
            self.motion.hovered = None;
            self.motion.attempted = None;
        }
    }
}

impl AppState {
    /// Report which cards are on screen (`SetVisibleRange`): pin them, keep the band
    /// warm, and fetch missing pictures front first.
    pub fn set_visible_range(&mut self, first: usize, last: usize) {
        let count = self.space.files.len();
        if count == 0 {
            return;
        }
        let first = first.min(count - 1);
        let last = last.clamp(first, count - 1);
        let visible: Vec<String> = self.space.files[first..=last]
            .iter()
            .map(|f| f.file.id.clone())
            .collect();
        self.media.budget.pin(&visible);
        let margin = modlight_core::library::VISIBLE_BAND_MARGIN;
        for index in first.saturating_sub(margin)..=(last + margin).min(count - 1) {
            let id = self.space.files[index].file.id.clone();
            self.media.budget.touch(&id);
        }
        let want: Vec<String> = visible_band(count, first, last, margin)
            .into_iter()
            .map(|i| &self.space.files[i].file)
            .filter(|f| !self.media.thumbs.contains_key(&f.id) && !f.thumbnail_url.is_empty())
            .map(|f| f.id.clone())
            .collect();
        let started = self.media.queue.want(&want);
        self.start_thumbnails(started);
    }

    /// Queue every new arrival (front of the grid first), as `LoadThumbnails` does.
    pub fn load_thumbnails(&mut self, arrived: &[String]) {
        let mut want: Vec<String> = Vec::new();
        for id in arrived
            .iter()
            .chain(self.space.files.iter().map(|f| &f.file.id))
        {
            if want.contains(id) {
                continue;
            }
            if let Some(file) = self.space.file(id)
                && !file.thumbnail_url.is_empty()
                && !self.media.thumbs.contains_key(id)
            {
                want.push(id.clone());
            }
        }
        let started = self.media.queue.want(&want);
        self.start_thumbnails(started);
    }

    fn start_thumbnails(&mut self, ids: Vec<String>) {
        for id in ids {
            self.fetch_thumbnail(id);
        }
    }

    fn fetch_thumbnail(&mut self, id: String) {
        let (Some(credential), Some(file)) = (self.credential(), self.space.file(&id).cloned())
        else {
            let next = self.media.queue.finished(&id);
            self.start_thumbnails(next);
            return;
        };
        let generation = self.space.generation;
        let api = self.api.clone();
        let cache = self.media.bytes.clone();
        self.rt.spawn(async move {
            let bytes = match cache.get(&file.code) {
                Some(bytes) => Some(bytes),
                None => match api.get_thumbnail(&credential, &file.thumbnail_url).await {
                    Ok(Some(bytes)) => {
                        let bytes = Arc::new(bytes);
                        cache.put(&file.code, bytes.clone());
                        Some(bytes)
                    }
                    _ => None,
                },
            };
            let decoded = bytes.and_then(|b| decode_still(&b, THUMBNAIL_DECODE_WIDTH));
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                if state.space.generation == generation
                    && let Some(image) = decoded
                    && state.space.file(&id).is_some()
                {
                    let cost = (image.size[0] * image.size[1] * 4) as u64;
                    let texture =
                        ctx.load_texture(format!("thumb-{id}"), image, TextureOptions::LINEAR);
                    state.media.thumbs.insert(id.clone(), texture);
                    for released in state.media.budget.add(id.clone(), cost) {
                        state.media.thumbs.remove(&released);
                    }
                }
                let next = state.media.queue.finished(&id);
                state.start_thumbnails(next);
            })
        });
    }

    /// The pointer entered a card (`HoverFile`).
    pub fn hover_file(&mut self, id: &str) {
        let Some(file) = self.space.file(id) else {
            return;
        };
        if !file.can_motion()
            || self.media.motion_bytes.is_refused(&file.code)
            || self.env.reduce_motion
        {
            return;
        }
        if self
            .media
            .motion
            .hovered
            .as_ref()
            .is_some_and(|(h, _)| h == id)
        {
            return;
        }
        self.media.motion.playing = None;
        self.media.motion.hovered = Some((id.to_string(), self.time));
        self.media.motion.attempted = None;
    }

    /// The pointer left a card (`UnhoverFile`).
    pub fn unhover_file(&mut self, id: &str) {
        if self
            .media
            .motion
            .hovered
            .as_ref()
            .is_some_and(|(h, _)| h == id)
        {
            self.media.motion.hovered = None;
            self.media.motion.playing = None;
            self.media.motion.attempted = None;
        }
    }

    /// Per frame: start the dwell-delayed fetch, advance the playing reel.
    pub fn tick_motion(&mut self, ctx: &egui::Context) {
        // Behind another tab no card is under the pointer, whatever was last hovered.
        if self.tab != crate::state::Tab::MySpace
            && let Some((hovered, _)) = self.media.motion.hovered.clone()
        {
            self.unhover_file(&hovered);
        }
        let dwell = modlight_core::library::MOTION_DWELL_MILLIS as f64 / 1000.0;
        if let Some((id, since)) = self.media.motion.hovered.clone() {
            let playing_this = self
                .media
                .motion
                .playing
                .as_ref()
                .is_some_and(|(p, _)| *p == id);
            if !playing_this && self.time - since >= dwell {
                // One attempt per hover: whatever it ends in, the frames that follow do
                // not ask again (the slots it may wait for are retried until it starts).
                if self.media.motion.attempted.as_deref() != Some(id.as_str()) {
                    self.begin_motion(&id);
                }
            } else if !playing_this {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    (since + dwell - self.time).max(0.01),
                ));
            }
        }
        if let Some((_, reel)) = &mut self.media.motion.playing {
            let next = reel.tick(self.time);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (next - self.time).max(0.005),
            ));
        }
    }

    fn begin_motion(&mut self, id: &str) {
        let Some(file) = self.space.file(id).cloned() else {
            return;
        };
        // Refused by the server, or bytes already known not to animate: nothing to ask.
        if self.media.motion_bytes.is_refused(&file.code) {
            self.media.motion.attempted = Some(id.to_string());
            return;
        }
        if let Some(bytes) = self.media.motion_bytes.get(&file.code) {
            // One decode at a time; while another runs, this card waits its turn.
            if self.media.motion.decoding.is_none() {
                self.media.motion.attempted = Some(id.to_string());
                self.decode_motion(id.to_string(), bytes);
            }
            return;
        }
        if self.media.motion.fetching.is_some() {
            return;
        }
        let Some(credential) = self.credential() else {
            return;
        };
        self.media.motion.attempted = Some(id.to_string());
        self.media.motion.fetching = Some(id.to_string());
        let api = self.api.clone();
        let id = id.to_string();
        self.rt.spawn(async move {
            let preview = api
                .get_motion_thumbnail(&credential, &file.motion_url)
                .await;
            Box::new(move |state: &mut AppState, _ctx: &egui::Context| {
                state.media.motion.fetching = None;
                match preview {
                    Ok(MotionPreview::Refused) => state.media.motion_bytes.refuse(&file.code),
                    Ok(MotionPreview::Ok(bytes)) => {
                        let bytes = Arc::new(bytes);
                        state.media.motion_bytes.put(&file.code, bytes.clone());
                        if state
                            .media
                            .motion
                            .hovered
                            .as_ref()
                            .is_some_and(|(h, _)| *h == id)
                        {
                            if state.media.motion.decoding.is_none() {
                                state.decode_motion(id, bytes);
                            } else {
                                // The decode slot is taken: the tick takes these bytes
                                // from the cache once it is free.
                                state.media.motion.attempted = None;
                            }
                        }
                    }
                    // Busy, timed out or cut off: about this moment, not the file, and
                    // not asked again until the next hover (no retry loop).
                    _ => {}
                }
            })
        });
    }

    fn decode_motion(&mut self, id: String, bytes: Arc<Vec<u8>>) {
        if self.media.motion.decoding.is_some() {
            return;
        }
        let Some(code) = self.space.file(&id).map(|f| f.code.clone()) else {
            return;
        };
        self.media.motion.decoding = Some(id.clone());
        self.rt.spawn_blocking(move || {
            let animation = decode_animation(&bytes);
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                state.media.motion.decoding = None;
                let Some(animation) = animation else {
                    state.media.motion_bytes.refuse(&code);
                    return;
                };
                if state
                    .media
                    .motion
                    .hovered
                    .as_ref()
                    .is_some_and(|(h, _)| *h == id)
                {
                    let reel = Reel::new(ctx, &format!("motion-{id}"), animation, state.time);
                    state.media.motion.playing = Some((id, reel));
                }
            })
        });
    }

    /// Fetch the account's picture, when it has one (`AccountAvatar.Show`).
    pub fn request_avatar(&mut self, _ctx: &egui::Context) {
        let url = self.account.account().avatar.clone();
        if url == self.media.avatar.url {
            return;
        }
        self.media.avatar.clear();
        self.media.avatar.url = url.clone();
        let Some(credential) = self.credential().filter(|_| !url.is_empty()) else {
            return;
        };
        let generation = self.media.avatar.generation;
        let api = self.api.clone();
        self.rt.spawn(async move {
            let bytes = api.get_thumbnail(&credential, &url).await.ok().flatten();
            let animation = bytes.as_deref().and_then(decode_animation);
            let still = if animation.is_none() {
                bytes.as_deref().and_then(|b| decode_still(b, 256))
            } else {
                None
            };
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                if state.media.avatar.generation != generation {
                    return;
                }
                if let Some(animation) = animation {
                    state.media.avatar.reel = Some(Reel::new(ctx, "avatar", animation, state.time));
                } else if let Some(still) = still {
                    state.media.avatar.still =
                        Some(ctx.load_texture("avatar", still, TextureOptions::LINEAR));
                }
            })
        });
    }

    /// Load the drawer's preview for `id`: the cached thumbnail at once, then the
    /// larger rendering (`ModlightDetailsPreview`).
    pub fn load_details_preview(&mut self, ctx: &egui::Context, id: &str) {
        self.media.preview_generation += 1;
        let generation = self.media.preview_generation;
        self.media.preview = None;
        let Some(file) = self.space.file(id).cloned() else {
            return;
        };
        if file.thumbnail_url.is_empty() {
            return;
        }
        if let Some(texture) = self.media.thumbs.get(id) {
            self.media.preview = Some((id.to_string(), texture.clone()));
        }
        let Some(credential) = self.credential() else {
            return;
        };
        let api = self.api.clone();
        let id = id.to_string();
        let url = enlarge_thumbnail_url(&file.thumbnail_url);
        self.rt.spawn(async move {
            let bytes = api.get_thumbnail(&credential, &url).await.ok().flatten();
            let decoded =
                bytes.and_then(|b| decode_still(&b, modlight_core::library::DETAILS_PREVIEW_EDGE));
            Box::new(move |state: &mut AppState, ctx: &egui::Context| {
                if state.media.preview_generation != generation {
                    return;
                }
                if let Some(image) = decoded {
                    let texture =
                        ctx.load_texture(format!("preview-{id}"), image, TextureOptions::LINEAR);
                    state.media.preview = Some((id, texture));
                }
            })
        });
        let _ = ctx;
    }

    /// The thumbnail queue's credential check, for completions that outlive a sign-out.
    pub fn credential_for(&self, host: &str) -> Option<Credential> {
        self.credential().filter(|c| c.host == host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animated_gif(frames: usize, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for i in 0..frames {
                let buffer =
                    image::RgbaImage::from_pixel(w, h, image::Rgba([(i * 40) as u8, 10, 200, 255]));
                let frame = image::Frame::from_parts(
                    buffer,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(83, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        out
    }

    #[test]
    fn animations_are_decoded_within_the_budget() {
        let gif = animated_gif(5, 480, 480);
        let animation = decode_animation(&gif).expect("an animation");
        assert_eq!(animation.frames.len(), 5);
        assert_eq!(
            animation.frames[0].size,
            [288, 288],
            "decoded no wider than the tile needs"
        );
        // GIF keeps delays in hundredths of a second: 83 ms is stored as 80.
        assert_eq!(animation.delays_ms[0], 80);
        assert!(
            decode_animation(&animated_gif(1, 10, 10)).is_none(),
            "one frame does not move"
        );
        assert!(decode_animation(b"not an image").is_none());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in bytes {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A real small PNG whose header now claims `width` x `height` (CRC fixed).
    fn png_claiming(width: u32, height: u32) -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(16, 16, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        assert_eq!(&png[12..16], b"IHDR");
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        png
    }

    /// S3: a thumbnail, preview or moving preview from the server whose header claims an
    /// enormous picture is refused by its size, not allocated.
    #[test]
    fn pictures_claiming_an_enormous_size_are_refused() {
        assert!(
            decode_still(&png_claiming(16, 16), 288).is_some(),
            "the real picture"
        );
        assert!(decode_still(&png_claiming(30_000, 30_000), 288).is_none());
        assert!(decode_still(&png_claiming(100_000, 1), 288).is_none());

        // A real animation whose canvas (logical screen) now says 30000 x 30000, and one
        // past the side limit.
        for side in [30_000u16, 40_000] {
            let mut gif = animated_gif(3, 48, 48);
            gif[6..8].copy_from_slice(&side.to_le_bytes());
            gif[8..10].copy_from_slice(&side.to_le_bytes());
            assert!(decode_animation(&gif).is_none(), "{side}");
        }
        assert!(decode_animation(&animated_gif(3, 48, 48)).is_some());
    }

    #[test]
    fn stills_are_shrunk_never_enlarged() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(600, 300, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        assert_eq!(decode_still(&png, 288).unwrap().size, [288, 144]);
        assert_eq!(decode_still(&png, 1024).unwrap().size, [600, 300]);
        assert_eq!(initials_of(" alex rivera"), "AL");
        assert_eq!(initials_of(""), "");
    }

    /// A signed-in state on `host`, motion allowed, with one video card ("v1", code
    /// "cv1") whose moving preview lives on that host.
    fn state_with_video(host: &str) -> AppState {
        use modlight_core::account::{AccountStore, StoredAccount};
        let mut state = crate::qa::state(&crate::qa::Look::default(), true);
        state.account = AccountStore::in_memory(
            StoredAccount {
                host: host.into(),
                tag: "alex".into(),
                ..StoredAccount::default()
            },
            "qa-token-not-a-secret",
        );
        state.space.has_account = true;
        state.env.reduce_motion = false;
        let row = modlight_api::Upload {
            id: Some("v1".into()),
            code: Some("cv1".into()),
            filename: Some("clip.mp4".into()),
            kind: Some("video".into()),
            motion_thumb_url: Some(format!("{host}/alex/cv1/motion?w=480&h=480")),
            ..modlight_api::Upload::default()
        };
        state.space.files.push(crate::myspace::model::FileEntry {
            file: modlight_core::library::CloudFile::new(row),
            selected: false,
        });
        state
    }

    /// Frames for `seconds` of wall time, the clock moving 50 ms a frame: completions
    /// first, then the motion tick, as the window runs them. `watch` sees the state
    /// after the completions and after the tick.
    fn run_frames(
        state: &mut AppState,
        ctx: &egui::Context,
        seconds: f64,
        mut watch: impl FnMut(&AppState, bool),
    ) {
        let started = std::time::Instant::now();
        while started.elapsed().as_secs_f64() < seconds {
            state.time += 0.05;
            for completion in state.rt.drain() {
                completion(state, ctx);
            }
            watch(state, false);
            state.tick_motion(ctx);
            watch(state, true);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// A server on 127.0.0.1 answering every request with `status` and `body`, counting
    /// the requests.
    fn server(
        status: &'static str,
        body: Vec<u8>,
    ) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut buffer = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buffer[..n]),
                    }
                }
                seen.fetch_add(1, Ordering::SeqCst);
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (host, count)
    }

    fn busy_server() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        server("503 Service Unavailable", Vec::new())
    }

    /// The working path: after the dwell the preview is fetched once, decoded and played.
    #[test]
    fn a_moving_preview_plays_after_the_dwell() {
        use std::sync::atomic::Ordering;
        let (host, requests) = server("200 OK", animated_gif(4, 48, 48));
        let mut state = state_with_video(&host);
        let ctx = egui::Context::default();
        state.time = 100.0;
        state.hover_file("v1");
        let started = std::time::Instant::now();
        while state.media.motion.playing.is_none() && started.elapsed().as_secs() < 10 {
            run_frames(&mut state, &ctx, 0.05, |_, _| {});
        }
        assert!(
            state
                .media
                .motion
                .playing
                .as_ref()
                .is_some_and(|(id, reel)| id == "v1" && reel.frames.len() == 4)
        );
        run_frames(&mut state, &ctx, 0.3, |_, _| {});
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert!(
            state.media.motion_bytes.get("cv1").is_some(),
            "kept for the next hover"
        );
    }

    /// `CloudMotionPlayer.BeginAsync`: a moving preview the server cannot give right now
    /// (503 from its transform limiter) is asked for once per hover. The tick used to
    /// start a new request on every frame after each answer, for as long as the pointer
    /// rested on the card.
    #[test]
    fn a_busy_server_is_asked_once_per_hover() {
        use std::sync::atomic::Ordering;
        let (host, requests) = busy_server();
        let mut state = state_with_video(&host);
        let ctx = egui::Context::default();
        state.time = 100.0;
        state.hover_file("v1");
        run_frames(&mut state, &ctx, 1.5, |_, _| {});
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "one request for one hover, however long the pointer rests"
        );
        assert!(
            !state.media.motion_bytes.is_refused("cv1"),
            "a 503 is not a refusal"
        );

        // Leaving and coming back is a new hover, which may ask again.
        state.unhover_file("v1");
        state.hover_file("v1");
        run_frames(&mut state, &ctx, 1.0, |_, _| {});
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    /// `AnimatedThumbnail.Decode` answering null files the bytes with the refusals; the
    /// same hover must not decode them again on every frame.
    #[test]
    fn bytes_that_do_not_animate_are_decoded_once() {
        let mut state = state_with_video("http://127.0.0.1:1");
        state
            .media
            .motion_bytes
            .put("cv1", Arc::new(animated_gif(1, 16, 16)));
        let ctx = egui::Context::default();
        state.time = 100.0;
        state.hover_file("v1");
        let mut decodes = 0;
        let mut was_decoding = false;
        run_frames(&mut state, &ctx, 1.0, |state, after_tick| {
            let decoding = state.media.motion.decoding.is_some();
            if after_tick && decoding && !was_decoding {
                decodes += 1;
            }
            was_decoding = decoding;
        });
        assert_eq!(decodes, 1, "decoded once, then known not to move");
        assert!(state.media.motion_bytes.is_refused("cv1"));
        assert!(state.media.motion.playing.is_none());
    }
}

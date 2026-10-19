//! Port of `lib/media/card-render.ts`: rasterise a file-type card.
//!
//! Split from the route so the same bytes can be produced by a script, and
//! split from `card_art` so that module stays free of libvips.
//!
//! The one word on the card goes through libvips' own text renderer with an
//! EXPLICIT font file rather than through the SVG, because the production
//! image ships no fontconfig configuration and no system font: `<text>` there
//! renders as empty boxes at the wrong size. Pointing Pango at a file
//! sidesteps that entirely (see `vips::init` for the one Pango setting the Mac
//! needs to do the same).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use bytes::Bytes;
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};

use crate::media::card::{CARD_HEIGHT, CARD_WIDTH, CardChoice, card_path, is_card_token};
use crate::media::card_art::{TOKEN_CENTER_Y, TOKEN_FONT_SIZE, TOKEN_MAX_WIDTH, card_art};
use crate::media::vips::{self, CompositeLayer, PngOptions, Sharp, TextInput, VipsError};

/// Shipped in the repo (`public/fonts/Geist-Bold.ttf`), and already served to
/// browsers, so nothing new is exposed.
static FONT_FILE: LazyLock<PathBuf> =
    LazyLock::new(|| crate::paths::Paths::resolve().public.join("fonts").join("Geist-Bold.ttf"));

/// Checked once: a missing font must cost one stat call, not one per request.
static FONT_AVAILABLE: LazyLock<bool> = LazyLock::new(|| FONT_FILE.exists());

type Pending = Shared<BoxFuture<'static, Result<Bytes, String>>>;

/// A card is fully determined by (category, token), and `card` admits only
/// pairs this app can actually emit, so an unevicted map is safe: the ceiling
/// is the extension table. Shared futures rather than buffers, so N
/// simultaneous first hits on a cold card share ONE render.
static CACHE: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// `CARD_DIMENSIONS`
pub const CARD_DIMENSIONS: (u32, u32) = (CARD_WIDTH, CARD_HEIGHT);

/// `renderCard(choice)`: the PNG for a card.
pub async fn render_card(choice: &CardChoice) -> Result<Bytes, String> {
    let key = card_path(choice);
    let pending = {
        let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .entry(key.clone())
            .or_insert_with(|| {
                let choice = choice.clone();
                async move { draw_card(choice).await.map(Bytes::from).map_err(|e| e.message().to_string()) }
                    .boxed()
                    .shared()
            })
            .clone()
    };
    let result = pending.await;
    if result.is_err() {
        // A failed draw must not be cached, or one transient fault poisons
        // this card for the life of the process.
        CACHE.lock().unwrap_or_else(|p| p.into_inner()).remove(&key);
    }
    result
}

async fn draw_card(choice: CardChoice) -> Result<Vec<u8>, VipsError> {
    vips::run_blocking(move || {
        let art = card_art(choice.category);
        let mut base = Sharp::new(Bytes::from(art.svg));
        if let Some(label) = draw_token(&choice.token, &art.accent) {
            base = base.composite(vec![CompositeLayer {
                input: label.png,
                left: ((f64::from(CARD_WIDTH) - f64::from(label.width)) / 2.0).round() as i32,
                // `Math.round`: halves go up, which `f64::round` matches for
                // the positive values here.
                top: (TOKEN_CENTER_Y - f64::from(label.height) / 2.0).round() as i32,
            }]);
        }
        base.png(PngOptions { compression_level: 9, ..Default::default() }).to_buffer_blocking()
    })
    .await
}

struct Token {
    png: Bytes,
    width: u32,
    height: u32,
}

/// `drawToken(token, colour)`: the token, or `None` when it cannot be drawn.
/// `None` is a real outcome: without the font the card still has its glyph,
/// its plate and its colour.
fn draw_token(token: &str, colour: &str) -> Option<Token> {
    if !*FONT_AVAILABLE || !is_card_token(token) {
        return None;
    }
    let mut rendered = render_text(token, colour, TOKEN_FONT_SIZE).ok()?;
    // Long words ("DOCUMENT") would run past the safe area, so measure and
    // shrink rather than guess a size per word.
    if rendered.width > TOKEN_MAX_WIDTH {
        let scaled = 24u32.max(((TOKEN_FONT_SIZE * TOKEN_MAX_WIDTH) as f64 / f64::from(rendered.width)).floor() as u32);
        rendered = render_text(token, colour, scaled).ok()?;
    }
    Some(rendered)
}

fn render_text(token: &str, colour: &str, size: u32) -> Result<Token, VipsError> {
    // Pango markup, so the colour and the tracking travel with the text.
    // `token` is [A-Z0-9] only, so there is nothing here to escape.
    let png = Sharp::text(TextInput {
        text: format!(r#"<span foreground="{colour}" letter_spacing="6144">{token}</span>"#),
        font: Some(format!("Geist {size}px")),
        fontfile: Some(FONT_FILE.to_string_lossy().into_owned()),
        rgba: true,
    })
    .png(PngOptions::default())
    .to_buffer_blocking()?;
    let png = Bytes::from(png);
    let meta = Sharp::new(png.clone()).metadata_blocking()?;
    Ok(Token { png, width: meta.width, height: meta.height })
}

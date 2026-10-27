//! Port of `lib/media/card-art.ts`: the drawing on a file-type card - one
//! family, eight glyphs.
//!
//! Two constraints shaped every decision here.
//!
//! The first is that the production container has no fontconfig and not one
//! system font, so librsvg draws `<text>` as empty boxes and ignores font-size
//! (an embedded @font-face does not help - librsvg ignores webfonts).
//! Everything in this module is therefore pure geometry; the one word on the
//! card is drawn separately by libvips' own text renderer with an explicit font
//! file, and composited on top (`card_render`). Nothing here may ever grow a
//! `<text>` element.
//!
//! The second is that the card is looked at OUTSIDE our site, in someone
//! else's chat window, so it inherits no theme and no language. What keeps
//! eight cards reading as one family is that only the hue changes: lightness
//! and chroma of the accent are pinned (L=0.80, C<=0.13), which holds every
//! card at the same visual weight.

use crate::i18n::format::js_number_to_string;
use crate::media::card::{CARD_HEIGHT, CARD_WIDTH, CardCategory};

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// OKLCH to linear sRGB.
fn oklch_to_linear_rgb(l: f64, c: f64, hue_deg: f64) -> [f64; 3] {
    let h = (hue_deg * std::f64::consts::PI) / 180.0;
    let a = c * h.cos();
    let b = c * h.sin();
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.291485548 * b;
    let (l3, m3, s3) = (l_.powf(3.0), m_.powf(3.0), s_.powf(3.0));
    [
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.707614701 * s3,
    ]
}

fn encode_gamma(u: f64) -> f64 {
    if u <= 0.0031308 { 12.92 * u } else { 1.055 * u.powf(1.0 / 2.4) - 0.055 }
}

fn in_gamut(rgb: &[f64; 3]) -> bool {
    rgb.iter().all(|v| *v >= -0.0005 && *v <= 1.0005)
}

/// Largest chroma at this lightness/hue that still fits sRGB, capped. Without
/// the walk, orange and green at C=0.13 clip differently and the family loses
/// the equal weight that is the entire point of pinning L.
fn fit_chroma(l: f64, hue: f64, cap: f64) -> f64 {
    let mut c = cap;
    while c > 0.0 && !in_gamut(&oklch_to_linear_rgb(l, c, hue)) {
        c -= 0.002;
    }
    c.max(0.0)
}

fn hex(l: f64, c: f64, hue: f64) -> String {
    let rgb = oklch_to_linear_rgb(l, c, hue);
    let mut out = String::from("#");
    for v in rgb {
        // `Math.round` on a value in [0, 255]: halves go up, as `f64::round` does there.
        let byte = (encode_gamma(v).clamp(0.0, 1.0) * 255.0).round() as u8;
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

const ACCENT_L: f64 = 0.8;
const ACCENT_C_CAP: f64 = 0.13;

/// Hue 260 is the Graphite accent (#3b82f6), so the unknown-type card's faint tint
/// is related to the product.
const NEUTRAL_HUE: f64 = 260.0;

// The Graphite surfaces, the same on every card: only the glyph carries a hue.
const BG: &str = "#121212";
const BG_EDGE: &str = "#0a0a0a";
const PLATE_FILL: &str = "#171717";
const PLATE_STROKE: &str = "#262626";
const GLOW: &str = "#262626";

fn bg() -> String {
    BG.to_string()
}

fn bg_edge() -> String {
    BG_EDGE.to_string()
}

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

struct CardStyle {
    hue: f64,
    /// Fallback art is deliberately grey: "I do not know what this is" should not shout.
    neutral: bool,
    /// Drawn in a 180x180 box; `CHIP` is substituted with the plate stroke colour.
    glyph: &'static str,
}

fn style(category: CardCategory) -> CardStyle {
    let (hue, neutral, glyph) = match category {
        CardCategory::Code => (
            195.0,
            false,
            r#"<path d="M70 45 L25 90 L70 135"/><path d="M110 45 L155 90 L110 135"/><path d="M100 30 L80 150"/>"#,
        ),
        // The latch is what separates a crate from a plain box at thumbnail size.
        CardCategory::Archive => (
            45.0,
            false,
            r#"<rect x="15" y="35" width="150" height="120" rx="16"/><path d="M15 75 H165"/><rect x="76" y="55" width="28" height="40" rx="10" fill="CHIP"/>"#,
        ),
        CardCategory::Document => (
            277.0,
            false,
            r#"<path d="M35 15 H105 L145 55 V165 H35 Z"/><path d="M105 15 V55 H145"/><path d="M60 100 H120"/><path d="M60 130 H100"/>"#,
        ),
        CardCategory::Sheet => (
            150.0,
            false,
            r#"<path d="M15 41 h150 v29 h-150 z" fill="CHIP" stroke="none"/><rect x="15" y="25" width="150" height="130" rx="16"/><path d="M15 70 H165"/><path d="M15 112 H165"/><path d="M78 70 V155"/>"#,
        ),
        // A symmetric level meter, not a quaver: five bars survive being 17px
        // tall, a note's stem and flag collapse into a smudge.
        CardCategory::Audio => (
            320.0,
            false,
            r#"<path d="M25 70 V110"/><path d="M62 40 V140"/><path d="M99 20 V160"/><path d="M136 45 V135"/><path d="M170 75 V105"/>"#,
        ),
        // The caret is the only difference from `document`, and it is what
        // says "characters" rather than "file".
        CardCategory::Text => (
            230.0,
            false,
            r#"<path d="M20 45 H150"/><path d="M20 85 H160"/><path d="M20 125 H105"/><path d="M135 108 V150"/>"#,
        ),
        CardCategory::Image => (
            15.0,
            false,
            r#"<rect x="15" y="30" width="150" height="120" rx="16"/><circle cx="60" cy="70" r="13"/><path d="M28 143 L72 99 L101 128 L128 101 L163 136"/>"#,
        ),
        CardCategory::Binary => (
            NEUTRAL_HUE,
            true,
            r#"<rect x="35" y="35" width="110" height="110" rx="18"/><path d="M70 15 V35 M110 15 V35 M70 145 V165 M110 145 V165 M15 70 H35 M15 110 H35 M145 70 H165 M145 110 H165"/>"#,
        ),
    };
    CardStyle { hue, neutral, glyph }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

const PLATE: f64 = 320.0;
const PLATE_X: f64 = (CARD_WIDTH as f64 - PLATE) / 2.0;
const PLATE_Y: f64 = 105.0;
const PLATE_RADIUS: f64 = 72.0;

/// The glyph fills 220 of the plate's 320, leaving 50 of air on every side.
const GLYPH_BOX: f64 = 220.0;
const GLYPH_SCALE: f64 = GLYPH_BOX / 180.0;
const GLYPH_X: f64 = PLATE_X + (PLATE - GLYPH_BOX) / 2.0;
const GLYPH_Y: f64 = PLATE_Y + (PLATE - GLYPH_BOX) / 2.0;

/// Vertical middle of the token, not its top: the label is measured after it
/// is drawn (libvips returns a tight ink box), and shrinking a long word must
/// not also float it upwards.
pub const TOKEN_CENTER_Y: f64 = 497.0;

/// Nominal token size; a long word is scaled down to fit TOKEN_MAX_WIDTH.
pub const TOKEN_FONT_SIZE: u32 = 78;
pub const TOKEN_MAX_WIDTH: u32 = 560;

/// `CardArt`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardArt {
    pub svg: String,
    /// Token colour, as a hex string pango markup can take.
    pub accent: String,
}

/// `cardArt(category)`: the whole card except its one word - background,
/// plate and glyph.
pub fn card_art(category: CardCategory) -> CardArt {
    let s = style(category);
    let chroma = if s.neutral { 0.015 } else { fit_chroma(ACCENT_L, s.hue, ACCENT_C_CAP) };
    let accent = hex(ACCENT_L, chroma, s.hue);
    let (plate_fill, plate_stroke, glow) = (PLATE_FILL, PLATE_STROKE, GLOW);
    // The glyph's own filled parts (a latch, a header row): a dim tint of its hue.
    let chip = hex(0.36, if s.neutral { 0.012 } else { chroma.min(0.06) }, s.hue);
    let glyph = s.glyph.replace("CHIP", &chip);
    let n = js_number_to_string;
    let (w, h) = (CARD_WIDTH, CARD_HEIGHT);
    let (bg, bg_edge) = (bg(), bg_edge());

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
<defs>
  <radialGradient id="glow" cx="50%" cy="42%" r="52%">
    <stop offset="0%" stop-color="{glow}" stop-opacity="0.55"/>
    <stop offset="100%" stop-color="{bg_edge}" stop-opacity="0"/>
  </radialGradient>
</defs>
<rect width="{w}" height="{h}" fill="{bg_edge}"/>
<rect width="{w}" height="{h}" fill="{bg}" opacity="0.85"/>
<rect width="{w}" height="{h}" fill="url(#glow)"/>
<rect x="{px}" y="{py}" width="{plate}" height="{plate}" rx="{pr}" fill="{plate_fill}" stroke="{plate_stroke}" stroke-width="3.5"/>
<g transform="translate({gx} {gy}) scale({gs})" fill="none" stroke="{accent}" stroke-width="13" stroke-linecap="round" stroke-linejoin="round">{glyph}</g>
</svg>"##,
        px = n(PLATE_X),
        py = n(PLATE_Y),
        plate = n(PLATE),
        pr = n(PLATE_RADIUS),
        gx = n(GLYPH_X),
        gy = n(GLYPH_Y),
        gs = n(GLYPH_SCALE),
    );
    CardArt { svg, accent }
}

#[cfg(test)]
mod tests {
    //! tests/unit/embed-card.test.ts, describe("card art")
    use super::*;
    use crate::media::card::CARD_CATEGORIES;

    fn relative_luminance(hex: &str) -> f64 {
        let n = u32::from_str_radix(&hex[1..], 16).unwrap();
        let ch = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map(|v| {
            let s = f64::from(v) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * ch[0] + 0.7152 * ch[1] + 0.0722 * ch[2]
    }

    #[test]
    fn draws_every_category_with_no_text_and_nothing_remote() {
        for c in CARD_CATEGORIES {
            let CardArt { svg, accent } = card_art(c);
            assert!(svg.starts_with("<svg"));
            assert!(svg.contains(r#"viewBox="0 0 1200 630""#));
            assert!(
                accent.len() == 7
                    && accent.starts_with('#')
                    && accent[1..].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
            for banned in ["<text", "<tspan", "@font-face", "font-family", "<image", "xlink:href"] {
                assert!(!svg.contains(banned), "{banned}");
            }
            for (i, _) in svg.match_indices("http") {
                assert!(svg[i..].starts_with("http://www.w3.org"));
            }
        }
    }

    #[test]
    fn holds_the_family_together() {
        let backdrop = relative_luminance(BG);
        let ratios: Vec<f64> = CARD_CATEGORIES
            .iter()
            .map(|c| (relative_luminance(&card_art(*c).accent) + 0.05) / (backdrop + 0.05))
            .collect();
        let max = ratios.iter().cloned().fold(f64::MIN, f64::max);
        let min = ratios.iter().cloned().fold(f64::MAX, f64::min);
        assert!(max - min < 1.5);
        assert!(min > 9.0);
        let accents: std::collections::HashSet<String> = CARD_CATEGORIES.iter().map(|c| card_art(*c).accent).collect();
        assert_eq!(accents.len(), CARD_CATEGORIES.len());
    }

    #[test]
    fn numbers_print_like_javascript() {
        let svg = card_art(CardCategory::Code).svg;
        assert!(svg.contains(r#"<rect x="440" y="105" width="320" height="320" rx="72""#));
        assert!(svg.contains("translate(490 155) scale(1.2222222222222223)"));
    }
}

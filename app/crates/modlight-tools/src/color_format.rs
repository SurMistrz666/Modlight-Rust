//! The colour text formats: `CodeMenuEntryPixelInfo.Parse` (what the screen colour
//! picker copies) and `ScreenColorPickerTextFormatter.Format` (the info text next to the
//! picker's magnifier). Both replace `$`-tokens in the user's pattern.

use modlight_core::culture::Culture;

use crate::color::{self, Color, ColorFormat};

/// `string.Replace(old, new, StringComparison.*IgnoreCase)` for ASCII tokens.
pub fn replace_ignore_case(input: &str, token: &str, value: &str) -> String {
    if token.is_empty() {
        return input.to_string();
    }
    let lower_input = input.to_ascii_lowercase();
    let lower_token = token.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut at = 0;
    while let Some(found) = lower_input[at..].find(&lower_token) {
        let start = at + found;
        out.push_str(&input[at..start]);
        out.push_str(value);
        at = start + token.len();
    }
    out.push_str(&input[at..]);
    out
}

/// `string.ForEachBetween(front, back)`: every `front...back` piece and what is inside.
fn each_between(text: &str, front: &str, back: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut f = 0;
    while text.len() > f {
        let Some(found) = text[f..].find(front) else {
            break;
        };
        let start = f + found;
        let Some(end) = text[start + front.len()..].find(back) else {
            break;
        };
        let end = start + front.len() + end;
        let whole = &text[start..end + back.len()];
        out.push((
            whole.to_string(),
            text[start + front.len()..end].to_string(),
        ));
        f = start + front.len();
    }
    out
}

/// `double.ToString()` in the current culture.
fn general(value: f64) -> String {
    Culture::current().general(value)
}

/// `CodeMenuEntryPixelInfo.Parse`.
pub fn pixel_info(input: &str, color: Color, x: i64, y: i64) -> String {
    let hex = color::to_hex(color, ColorFormat::Rgb);
    let cmyk = color::to_cmyk(color);
    let round2 = |v: f64| general(color::round_away(v, 2));
    let mut text = input.to_string();
    text = replace_ignore_case(&text, "$r255", &color.r.to_string());
    text = replace_ignore_case(&text, "$g255", &color.g.to_string());
    text = replace_ignore_case(&text, "$b255", &color.b.to_string());
    text = text.replace("$rHEX", &format!("{:02X}", color.r));
    text = text.replace("$gHEX", &format!("{:02X}", color.g));
    text = text.replace("$bHEX", &format!("{:02X}", color.b));
    text = replace_ignore_case(&text, "$rhex", &format!("{:02x}", color.r));
    text = replace_ignore_case(&text, "$ghex", &format!("{:02x}", color.g));
    text = replace_ignore_case(&text, "$bhex", &format!("{:02x}", color.b));
    text = text.replace("$HEX", &hex);
    text = replace_ignore_case(&text, "$hex", &hex.to_ascii_lowercase());
    text = replace_ignore_case(&text, "$c100", &round2(cmyk.cyan100()));
    text = replace_ignore_case(&text, "$m100", &round2(cmyk.magenta100()));
    text = replace_ignore_case(&text, "$y100", &round2(cmyk.yellow100()));
    text = replace_ignore_case(&text, "$k100", &round2(cmyk.key100()));
    text = replace_ignore_case(&text, "$name", &color::color_name(color));
    text = replace_ignore_case(&text, "$x", &x.to_string());
    text = replace_ignore_case(&text, "$y", &y.to_string());
    text = replace_ignore_case(&text, "$n", "\n");

    for (token, component) in [("$r1", color.r), ("$g1", color.g), ("$b1", color.b)] {
        for (whole, inside) in each_between(&text.clone(), &format!("{token}{{"), "}") {
            let digits = inside.split(',').next().unwrap_or("").trim();
            let digits = digits.parse::<i32>().unwrap_or(0).clamp(0, 15);
            let value = general(color::round_away(component as f64 / 255.0, digits));
            text = replace_ignore_case(&text, &whole, &value);
        }
    }
    for (token, component) in [("$r1", color.r), ("$g1", color.g), ("$b1", color.b)] {
        let value = general(color::round_away(component as f64 / 255.0, 3));
        text = replace_ignore_case(&text, token, &value);
    }
    text = replace_ignore_case(&text, "$r", &color.r.to_string());
    text = replace_ignore_case(&text, "$g", &color.g.to_string());
    text = replace_ignore_case(&text, "$b", &color.b.to_string());
    text
}

/// `ScreenColorPickerTextFormatter.Format`: the info text under the magnifier.
pub fn screen_picker_text(format: &str, color: Color, x: i64, y: i64) -> String {
    if format.is_empty() {
        return String::new();
    }
    let upper = format!("{:02X}{:02X}{:02X}", color.r, color.g, color.b);
    // `ToCmyk`: the formatter's own conversion (key from the brightest channel).
    let (red, green, blue) = (
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
    );
    let key = 1.0 - red.max(green.max(blue));
    let (c, m, yy, k) = if key >= 1.0 {
        (0.0, 0.0, 0.0, 1.0)
    } else {
        let d = 1.0 - key;
        (
            (1.0 - red - key) / d,
            (1.0 - green - key) / d,
            (1.0 - blue - key) / d,
            key,
        )
    };
    let round2 = |v: f64| general(color::round_away(v * 100.0, 2));
    let mut text = format.to_string();
    text = replace_ignore_case(&text, "$r255", &color.r.to_string());
    text = replace_ignore_case(&text, "$g255", &color.g.to_string());
    text = replace_ignore_case(&text, "$b255", &color.b.to_string());
    text = text.replace("$rHEX", &format!("{:02X}", color.r));
    text = text.replace("$gHEX", &format!("{:02X}", color.g));
    text = text.replace("$bHEX", &format!("{:02X}", color.b));
    text = replace_ignore_case(&text, "$rhex", &format!("{:02x}", color.r));
    text = replace_ignore_case(&text, "$ghex", &format!("{:02x}", color.g));
    text = replace_ignore_case(&text, "$bhex", &format!("{:02x}", color.b));
    text = text.replace("$HEX", &upper);
    text = replace_ignore_case(&text, "$hex", &upper.to_ascii_lowercase());
    text = replace_ignore_case(&text, "$c100", &round2(c));
    text = replace_ignore_case(&text, "$m100", &round2(m));
    text = replace_ignore_case(&text, "$y100", &round2(yy));
    text = replace_ignore_case(&text, "$k100", &round2(k));
    text = replace_ignore_case(&text, "$name", color::exact_known_name(color));
    text = replace_ignore_case(&text, "$x", &x.to_string());
    text = replace_ignore_case(&text, "$y", &y.to_string());
    text = replace_ignore_case(&text, "$n", "\n");
    // `\$(r|g|b)1(?:\{(\d+)\})?`, case-insensitive.
    let pattern = regex::Regex::new(r"(?i)\$(r|g|b)1(?:\{(\d+)\})?").expect("valid");
    text = pattern
        .replace_all(&text, |captures: &regex::Captures| {
            let precision = captures
                .get(2)
                .and_then(|m| m.as_str().parse::<i32>().ok())
                .map(|p| p.clamp(0, 15))
                .unwrap_or(3);
            let value = match captures[1].to_ascii_lowercase().as_str() {
                "r" => color.r,
                "g" => color.g,
                _ => color.b,
            };
            general(color::round_away(value as f64 / 255.0, precision))
        })
        .into_owned();
    text = replace_ignore_case(&text, "$r", &color.r.to_string());
    text = replace_ignore_case(&text, "$g", &color.g.to_string());
    text = replace_ignore_case(&text, "$b", &color.b.to_string());
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::culture::set_thread_culture;

    #[test]
    fn the_default_formats() {
        set_thread_culture(Some(Culture::english()));
        let c = Color::rgb(18, 140, 200);
        assert_eq!(pixel_info("$HEX", c, 5, 6), "128CC8");
        assert_eq!(pixel_info("$hex", c, 5, 6), "128cc8");
        assert_eq!(pixel_info("$r255, $g255, $b255", c, 5, 6), "18, 140, 200");
        assert_eq!(
            pixel_info("RGB: $r255, $g255, $b255$nHex: $hex$nX: $x Y: $y", c, 5, 6),
            "RGB: 18, 140, 200\nHex: 128cc8\nX: 5 Y: 6"
        );
        assert_eq!(pixel_info("$r1 $g1{1} $b1{0}", c, 0, 0), "0.071 0.5 1");
        assert_eq!(pixel_info("$c100 $k100", c, 0, 0), "91 21.57");
        assert_eq!(pixel_info("$name", c, 0, 0), "Dodger blue");
        assert_eq!(pixel_info("$rHEX$rhex", c, 0, 0), "1212");
        set_thread_culture(None);
    }

    #[test]
    fn the_info_text() {
        set_thread_culture(Some(Culture::polish()));
        let c = Color::rgb(255, 255, 255);
        assert_eq!(screen_picker_text("#$HEX", c, 1, 2), "#FFFFFF");
        assert_eq!(screen_picker_text("$name", c, 1, 2), "ControlLightLight");
        assert_eq!(
            screen_picker_text("$R1{2} $x,$y", Color::rgb(128, 0, 0), 3, 4),
            "0,5 3,4"
        );
        assert_eq!(screen_picker_text("", c, 0, 0), "");
        set_thread_culture(None);
    }

    #[test]
    fn replace_ignores_case() {
        assert_eq!(replace_ignore_case("a$X b$x", "$x", "1"), "a1 b1");
        assert_eq!(replace_ignore_case("ąę$xż", "$X", "1"), "ąę1ż");
    }
}

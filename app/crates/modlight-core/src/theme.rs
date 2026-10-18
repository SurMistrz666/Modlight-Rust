//! Skins and modes: `Skins.axaml`, `SkinVariants.g.cs`, `ThemeManager.cs`,
//! `ApplicationThemeOptions.cs` and `AccentForeground.cs`.
//!
//! Two independent axes, as on the website: the SKIN decides what the app looks like,
//! the MODE whether it is on a light or a dark ground. Every skin ships both palettes.

use serde::{Deserialize, Serialize};

/// A colour as the skins write it: red, green, blue, alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const WHITE: Rgba = Rgba([255, 255, 255, 255]);
    pub const BLACK: Rgba = Rgba([0, 0, 0, 255]);

    pub fn r(self) -> u8 {
        self.0[0]
    }
    pub fn g(self) -> u8 {
        self.0[1]
    }
    pub fn b(self) -> u8 {
        self.0[2]
    }
    pub fn a(self) -> u8 {
        self.0[3]
    }

    /// `#RRGGBB` or `#AARRGGBB` (Avalonia's `Color.TryParse` forms).
    pub fn parse(text: &str) -> Option<Rgba> {
        let hex = text.trim().strip_prefix('#')?;
        let value = u32::from_str_radix(hex, 16).ok()?;
        match hex.len() {
            6 => Some(Rgba([
                (value >> 16) as u8,
                (value >> 8) as u8,
                value as u8,
                255,
            ])),
            8 => Some(Rgba([
                (value >> 16) as u8,
                (value >> 8) as u8,
                value as u8,
                (value >> 24) as u8,
            ])),
            _ => None,
        }
    }

    /// `#AARRGGBB`, the way `ApplicationThemeOptions.AccentColor` writes it back.
    pub fn to_hex_argb(self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            self.a(),
            self.r(),
            self.g(),
            self.b()
        )
    }

    /// `ThemeManager.DarkenColor`: every channel times `1 - amount`, rounded.
    pub fn darken(self, amount: f64) -> Rgba {
        let factor = (1.0 - amount).clamp(0.0, 1.0);
        let channel = |c: u8| (c as f64 * factor).round() as u8;
        Rgba([
            channel(self.r()),
            channel(self.g()),
            channel(self.b()),
            self.a(),
        ])
    }
}

/// One palette: the `ShareX.Color.*` keys of one skin in one mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub accent_start: Rgba,
    pub accent_end: Rgba,
    pub accent_foreground: Rgba,
    pub skin_accent: Rgba,
    pub skin_accent_foreground: Rgba,
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub text_on_panel: Rgba,
    pub text_secondary_on_panel: Rgba,
    pub focus: Rgba,
    pub focus_on_panel: Rgba,
    pub background_main: Rgba,
    pub background_panel: Rgba,
    pub background_popup: Rgba,
    pub background_menu: Rgba,
    pub background_toolbar: Rgba,
    pub border: Rgba,
    pub control_background: Rgba,
    pub control_background_hover: Rgba,
    pub control_background_disabled: Rgba,
    pub control_border: Rgba,
    pub separator: Rgba,
    pub cancel_background: Rgba,
    pub cancel_border: Rgba,
    pub continue_background: Rgba,
    pub continue_background_hover: Rgba,
    pub continue_border: Rgba,
    pub task_foreground: Rgba,
    pub checker_a: Rgba,
    pub checker_b: Rgba,
}

/// One skin, as the picker lists it.
#[derive(Debug, PartialEq, Eq)]
pub struct Skin {
    /// The panel's id: the value written to and read from `/api/theme`.
    pub id: &'static str,
    pub name_english: &'static str,
    pub name_polish: &'static str,
    pub light: Palette,
    pub dark: Palette,
}

mod generated {
    use super::{Palette, Rgba, Skin};
    include!(concat!(env!("OUT_DIR"), "/skins.rs"));
}

pub use generated::{DEFAULT_SKIN_ID, SKINS};

impl Skin {
    pub fn name(&self, language: crate::l10n::Language) -> &'static str {
        match language {
            crate::l10n::Language::English => self.name_english,
            crate::l10n::Language::Polish => self.name_polish,
        }
    }

    pub fn palette(&self, dark: bool) -> &Palette {
        if dark { &self.dark } else { &self.light }
    }
}

/// The skin with this id (case-insensitive), or the default when the id is unknown:
/// the website can be a release ahead of this build.
pub fn resolve_skin(id: Option<&str>) -> &'static Skin {
    if let Some(id) = id
        && let Some(skin) = SKINS.iter().find(|s| s.id.eq_ignore_ascii_case(id))
    {
        return skin;
    }
    SKINS
        .iter()
        .find(|s| s.id == DEFAULT_SKIN_ID)
        .unwrap_or(&SKINS[0])
}

/// Whether this id names a skin this build can paint.
pub fn is_known_skin(id: &str) -> bool {
    SKINS.iter().any(|s| s.id.eq_ignore_ascii_case(id))
}

/// `ApplicationThemeOptions`, stored in ApplicationConfig under `ThemeOptions` and,
/// like upstream, never part of the synced document (the skin has its own endpoint).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ThemeOptions {
    pub skin: String,
    /// What the server last confirmed. Advanced only when the server answered.
    pub skin_confirmed: String,
    pub brand_version: i32,
    /// "Dark" or "Light"; used when not following the system.
    pub theme: String,
    pub use_system_theme: bool,
    pub accent_color_hex: String,
    pub use_system_accent_color: bool,
}

pub const DEFAULT_THEME: &str = "Dark";
pub const DEFAULT_ACCENT_COLOR_HEX: &str = "#5857E9";
pub const DEFAULT_ACCENT_COLOR_HEX_DARK: &str = "#7E88FB";

impl Default for ThemeOptions {
    fn default() -> Self {
        ThemeOptions {
            skin: DEFAULT_SKIN_ID.to_string(),
            skin_confirmed: DEFAULT_SKIN_ID.to_string(),
            brand_version: 1,
            theme: DEFAULT_THEME.to_string(),
            use_system_theme: true,
            accent_color_hex: DEFAULT_ACCENT_COLOR_HEX.to_string(),
            use_system_accent_color: false,
        }
    }
}

impl ThemeOptions {
    /// Whether the accent is either of the two brand defaults (then the skin's own wins).
    pub fn is_accent_default(&self) -> bool {
        self.accent_color_hex
            .eq_ignore_ascii_case(DEFAULT_ACCENT_COLOR_HEX)
            || self
                .accent_color_hex
                .eq_ignore_ascii_case(DEFAULT_ACCENT_COLOR_HEX_DARK)
    }

    pub fn accent_color(&self) -> Rgba {
        Rgba::parse(&self.accent_color_hex)
            .filter(|c| c.a() > 0)
            .unwrap_or(Rgba::parse(DEFAULT_ACCENT_COLOR_HEX).unwrap())
    }

    /// A blank skin id reads as the default, like the property setter.
    pub fn set_skin(&mut self, value: &str) {
        self.skin = if value.trim().is_empty() {
            DEFAULT_SKIN_ID.to_string()
        } else {
            value.to_string()
        };
    }
}

fn is_light_theme(theme: &str) -> bool {
    !theme.trim().is_empty() && theme.to_ascii_lowercase().contains("light")
}

/// What the screen is painted with right now.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTheme {
    pub skin: &'static Skin,
    pub dark: bool,
    pub palette: Palette,
}

/// `ThemeManager.ApplyConfiguredSettings`.
///
/// `system_dark` is the system appearance when known; `system_accent` the system accent
/// colour when known and opaque.
pub fn resolve(
    options: &ThemeOptions,
    system_dark: Option<bool>,
    system_accent: Option<Rgba>,
) -> ResolvedTheme {
    let dark = if options.use_system_theme {
        match system_dark {
            Some(dark) => dark,
            None => !is_light_theme(&options.theme),
        }
    } else {
        !is_light_theme(&options.theme)
    };

    let skin = resolve_skin(Some(&options.skin));
    let mut palette = *skin.palette(dark);

    // Precedence: the system accent if asked for, then a colour the user picked, then
    // the skin's own (read from Skin.Accent, a name the overwrite never touches).
    let (start, from_skin) = match system_accent {
        Some(accent) if options.use_system_accent_color && accent.a() > 0 => (accent, false),
        _ if options.is_accent_default() => (palette.skin_accent, true),
        _ => (options.accent_color(), false),
    };

    let end = start.darken(0.10);
    let foreground = if from_skin {
        // The skin states its own pairing, measured against 4.5:1 when it was generated.
        palette.skin_accent_foreground
    } else {
        choose_accent_foreground(
            start,
            end,
            Rgba::parse("#FAFAFA").unwrap(),
            Rgba::parse("#0A0C1A").unwrap(),
        )
    };

    palette.accent_start = start;
    palette.accent_end = end;
    palette.accent_foreground = foreground;

    ResolvedTheme {
        skin,
        dark,
        palette,
    }
}

pub const MINIMUM_CONTRAST: f64 = 4.5;

/// `AccentForeground.Choose`: the palette colour with the better contrast against both
/// ends of the accent; white or black only when neither palette colour reaches 4.5:1 and
/// one of them does better.
pub fn choose_accent_foreground(start: Rgba, end: Rgba, light: Rgba, dark: Rgba) -> Rgba {
    let light_contrast = against(light, start, end);
    let dark_contrast = against(dark, start, end);
    let best = if dark_contrast > light_contrast {
        dark
    } else {
        light
    };
    let best_contrast = light_contrast.max(dark_contrast);
    if best_contrast >= MINIMUM_CONTRAST {
        return best;
    }
    let white = against(Rgba::WHITE, start, end);
    let black = against(Rgba::BLACK, start, end);
    if white.max(black) <= best_contrast {
        return best;
    }
    if white > black {
        Rgba::WHITE
    } else {
        Rgba::BLACK
    }
}

fn against(foreground: Rgba, start: Rgba, end: Rgba) -> f64 {
    contrast_ratio(foreground, start).min(contrast_ratio(foreground, end))
}

pub fn contrast_ratio(first: Rgba, second: Rgba) -> f64 {
    let a = relative_luminance(first);
    let b = relative_luminance(second);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

pub fn relative_luminance(color: Rgba) -> f64 {
    fn linearize(channel: u8) -> f64 {
        let value = channel as f64 / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linearize(color.r()) + 0.7152 * linearize(color.g()) + 0.0722 * linearize(color.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skin_of_the_panel_is_here() {
        assert_eq!(SKINS.len(), 14);
        assert_eq!(DEFAULT_SKIN_ID, "discord");
        let ids: Vec<&str> = SKINS.iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            vec![
                "default",
                "glass",
                "synthwave",
                "terminal",
                "paper",
                "win98",
                "nebula",
                "grimoire",
                "cartoon",
                "mono",
                "deco",
                "riso",
                "discord",
                "spotify"
            ]
        );
        assert_eq!(resolve_skin(Some("nebula")).name_polish, "Mgławica");
        let classic = resolve_skin(Some("DEFAULT"));
        assert_eq!(classic.dark.background_main, Rgba([0x0A, 0x0A, 0x0A, 255]));
        assert_eq!(classic.dark.accent_start, Rgba::parse("#7E88FB").unwrap());
        assert_eq!(resolve_skin(Some("not-yet")).id, "discord");
        assert!(is_known_skin("Spotify"));
        assert!(!is_known_skin("not-yet"));
    }

    #[test]
    fn the_mode_follows_the_system_unless_told_otherwise() {
        let mut options = ThemeOptions::default();
        assert!(resolve(&options, Some(true), None).dark);
        assert!(!resolve(&options, Some(false), None).dark);
        assert!(
            resolve(&options, None, None).dark,
            "no system answer: the configured theme"
        );
        options.use_system_theme = false;
        options.theme = "Light".into();
        assert!(!resolve(&options, Some(true), None).dark);
    }

    #[test]
    fn the_accent_comes_from_the_skin_unless_picked() {
        let mut options = ThemeOptions {
            skin: "terminal".into(),
            ..Default::default()
        };
        let skin = resolve_skin(Some("terminal"));
        let theme = resolve(&options, Some(true), None);
        assert_eq!(theme.palette.accent_start, skin.dark.skin_accent);
        assert_eq!(
            theme.palette.accent_foreground,
            skin.dark.skin_accent_foreground
        );
        assert_eq!(theme.palette.accent_end, skin.dark.skin_accent.darken(0.10));

        options.accent_color_hex = "#FFFF00".into();
        let picked = resolve(&options, Some(true), None);
        assert_eq!(picked.palette.accent_start, Rgba::parse("#FFFF00").unwrap());
        assert_eq!(
            picked.palette.accent_foreground,
            Rgba::parse("#0A0C1A").unwrap()
        );

        options.use_system_accent_color = true;
        let system = resolve(&options, Some(true), Some(Rgba::parse("#0000FF").unwrap()));
        assert_eq!(system.palette.accent_start, Rgba::parse("#0000FF").unwrap());
        assert_eq!(
            system.palette.accent_foreground,
            Rgba::parse("#FAFAFA").unwrap()
        );
    }

    #[test]
    fn colours_parse_and_print_like_avalonia() {
        assert_eq!(
            Rgba::parse("#80112233"),
            Some(Rgba([0x11, 0x22, 0x33, 0x80]))
        );
        assert_eq!(Rgba([0x11, 0x22, 0x33, 0xFF]).to_hex_argb(), "#FF112233");
        assert_eq!(
            Rgba([100, 200, 255, 255]).darken(0.1),
            Rgba([90, 180, 230, 255])
        );
        assert!((contrast_ratio(Rgba::WHITE, Rgba::BLACK) - 21.0).abs() < 1e-9);
    }
}

//! Port of `lib/themes.ts`: the skins a user can pick, in the order the picker
//! shows them, and how any stored value is normalised to one of them.

/// One skin. `name` / `description` are the English fallbacks; the picker
/// translates them through `profile.themes.<id>.{name,desc}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub const THEMES: &[Theme] = &[
    // The id stays `default` - it is written into every existing row and into
    // the CSS selector. Only the LABEL changes.
    Theme {
        id: "default",
        name: "Classic",
        description: "Clean, violet-accented and quiet. The original Modlight look.",
    },
    Theme {
        id: "glass",
        name: "Liquid Glass",
        description: "Neutral translucent panels with a lit edge, floating over light.",
    },
    Theme { id: "synthwave", name: "Synthwave", description: "Neon pink & cyan on deep purple." },
    Theme { id: "terminal", name: "Terminal", description: "Green-on-black hacker console, monospace." },
    Theme { id: "paper", name: "Paper", description: "Warm, serif, notebook feel." },
    Theme { id: "win98", name: "Windows 98", description: "Retro beveled nostalgia." },
    Theme {
        id: "nebula",
        name: "Nebula",
        description: "A real starfield under nebula light. Panels lit along one edge.",
    },
    Theme {
        id: "grimoire",
        name: "Grimoire",
        description: "Parchment and gilt. Old-style serif, framed panels, candlelit after dark.",
    },
    Theme {
        id: "cartoon",
        name: "Cartoon",
        description: "Ink outlines, flat colour, hard shadows and buttons that bounce.",
    },
    Theme {
        id: "mono",
        name: "Monochrome",
        description: "Black, white and one red. Grid, hairlines and typography do the rest.",
    },
    Theme { id: "deco", name: "Art Deco", description: "Gold on navy, a sunburst on the wall, capitals set wide." },
    Theme {
        id: "riso",
        name: "Riso Press",
        description: "Duplicator print: halftone screen, paper grain, ink that lands off register.",
    },
    Theme {
        id: "discord",
        name: "Late Night",
        description: "Deep blue-slate in layers, one indigo accent, everything softly rounded.",
    },
    Theme {
        id: "spotify",
        name: "Night Player",
        description: "Near-black with lifted cards and a single vivid green doing all the work.",
    },
];

/// What an account gets when it has never chosen ("Late Night"). Must stay in
/// step with the `user.theme` column default (migration 0009).
pub const DEFAULT_THEME: &str = "discord";

/// Skin ids that were renamed, mapped to what they are called now. Keep
/// entries here forever: `user.theme` rows still hold "modern".
pub const THEME_ALIASES: &[(&str, &str)] = &[("modern", "default")];

pub fn is_theme_id(v: &str) -> bool {
    THEMES.iter().any(|t| t.id == v)
}

/// Normalize any stored value to a valid theme id: current ids, retired ids
/// (via [`THEME_ALIASES`]) and anything else, which lands on the default.
pub fn resolve_theme(v: Option<&str>) -> &'static str {
    let Some(v) = v else {
        return DEFAULT_THEME;
    };
    if let Some(t) = THEMES.iter().find(|t| t.id == v) {
        return t.id;
    }
    if let Some((_, to)) = THEME_ALIASES.iter().find(|(from, _)| *from == v) {
        return to;
    }
    DEFAULT_THEME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ids_aliases_and_junk() {
        assert_eq!(resolve_theme(Some("glass")), "glass");
        assert_eq!(resolve_theme(Some("modern")), "default");
        assert_eq!(resolve_theme(Some("nope")), "discord");
        assert_eq!(resolve_theme(None), "discord");
        assert_eq!(THEMES.len(), 14);
        assert!(is_theme_id(DEFAULT_THEME));
    }
}

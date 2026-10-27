// Port of upstream lib/themes.ts with the Graphite skin added.
// @upstream lib/themes.ts
// Deviations: "graphite" leads THEMES and is DEFAULT_THEME, and the retired id
// "modern" (what sign-up writes for an account that never chose) follows the
// default instead of Classic. Mirrors server/src/themes.rs.
/**
 * The skins a user can pick, in the order the picker shows them.
 *
 * `name` / `description` are the English fallbacks. The picker translates them
 * through `profile.themes.<id>.{name,desc}`; these strings are what a caller
 * without an i18n context (scripts, tests, an admin table) gets to show.
 */
export const THEMES = [
  {
    // The panel's own look and what an account starts with: deep neutrals,
    // hairline borders, white primary buttons and a single blue accent.
    id: "graphite",
    name: "Graphite",
    description: "Deep neutrals, hairline borders, white buttons and one blue accent.",
  },
  {
    // The id stays `default` - it is written into every existing row and into the
    // CSS selector. Only the LABEL changes: this skin is no longer what you get
    // by default, and a picker entry called "Default" that is not would be the
    // most confusing thing on the page.
    id: "default",
    name: "Classic",
    description: "Clean, violet-accented and quiet. The original Modlight look.",
  },
  {
    id: "glass",
    name: "Liquid Glass",
    description: "Neutral translucent panels with a lit edge, floating over light.",
  },
  { id: "synthwave", name: "Synthwave", description: "Neon pink & cyan on deep purple." },
  { id: "terminal", name: "Terminal", description: "Green-on-black hacker console, monospace." },
  { id: "paper", name: "Paper", description: "Warm, serif, notebook feel." },
  { id: "win98", name: "Windows 98", description: "Retro beveled nostalgia." },
  {
    id: "nebula",
    name: "Nebula",
    description: "A real starfield under nebula light. Panels lit along one edge.",
  },
  {
    id: "grimoire",
    name: "Grimoire",
    description: "Parchment and gilt. Old-style serif, framed panels, candlelit after dark.",
  },
  {
    id: "cartoon",
    name: "Cartoon",
    description: "Ink outlines, flat colour, hard shadows and buttons that bounce.",
  },
  {
    id: "mono",
    name: "Monochrome",
    description: "Black, white and one red. Grid, hairlines and typography do the rest.",
  },
  {
    id: "deco",
    name: "Art Deco",
    description: "Gold on navy, a sunburst on the wall, capitals set wide.",
  },
  {
    id: "riso",
    name: "Riso Press",
    description: "Duplicator print: halftone screen, paper grain, ink that lands off register.",
  },
  // Inspired by, not copied from. Neither uses the other product's brand colour:
  // the indigo here is lighter and less saturated than Discord's blurple and sits
  // in Modlight's own blue-violet family, and the green is a hue away from
  // Spotify's. What is borrowed is the arrangement of light and dark, which is
  // what people actually mean when they name those apps.
  {
    id: "discord",
    name: "Late Night",
    description: "Deep blue-slate in layers, one indigo accent, everything softly rounded.",
  },
  {
    id: "spotify",
    name: "Night Player",
    description: "Near-black with lifted cards and a single vivid green doing all the work.",
  },
] as const;

export type ThemeId = (typeof THEMES)[number]["id"];
export const THEME_IDS: readonly string[] = THEMES.map((t) => t.id);
/**
 * What an account gets when it has never chosen: Graphite. Must stay in step
 * with server/src/themes.rs and the `user.theme` column default (migration 0014).
 * An account that already holds an explicit theme keeps it.
 */
export const DEFAULT_THEME: ThemeId = "graphite";

/**
 * Stored values that are not skin ids, mapped to the skin they render.
 *
 * "modern" is what sign-up writes into `user.theme` for every new account, so a
 * row holding it is an account that never opened the picker: it follows the
 * default skin. A choice made in the picker is always stored under a real id
 * ("default" for Classic). Keep entries here forever.
 */
export const THEME_ALIASES: Readonly<Record<string, ThemeId>> = {
  modern: DEFAULT_THEME,
};

export function isThemeId(v: unknown): v is ThemeId {
  return typeof v === "string" && THEME_IDS.includes(v);
}

/**
 * Normalize any stored value to a valid theme id.
 *
 * Accepts current ids, retired ids (via THEME_ALIASES) and anything else -
 * the last of which lands on the default rather than on an empty page.
 */
export function resolveTheme(v: unknown): ThemeId {
  if (isThemeId(v)) return v;
  if (typeof v === "string" && v in THEME_ALIASES) return THEME_ALIASES[v]!;
  return DEFAULT_THEME;
}

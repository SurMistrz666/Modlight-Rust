/**
 * The two cookies that let the server render the theme onto <html> before the
 * first paint. They mirror client-side state; nothing security-sensitive lives
 * here, so a plain readable cookie is the right tool.
 *
 *   ml_mode — light | dark | system   (colour mode, a viewer preference)
 *   ml_skin — a ThemeId               (skin, the account's own choice)
 */
export const MODES = ["light", "dark", "system"] as const;
export type Mode = (typeof MODES)[number];

export const MODE_COOKIE = "ml_mode";
export const SKIN_COOKIE = "ml_skin";

/** One year — a preference, not a session. */
export const THEME_COOKIE_MAX_AGE = 60 * 60 * 24 * 365;

export function resolveMode(v: unknown): Mode {
  return typeof v === "string" && (MODES as readonly string[]).includes(v) ? (v as Mode) : "system";
}

"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { resolveTheme } from "@/lib/themes";
import { SKIN_COOKIE, THEME_COOKIE_MAX_AGE } from "@/lib/theme-cookies";
import type { BackgroundStyle } from "@/lib/background";

/**
 * Switches the active skin (`data-theme` on <html>).
 *
 * The panel renders this with the signed-in user's skin; public pages render it
 * with the *owner's* skin, so a visitor sees the page the way its owner styled
 * it. Since the root layout can only know the viewer's own cookie, the override
 * has to happen per route - and it has to happen before the browser paints, or
 * the visitor sees a frame of the wrong theme.
 *
 * Two mechanisms, because one alone is not enough:
 *
 *  1. A blocking inline <script>. Every caller renders this as the first child
 *    of the page, so it executes while the rest of the document is still being
 *    parsed - before the first paint. (The original `useEffect` version ran
 *    after hydration, which is exactly the flash it was meant to prevent.)
 *
 *  2. An effect that re-asserts the same attribute. `data-theme` is *also* a
 *    rendered prop of <html> in the root layout, fed by the `ml_skin` cookie;
 *    any client render where that prop changes writes it straight back onto the
 *    DOM node, undoing step 1. That is the "theme sometimes disappears" case,
 *    and on a public page it would silently swap the owner's skin for the
 *    visitor's. The effect runs after every commit, so this component always has
 *    the last word.
 *
 * Only the panel mirrors the skin into the cookie: a public page is showing
 * somebody else's skin and must not overwrite the visitor's own.
 *
 * `key` is the resolved skin so client-side navigation between two differently
 * themed pages remounts the node - updating a <script>'s text in place would
 * not re-execute it.
 */
export function ThemeApplier({
  theme,
  background,
  persist,
}: {
  theme?: string | null;
  /**
   * The account's wallpaper, from the same row the skin came from - so a public
   * page shows the OWNER's wallpaper, exactly as it shows the owner's skin.
   * Omitted or null means "no wallpaper", which also clears any left over from
   * the previous page during client-side navigation.
   */
  background?: BackgroundStyle | null;
  /**
   * Mirror the skin into a cookie so the next server render already has it.
   * Defaults to "only inside the panel".
   */
  persist?: boolean;
}) {
  const resolved = resolveTheme(theme);
  const pathname = usePathname();
  const shouldPersist =
    persist ?? (pathname === "/dashboard" || pathname.startsWith("/dashboard/"));

  const bgUrl = background?.url ?? null;
  const bgDim = background ? background.dim : 0;

  React.useEffect(() => {
    const root = document.documentElement;
    if (root.getAttribute("data-theme") !== resolved) {
      root.setAttribute("data-theme", resolved);
    }
    applyBackground(bgUrl, bgDim);
    if (shouldPersist) {
      document.cookie = `${SKIN_COOKIE}=${resolved};path=/;max-age=${THEME_COOKIE_MAX_AGE};samesite=lax`;
    }
  }, [resolved, shouldPersist, bgUrl, bgDim]);

  const script =
    `(function(){var r=document.documentElement,t=${JSON.stringify(resolved)},u=${JSON.stringify(bgUrl)};` +
    `r.setAttribute("data-theme",t);` +
    // Mirrors applyBackground() below. Duplicated rather than shared because
    // this string has to run standalone, before any bundle has loaded.
    `if(u){r.style.setProperty("--ml-bg-image",'url("'+u+'")');` +
    `r.style.setProperty("--ml-bg-dim",${JSON.stringify(String(bgDim))});` +
    `r.setAttribute("data-bg","on")}` +
    `else{r.style.removeProperty("--ml-bg-image");r.style.removeProperty("--ml-bg-dim");r.removeAttribute("data-bg")}` +
    `if(${shouldPersist ? "true" : "false"}){try{document.cookie=${JSON.stringify(SKIN_COOKIE)}+"="+t+";path=/;max-age=${THEME_COOKIE_MAX_AGE};samesite=lax"}catch(e){}}` +
    `})()`;

  return (
    <script
      key={`${resolved}|${bgUrl ?? ""}`}
      suppressHydrationWarning
      dangerouslySetInnerHTML={{ __html: script }}
    />
  );
}

/**
 * The wallpaper is three custom properties on <html>, not a rendered element:
 * globals.css owns the painting, so a skin can restyle or suppress it without
 * this component knowing anything about it.
 *
 * The URL is injected through a CSS custom property, which is why it is built
 * server-side from a tag we control (lib/background.ts) rather than from
 * anything a visitor supplies.
 */
function applyBackground(url: string | null, dim: number): void {
  const root = document.documentElement;
  if (url) {
    root.style.setProperty("--ml-bg-image", `url("${url}")`);
    root.style.setProperty("--ml-bg-dim", String(dim));
    root.setAttribute("data-bg", "on");
  } else {
    root.style.removeProperty("--ml-bg-image");
    root.style.removeProperty("--ml-bg-dim");
    root.removeAttribute("data-bg");
  }
}

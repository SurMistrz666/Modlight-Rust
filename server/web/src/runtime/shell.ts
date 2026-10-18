/**
 * The HTML document normally arrives with `lang`, `data-mode` and `data-theme`
 * already on <html> (the Rust server's shell, server/src/pages/shell.rs). When
 * they are missing (the vite dev server, fixture mode), fill them in from the
 * first page payload, the way upstream's root layout rendered them and
 * next-themes' blocking script (index.html) resolved the mode.
 */
import type { createBrowserRouter } from "react-router";
import { getRootData } from "./root-data";

type DataRouter = ReturnType<typeof createBrowserRouter>;

function apply(): boolean {
  const html = document.documentElement;
  if (html.hasAttribute("data-theme")) return true;
  const root = getRootData();
  if (!root) return false;
  html.setAttribute("lang", root.locale);
  html.setAttribute("data-theme", root.theme);
  if (!html.hasAttribute("data-mode")) {
    let mode: string = root.mode;
    try {
      mode = window.localStorage.getItem("theme") || root.mode;
    } catch {
      /* storage unavailable */
    }
    if (mode === "system") {
      mode = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    }
    html.setAttribute("data-mode", mode);
    html.style.colorScheme = mode;
  }
  return true;
}

export function applyShellDefaults(router: DataRouter): void {
  if (apply()) return;
  const unsubscribe = router.subscribe(() => {
    if (apply()) unsubscribe();
  });
}

/**
 * The HTML shell normally arrives with <html> attributes and the head already
 * filled in by the Rust server (docs/CONTRACT.md "HTML shell"). When it did not
 * (the vite dev server, fixture mode), the `data-ml-html` marker is still on
 * <html>: fill in what the server would have, from the first page payload, the
 * way upstream's root layout and next-themes' inline script set them.
 */
import type { createBrowserRouter } from "react-router";
import { getRootData } from "./root-data";

type DataRouter = ReturnType<typeof createBrowserRouter>;

function apply(): boolean {
  const html = document.documentElement;
  if (!html.hasAttribute("data-ml-html")) return true;
  const root = getRootData();
  if (!root) return false;
  html.removeAttribute("data-ml-html");
  html.setAttribute("lang", root.locale);
  html.setAttribute("data-theme", root.skin);
  // next-themes' blocking script: the stored choice, else the cookie default.
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
  return true;
}

export function applyShellDefaults(router: DataRouter): void {
  if (apply()) return;
  const unsubscribe = router.subscribe(() => {
    if (apply()) unsubscribe();
  });
}

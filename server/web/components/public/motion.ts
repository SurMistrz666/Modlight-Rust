"use client";

import { useEffect, useState } from "react";

/**
 * Tracks the visitor's `prefers-reduced-motion` setting (SSR safe).
 *
 * A local copy of the dashboard hook rather than an import from `FileCard`:
 * that module pulls the whole file menu, toasts and dialogs behind it, and a
 * public gallery has no business shipping any of that to a visitor.
 */
export function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);

  useEffect(() => {
    const mql = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onChange = () => setReduced(mql.matches);
    onChange();
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, []);

  return reduced;
}

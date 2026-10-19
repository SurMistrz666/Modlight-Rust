"use client";

import * as React from "react";

/**
 * The viewer's chrome: a wide panel welded to the bottom edge of the viewport,
 * above the media but out of its way.
 *
 * It spans roughly four fifths of the window (nearly the whole width on a
 * phone) and is capped on very large screens, which is what keeps the file's
 * stats on a single line instead of wrapping into a ragged second row. It sits
 * flush against the bottom — no gap, no bottom border, rounded only at the top —
 * so it reads as something rising out of the edge of the screen rather than a
 * pill floating above it. The home-indicator inset is absorbed as inner padding,
 * so the panel still touches the edge on a phone.
 *
 * It publishes its own height as `--viewer-bar-space` on the document element so
 * the stage above can reserve exactly that much room. The bar still wraps to two
 * rows on narrow screens (long filename, six actions), and any hard-coded
 * padding would either crop the media or leave a dead gap under it. The variable
 * has to live on <html> because the stage is a server-rendered sibling, not a
 * child of this component.
 *
 * The wrapper is `pointer-events-none` so only the panel itself swallows clicks —
 * the media keeps the full width of the screen underneath it.
 */
export function FloatingBar({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement | null>(null);

  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const root = document.documentElement;

    const publish = () => {
      root.style.setProperty(
        "--viewer-bar-space",
        `${Math.ceil(el.getBoundingClientRect().height)}px`,
      );
    };

    publish();
    const ro = new ResizeObserver(publish);
    ro.observe(el);

    // Belt and braces. A ResizeObserver only reports during a rendering update,
    // so a viewport change that lands while the tab isn't painting leaves the
    // reserved space describing the *previous* bar — and a narrow viewport wraps
    // the bar to three rows, which is exactly when a stale number would let it
    // sit on top of the picture. `resize` fires regardless. Late-swapping fonts
    // change the bar's text metrics after first paint, so re-measure then too.
    window.addEventListener("resize", publish);
    document.fonts?.ready.then(publish).catch(() => {});

    return () => {
      ro.disconnect();
      window.removeEventListener("resize", publish);
      root.style.removeProperty("--viewer-bar-space");
    };
  }, []);

  return (
    <>
      {/*
        Pre-hydration defaults, deliberately generous: roughly what the bar
        measures at each breakpoint (identity plus a wrapped action row on a
        phone, two rows on a tablet, one on a desktop). Without them the
        server-rendered frame would
        reserve one fixed number and a tall picture would tuck under the bar on
        a phone until the effect above ran. The inline style the effect writes on
        <html> outranks these rules, so the measured value always wins.
      */}
      <style>
        {":root{--viewer-bar-space:8.5rem}" +
          "@media (min-width:640px){:root{--viewer-bar-space:6.5rem}}" +
          "@media (min-width:1024px){:root{--viewer-bar-space:4.5rem}}"}
      </style>
      <div
        ref={ref}
        className="pointer-events-none fixed inset-x-0 bottom-0 z-40 flex justify-center"
      >
        <div
          className={
            // Flush with the bottom edge: rounded and bordered on the top three
            // sides only, so it looks like it is pushing up out of the screen.
            "pointer-events-auto w-[calc(100%-1.5rem)] max-w-[90rem] rounded-t-2xl border border-b-0 " +
            "border-border/70 bg-card/85 px-3 pt-2.5 shadow-2xl shadow-foreground/10 backdrop-blur " +
            "supports-[backdrop-filter]:bg-card/70 sm:w-[92%] sm:px-4 sm:pt-3 lg:w-4/5"
          }
          // Keep clear of the home indicator / gesture bar on phones without
          // lifting the panel off the edge.
          style={{ paddingBottom: "calc(0.625rem + env(safe-area-inset-bottom, 0px))" }}
        >
          {children}
        </div>
      </div>
    </>
  );
}

/**
 * `geist/font/sans` and `geist/font/mono` (next/font/local objects) for the
 * browser build. The @font-face rules and classes live in src/fonts/geist.css,
 * which reproduces next/font's output for geist@1.7.2 with stable class names;
 * <html> carries the two `variable` classes from the document template.
 */
import "../../fonts/geist.css";

export interface NextFont {
  className: string;
  variable: string;
  style: { fontFamily: string; fontWeight?: number; fontStyle?: string };
}

export const GeistSans: NextFont = {
  className: "geist-sans-class",
  variable: "geist-sans",
  style: { fontFamily: "GeistSans, GeistSans Fallback" },
};

export const GeistMono: NextFont = {
  className: "geist-mono-class",
  variable: "geist-mono",
  style: {
    fontFamily:
      "GeistMono, ui-monospace, SFMono-Regular, Roboto Mono, Menlo, Monaco, Liberation Mono, DejaVu Sans Mono, Courier New, monospace",
  },
};

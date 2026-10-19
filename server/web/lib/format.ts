/** Human-readable byte size (e.g. 1.5 GB). */
export function humanFileSize(bytes: number, precision = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${i === 0 ? n : n.toFixed(precision)} ${units[i]}`;
}

/**
 * The language a relative time falls back to when the caller names none.
 *
 * Deliberately a fixed locale rather than the runtime default. `Intl` with an
 * undefined locale asks the host, and the host is not the reader: the container
 * resolves to whatever ICU was built with, the browser to `navigator.language`,
 * and neither is the locale next-intl picked for this request. Two different
 * answers on the two sides of the same render is also a hydration mismatch,
 * which is why the previous hardcoded-English version never produced one.
 *
 * So: callers inside a next-intl tree pass `useLocale()` / `getLocale()` and get
 * the reader's language; everyone else keeps exactly the language they had.
 */
const FALLBACK_LOCALE = "en";

/**
 * `short` rather than `narrow`: narrow is the compact style that matches what
 * this function used to emit in English ("2h ago", "3d ago"), but Polish narrow
 * abbreviates to "wcz." and "2 g. temu", which reads like a typo. `short` is the
 * shortest style that stays a word in both dictionaries.
 *
 * `numeric: "auto"` is what turns 0 into "now" / "teraz" and one day into
 * "yesterday" / "wczoraj" instead of counting them.
 */
const RELATIVE_OPTIONS: Intl.RelativeTimeFormatOptions = { numeric: "auto", style: "short" };

/**
 * Formatters are cached: constructing an `Intl.RelativeTimeFormat` loads locale
 * data, and these render once per row in tables that hold hundreds of rows.
 * The key space is bounded by the locales the app ships (en, pl).
 */
const formatters = new Map<string, Intl.RelativeTimeFormat>();

function relativeFormatter(locale: string): Intl.RelativeTimeFormat {
  const cached = formatters.get(locale);
  if (cached) return cached;
  let fmt: Intl.RelativeTimeFormat;
  try {
    fmt = new Intl.RelativeTimeFormat(locale, RELATIVE_OPTIONS);
  } catch {
    // A locale tag reaches here from a cookie, so a malformed one is a client's
    // to send, not a bug to crash on.
    fmt = new Intl.RelativeTimeFormat(FALLBACK_LOCALE, RELATIVE_OPTIONS);
  }
  formatters.set(locale, fmt);
  return fmt;
}

/**
 * Compact relative time in the reader's language ("2 hr. ago", "2 godz. temu").
 *
 * Future instants are formatted as such ("in 3 days"), which the previous
 * implementation could not do: it subtracted in one direction only, so an expiry
 * a week away rendered as "just now" next to the word "Expires".
 *
 * @param locale BCP 47 tag from next-intl. Omitted means English (see
 *   FALLBACK_LOCALE) rather than "whatever the machine is set to".
 */
export function relativeTime(
  date: Date | string | number,
  locale: string = FALLBACK_LOCALE,
): string {
  const d = date instanceof Date ? date : new Date(date);
  const ms = d.getTime() - Date.now();
  // An unparseable date used to come out as "NaNy ago". Nothing is better.
  if (!Number.isFinite(ms)) return "";

  const fmt = relativeFormatter(locale);
  // Magnitude and direction are kept apart on purpose: Math.round(-1.5) is -1,
  // not -2, so rounding a signed difference bends future times towards zero.
  const ahead = ms > 0;
  const signed = (n: number) => (ahead ? n : -n);

  const s = Math.round(Math.abs(ms) / 1000);
  if (s < 60) return fmt.format(0, "second");
  const m = Math.round(s / 60);
  if (m < 60) return fmt.format(signed(m), "minute");
  const h = Math.round(m / 60);
  if (h < 24) return fmt.format(signed(h), "hour");
  const days = Math.round(h / 24);
  if (days < 30) return fmt.format(signed(days), "day");
  const mo = Math.round(days / 30);
  if (mo < 12) return fmt.format(signed(mo), "month");
  return fmt.format(signed(Math.round(mo / 12)), "year");
}

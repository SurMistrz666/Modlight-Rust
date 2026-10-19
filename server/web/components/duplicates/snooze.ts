/**
 * "Not now" for the duplicate reminder.
 *
 * Two different noes live in this feature and they must not be confused:
 *
 *  - "keep these copies" is a decision ABOUT FILES. It is permanent, it belongs
 *    to the account rather than to the browser, and it is stored per checksum in
 *    `duplicate_dismissals` so the reminder never counts those bytes again.
 *  - "not now" is a decision about THIS MOMENT. It is a cookie, it lapses, and
 *    it says nothing about the files at all.
 *
 * The second one is why the strip can be described as periodic rather than as a
 * nag: it goes away for a week on request, and it can be silenced for good from
 * the page it links to. Putting it in a cookie rather than a column is
 * deliberate - a snooze is worth exactly one row's write on the busiest path in
 * the panel, which is to say it is not worth one.
 *
 * Nothing here is a security boundary. Forging the cookie hides a reminder from
 * yourself, which is also what the button does.
 */

export const SNOOZE_COOKIE = "ml_dupes_snooze";

/** How long "not now" lasts. Long enough not to be a nag, short enough to return. */
export const SNOOZE_DAYS = 7;

const DAY_MS = 86_400_000;

/** The instant a snooze started `now` stops holding. */
export function snoozeUntil(now: number): number {
  return now + SNOOZE_DAYS * DAY_MS;
}

/**
 * Is the reminder currently muted?
 *
 * Anything unparseable reads as "not snoozed". A cookie somebody typed by hand
 * must not be able to silence the reminder forever, and the failure direction
 * that shows a strip too often is the recoverable one.
 */
export function isSnoozed(value: string | null | undefined, now: number): boolean {
  if (!value) return false;
  const until = Number(value);
  return Number.isFinite(until) && until > now;
}

/**
 * The cookie the strip writes, as a `document.cookie` string.
 *
 * `Max-Age` matches the value inside it, so a browser that keeps the cookie and
 * a server that reads it agree on when the week is up.
 */
export function snoozeCookie(now: number): string {
  const until = snoozeUntil(now);
  return `${SNOOZE_COOKIE}=${until}; Path=/; Max-Age=${SNOOZE_DAYS * 86_400}; SameSite=Lax`;
}

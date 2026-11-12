/**
 * `?next=` on /login and /2fa: where to go after signing in. Upstream's
 * safeNext (app/(auth)/login/page.tsx, app/(auth)/2fa/page.tsx) keeps it to
 * "an in-app path so it can't be used as an open redirect" by accepting any
 * value that starts with one "/" and not with "//".
 *
 * The URL parser reads more than that as another host: a backslash counts as a
 * slash in an http(s) URL ("/\evil.example" is "//evil.example"), and tabs and
 * line breaks are dropped wherever they stand ("/<TAB>/evil.example" too). The
 * sign-in forms hand the value to router.push, which loads any other origin as
 * a document, so /login?next=/%5Cevil.example sent a freshly signed-in user off
 * the site. Refused here as server/src/vault/gate.rs `safe_next` refuses them,
 * and anything that still resolves to another origin falls back as well.
 */
const FALLBACK = "/dashboard";
const BASE = "https://modlight.invalid";

export function safeNext(value: string | string[] | undefined): string {
  const raw = Array.isArray(value) ? value[0] : value;
  if (!raw || !raw.startsWith("/") || raw.startsWith("//")) return FALLBACK;
  // Backslashes and control characters (C0, DEL, C1) have no business in a path.
  if (/[\\\u0000-\u001f\u007f-\u009f]/.test(raw)) return FALLBACK;
  try {
    if (new URL(raw, BASE).origin !== BASE) return FALLBACK;
  } catch {
    return FALLBACK;
  }
  return raw;
}

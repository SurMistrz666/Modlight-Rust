/**
 * Normalize a better-auth `{ error }` object (or any thrown value) into a short,
 * human-readable string. better-auth returns `{ message, code, status }`; we
 * fall back to the code and finally a generic line so the UI is never blank.
 */
export function authErrorMessage(
  error: unknown,
  fallback = "Something went wrong. Please try again.",
): string {
  if (!error) return fallback;
  if (typeof error === "string") return error;
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "object") {
    const e = error as { message?: string; statusText?: string; code?: string };
    if (e.message) return e.message;
    if (e.statusText) return e.statusText;
    if (e.code) {
      return e.code
        .toLowerCase()
        .replace(/_/g, " ")
        .replace(/^\w/, (c) => c.toUpperCase());
    }
  }
  return fallback;
}

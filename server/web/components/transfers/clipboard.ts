/**
 * Clipboard write that never throws.
 *
 * `navigator.clipboard` is unavailable on insecure origins and rejects when the
 * document is not focused, which is exactly the case when an upload finishes in
 * a background tab. Callers get a boolean and show the link instead.
 *
 * Lives in its own module so the provider and the dock can share it without
 * importing the Uppy runtime.
 */
export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    return false;
  }
}

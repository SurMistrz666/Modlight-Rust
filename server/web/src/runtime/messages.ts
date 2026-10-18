/**
 * Translations arrive with the root layout data, as upstream's root layout
 * handed them to NextIntlClientProvider: `layouts.root.messages` (the 17 client
 * namespaces of the request's locale) plus `messagesHash`. Once the SPA holds a
 * dictionary it says so on every /_page request with
 * `X-Modlight-Messages: <locale>:<messagesHash>`, and the server answers
 * `messages: null` while that is still current (server/src/pages/root_layout.rs).
 */
import type { RootLayoutData } from "../contract/layouts";

export type Locale = "en" | "pl";
export type Messages = Record<string, unknown>;

const held = new Map<string, Messages>(); // "<locale>:<hash>" -> messages
let current: string | null = null;

export function isLocale(v: unknown): v is Locale {
  return v === "en" || v === "pl";
}

function keyOf(root: Pick<RootLayoutData, "locale" | "messagesHash">): string {
  return `${root.locale}:${root.messagesHash}`;
}

/**
 * Take the dictionary a root layout brought, or confirm the one it refers to.
 * Returns false when the payload says "you have them" but this tab does not
 * (the caller then asks again without the header).
 */
export function acceptMessages(root: RootLayoutData): boolean {
  const key = keyOf(root);
  if (root.messages) {
    held.set(key, root.messages);
    current = key;
    return true;
  }
  if (held.has(key)) {
    current = key;
    return true;
  }
  return false;
}

/** Synchronous read for render; acceptMessages() has run for this root. */
export function messagesFor(root: RootLayoutData): Messages {
  const hit = held.get(keyOf(root));
  if (!hit) throw new Error(`messages for "${root.locale}" (${root.messagesHash}) were not received`);
  return hit;
}

/** The `X-Modlight-Messages` value to send, once a dictionary is held. */
export function messagesHeader(): string | null {
  return current;
}

/** Forget everything (tests). */
export function resetMessages(): void {
  held.clear();
  current = null;
}

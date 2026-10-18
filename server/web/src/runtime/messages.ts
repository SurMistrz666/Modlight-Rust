/**
 * Translations, bundled per locale (see src/messages/*.ts). The root layout's
 * `locale` decides which bundle is loaded; switching languages loads the other
 * one before the page that asked for it renders.
 */
export type Locale = "en" | "pl";
export type Messages = Record<string, unknown>;

const loaders: Record<Locale, () => Promise<{ default: Messages }>> = {
  en: () => import("../messages/en"),
  pl: () => import("../messages/pl"),
};

const loaded = new Map<Locale, Messages>();

export function isLocale(v: unknown): v is Locale {
  return v === "en" || v === "pl";
}

export async function loadMessages(locale: Locale): Promise<Messages> {
  const hit = loaded.get(locale);
  if (hit) return hit;
  const mod = await loaders[locale]();
  loaded.set(locale, mod.default);
  return mod.default;
}

/** Synchronous read for render; loadMessages() must have run for this locale. */
export function messagesFor(locale: Locale): Messages {
  const hit = loaded.get(locale);
  if (!hit) throw new Error(`messages for "${locale}" were not loaded`);
  return hit;
}

/**
 * `next-intl` for the browser build: the same thing next-intl 4.13.0's own
 * client entry (dist/esm/production/index.react-client.js) exports, built on the
 * same use-intl 4.13.0, so ICU formatting is identical. Nothing here knows about
 * Next.js; the locale comes from the root layout payload.
 */
import {
  IntlProvider,
  useFormatter as useFormatterBase,
  useTranslations as useTranslationsBase,
} from "use-intl/react";
import type { ComponentProps } from "react";

// eslint-disable-next-line @typescript-eslint/no-unsafe-function-type
function callHook<T extends Function>(name: string, hook: T): T {
  return ((...args: unknown[]) => {
    try {
      return hook(...args);
    } catch {
      throw new Error(
        `Failed to call \`${name}\` because the context from \`NextIntlClientProvider\` was not found.`,
      );
    }
  }) as unknown as T;
}

export const useTranslations = callHook("useTranslations", useTranslationsBase);
export const useFormatter = callHook("useFormatter", useFormatterBase);

export function NextIntlClientProvider({ locale, ...rest }: ComponentProps<typeof IntlProvider>) {
  if (!locale) {
    throw new Error(
      "Couldn't infer the `locale` prop in `NextIntlClientProvider`, please provide it explicitly.",
    );
  }
  return <IntlProvider locale={locale} {...rest} />;
}

export { IntlProvider, useLocale, useMessages, useNow, useTimeZone } from "use-intl/react";
export * from "use-intl/core";

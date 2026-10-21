/**
 * Optional Cloudflare Turnstile ("verify you are human") on the sign-in,
 * registration, password reset and invitation forms. Not upstream: an addition
 * on top of the port (server/src/turnstile.rs, SKELETON.md section 9 item 43).
 *
 * The server says whether it is on: `layouts.root.config.turnstileSiteKey` is
 * the site key, or null. With no site key nothing here does anything: no
 * script, no widget, no header, the forms behave exactly as upstream's.
 *
 * With one, a form renders `captcha.widget` and hands `captcha.headers()` to
 * its request: the token goes in `x-captcha-response` (the header Better Auth's
 * captcha plugin reads), to an RPC call through `withActionHeaders` and to a
 * Better Auth client call through `fetchOptions.headers`. Tokens are good for
 * one verification, so the form calls `captcha.reset()` after every attempt
 * and the widget fetches a fresh one.
 *
 * Cloudflare's script is loaded once, from its own origin (it must not be
 * bundled or proxied), only when a site key is present; the server's CSP lets
 * https://challenges.cloudflare.com in only while the check is on.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTheme } from "next-themes";
import { useLocale, useTranslations } from "next-intl";
import { getRootData } from "../runtime/root-data";

export const TURNSTILE_HEADER = "x-captcha-response";
export const TURNSTILE_SCRIPT = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";

/** What `turnstile.render()` takes (the subset used here). */
interface RenderOptions {
  sitekey: string;
  theme: "light" | "dark" | "auto";
  language: string;
  size: "normal" | "flexible" | "compact";
  "refresh-expired": "auto" | "manual" | "never";
  callback: (token: string) => void;
  "expired-callback": () => void;
  "timeout-callback": () => void;
  "error-callback": (code: string) => boolean | void;
}

/** `window.turnstile` (https://developers.cloudflare.com/turnstile/get-started/client-side-rendering/). */
export interface TurnstileApi {
  render(container: HTMLElement, options: RenderOptions): string | undefined | null;
  reset(widgetId?: string): void;
  remove(widgetId?: string): void;
}

declare global {
  interface Window {
    turnstile?: TurnstileApi;
  }
}

/** The site key while the check is on, else null. */
export function turnstileSiteKey(): string | null {
  return getRootData()?.config?.turnstileSiteKey || null;
}

let loading: Promise<TurnstileApi> | null = null;

/** Cloudflare's api.js, once per page (a failed load may be retried). */
export function loadTurnstile(): Promise<TurnstileApi> {
  if (window.turnstile) return Promise.resolve(window.turnstile);
  if (loading) return loading;
  loading = new Promise<TurnstileApi>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = TURNSTILE_SCRIPT;
    script.async = true;
    script.dataset.modlightTurnstile = "";
    script.onload = () => {
      if (window.turnstile) resolve(window.turnstile);
      else reject(new Error("Turnstile did not start"));
    };
    script.onerror = () => {
      loading = null;
      script.remove();
      reject(new Error("Turnstile could not be loaded"));
    };
    document.head.appendChild(script);
  });
  return loading;
}

function TurnstileWidget({
  siteKey,
  generation,
  onToken,
}: {
  siteKey: string;
  /** Bumped by `reset()`: start over with a fresh token. */
  generation: number;
  onToken: (token: string | null) => void;
}) {
  const t = useTranslations("turnstile.widget");
  const locale = useLocale();
  const { resolvedTheme } = useTheme();
  const theme = resolvedTheme === "dark" ? "dark" : resolvedTheme === "light" ? "light" : "auto";
  const container = useRef<HTMLDivElement>(null);
  const widgetId = useRef<string | null>(null);
  const report = useRef(onToken);
  report.current = onToken;
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setFailed(false);
    loadTurnstile().then(
      (api) => {
        if (cancelled || !container.current) return;
        widgetId.current =
          api.render(container.current, {
            sitekey: siteKey,
            theme,
            language: locale,
            size: "flexible",
            "refresh-expired": "auto",
            callback: (token) => report.current(token),
            "expired-callback": () => report.current(null),
            "timeout-callback": () => report.current(null),
            "error-callback": () => {
              report.current(null);
              // Handled: the widget shows its own state and retries by itself.
              return true;
            },
          }) ?? null;
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
      if (widgetId.current && window.turnstile) window.turnstile.remove(widgetId.current);
      widgetId.current = null;
      report.current(null);
    };
  }, [siteKey, theme, locale]);

  useEffect(() => {
    if (generation > 0 && widgetId.current && window.turnstile) window.turnstile.reset(widgetId.current);
  }, [generation]);

  return (
    <div className="space-y-2" data-turnstile-widget="">
      <div ref={container} role="group" aria-label={t("label")} className="min-h-[65px]" />
      {failed ? <p className="text-destructive text-xs leading-relaxed">{t("loadFailed")}</p> : null}
    </div>
  );
}

export interface Captcha {
  /** The server asks for the check (a site key is configured). */
  enabled: boolean;
  /** The widget, to render inside the form (null while off). */
  widget: ReactNode;
  /** The token for the next attempt, or null (none yet, or spent). */
  token: string | null;
  /** The request headers for the next attempt ({} while off or without a token). */
  headers(): Record<string, string>;
  /** After every attempt: the token is spent, fetch a new one. */
  reset(): void;
}

export function useCaptcha(): Captcha {
  const siteKey = turnstileSiteKey();
  const [token, setToken] = useState<string | null>(null);
  const [generation, setGeneration] = useState(0);
  const tokenRef = useRef<string | null>(null);
  const onToken = useCallback((next: string | null) => {
    tokenRef.current = next;
    setToken(next);
  }, []);
  const reset = useCallback(() => {
    tokenRef.current = null;
    setToken(null);
    setGeneration((g) => g + 1);
  }, []);
  const headers = useCallback((): Record<string, string> => {
    const current = tokenRef.current;
    return current ? { [TURNSTILE_HEADER]: current } : {};
  }, []);
  const widget = useMemo(
    () => (siteKey ? <TurnstileWidget siteKey={siteKey} generation={generation} onToken={onToken} /> : null),
    [siteKey, generation, onToken],
  );
  return { enabled: siteKey !== null, widget, token, headers, reset };
}

/** The codes the server answers a refused check with (Better Auth routes). */
const AUTH_CODES: Record<string, "required" | "failed" | "unavailable"> = {
  MISSING_RESPONSE: "required",
  VERIFICATION_FAILED: "failed",
  SERVICE_UNAVAILABLE: "unavailable",
};

/**
 * The message for a Better Auth `{ error }` the check produced, or null when
 * the error is something else.
 */
export function captchaErrorKey(error: unknown): "required" | "failed" | "unavailable" | null {
  if (!error || typeof error !== "object") return null;
  const code = (error as { code?: unknown }).code;
  return typeof code === "string" && code in AUTH_CODES ? AUTH_CODES[code] : null;
}

/** The two refusals the server actions add (`{status}` / `{reason}`). */
export type CaptchaRefusal = "captcha" | "captcha-unavailable";

export function captchaRefusalKey(value: unknown): "failed" | "unavailable" | null {
  if (value === "captcha") return "failed";
  if (value === "captcha-unavailable") return "unavailable";
  return null;
}

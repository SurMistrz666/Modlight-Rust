/**
 * `next/navigation` for the browser build, on top of react-router.
 *
 * Hooks read react-router's location; the router verbs go through
 * src/runtime/navigation.ts so links, pushes and redirects share one set of
 * rules. `redirect()` / `notFound()` throw Next's own error shapes, which the
 * SPA's boundaries and global handlers act on the way Next's do.
 */
import { useMemo } from "react";
import { useLocation, useParams as useRouterParams } from "react-router";
import * as nav from "../../runtime/navigation";
import { makeNotFoundError, makeRedirectError, type RedirectType as RedirectKind } from "../../runtime/errors";

export interface NavigateOptions {
  scroll?: boolean;
}

export interface PrefetchOptions {
  kind?: unknown;
  onInvalidate?: () => void;
}

export interface AppRouterInstance {
  back(): void;
  forward(): void;
  refresh(): void;
  push(href: string, options?: NavigateOptions): void;
  replace(href: string, options?: NavigateOptions): void;
  prefetch(href: string, options?: PrefetchOptions): void;
}

const appRouter: AppRouterInstance = {
  back: () => nav.back(),
  forward: () => nav.forward(),
  refresh: () => {
    void nav.refresh();
  },
  push: (href, options) => nav.navigate(href, { replace: false, scroll: options?.scroll }),
  replace: (href, options) => nav.navigate(href, { replace: true, scroll: options?.scroll }),
  prefetch: (href) => nav.prefetch(href),
};

/** Stable across renders, like Next's. */
export function useRouter(): AppRouterInstance {
  return appRouter;
}

export function usePathname(): string {
  return useLocation().pathname;
}

/** Next's ReadonlyURLSearchParams: a URLSearchParams whose mutators throw. */
export class ReadonlyURLSearchParams extends URLSearchParams {
  append(): never {
    throw new ReadonlyURLSearchParamsError();
  }
  delete(): never {
    throw new ReadonlyURLSearchParamsError();
  }
  set(): never {
    throw new ReadonlyURLSearchParamsError();
  }
  sort(): never {
    throw new ReadonlyURLSearchParamsError();
  }
}

class ReadonlyURLSearchParamsError extends Error {
  constructor() {
    super("Method unavailable on `ReadonlyURLSearchParams`. Read more: https://nextjs.org/docs/app/api-reference/functions/use-search-params#updating-searchparams");
  }
}

export function useSearchParams(): ReadonlyURLSearchParams {
  const { search } = useLocation();
  return useMemo(() => new ReadonlyURLSearchParams(search), [search]);
}

export function useParams<T extends Record<string, string | string[]> = Record<string, string | string[]>>(): T {
  return useRouterParams() as unknown as T;
}

export const RedirectType = { push: "push", replace: "replace" } as const;
export type RedirectType = RedirectKind;

export function redirect(url: string, type: RedirectKind = "replace"): never {
  throw makeRedirectError(url, type, 307);
}

export function permanentRedirect(url: string, type: RedirectKind = "replace"): never {
  throw makeRedirectError(url, type, 308);
}

export function notFound(): never {
  throw makeNotFoundError();
}

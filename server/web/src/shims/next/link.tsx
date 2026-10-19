/**
 * `next/link` for the browser build.
 *
 * Renders the same <a> Next's <Link> renders and intercepts the same clicks Next
 * intercepts (app-dir link.js `linkClicked`): a plain primary-button click,
 * without modifier keys, on a same-origin href, not `download`, not a `target`
 * other than _self, and not already prevented by the caller's onClick. Those
 * become client navigations; everything else is left to the browser. A
 * same-origin href that is not a page of the SPA (a route handler) is left to
 * the browser too, which is where Next's MPA fallback would send it.
 */
import * as React from "react";
import * as nav from "../../runtime/navigation";

type Query = Record<string, string | number | boolean | null | undefined | ReadonlyArray<string | number | boolean>>;

export interface UrlObject {
  pathname?: string | null;
  query?: Query | string | null;
  search?: string | null;
  hash?: string | null;
}

export type LinkProps = Omit<React.AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & {
  href: string | UrlObject;
  as?: string | UrlObject;
  replace?: boolean;
  scroll?: boolean;
  prefetch?: boolean | "auto" | null;
  shallow?: boolean;
  passHref?: boolean;
  legacyBehavior?: boolean;
  locale?: string | false;
  onNavigate?: (event: { preventDefault: () => void }) => void;
  ref?: React.Ref<HTMLAnchorElement>;
};

export function formatUrl(url: string | UrlObject): string {
  if (typeof url === "string") return url;
  let pathname = url.pathname ?? "";
  let search = url.search ?? "";
  if (!search && url.query) {
    if (typeof url.query === "string") search = url.query;
    else {
      const qs = new URLSearchParams();
      for (const [k, v] of Object.entries(url.query)) {
        if (v === undefined || v === null) continue;
        if (Array.isArray(v)) v.forEach((item) => qs.append(k, String(item)));
        else qs.append(k, String(v));
      }
      search = qs.toString();
    }
  }
  if (search && !search.startsWith("?")) search = `?${search}`;
  let hash = url.hash ?? "";
  if (hash && !hash.startsWith("#")) hash = `#${hash}`;
  if (!pathname && !search && !hash) pathname = "";
  return `${pathname}${search}${hash}`;
}

function isModifiedEvent(event: React.MouseEvent<HTMLAnchorElement>): boolean {
  const target = event.currentTarget.getAttribute("target");
  return (
    (target && target !== "_self") ||
    event.metaKey ||
    event.ctrlKey ||
    event.shiftKey ||
    event.altKey ||
    (event.nativeEvent && event.nativeEvent.which === 2) ||
    event.button !== 0
  ) as boolean;
}

export default function Link({
  href,
  as,
  replace = false,
  scroll = true,
  prefetch: _prefetch,
  shallow: _shallow,
  passHref: _passHref,
  legacyBehavior: _legacy,
  locale: _locale,
  onNavigate,
  onClick,
  ref,
  children,
  ...rest
}: LinkProps) {
  const resolved = formatUrl(as ?? href);

  const handleClick = (event: React.MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (event.defaultPrevented) return;
    if (isModifiedEvent(event) || event.currentTarget.hasAttribute("download")) return;

    const url = new URL(resolved, window.location.href);
    if (url.origin !== window.location.origin) return;
    if (!nav.isSpaPath(url.pathname)) return;

    event.preventDefault();
    if (onNavigate) {
      let prevented = false;
      onNavigate({ preventDefault: () => (prevented = true) });
      if (prevented) return;
    }
    React.startTransition(() => {
      nav.navigate(resolved, { replace, scroll });
    });
  };

  return (
    <a {...rest} ref={ref} href={resolved} onClick={handleClick}>
      {children}
    </a>
  );
}

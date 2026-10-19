// @upstream lib/config.ts
//
// The browser's view of upstream `config`: the five public fields, read from
// the root layout payload (`layouts.root.config`, contract/layouts.ts
// PublicConfig). Everything else in the upstream object (auth secret, database
// URL, storage, mail, quotas) is server-only and has no counterpart here, so code
// that reaches for it does not compile.
//
// Until the server sends `config`, the defaults upstream would use without any
// environment are assumed: APP_NAME "Modlight" and the page's own origin.
import { getRootData } from "../../runtime/root-data";
import type { PublicConfig } from "../../contract/layouts";

let warned = false;

function read(): PublicConfig {
  const fromServer = getRootData()?.config;
  if (fromServer) return fromServer;
  if (!warned && import.meta.env.DEV) {
    warned = true;
    console.warn("[modlight] layouts.root.config is missing from the page data; using defaults");
  }
  const origin = typeof window === "undefined" ? "http://localhost:8420" : window.location.origin;
  return { appName: "Modlight", embedBrand: "Modlight", appUrl: origin, appDomains: [origin], shortUrl: origin };
}

export const config = {
  get appName(): string {
    return read().appName;
  },
  get embedBrand(): string {
    return read().embedBrand;
  },
  get appUrl(): string {
    return read().appUrl;
  },
  get appDomains(): readonly string[] {
    return read().appDomains;
  },
  get shortUrl(): string {
    return read().shortUrl;
  },
};

export type AppConfig = typeof config;

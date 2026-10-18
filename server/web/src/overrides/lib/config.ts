// @upstream lib/config.ts
//
// The browser's view of upstream `config`: the five public fields, read from the
// root layout payload (contract/layouts.ts PublicConfig). Everything else in the
// upstream object (auth secret, database URL, storage, mail, quotas) is
// server-only and has no counterpart here, so code that reaches for it does not
// compile.
import { getRootData } from "../../runtime/root-data";
import type { PublicConfig } from "../../contract/layouts";

function read(): PublicConfig {
  const root = getRootData();
  if (!root) throw new Error("config read before the first page payload arrived");
  return root.config;
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

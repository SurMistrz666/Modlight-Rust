/**
 * The root layout's data for code that is not a component (the `config` shim,
 * the RPC runtime). Set by the router loader before each render it feeds.
 */
import type { RootLayoutData } from "../contract/layouts";

let current: RootLayoutData | null = null;

export function setRootData(data: RootLayoutData): void {
  current = data;
}

export function getRootData(): RootLayoutData | null {
  return current;
}

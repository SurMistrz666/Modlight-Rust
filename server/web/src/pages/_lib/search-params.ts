/**
 * The `searchParams` object a Next page receives, built from the current URL:
 * one string per key, an array when the key repeats, undefined when absent.
 * Pages whose query handling is presentation only (where to go after sign-in,
 * which reset token to submit) read it here instead of from the server.
 */
import { useMemo } from "react";
import { useSearchParams } from "next/navigation";

export type NextSearchParams = Record<string, string | string[] | undefined>;

export function useNextSearchParams(): NextSearchParams {
  const params = useSearchParams();
  return useMemo(() => {
    const out: NextSearchParams = {};
    for (const key of new Set(params.keys())) {
      const all = params.getAll(key);
      out[key] = all.length > 1 ? all : all[0];
    }
    return out;
  }, [params]);
}

/**
 * Disk-quota parsing / formatting.
 *
 * Quotas are stored on `user.max_quota` as a **bigint byte count**, with `-1`
 * meaning "unlimited" (a convention stored rows rely on).
 *
 * Admins think in "20G", not in 21474836480, so the UI takes human input and
 * this module is the single place that translates in both directions.
 */

/** Sentinel stored in `user.max_quota` for accounts with no limit. */
export const UNLIMITED_QUOTA = -1;

const UNITS: Record<string, number> = {
  b: 1,
  k: 1024,
  kb: 1024,
  kib: 1024,
  m: 1024 ** 2,
  mb: 1024 ** 2,
  mib: 1024 ** 2,
  g: 1024 ** 3,
  gb: 1024 ** 3,
  gib: 1024 ** 3,
  t: 1024 ** 4,
  tb: 1024 ** 4,
  tib: 1024 ** 4,
  p: 1024 ** 5,
  pb: 1024 ** 5,
  pib: 1024 ** 5,
};

const UNLIMITED_WORDS = new Set(["-1", "unlimited", "none", "no limit", "∞", "inf"]);

/**
 * Parse an admin-typed size into bytes.
 *
 * Accepts `20G`, `512 MB`, `1.5TiB`, `1,5G` (comma decimal — Polish keyboards),
 * a bare byte count, and the "unlimited" spellings. Returns `null` when the
 * input cannot be understood, so callers can reject instead of silently
 * writing a wrong limit.
 */
export function parseQuota(input: string): number | null {
  const raw = input.trim().toLowerCase();
  if (!raw || UNLIMITED_WORDS.has(raw)) return UNLIMITED_QUOTA;

  const m = /^(\d+(?:[.,]\d+)?)\s*([a-z]+)?$/.exec(raw);
  if (!m) return null;

  const amount = Number(m[1].replace(",", "."));
  if (!Number.isFinite(amount) || amount < 0) return null;

  const unit = m[2] ?? "b";
  const factor = UNITS[unit];
  if (!factor) return null;

  const bytes = Math.round(amount * factor);
  // Postgres bigint would take more, but anything past 2^53 stops being an
  // exact JS number and the quota arithmetic in finalize.ts is plain numbers.
  if (!Number.isSafeInteger(bytes)) return null;
  return bytes;
}

/** True when the stored value means "no limit". */
export function isUnlimited(bytes: number | null | undefined): boolean {
  return bytes == null || bytes < 0;
}

/**
 * Render a stored quota back into something an admin can edit — the inverse of
 * `parseQuota` for round numbers (`21474836480` → `20G`), so re-saving the form
 * without touching the field is a no-op.
 */
export function formatQuotaInput(bytes: number | null | undefined): string {
  if (isUnlimited(bytes)) return "";
  const n = Number(bytes);
  if (n === 0) return "0";
  const steps: Array<[number, string]> = [
    [1024 ** 4, "T"],
    [1024 ** 3, "G"],
    [1024 ** 2, "M"],
    [1024, "K"],
  ];
  for (const [factor, suffix] of steps) {
    if (n % factor === 0) return `${n / factor}${suffix}`;
  }
  return String(n);
}

/** Fraction of the quota consumed (0..1), or `null` when unlimited. */
export function quotaRatio(used: number, max: number | null | undefined): number | null {
  if (isUnlimited(max) || !max) return null;
  return Math.min(1, Math.max(0, used / max));
}

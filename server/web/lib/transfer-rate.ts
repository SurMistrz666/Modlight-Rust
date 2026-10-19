/**
 * Throughput meter for the upload dock.
 *
 * Written as a sliding window rather than exponential smoothing because of a
 * concrete failure the old meter had: a browser suspends `requestAnimationFrame`
 * and throttles `setInterval` to roughly once a minute in a hidden tab. An EMA
 * fed by those samples decays to zero while the tab is in the background and
 * then reports an enormous spike on the first tick after the user comes back,
 * because a minute of bytes gets divided by a one second interval. The transfer
 * itself was fine the whole time; only the number lied.
 *
 * A window over (bytes, timestamp) pairs cannot produce that spike: it always
 * divides a byte delta by the time that actually elapsed between the two
 * samples it came from. A sixty second gap yields an honest sixty second
 * average, which is exactly what a user who was away for a minute should see.
 */

export interface RateSample {
  /** Monotonic timestamp in milliseconds (`performance.now()`). */
  at: number;
  /** Cumulative bytes transferred at that moment. */
  bytes: number;
}

/** How much history the instantaneous reading averages over. */
export const RATE_WINDOW_MS = 12_000;

/**
 * Shortest span that may be turned into a rate. Below this, one late chunk
 * acknowledgement is enough to invent a number several times the truth.
 */
export const RATE_MIN_SPAN_MS = 750;

/** Rates under this are treated as "not moving" for the purposes of an ETA. */
export const ETA_MIN_RATE = 1024;

export class RateMeter {
  private samples: RateSample[] = [];
  /** First sample of the current run, kept for the since-the-start average. */
  private origin: RateSample | null = null;

  constructor(private readonly windowMs: number = RATE_WINDOW_MS) {}

  /** Forget everything, as when a new batch starts. */
  reset(): void {
    this.samples = [];
    this.origin = null;
  }

  /**
   * Record a cumulative byte count.
   *
   * A count that went DOWN means the queue changed under us: a file was
   * cancelled, or a retry rewound to an earlier offset. There is no meaningful
   * rate across that discontinuity, so the history restarts from the new value
   * instead of reporting a negative speed or a fake stall.
   */
  sample(bytes: number, at: number): void {
    const last = this.samples[this.samples.length - 1];
    if (last && (bytes < last.bytes || at < last.at)) {
      this.samples = [];
      this.origin = null;
    }

    const entry: RateSample = { at, bytes };
    this.origin ??= entry;
    this.samples.push(entry);
    this.trim(at);
  }

  /**
   * Recent throughput in bytes per second, or 0 while it cannot be told.
   *
   * This is the number the dock shows as the live speed.
   */
  get current(): number {
    return rateBetween(this.samples[0], this.samples[this.samples.length - 1]);
  }

  /**
   * Mean throughput since the batch started, in bytes per second.
   *
   * Shown next to the live reading because it cannot be fooled by a gap in
   * sampling: total bytes divided by total time is the same answer whether the
   * tab was visible the whole time or not. When the two disagree wildly, the
   * average is the one to believe.
   */
  get average(): number {
    return rateBetween(this.origin, this.samples[this.samples.length - 1]);
  }

  /**
   * Seconds left to move `remaining` bytes, or null when no honest estimate
   * exists. Prefers the live rate and falls back to the average, so a transfer
   * that has only just resumed still shows something rather than nothing.
   */
  eta(remaining: number): number | null {
    const rate = this.current || this.average;
    if (rate < ETA_MIN_RATE || remaining <= 0) return null;
    return Math.round(remaining / rate);
  }

  /**
   * Drop samples that fell out of the window, but never go below two: after a
   * long gap the only pair we have spans that gap, and averaging over it is the
   * correct answer, not a reason to report nothing.
   */
  private trim(now: number): void {
    const cutoff = now - this.windowMs;
    while (this.samples.length > 2 && this.samples[1].at < cutoff) this.samples.shift();
  }
}

function rateBetween(first: RateSample | null | undefined, last: RateSample | undefined): number {
  if (!first || !last) return 0;
  const span = last.at - first.at;
  if (span < RATE_MIN_SPAN_MS) return 0;
  const delta = last.bytes - first.bytes;
  if (delta <= 0) return 0;
  return (delta * 1000) / span;
}

// Browser-side measurement helpers. All functions execute in-page via
// Playwright's `page.evaluate` so that timing is measured by the browser's
// own performance clock (monotonic, not node's Date.now).

import type { Page } from "@playwright/test";

/**
 * Largest Contentful Paint, observed via PerformanceObserver. Returns the
 * timestamp of the final LCP entry reported before `timeoutMs` elapses, in ms
 * relative to navigationStart.
 *
 * The returned value is a single sample. Callers should wrap in a cold-load
 * loop (navigate → measure → discard browser context → repeat) and compute
 * P95 — see tests/nfr-201-render-latency.spec.ts once implemented.
 */
export async function lcpMs(page: Page, { timeoutMs = 10_000 } = {}): Promise<number> {
  return page.evaluate(
    ({ timeoutMs }) =>
      new Promise<number>((resolve, reject) => {
        let lastLcp = 0;
        const observer = new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            lastLcp = Math.max(lastLcp, entry.startTime);
          }
        });
        try {
          observer.observe({ type: "largest-contentful-paint", buffered: true });
        } catch (err) {
          reject(err);
          return;
        }
        setTimeout(() => {
          observer.disconnect();
          if (lastLcp === 0) reject(new Error("no LCP entries observed"));
          else resolve(lastLcp);
        }, timeoutMs);
      }),
    { timeoutMs },
  );
}

/**
 * Count requestAnimationFrame ticks over `durationMs` and return the
 * observed fps. The caller is responsible for triggering any scripted
 * interaction (e.g. a simulated drag) before/during the window.
 */
export async function rafFps(page: Page, durationMs: number): Promise<number> {
  return page.evaluate(
    ({ durationMs }) =>
      new Promise<number>((resolve) => {
        let frames = 0;
        const start = performance.now();
        const tick = () => {
          frames += 1;
          if (performance.now() - start < durationMs) {
            requestAnimationFrame(tick);
          } else {
            const elapsed = performance.now() - start;
            resolve((frames * 1000) / elapsed);
          }
        };
        requestAnimationFrame(tick);
      }),
    { durationMs },
  );
}

/** Percentile (0..100) over a numeric sample. Linear interpolation. */
export function percentile(samples: number[], p: number): number {
  if (samples.length === 0) return NaN;
  const sorted = [...samples].sort((a, b) => a - b);
  const rank = (p / 100) * (sorted.length - 1);
  const lo = Math.floor(rank);
  const hi = Math.ceil(rank);
  if (lo === hi) return sorted[lo]!;
  const frac = rank - lo;
  return sorted[lo]! * (1 - frac) + sorted[hi]! * frac;
}

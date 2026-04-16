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

export interface ScriptedDragFpsOptions {
  /** Total drag duration, ms. Matches the fps sampling window. */
  durationMs?: number;
  /** Radius of the circular drag path, in CSS pixels. */
  radiusPx?: number;
  /** CSS selector for the drag target. Defaults to the Sigma canvas. */
  selector?: string;
  /** Interval between scripted mouse-move events, ms. */
  stepMs?: number;
}

/**
 * Drive a continuous drag gesture on the Sigma canvas while measuring the
 * fps sustained over the same window. The drag traces a circular path so
 * motion stays bounded inside the canvas and the camera keeps moving —
 * Sigma schedules a render per frame while the pointer is moving, which is
 * exactly the work NFR-102 gates on.
 *
 * The fps counter is started in-page, then mouse events are dispatched via
 * CDP. Because the browser event loop remains free during CDP-driven input,
 * the rAF callbacks interleave with the pan renders as they would under a
 * real drag.
 */
export async function scriptedDragFps(
  page: Page,
  opts: ScriptedDragFpsOptions = {},
): Promise<number> {
  const durationMs = opts.durationMs ?? 2000;
  const radiusPx = opts.radiusPx ?? 80;
  const selector = opts.selector ?? "#zetl-graph canvas";
  const stepMs = opts.stepMs ?? 25;

  const locator = page.locator(selector).first();
  const box = await locator.boundingBox();
  if (!box) {
    throw new Error(`scriptedDragFps: no bounding box for selector ${selector}`);
  }
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;

  // Position the pointer before starting the fps counter so the sampled
  // window covers the drag, not the pre-drag move.
  await page.mouse.move(cx, cy);
  const fpsPromise = rafFps(page, durationMs);
  await page.mouse.down();

  const steps = Math.max(1, Math.floor(durationMs / stepMs));
  const dragStart = Date.now();
  for (let i = 0; i < steps && Date.now() - dragStart < durationMs; i += 1) {
    const theta = (i / steps) * Math.PI * 4; // two full rotations over the window
    const x = cx + radiusPx * Math.cos(theta);
    const y = cy + radiusPx * Math.sin(theta);
    // `steps: 2` splits each hop into two intermediate moves — enough to
    // generate pointer-move events without saturating the event loop.
    await page.mouse.move(x, y, { steps: 2 });
    await page.waitForTimeout(stepMs);
  }

  await page.mouse.up();
  return fpsPromise;
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

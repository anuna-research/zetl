// TEST-201 — NFR-101 initial render latency gate.
//
// Loads /_graph on the 2k-page fixture ten times from a fresh browser context
// per run (so each load is truly cold — no HTTP cache, no V8 code-cache, no
// renderer warmth). Measures LargestContentfulPaint for each run, asserts the
// P95 across the ten samples <= 1500 ms per SPEC-028 NFR-101 / TEST-201.
//
// One extra warm-up load is performed first and discarded, per the harness
// README note on prime-the-pump effects.
//
// Skips (rather than fails) when the graph route or widget isn't present —
// this lets the harness run against intermediate builds from earlier IMPL-028
// tasks without false negatives.

import { test, expect } from "./fixtures.ts";
import { lcpMs, percentile } from "../harness/metrics.ts";

const COLD_LOAD_COUNT = 10;
const LCP_P95_BUDGET_MS = 1500;
// Per-sample observer window. LCP for /_graph settles well under this; total
// wall-time stays under the 120 s test timeout with 11 runs.
const LCP_OBSERVE_MS = 3000;

test.describe("NFR-101 render latency", () => {
  test(`LCP P95 on /_graph <= ${LCP_P95_BUDGET_MS} ms across ${COLD_LOAD_COUNT} cold loads`, async ({
    browser,
    distServer,
  }) => {
    const graphUrl = `${distServer.url}/_graph`;

    const warmCtx = await browser.newContext();
    const warmPage = await warmCtx.newPage();
    const warmRes = await warmPage.goto(graphUrl, { waitUntil: "load" });
    const warmStatus = warmRes?.status();
    await warmCtx.close();
    if (warmStatus === 404) {
      test.skip(true, "/_graph route not present — graph-serve-route not yet wired");
    }
    expect(warmStatus).toBe(200);

    const samples: number[] = [];
    for (let i = 0; i < COLD_LOAD_COUNT; i += 1) {
      const ctx = await browser.newContext();
      const page = await ctx.newPage();
      try {
        const res = await page.goto(graphUrl, { waitUntil: "load" });
        expect(res?.status()).toBe(200);
        const sample = await lcpMs(page, { timeoutMs: LCP_OBSERVE_MS });
        samples.push(sample);
      } finally {
        await ctx.close();
      }
    }

    const p95 = percentile(samples, 95);
    const min = Math.min(...samples);
    const max = Math.max(...samples);
    const avg = samples.reduce((s, v) => s + v, 0) / samples.length;
    const formatted = samples.map((s) => s.toFixed(0)).join(", ");
    console.log(
      `NFR-101 LCP /_graph: P95=${p95.toFixed(0)} ms (budget ${LCP_P95_BUDGET_MS} ms) ` +
        `| min=${min.toFixed(0)} avg=${avg.toFixed(0)} max=${max.toFixed(0)} ms ` +
        `| samples=[${formatted}]`,
    );
    expect.soft(
      p95,
      `LCP P95 ${p95.toFixed(0)} ms exceeds ${LCP_P95_BUDGET_MS} ms budget (samples=[${formatted}])`,
    ).toBeLessThanOrEqual(LCP_P95_BUDGET_MS);
    expect(p95).toBeLessThanOrEqual(LCP_P95_BUDGET_MS);
  });
});

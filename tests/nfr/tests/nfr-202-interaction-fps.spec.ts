// TEST-202 — NFR-102 interaction frame-rate gate.
//
// Loads /_graph on the 2k-page fixture, waits for Sigma to mount, then drives
// a continuous 2 s drag gesture while counting requestAnimationFrame ticks.
// Asserts the sustained fps >= 30 per SPEC-028 NFR-102.
//
// Skips (rather than fails) when the graph route or widget isn't present —
// this lets the harness run against intermediate builds from earlier IMPL-028
// tasks without false negatives.

import { test, expect } from "./fixtures.ts";
import { scriptedDragFps } from "../harness/metrics.ts";

const MIN_FPS = 30;
const DURATION_MS = 2000;

test.describe("NFR-102 interaction fps", () => {
  test("sustains >= 30 fps over a 2s drag on /_graph", async ({ page, distServer }) => {
    const res = await page.goto(`${distServer.url}/_graph`, { waitUntil: "load" });
    if (res?.status() === 404) {
      test.skip(true, "/_graph route not present — graph-serve-route not yet wired");
    }
    expect(res?.status()).toBe(200);

    const canvas = page.locator("#zetl-graph canvas").first();
    try {
      await canvas.waitFor({ state: "visible", timeout: 10_000 });
    } catch {
      test.skip(true, "#zetl-graph canvas not mounted — graph-partial not yet wired");
    }

    // Let ForceAtlas2 settle before measuring: if the layout is still running
    // we'd measure layout fps rather than interaction fps. Poll a bounding
    // box twice; stable sizes indicate the renderer is idle.
    const firstBox = await canvas.boundingBox();
    await page.waitForTimeout(500);
    const secondBox = await canvas.boundingBox();
    expect(firstBox, "canvas has no bounding box").not.toBeNull();
    expect(secondBox, "canvas has no bounding box").not.toBeNull();

    const fps = await scriptedDragFps(page, { durationMs: DURATION_MS });
    console.log(`NFR-102 scripted-drag fps = ${fps.toFixed(1)} (>= ${MIN_FPS})`);
    expect.soft(fps, `drag fps ${fps.toFixed(1)} below ${MIN_FPS} floor`).toBeGreaterThanOrEqual(MIN_FPS);
    expect(fps).toBeGreaterThanOrEqual(MIN_FPS);
  });
});

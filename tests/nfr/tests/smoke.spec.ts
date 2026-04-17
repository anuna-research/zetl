// Smoke tests — confirm the harness itself is healthy: the server is up, the
// 2k-page dist/ serves the vault-wide graph route, and graph-index.json is
// retrievable. Real NFR assertions live alongside in tests/nfr-201*, etc.

import { test, expect } from "./fixtures.ts";

test.describe("@smoke harness", () => {
  test("serves index.html", async ({ page, distServer }) => {
    const res = await page.goto(`${distServer.url}/`);
    expect(res?.status()).toBe(200);
  });

  test("serves graph-index.json", async ({ request, distServer }) => {
    const res = await request.get(`${distServer.url}/graph-index.json`);
    // Accept 200; a pre-emit build will 404 — flag as a harness readiness
    // check rather than a hard failure so the file can land before
    // `task-graph-build-emit` integration.
    if (res.status() === 404) {
      test.skip(true, "graph-index.json not present — graph-build-emit not yet wired");
    }
    expect(res.status()).toBe(200);
    const json = (await res.json()) as { nodes?: unknown[]; edges?: unknown[] };
    expect(Array.isArray(json.nodes)).toBe(true);
    expect(Array.isArray(json.edges)).toBe(true);
  });

  test("serves /_graph route", async ({ page, distServer }) => {
    const res = await page.goto(`${distServer.url}/_graph`);
    if (res?.status() === 404) {
      test.skip(true, "/_graph route not present — graph-serve-route not yet wired");
    }
    expect(res?.status()).toBe(200);
  });
});

import { defineConfig, devices } from "@playwright/test";
import { DEFAULT_HARNESS_PORT } from "./harness/server.ts";

// Single-worker, chromium-only configuration. Determinism across machines is
// achieved by (1) Playwright's pinned Chromium, (2) one worker, (3) CPU-bound
// measurements done inside the test via the CDP session — see harness/metrics.ts.
export default defineConfig({
  testDir: "./tests",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["github"], ["list"]] : [["list"]],
  timeout: 120_000,
  expect: { timeout: 15_000 },

  use: {
    baseURL: process.env.ZETL_NFR_BASE_URL ?? `http://127.0.0.1:${DEFAULT_HARNESS_PORT}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "off",
    headless: true,
    launchOptions: {
      // Keep Chromium args identical across machines; disable GPU jitter.
      args: ["--disable-gpu-rasterization", "--disable-dev-shm-usage"],
    },
  },

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"], channel: undefined },
    },
  ],
});

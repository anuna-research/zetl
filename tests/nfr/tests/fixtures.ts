// Shared Playwright fixtures: `distServer` spins up a static server against
// the 2k-page dist/ for the duration of a worker. Tests extend `test` from
// here instead of `@playwright/test`.

import { test as base, type Page } from "@playwright/test";
import { existsSync } from "node:fs";

import { DEFAULT_HARNESS_PORT, serveDist, type StartedServer } from "../harness/server.ts";
import { distPath } from "../harness/paths.ts";

export interface HarnessWorkerFixtures {
  distServer: StartedServer;
  vaultSize: 2000 | 5000;
}

// eslint-disable-next-line @typescript-eslint/no-empty-object-type
export const test = base.extend<{}, HarnessWorkerFixtures>({
  vaultSize: [2000, { option: true, scope: "worker" }],

  distServer: [
    async ({ vaultSize }, use) => {
      const root = distPath(vaultSize);
      if (!existsSync(root)) {
        throw new Error(
          `dist/ missing for ${vaultSize}-page fixture: ${root}\n` +
            `run: npm run build:${vaultSize === 2000 ? "2k" : "5k"} (from tests/nfr/)`,
        );
      }
      const port = Number(process.env.ztl_NFR_PORT ?? DEFAULT_HARNESS_PORT);
      const server = await serveDist({ root, port });
      await use(server);
      await server.stop();
    },
    { scope: "worker" },
  ],
});

export const expect = test.expect;

/** Warm-up navigation; the first cold load is discarded when measuring LCP. */
export async function warmup(page: Page, url = "/"): Promise<void> {
  await page.goto(url, { waitUntil: "load" });
}

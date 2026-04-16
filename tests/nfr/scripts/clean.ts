// CLI: `node --experimental-strip-types scripts/clean.ts`
// Removes .fixtures/ and .dist/.

import { rm } from "node:fs/promises";
import { DIST_ROOT, FIXTURES_ROOT } from "../harness/paths.ts";

await rm(FIXTURES_ROOT, { recursive: true, force: true });
await rm(DIST_ROOT, { recursive: true, force: true });
console.log("removed fixtures and dist caches");

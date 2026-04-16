// CLI: `node --experimental-strip-types scripts/seed.ts <size> [--force]`
// Creates a synthetic vault at .fixtures/vault-<size>.

import { seedVault } from "../harness/fixtures.ts";
import { vaultPath } from "../harness/paths.ts";

const args = process.argv.slice(2);
const sizeArg = args.find((a) => /^\d+$/.test(a));
if (!sizeArg) {
  console.error("usage: seed.ts <size> [--force]");
  process.exit(2);
}
const force = args.includes("--force");
const size = Number(sizeArg);
const root = vaultPath(size);

const started = Date.now();
const result = await seedVault(root, size, { force });
console.log(
  `seeded ${result.pageCount} pages at ${result.root} in ${Date.now() - started}ms`,
);

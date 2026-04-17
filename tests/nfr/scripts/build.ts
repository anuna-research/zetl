// CLI: `node --experimental-strip-types scripts/build.ts <size>`
// Seeds if needed, then runs `zetl build` into .dist/vault-<size>.

import { seedVault } from "../harness/fixtures.ts";
import { buildDist } from "../harness/build.ts";
import { distPath, vaultPath } from "../harness/paths.ts";

const args = process.argv.slice(2);
const sizeArg = args.find((a) => /^\d+$/.test(a));
if (!sizeArg) {
  console.error("usage: build.ts <size>");
  process.exit(2);
}
const size = Number(sizeArg);
const vault = vaultPath(size);
const dist = distPath(size);

console.log(`seed → ${vault}`);
await seedVault(vault, size);
console.log(`build → ${dist}`);
await buildDist({ vaultRoot: vault, distDir: dist, zetlBin: process.env.ZETL_BIN });
console.log("done");

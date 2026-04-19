#!/usr/bin/env node
// Rename .js files produced by the CJS tsc build to .cjs so Node's dual-
// package resolver does not interpret them as ESM when the root
// package.json sets "type": "module".
import { promises as fs } from "node:fs";
import path from "node:path";

const root = new URL("../dist/cjs/", import.meta.url);

async function walk(dir) {
    const entries = await fs.readdir(dir, { withFileTypes: true });
    for (const entry of entries) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
            await walk(full);
        } else if (entry.name.endsWith(".js")) {
            const next = full.slice(0, -3) + ".cjs";
            const text = await fs.readFile(full, "utf8");
            const rewritten = text.replace(
                /require\(("|')(\.\.?\/[^"']+)\1\)/g,
                (_, q, p) => (p.endsWith(".js") ? `require(${q}${p.slice(0, -3)}.cjs${q})` : `require(${q}${p}${q})`),
            );
            await fs.writeFile(next, rewritten);
            await fs.rm(full);
        } else if (entry.name.endsWith(".js.map")) {
            const next = full.slice(0, -7) + ".cjs.map";
            await fs.rename(full, next);
        }
    }
}

await fs.writeFile(new URL("./package.json", root), JSON.stringify({ type: "commonjs" }, null, 2) + "\n");
await walk(root.pathname);

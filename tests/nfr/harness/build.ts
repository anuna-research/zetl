// Drives `cargo run --release -- build` against a synthetic vault and produces
// a dist/ folder the HTTP server can serve. Kept intentionally dumb: one
// spawn, inherit stdio, no orchestration — the tests depend on a real
// `zetl build` output.

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { rm } from "node:fs/promises";
import { join, resolve } from "node:path";

export interface BuildOptions {
  /** Vault root (must have been seeded). */
  vaultRoot: string;
  /** Destination dist/ directory (will be deleted if `clean` is true). */
  distDir: string;
  /** Path to the zetl binary; defaults to `cargo run --release --`. */
  zetlBin?: string;
  /** Remove `distDir` before building. */
  clean?: boolean;
  /** Repo root for cargo. Defaults to 3 levels above this file. */
  repoRoot?: string;
}

export async function buildDist(opts: BuildOptions): Promise<void> {
  const { vaultRoot, distDir, zetlBin, clean = true } = opts;
  const repoRoot = opts.repoRoot ?? resolve(import.meta.dirname, "..", "..", "..");

  if (clean && existsSync(distDir)) {
    await rm(distDir, { recursive: true, force: true });
  }

  const [cmd, ...prefixArgs] = zetlBin
    ? [zetlBin]
    : ["cargo", "run", "--release", "--quiet", "--manifest-path", join(repoRoot, "Cargo.toml"), "--"];

  const args = [...prefixArgs, "build", "--out", distDir];

  await new Promise<void>((resolvePromise, reject) => {
    const child = spawn(cmd!, args, {
      cwd: vaultRoot,
      stdio: "inherit",
      env: { ...process.env, RUST_LOG: process.env.RUST_LOG ?? "warn" },
    });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else reject(new Error(`zetl build exited with code ${code}`));
    });
  });
}

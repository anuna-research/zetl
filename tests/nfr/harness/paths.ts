// Canonical paths for fixture vaults and their built dist/ folders. Kept in
// one place so scripts and tests can't drift.

import { join, resolve } from "node:path";

export const HARNESS_ROOT = resolve(import.meta.dirname, "..");
export const FIXTURES_ROOT = join(HARNESS_ROOT, ".fixtures");
export const DIST_ROOT = join(HARNESS_ROOT, ".dist");

export function vaultPath(size: number): string {
  return join(FIXTURES_ROOT, `vault-${size}`);
}

export function distPath(size: number): string {
  return join(DIST_ROOT, `vault-${size}`);
}

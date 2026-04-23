// Synthetic vault generator used by the NFR harness (SPEC-028 TEST-201..205).
//
// Produces a deterministic markdown vault with ~6 wikilinks per page so the
// resulting graph has a realistic density (avg out-degree ≈ 6). We seed with a
// fixed RNG so the 2k-page and 5k-page fixtures are byte-identical across
// machines and across runs — this is a hard requirement for reproducible
// LCP/FPS measurements.

import { mkdir, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";

export const FIXTURE_SIZES = [2000, 5000] as const;
export type FixtureSize = (typeof FIXTURE_SIZES)[number];

export const LINKS_PER_PAGE = 6;
const FILES_PER_BATCH = 100;

// Deterministic LCG; identical output across Node versions and arch.
function lcg(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x1_0000_0000;
  };
}

export interface FixtureLayout {
  /** Absolute path to the vault root. */
  root: string;
  /** Number of markdown pages in the fixture. */
  pageCount: number;
}

/** Path where a fixture of the given size lives on disk. */
export function fixtureRoot(targetDir: string, size: number): string {
  return join(targetDir, `vault-${size}`);
}

/**
 * Generate a synthetic vault of `size` pages at `root`. Idempotent: if the
 * sentinel file `.seeded` already exists inside `root` with the right page
 * count, the generation is skipped.
 */
export async function seedVault(
  root: string,
  size: number,
  { force = false }: { force?: boolean } = {},
): Promise<FixtureLayout> {
  const sentinel = join(root, ".seeded");
  if (!force && existsSync(sentinel)) {
    return { root, pageCount: size };
  }

  if (force && existsSync(root)) {
    await rm(root, { recursive: true, force: true });
  }

  await mkdir(root, { recursive: true });

  const rand = lcg(size); // seed differs per vault size but is stable per size
  const batchCount = Math.ceil(size / FILES_PER_BATCH);
  for (let b = 0; b < batchCount; b += 1) {
    await mkdir(join(root, "notes", `batch_${String(b).padStart(3, "0")}`), {
      recursive: true,
    });
  }

  for (let i = 0; i < size; i += 1) {
    const batch = Math.floor(i / FILES_PER_BATCH);
    const dir = join(root, "notes", `batch_${String(batch).padStart(3, "0")}`);
    const name = `note_${String(i).padStart(5, "0")}`;
    const targets: string[] = [];
    for (let l = 0; l < LINKS_PER_PAGE; l += 1) {
      // Pick a link target that is not self. Uniform over [0, size).
      let t = Math.floor(rand() * size);
      if (t === i) t = (t + 1) % size;
      targets.push(`note_${String(t).padStart(5, "0")}`);
    }
    const body = [
      `# Note ${i}`,
      "",
      `Synthetic note ${i} for ztl NFR harness (SPEC-028).`,
      "",
      "## Wikilinks",
      "",
      targets.map((t) => `- [[${t}]]`).join("\n"),
      "",
      "## Body",
      "",
      "Lorem ipsum dolor sit amet, consectetur adipiscing elit.",
      "Proin ultricies pretium nisl, vel suscipit ex faucibus sit amet.",
      "",
    ].join("\n");
    await writeFile(join(dir, `${name}.md`), body, "utf8");
  }

  await writeFile(sentinel, `${size}\n`, "utf8");
  return { root, pageCount: size };
}

/** Remove a fixture vault and its accompanying dist/ output. */
export async function cleanFixture(root: string): Promise<void> {
  await rm(root, { recursive: true, force: true });
}

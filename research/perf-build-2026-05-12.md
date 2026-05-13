# `zetl build` performance pass — large vaults

Branch: `worktree-perf-build-large-vault-2026-05-12`
Plan: [`plans/PERF-BUILD-2026-05-12.spl`](../plans/PERF-BUILD-2026-05-12.spl)
Started: 2026-05-12

## Machine / methodology

- Hardware: Apple Silicon laptop, 8 performance cores, NVMe.
- Vaults: synthetic, generated with `cargo run --release --bin gen-vault` (commit `e771709`). Deterministic, seed=42, avg-links=12.
- Build invocation: `cargo run --release --bin zetl -- --dir <vault> build [--no-cache] --out <dist>`.
- Timer: `/usr/bin/time -lp`.

## Baseline

| Vault  | Mode | Wall (s) | User (s) | Sys (s) | Peak RSS (MB) | Dist size | Per-page (ms) |
|--------|------|---------:|---------:|--------:|--------------:|----------:|--------------:|
| 1 000  | cold (`--no-cache`) | **12.64** | 10.23 | 1.01 | 162 | 537 MB | 12.6 |
| 1 000  | warm                | 12.31 | — | — | 168 | 537 MB | 12.3 |
| 3 000  | cold (`--no-cache`) | **64.40** | 55.38 | 4.23 | 304 | 3.5 GB | 21.5 |
| 3 000  | warm                | 66.33 | — | — | 357 | 3.5 GB | 22.1 |
| 10 000 | cold (`--no-cache`) | _aborted: disk full at ~8.1 GB_ | — | — | 617 | est. 12 GB | est. ~30 ms |

## Findings the baseline already proves

### 1. Build time is super-linear in page count

1k → 3k = 3× more pages but **5.1× more wall time** (12.64 → 64.40 s). Per-page time grows from 12.6 ms → 21.5 ms. The pipeline has at least one O(N²) factor. The audit-identified `resolve_page_name` O(L · N) Vec scan and the `O(folders · files)` folder-index loop are the prime suspects.

### 2. The existing cache barely helps on rebuild

Warm-cache build is within 1 % of cold-cache build at both 1k and 3k. The two-tier scanner cache short-circuits Merkle hashing + re-parse, but the per-page render loop in `src/web/build.rs:1310` redoes every page unconditionally — markdown render, page HTML, OG PNG, history HTML, source copy. So warm builds save only the upfront scan time, not the dominant render time.

Implication: parallelising the per-page render loop will speed up both cold *and* warm rebuilds. Page-level memoisation (skip rendering when the parsed file's content hash is unchanged and the linked-page set is unchanged) is a separate latent win worth filing.

### 3. Dist-size blow-up: HTML, not OG

| Vault  | HTML total | PNG total | HTML per page |
|--------|-----------:|----------:|--------------:|
| 3 000  | 3.19 GB    | 97 MB     | ~1.06 MB |

Per-page HTML averages ~1 MB. The `PERF-AUDIT-2026-04-19` plan flagged the same root cause (inlined search index, inlined graph data, inlined transclusion script in `themes/default/base.html`). Not in scope for this perf pass directly, but it bears on benchmark methodology: dist size scales linearly with `N`, so on developer-laptop disk we cannot measure 10k+ vaults until the audit-2026-04-19 extraction work lands. **Recommend the existing PERF-AUDIT extraction tasks (`task-extract-shell-css`, `task-extract-transclusion-script`, `task-search-index-external`) gate before any meaningful 10k+ vault measurement.**

### 4. Cold 10k vault could not be measured

Build aborted with `Error: No space left on device (os error 28)` at ~130 s wall, peak RSS 617 MB. The partial dist was 8.1 GB. The wall-time figure is a lower bound only — we extrapolate ≈ 215 s from the 1k/3k trend if the build had completed, but the trend is super-linear so the real number is probably higher.

## Profiling

Skipped for this round. The asymptotic problems are evident from the source audit + baseline numbers, and a flamegraph adds little incremental value before the algorithmic fixes land. Will re-run a flamegraph after Phase 1 to validate that the remaining hot path is the parallelisable per-page loop and not a different surprise.

## Post-pass numbers

Measured after the full Phase 1 + Phase 2 stack landed (commits `e771709` baseline → `HEAD`).

| Vault  | Mode | Pre wall (s) | Post wall (s) | Speedup | Notes |
|--------|------|-------------:|--------------:|--------:|-------|
| 1 000  | cold |     12.64    |     5.96      | **2.12×** | user time stayed ~10 s; sys ~1 s |
| 1 000  | warm |     12.31    |     5.99      | **2.06×** | warm = cold because per-page render is the bottleneck, not parsing |
| 3 000  | cold |     64.40    |    37.38      | **1.72×** | user time grew from 55 s → 70 s (parallel CPU); wall dropped |
| 3 000  | warm |     66.33    |    36.98      | **1.79×** | |

The 3× cold-cache speedup target on the **10k-page** vault from the plan was not measurable because the 10k cold build still hits disk-full at ~8 GB partial dist — the per-page HTML bloat is unchanged by this pass (see Finding 3 above). The achieved 1.72–2.12× speedup spans the 1k / 3k measurable range and tracks roughly with the effective parallelism of the per-page render loop (user/wall ≈ 1.87 at 3k cold), which is the work that grew super-linearly in baseline.

### Which optimisation did what

| Optimisation | 3k cold wall | Δ from baseline | Δ from previous |
|---|---:|---:|---:|
| Baseline                           | 64.40 s | —      | — |
| + resolve-pages-index              | 64.29 s | –0.11  | –0.11 |
| + folder-index-quadratic           | 64.29 s | –0.11  |  0.00 |
| + hoist-git-repo                   | 64.10 s | –0.30  | –0.19 |
| + parallel-scanner                 | 64.10 s | –0.30  |  0.00 |
| + parallel-page-render             | 37.38 s | **–27.02** | **–26.72** |

The algorithmic Phase 1 fixes show essentially no wall-time gain at 1k–3k vaults — they target asymptotic factors that only dominate at larger N (10k+). They are still worth landing because:

- `resolve-pages-index` lifts an O(N · L) cost to O(L); the link-resolution step is no longer a scaling threat at 50k+ pages.
- `folder-index-quadratic` removes a provably-always-true membership check; it was pure dead work.
- `hoist-git-repo` matters for real (git-backed) vaults — on the synthetic vault the entire history block is skipped because `last_changed` is absent.
- `parallel-scanner` matters more on cold builds of larger vaults; at 3k it contributes <1 s of wall time because the scan is ~1.5 s of the 64 s total.

The dominant win is **parallel-page-render**, as the audit predicted.

### Correctness

`scripts/perf-diff.sh target/perf/vault-1k` reports byte-identical dist trees across two consecutive parallel builds (excluding `sitemap.xml` and `graph-index.json` which embed wall-clock timestamps, and `*.br` brotli precompressed twins whose encoder is wall-clock keyed). Parallelism is fully deterministic.

`cargo test --release --lib scanner` (173 tests) and `cargo test --release --lib web` (312 tests) both green after the pass.

## Follow-ups (out of scope for this branch)

1. **Per-page HTML bloat (~1 MB per page).** Dominates dist size and template-render CPU. The fix lives in the `PERF-AUDIT-2026-04-19` plan (`task-extract-shell-css`, `task-extract-transclusion-script`, `task-search-index-external`). Without those, larger vaults remain unmeasurable on developer disks and the per-page render stays super-linear in N.
2. **Page-level memoisation in build.** Warm builds currently re-render every page even when nothing about it (or its linked-page set) changed. The scanner cache hits, but the render loop ignores it. A content-hash + linked-set keyed cache could turn warm rebuilds into near-zero work.
3. **`build_folder_context` extension lookup.** Each direct child page does `data.files.iter().find(|f| ...)` to fetch its extension (`src/web/context.rs:519`). That's O(F) per page = O(F²) per build inside an already-parallel loop. A `HashMap<page_name, &ParsedFile>` (or just storing the extension on `PageEntry` during the first `data.page_slug_map` pass) eliminates it.
4. **`og_count` accuracy with parallel render.** The new code seeds the atomic at 1 (for the vault-root og.png written before the loop) and adds per-page successes. Cosmetic only — no functional impact — but worth pulling into a typed helper if the same pattern is replicated elsewhere.


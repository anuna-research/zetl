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

_To be filled in by `task-final-bench`._

| Vault  | Mode | Pre wall (s) | Post wall (s) | Speedup |
|--------|------|-------------:|--------------:|--------:|
| 1 000  | cold |     12.64    |  _tbd_        |  _tbd_  |
| 3 000  | cold |     64.40    |  _tbd_        |  _tbd_  |
| 1 000  | warm |     12.31    |  _tbd_        |  _tbd_  |
| 3 000  | warm |     66.33    |  _tbd_        |  _tbd_  |

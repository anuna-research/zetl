# NFR harness (SPEC-028)

Headless-browser harness for the Phase-7 NFR tests on the graph view.
Runs Playwright against a `zetl build` output served by a local static server.

| NFR     | Test ID  | Assertion                                                            |
| ------- | -------- | -------------------------------------------------------------------- |
| NFR-101 | TEST-201 | LCP ≤ 1500 ms (P95 over 10 cold loads) on `/_graph`, 2k-page vault   |
| NFR-102 | TEST-202 | ≥ 30 fps sustained over a 2 s scripted drag, 2k-page vault           |
| NFR-103 | TEST-203 | `vendor/sigma/*.min.js` ≤ 250 kB gzip _(Rust test, see `tests/nfr/`)_ |
| NFR-104 | TEST-204 | `graph-index.json` ≤ 1 MB for 2k vault; stderr warning for 5k        |
| NFR-105 | TEST-205 | Keyboard + SR `<details>` fallback reachable with JS disabled        |

TEST-203 / TEST-204 ship as Rust integration tests under
`tests/nfr_*.rs`; TEST-201 / TEST-202 / TEST-205 live here.

## Prerequisites

- Node ≥ 22 (uses `--experimental-strip-types`)
- Rust toolchain for `cargo run --release -- build` (skipped if `ZETL_BIN` points to a prebuilt binary)
- `npm install` once, then `npm run install-browsers` to fetch the pinned Chromium

## Workflow

```
npm install
npm run install-browsers

# Seed + build 2k-page fixture (cached under .fixtures/, .dist/)
npm run build:2k

# Run the Playwright suite
npm test

# Or just the smoke subset:
npm run test:smoke
```

`build:5k` is provided for NFR-104 (the size-warning check) and for
exploratory profiling — the NFR-101/102 suites target the 2k fixture.

## Determinism

- **One worker.** Playwright is configured `workers: 1` so CPU contention
  doesn't skew timing.
- **Pinned browser.** Tests run against Playwright's bundled Chromium, not
  the host Chrome channel.
- **Fixed RNG.** `harness/fixtures.ts` seeds the vault with a size-keyed LCG,
  so identical bytes land on disk across machines.
- **No browser cache.** The static server sends `Cache-Control: no-store`.
- **Cold-load samples.** `harness/metrics.ts#lcpMs` observes `largest-contentful-paint`
  with `buffered: true`; callers should discard the first navigation per
  browser context to avoid prime-the-pump effects.

## Layout

```
tests/nfr/
├── harness/
│   ├── build.ts        # spawn `zetl build` into .dist/
│   ├── fixtures.ts     # synthetic vault generator (2k / 5k)
│   ├── metrics.ts      # LCP, rAF-fps, percentile helpers
│   ├── paths.ts        # .fixtures/, .dist/ layout
│   └── server.ts       # zero-dep static file server
├── scripts/
│   ├── build.ts        # seed + build CLI
│   ├── clean.ts        # remove caches
│   └── seed.ts         # seed-only CLI
├── tests/
│   ├── fixtures.ts     # shared Playwright fixtures (distServer, vaultSize)
│   └── smoke.spec.ts   # harness health checks
├── playwright.config.ts
├── tsconfig.json
└── package.json
```

`.fixtures/` and `.dist/` are gitignored. Both are idempotent caches: re-running
`build:2k` is a no-op when a `.seeded` sentinel matches; pass `--force` to
`scripts/seed.ts` or `npm run clean` to rebuild from scratch.

## Overriding the zetl binary

By default `harness/build.ts` runs `cargo run --release --quiet --manifest-path <repo>/Cargo.toml -- build`. Set `ZETL_BIN=/path/to/zetl` to bypass `cargo` — useful in CI after a prior build step.

## Overriding the server port

Set `ZETL_NFR_PORT=NNNN` (and/or `ZETL_NFR_BASE_URL`) to run against a
long-lived server. Default is `127.0.0.1:4873`.

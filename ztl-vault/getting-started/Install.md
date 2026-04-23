# Install

ztl requires a Rust toolchain. Install one via [rustup](https://rustup.rs/) if you don't already have it.

```spl
(given single-binary)
(given fast-startup)
(given type-safe)
```

## Basic install (wikilinks only)

```bash
make install
```

This builds a lean binary with wikilink parsing, graph queries, search, TUI, and the web UI — everything except [[concepts/Defeasible Reasoning]].

## With reasoning support

```bash
cargo install --path . --features reason
```

The `reason` feature gate pulls in the [[concepts/Spindle Lisp]] runtime (spindle-core). Without it, running `ztl reason` prints a helpful error instead of failing silently — see [[Feature Gates]] for the design rationale.

## With vault history

```bash
cargo install --path . --features history
```

The `history` feature gate pulls in jj-lib for automatic temporal snapshots stored in `.ztl/jj/`. Enables the `--at` flag, `ztl history` commands, and history data in templates, hooks, and API endpoints. See [[History Command]].

## With both

```bash
cargo install --path . --features "reason,history"
```

## From source

```bash
git clone <repo-url>
cd ztl
make           # fmt + clippy + build + test
make release   # optimized binary
```

## Verify installation

```bash
ztl --version
ztl -d ./demo-vault stats
```

You should see page counts and link statistics from the included [[Demo Vault]].

See also: [[Quick Start]], [[CLI Reference]], [[Compatibility]]

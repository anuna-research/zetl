---
title: Feature Gates
---

# Feature Gates

The [[Reasoning Engine]] and all [[Reason Commands]] are behind a Cargo feature flag: `--features reason`. This keeps the default binary lean and the dependency tree small.

```spl
(given reason-feature-optional)
(given graceful-degradation)
```

## How it works

In `Cargo.toml`:

```toml
[features]
default = []
reason = ["dep:spindle-core", "dep:spindle-parser"]
```

Building without the flag produces a binary that handles [[Wikilinks]], [[Graph Queries]], [[Search]], [[Vault Diagnostics]], and the [[TUI]] — but not reasoning.

## Graceful degradation

When a user runs `ztl reason` on a binary built without the feature, they get a clear error message explaining how to rebuild with `--features reason`. This is better than a confusing "unknown command" error.

## Why not always include it?

- `spindle-core` and `spindle-parser` add compile time and binary size
- Not every user needs [[Defeasible Reasoning]]
- [[Spindle Lisp]] is a niche feature — progressive disclosure serves users better

## Install with reasoning

```bash
cargo install --path . --features reason
```

See also: [[Rust for CLI]], [[Reasoning Engine]], [[Reason Commands]]

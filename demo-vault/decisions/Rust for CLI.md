---
title: Rust for CLI
---

# Rust for CLI

ztl is written in Rust. This was a deliberate choice driven by the requirements of a CLI tool that handles large vaults and complex reasoning.

```spl
(given type-safe)
(given single-binary)
(given fast-startup)
```

## Why Rust

- **Type safety** — the [[Scanner]], [[Link Graph]], and [[Reasoning Engine]] involve complex data flows. Rust's type system catches entire classes of bugs at compile time.
- **Single binary** — `cargo install` produces one executable with no runtime dependencies. Users don't need Python, Node, or a JVM.
- **Fast startup** — CLI tools that take hundreds of milliseconds to start feel sluggish. Rust's zero-cost abstractions keep startup instant even on large vaults.
- **Memory safety** — no garbage collector pauses, no null pointer surprises. Important when processing thousands of files.

## Trade-offs

- Slower to iterate during development compared to a scripting language
- The [[Feature Gates]] mechanism adds build complexity
- Compile times are non-trivial

These trade-offs are acceptable for a tool meant to be installed once and used daily.

## Dependencies

Key crates: `clap` (CLI parsing), `petgraph` ([[Link Graph]]), `pulldown-cmark` ([[Scanner]]), `ratatui` ([[TUI]]), `spindle-core` / `spindle-parser` ([[Reasoning Engine]]).

See also: [[Feature Gates]], [[JSON by Default]], [[Local-first Design]]

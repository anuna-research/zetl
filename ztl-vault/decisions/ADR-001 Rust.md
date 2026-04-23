# ADR-001: Rust for CLI

**Status:** Accepted

## Context

ztl needs to be fast, safe, and easy to distribute. It processes thousands of Markdown files, builds an in-memory graph, and optionally runs a defeasible reasoning engine — all of which demand reliable memory management and low startup latency.

```spl
(given type-safe)
(given single-binary)
```

## Decision

ztl is written in Rust.

## Rationale

- **Type safety** — the [[architecture/Scanner]], [[Link Graph]], and [[Reasoning Engine]] involve complex data flows. Rust's type system catches entire classes of bugs at compile time.
- **Single binary** — `cargo install` produces one executable with no runtime dependencies. Users don't need Python, Node, or a JVM.
- **Fast startup** — CLI tools that take hundreds of milliseconds to start feel sluggish. Rust's zero-cost abstractions keep startup instant.
- **Memory safety** — no garbage collector pauses, no null pointer surprises.

## Key crates

- `clap` — CLI parsing
- `petgraph` — [[Link Graph]]
- `pulldown-cmark` — [[architecture/Scanner]]
- `ratatui` + `crossterm` — [[TUI]] and [[View Command]]
- `serde` + `serde_json` — JSON serialization
- `ignore` — `.gitignore`-aware file walking
- `blake3` — [[Merkle Tree]] hashing
- `spindle-core` + `spindle-parser` — [[Reasoning Engine]] (behind [[Feature Gates]])

## Trade-offs

- Slower development iteration compared to scripting languages
- Feature gate mechanism adds build complexity
- Non-trivial compile times

See also: [[Feature Gates]], [[JSON by Default]], [[decisions/Local-first Design]]

# Feature Gates

The [[Reasoning Engine]] and all [[Reason Commands]] are behind a Cargo feature flag: `--features reason`. The [[History Command]] and vault history system are behind `--features history`. The MCP server is behind `--features mcp`. This keeps the default binary lean and the dependency tree small.

```spl
(given reason-feature-optional)
(given history-feature-optional)
(given mcp-feature-optional)
(given graceful-degradation)
```

## How it works

In `Cargo.toml`:

```toml
[features]
default = []
reason = ["dep:spindle-core", "dep:spindle-parser"]
history = ["dep:jj-lib", ...]
mcp = ["dep:rmcp", "dep:jsonwebtoken"]
```

Building without flags produces a binary that handles [[concepts/Wikilinks]], graph queries, [[Search Command]], [[Check Command]], and the [[TUI]] — everything except reasoning, history, and MCP. Features can be combined: `--features "reason,history,mcp"`.

## Graceful degradation

When a user runs `zetl reason` on a binary built without the feature, they get a clear error message explaining how to rebuild with `--features reason`. This is better than a confusing "unknown command" error.

When built without `--features history`, template variables like `vault.history` and `page.history` are null, the `--at` flag is unavailable, and history API endpoints return empty results. All other commands work normally.

When built without `--features mcp`, `zetl mcp` prints an error directing the user to rebuild with `--features mcp`. The `delegate` command is similarly gated.

## Why not always include it?

- `spindle-core` and `spindle-parser` add compile time and binary size
- `jj-lib` adds significant dependencies (2000+ lines in Cargo.lock)
- `rmcp` pulls in HTTP/async machinery for MCP protocol support
- Not every user needs [[concepts/Defeasible Reasoning]], temporal history, or MCP
- These are niche features — progressive disclosure serves users better

## Progressive disclosure

```spl
; Feature gates + graceful degradation = progressive disclosure
(normally r-progressive-disclosure
  (and lean-default-binary good-developer-experience)
  progressive-disclosure)
```

See also: [[ADR-001 Rust]], [[Reasoning Engine]], [[Reason Commands]], [[Install]]

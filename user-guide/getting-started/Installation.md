---
title: Installation
tags: [install, setup, getting-started]
---

# Installation

zetl installs from source with `cargo`. There are no prebuilt binaries yet, so you need a working Rust toolchain. The binary, man page, and shell completions land in `~/.local/` by default.

## Requirements

- A Rust toolchain. Install from [rustup.rs](https://rustup.rs/) if you don't have one.
- `make` (optional — used by the convenience target).
- `~/.local/bin` on your `PATH`, and `~/.local/share/man` on your `MANPATH`, for the default `make install` layout.

## The one-liner

From a clone of the zetl repo:

```bash
make install
```

That gets you the core wikilink features — parsing, graph queries, search, diagnostics, the web UI, static-site export, and collaboration. No optional features.

## Feature flags

Everything beyond the wikilink core is gated behind cargo feature flags. Pick the ones you want at install time:

```bash
# Defeasible reasoning (SPL code blocks)
cargo install --path . --features reason

# Vault history (jj-backed snapshots, --at time travel)
cargo install --path . --features history

# Combine features
cargo install --path . --features "reason,history"

# Everything
cargo install --path . --features "reason,history,semantic,mcp"
```

| Flag | Unlocks |
|------|---------|
| *(none)* | Wikilink parsing, graph queries, search, `check`, `view`, `serve`, `build`, collaboration, hooks. |
| `reason` | [[Running Queries]], SPL extraction from Markdown, proof trees, what-if, conflict detection, SPL-based access control. |
| `history` | [[Time Travel]] with `--at "3 days ago"`, [[Watching for Changes]] via `zetl watch`, per-page timeline, `vault.history` in templates. |
| `semantic` | Semantic search via embedding model (alongside full-text). |
| `mcp` | [[MCP Server]] — expose the graph, search, and reasoning to AI agents. |

Collaboration (`--collab`) is always on — it doesn't need a feature flag. SPL-based access control, however, requires `--features reason`.

## Where things land

`make install` installs to `$PREFIX` (default `~/.local`):

```
~/.local/bin/zetl                          # the binary
~/.local/share/man/man1/zetl.1             # man page
~/.local/share/bash-completion/completions/zetl
~/.local/share/zsh/site-functions/_zetl
~/.local/share/fish/vendor_completions.d/zetl.fish
```

After install, `man zetl` works directly, provided `~/.local/share/man` is on your `MANPATH`.

## Shell completions

If you prefer to wire completions up by hand (or your shell isn't bash/zsh/fish), the binary prints them on demand:

```bash
zetl completions bash       > /etc/bash_completion.d/zetl
zetl completions zsh        > ~/.zfunc/_zetl
zetl completions fish       > ~/.config/fish/completions/zetl.fish
zetl completions powershell > $PROFILE/zetl.ps1
zetl completions elvish     > ~/.config/elvish/lib/zetl.elv
```

Same story for the man page:

```bash
zetl man > /usr/local/share/man/man1/zetl.1    # install
zetl man | man -l -                            # preview without installing
```

## Verifying the install

```bash
zetl --version
# zetl 0.5.0
```

If that prints, you're done. Head to [[Quick Start]] for your first vault query.

## No prebuilt binaries (yet)

zetl is at v0.5.0. Binary releases for Linux, macOS, and Windows are planned but not yet published. Until then, install from source.

## Related

- [[Quick Start]]
- [[What is zetl]]
- [[Your First Vault]]
- [[CLI Overview]]
- [[Configuration]]

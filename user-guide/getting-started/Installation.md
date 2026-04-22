---
title: Installation
tags: [install, setup, getting-started]
---

# Installation

zetl ships prebuilt binaries for Linux, macOS, and Windows. The installer script handles platform detection, download, and wiring up the man page and shell completions. If you prefer to build from source, that path is documented below too.

## Prebuilt binaries (recommended)

**macOS and Linux:**

```bash
curl -fsSL https://files.anuna.io/zetl/latest/install.sh | bash
```

The script detects your OS and architecture, downloads the right tarball, installs the binary to `~/.local/bin`, and generates the man page and shell completions.

**Windows:**

Download `zetl-windows-x86_64.zip` from [files.anuna.io/zetl/latest](https://files.anuna.io/zetl/latest/zetl-windows-x86_64.zip), extract `zetl.exe`, and place it somewhere on your `PATH`.

### Pinning to a specific version

```bash
VERSION=0.6.1 curl -fsSL https://files.anuna.io/zetl/latest/install.sh | bash
```

### Custom install location

```bash
INSTALL_DIR=/usr/local/bin curl -fsSL https://files.anuna.io/zetl/latest/install.sh | bash
```

### What gets installed

```
~/.local/bin/zetl                                    # the binary
~/.local/share/man/man1/zetl.1                       # man page (run 'man zetl')
~/.local/share/bash-completion/completions/zetl
~/.local/share/zsh/site-functions/_zetl
~/.local/share/fish/vendor_completions.d/zetl.fish
```

The prebuilt binaries include the `reason`, `history`, and `mcp` features. See [[#Feature flags]] below if you need a different set.

### PATH check

The installer warns if `~/.local/bin` isn't on your `PATH`. If it isn't:

```bash
export PATH="$HOME/.local/bin:$PATH"
# Add to ~/.bashrc, ~/.zshrc, or ~/.config/fish/config.fish to persist
```

---

## Install from source

Requires a Rust toolchain ([rustup.rs](https://rustup.rs/)).

```bash
git clone https://codeberg.org/anuna/zetl && cd zetl
make install
```

`make install` builds with core features only and installs to `$PREFIX` (default `~/.local`). For optional features:

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

## Feature flags

| Flag | Unlocks |
|------|---------|
| *(none)* | Wikilink parsing, graph queries, search, `check`, `view`, `serve`, `build`, collaboration, hooks. |
| `reason` | [[Running Queries]], SPL extraction from Markdown, proof trees, what-if, conflict detection, SPL-based access control. |
| `history` | [[Time Travel]] with `--at "3 days ago"`, [[Watching for Changes]] via `zetl watch`, per-page timeline, `vault.history` in templates. |
| `semantic` | Semantic search via embedding model (alongside full-text). |
| `mcp` | [[MCP Server]] — expose the graph, search, and reasoning to AI agents. |

Collaboration (`--collab`) is always on — no feature flag required. SPL-based access control requires `--features reason`.

## Shell completions

The binary generates completions on demand, in case you want to wire them up manually or use a shell not covered by the installer:

```bash
zetl completions bash       > /etc/bash_completion.d/zetl
zetl completions zsh        > ~/.zfunc/_zetl
zetl completions fish       > ~/.config/fish/completions/zetl.fish
zetl completions powershell > $PROFILE/zetl.ps1
zetl completions elvish     > ~/.config/elvish/lib/zetl.elv
```

Man page:

```bash
zetl man > /usr/local/share/man/man1/zetl.1    # install
zetl man | man -l -                            # preview without installing
```

## Verifying the install

```bash
zetl --version
# zetl 0.6.1
```

If that prints, you're done. Head to [[Quick Start]] for your first vault query.

## Related

- [[Quick Start]]
- [[What is zetl]]
- [[Your First Vault]]
- [[CLI Overview]]
- [[Configuration]]

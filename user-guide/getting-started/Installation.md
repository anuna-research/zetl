---
title: Installation
tags: [install, setup, getting-started]
---

# Installation

ztl ships prebuilt binaries for Linux, macOS, and Windows. The installer script handles platform detection, download, and wiring up the man page and shell completions. If you prefer to build from source, that path is documented below too.

## Prebuilt binaries (recommended)

**macOS and Linux:**

```bash
curl -fsSL https://files.anuna.io/ztl/latest/install.sh | bash
```

The script detects your OS and architecture, downloads the right tarball, installs the binary to `~/.local/bin`, and generates the man page and shell completions.

**Windows:**

Download `ztl-windows-x86_64.zip` from [files.anuna.io/ztl/latest](https://files.anuna.io/ztl/latest/ztl-windows-x86_64.zip), extract `ztl.exe`, and place it somewhere on your `PATH`.

### Pinning to a specific version

```bash
VERSION=0.6.1 curl -fsSL https://files.anuna.io/ztl/latest/install.sh | bash
```

### Custom install location

```bash
INSTALL_DIR=/usr/local/bin curl -fsSL https://files.anuna.io/ztl/latest/install.sh | bash
```

### What gets installed

```
~/.local/bin/ztl                                    # the binary
~/.local/share/man/man1/ztl.1                       # man page (run 'man ztl')
~/.local/share/bash-completion/completions/ztl
~/.local/share/zsh/site-functions/_ztl
~/.local/share/fish/vendor_completions.d/ztl.fish
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
git clone https://codeberg.org/anuna/ztl && cd ztl
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
| `history` | [[Time Travel]] with `--at "3 days ago"`, [[Watching for Changes]] via `ztl watch`, per-page timeline, `vault.history` in templates. |
| `semantic` | Semantic search via embedding model (alongside full-text). |
| `mcp` | [[MCP Server]] — expose the graph, search, and reasoning to AI agents. |

Collaboration (`--collab`) is always on — no feature flag required. SPL-based access control requires `--features reason`.

## Shell completions

The binary generates completions on demand, in case you want to wire them up manually or use a shell not covered by the installer:

```bash
ztl completions bash       > /etc/bash_completion.d/ztl
ztl completions zsh        > ~/.zfunc/_ztl
ztl completions fish       > ~/.config/fish/completions/ztl.fish
ztl completions powershell > $PROFILE/ztl.ps1
ztl completions elvish     > ~/.config/elvish/lib/ztl.elv
```

Man page:

```bash
ztl man > /usr/local/share/man/man1/ztl.1    # install
ztl man | man -l -                            # preview without installing
```

## Verifying the install

```bash
ztl --version
# ztl 0.6.1
```

If that prints, you're done. Head to [[Quick Start]] for your first vault query.

## Related

- [[Quick Start]]
- [[What is ztl]]
- [[Your First Vault]]
- [[CLI Overview]]
- [[Configuration]]

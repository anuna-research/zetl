---
title: Configuration
tags: [reference, config, environment]
---

# Configuration

Every knob ztl exposes: environment variables, ignore files, `theme.toml`, and the `.ztl/` state directory. Nothing here is required — ztl runs on a bare directory of Markdown with zero configuration. Read this page when you want to change defaults.

## Environment variables

ztl-specific variables are prefixed `ztl_`. The binary also honours `NO_COLOR` (a de-facto standard).

### Read by the CLI

| Variable | Equivalent flag | Purpose |
| --- | --- | --- |
| `ztl_DIR` | `-d, --dir` | Default vault root. |
| `ztl_FORMAT` | `-f, --format` | `json`, `table`, or `auto`. |
| `ztl_NO_CACHE` | `--no-cache` | Force full rescan. |
| `NO_COLOR` | `--no-color` | Disable ANSI colour. |
| `ztl_HOSTNAME` | `--hostname` (on `serve --collab`) | WebAuthn relying-party hostname. |
| `ztl_SERVER_KEY_SEED` | `--server-key-seed` (on `serve --collab`, `derive-ssh-key`) | BIP39 mnemonic for deterministic key derivation in ephemeral deployments. |
| `ztl_CAP_SITE_URL` | `--site-url` (on `cap invite`) | Canonical site URL for capability-URL rendering. |
| `ztl_CAP_PAIR_PHRASE` | `--phrase` (on `cap pair`) | Pinned SPAKE2 phrase; test-harness hook. |

Pass any of these once in your shell profile and skip the corresponding flag forever.

### Exported to hooks

Every hook ztl launches inherits these:

| Variable | Value |
| --- | --- |
| `ztl_HOOK` | Hook name (e.g. `post-build`, `callouts`). |
| `ztl_VAULT_ROOT` | Absolute path to the vault. |
| `ztl_THEME` | Active theme name. Empty string if no theme. |
| `ztl_VERSION` | ztl binary version (e.g. `0.5.0`). |
| `ztl_AST_SCHEMA_VERSION` | AST schema version (SemVer, e.g. `1.0.0`). Independent of `ztl_VERSION`. |
| `ztl_MODE` | `build` or `serve`. |
| `ztl_OUT_DIR` | Build output directory. Empty string under `serve`. |
| `ztl_VERBOSE` | `true` / `false`. |
| `ztl_HOOK_PATH` | Absolute path to the hook executable. Render-pipeline only. |
| `ztl_EXTENSION_ID` | Hook extension id (filename stem, minus `\d+-` ordering prefix). Render-pipeline only. |
| `ztl_HOOK_DEPTH` | Nesting depth; ztl increments on every child invocation to detect runaway recursion. |
| `ztl_AT` | Time expression passed via `--at`. Set only when time-travel is active. |

Pin your hook against `ztl_AST_SCHEMA_VERSION`, not `ztl_VERSION` — the AST schema changes far less often. See [[Render Pipeline Hooks]].

Persistent-mode hooks also receive both versions in their JSON init message, alongside a capability probe. See `tools/ztl-ast-reference.md` in the source tree for the wire protocol.

## Ignore files

ztl layers five sources of file-exclusion. Later layers override earlier ones except for level 1 (which is absolute).

| Level | Source | Scope |
| --- | --- | --- |
| 1 | Hardcoded | `.git/`, `.ztl/`, `node_modules/`. Always excluded. Not negatable. |
| 2 | Dotdir default | Any top-level directory whose name starts with `.` (`.claude/`, `.obsidian/`, `.cache/`, …). Disable with `--include-hidden`. |
| 3 | `.gitignore` | Standard git rules, walked from the vault root. |
| 4 | `.ztlignore` | Same syntax as `.gitignore`. ztl-specific overrides of the above. |
| 5 | `--exclude <PATTERN>` | Gitignore-syntax, repeatable on the CLI. Overrides everything except level 1. |

A `.ztlignore` negation (`!drafts/notes.md`) also overrides the level-2 dotdir default, letting you pull single files out of otherwise-excluded dot-directories without changing git's behaviour.

Dotfiles at the top level (e.g. `.editorconfig`) are *not* excluded by the level-2 rule — only directories.

## `theme.toml`

Themes live at `.ztl/themes/<name>/theme.toml` for installed themes, or bundled inside the binary at build time. See [[Customising the Look]].

### `[theme]`

| Key | Purpose |
| --- | --- |
| `name` | Theme name. Matches the directory name. |
| `version` | SemVer. |
| `description` | Optional one-line description. |
| `author`, `license`, `homepage`, `min_ztl_version` | Metadata. Optional. |

### `[theme.templates]`

```toml
[theme.templates]
overrides = ["base.html", "page.html", "folder.html"]
```

Tells ztl which template files this theme provides. Anything not listed falls back to the default theme.

### `[[theme.hooks]]`

Array-of-tables declaring hooks shipped under the theme's `hooks/` directory. Under `--safe-mode`, only declared hooks run; everything else (and every vault hook) is skipped.

```toml
[[theme.hooks]]
stage = "transform"
extension_id = "callouts"
ecosystem = "pandoc"     # optional; SPEC-033
summary = "Expand GFM admonitions into styled blockquotes"

[theme.hooks.contract]
preserves = ["Wikilink", "Embed"]
idempotent = true
```

`stage` is `pre-parse`, `transform`, or `post-render`. `extension_id` matches the hook executable's filename stem (minus any `\d+-` ordering prefix).

### `[spa]`

```toml
[spa]
enabled = true
```

Enables the persistent-shell SPA navigation module in the default theme. The element carrying `data-ztl-volatile` is swapped on same-origin `<a>` clicks instead of triggering a full reload. Sidebar and graph stay mounted.

### `[graph]`

```toml
[graph]
placement = "docked"     # "docked" | "tabs" | "stacked"
```

`graph_inline = true` at the top level opts the theme into inlined graph JSON on every render (the page's local graph is serialised into the template context).

### `[vendor.*]`

Pinned client-side assets — `sigma`, `graphology`, `graphology-layout-forceatlas2`. Each entry locks a version, source URL, bundled filename, SHA-256, and license. `ztl build` fails loudly if an asset hash doesn't match.

## The `.ztl/` directory

ztl never writes to your Markdown files. Everything it caches, derives, or persists lives under `.ztl/` at the vault root. Safe to delete — it'll rebuild.

| Path | Contents |
| --- | --- |
| `.ztl/index.json` | Cached link graph. `ztl index` writes it. |
| `.ztl/theory.json` | Compiled SPL theory (requires `reason`). |
| `.ztl/search/` | Tantivy BM25 index (one segment per `.fast`/`.idx`/`.store`/etc. shard). |
| `.ztl/search/vectors/` | Semantic-search embeddings (requires `semantic`). |
| `.ztl/history/` | Snapshot metadata JSON files (requires `history`). |
| `.ztl/jj/` | Jujutsu repo backing the silent snapshots (requires `history`). |
| `.ztl/themes/` | Installed (non-bundled) themes. |
| `.ztl/hooks/` | Vault-local hook executables and their sidecar manifests (`<name>.<ext>.toml`). |
| `.ztl/collab/server.key` | Multi-user signing key. Derived from `ztl_SERVER_KEY_SEED` when set. |
| `.ztl/crdt/` | Peritext CRDT documents, one per page under `--collab`. |
| `.ztl/users/` | Per-user records (passkey credentials, roles). |
| `.ztl/models/` | Cached ONNX models + tokenisers (semantic search, agents). |
| `.ztl/caps/` | Capability-URL cohorts, grants, and pairing nonces. |

Add `.ztl/` to your `.gitignore` unless you have a specific reason to commit the cache (e.g. reproducible search-index tests).

## Related

- [[CLI Overview]]
- [[Installation]]
- [[Customising the Look]]
- [[Frontmatter Fields]]
- [[Snapshots Under the Hood]]

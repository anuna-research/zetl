---
name: zetl
description: Use when working in a zetl vault — bidirectional wikilink knowledge base built on Markdown + jj. Triggers on `[[wikilink]]` syntax, `.zetl/` directories, SPL reasoning blocks, or user mentions of zetl, vaults, backlinks, or `zetl build/serve/check`.
license: AGPL-3.0-or-later
compatibility: Works with any text-capable model. Requires the `zetl` binary on PATH.
metadata:
  version: __ZETL_VERSION__
  homepage: https://codeberg.org/anuna/zetl
---

# zetl

`zetl` is a CLI for navigating, validating, and building a **vault** — a directory of Markdown pages connected by `[[wikilinks]]`. Vaults live under version control (jj or git) and are addressed by **slug** (kebab-case filename, no extension).

Reach for `zetl` when the user is:
- Asking about page connectivity (backlinks, dead links, orphans, shortest path).
- Editing `.md` files that contain `[[...]]` references.
- Running a local site preview (`zetl serve`) or a static build (`zetl build`).
- Authoring or debugging build hooks under `.zetl/themes/<name>/hooks/`.
- Working with SPL (Spindle) defeasible-logic blocks (` ```spl ` fenced).

## Reading the vault

| Question                          | Command                                       |
|-----------------------------------|-----------------------------------------------|
| What does this page link to?      | `zetl links "<page>"`                          |
| Who links here?                   | `zetl backlinks "<page>"`                      |
| Anything broken?                  | `zetl check` (`--dead-links`, `--orphans`, `--syntax`) |
| Shortest path between pages       | `zetl path "<from>" "<to>"`                    |
| Find pages by content             | `zetl search "<query>"`                        |
| Similar page names                | `zetl similar "<query>"`                       |
| List every page                   | `zetl list`                                    |
| Aggregate stats                   | `zetl stats`                                   |
| Dump full graph (JSON)            | `zetl export -f json`                          |

Add `-f json` to (almost) any query for machine-readable output — useful when chaining `zetl` calls with `jq`.

## Building & serving

```sh
zetl build --out-dir dist                  # static HTML build
zetl serve --port 3000                     # live preview, file-watching
zetl serve --collab --init-owner --owner-name <name>   # multi-user mode
```

The build pipeline runs **three stages** of hooks per page: `pre-parse → transform → post-render`. Hooks are persistent subprocesses speaking a line-delimited JSON protocol (one process per hook for the whole build). See `docs/hook-security.md` and `SPEC-032`.

## Authoring hooks

```sh
zetl hook new <stage>/<name> --lang python      # scaffold a persistent hook
zetl hook test <name>                            # diff against golden fixture
zetl hook dry-run <stage>/<name>                 # list pages a selector matches
zetl hook capabilities --json                    # probe version / schema compat
zetl hook watch <name>                           # re-spawn on source change
```

## Conventions to respect

- **Wikilinks are case-insensitive**, slug-resolved. `[[My Page]]` resolves to `my-page.md`.
- **Don't hand-edit `.zetl/`** — it's regenerated cache, drift state, and theme assets.
- **Frontmatter is YAML**, terminated by `---` on its own line. The build skips files with `draft: true` by default.
- **SPL blocks** are fenced ` ```spl ` and parsed by the `spindle-parser` crate; report-only unless `--features reason`.

## Where to look first

- `README.md` — feature overview and install steps.
- `docs/` — task-shaped guides (hook authoring, capability mode, mobile sync).
- `specs/` — formal requirements (`SPEC-NNN`); cited by failure diagnostics.
- `.zetl/themes/default/hooks/` — reference hooks to copy from.

## Output discipline

`zetl` honours `--format json` for scripting and `--format table` for humans. Prefer JSON when piping output into another tool or into your own reasoning — never grep the table form, it's not stable.

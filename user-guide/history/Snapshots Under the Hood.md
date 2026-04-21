---
title: Snapshots Under the Hood
tags: [history, jj, internals]
---

# Snapshots Under the Hood

> **Requires `--features history` at install.** See [[Installation]].

zetl's history feature is backed by [jj](https://jj-vcs.github.io/jj/) (Jujutsu), a VCS that treats the working copy as the commit. This page explains how snapshots actually work, where they live, and how they interact with the rest of zetl.

## Why jj, not git

git is excellent when you want deliberate, human-curated commits with messages and author intent. For a note-taking vault, that ceremony is friction:

- You don't want to `git add` your notes.
- You don't want to write a commit message every time you fix a typo.
- You don't want your `git log` to become unreadable.

jj gets out of the way. Every change to the working copy is automatically a new change. There's no staging area. There are no merge conflicts to resolve by hand during normal use. And critically for zetl, jj has a library API (`jj-lib`) that lets a host program drive snapshotting without spawning a subprocess.

The tradeoff: jj assumes one user per working copy. For multi-writer scenarios, zetl uses CRDTs ([[Co-editing]]), not jj.

## Where snapshots live

```
your-vault/
├── Page One.md
├── Page Two.md
└── .zetl/
    ├── index.db
    └── jj/            ← the snapshot store
        ├── repo/
        └── working_copy/
```

**`.zetl/jj/`**, not `.git/`. This matters:

- You can keep your own `.git/` alongside — for publishing, collaborating via pull request, whatever — and zetl's history won't touch it.
- Your `.gitignore` should include `.zetl/` (it's generated state). The official examples do.
- Backing up `.zetl/jj/` preserves your full history; deleting it removes history without touching your notes.

## When snapshots happen

| Trigger | What happens |
|---------|--------------|
| `zetl index` | A snapshot is taken before indexing writes the cache. Automatic when history is built in. |
| `zetl watch` | Debounced snapshot on each meaningful FS event. See [[Watching for Changes]]. |
| Manual `zetl history` | No snapshot — read-only. |
| `zetl build` / `zetl serve` | Snapshot via the index step they run; no additional work. |

There is no `zetl snapshot` command. Snapshots are always a side effect of something you were doing anyway.

## Reading snapshots

Three subcommands expose the store:

```bash
zetl history timeline          # recent snapshots, brief stats
zetl history log               # graph-level deltas between snapshots
zetl history page "Some Page"  # one page's evolution
```

And the global `--at <TIME-EXPR>` flag (see [[Time Travel]]) resolves any read-only command to a past snapshot.

## Graceful absence

If your binary was *not* built with `--features history`:

- `zetl --at ...` fails with a helpful error pointing at [[Installation]] — not a silent wrong answer.
- `zetl history <subcommand>` prints the same error and exits non-zero.
- `zetl watch` still runs for filesystem-event emission, but won't snapshot.
- `page.history` and `vault.history` template variables are `null` — themes that guard with `{% if page.history %}` render cleanly.
- The rendered page metadata strip doesn't appear. No empty `—` placeholders.

The degradation is deliberate. You can share a theme, a hook, or a static export between feature builds without breaking anything.

## `/_history` — the vault timeline

When running under `zetl serve` (or after `zetl build`), the URL `/_history` renders a vault-wide recent-changes page:

- **Snapshot count** — how many you've accumulated.
- **First / latest snapshot dates** — the window of time covered.
- **Link-count sparkline** — an inline-SVG trend of total links across time. You can see at a glance when your graph grew most.
- **Reverse-chronological list** — up to 50 entries of added / modified / removed pages.

The default theme's left rail adds a *Recent changes* link. Strip it by overriding `base.html` in your theme if you want a quieter look.

## What a snapshot actually contains

A jj change, pointing at the full tree of your vault at that instant:

- Every `.md` file, byte-exact.
- Every file in `.zetl/` *except* the jj store itself (avoiding recursion).
- The link graph itself is *not* stored — zetl re-derives it from the files when you query a historical state. This keeps the store small and future-compatible: upgrade zetl's link parser, and past snapshots benefit automatically.

## Interaction with `zetl index`

When history is enabled, `zetl index` implicitly snapshots before writing its cache. The ordering matters: the snapshot captures the pre-index state, so if indexing crashes, the working copy is still recoverable. You can see this in the timeline as an index-triggered entry.

If you run `zetl index --no-cache`, the snapshot still happens — `--no-cache` only affects the index-derivation step.

## Related

- [[Time Travel]]
- [[Watching for Changes]]
- [[Page History]]
- [[Installation]]
- [[Local-first]]

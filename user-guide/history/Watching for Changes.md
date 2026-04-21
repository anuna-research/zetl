---
title: Watching for Changes
tags: [history, watch, snapshots]
---

# Watching for Changes

> **Requires `--features history` at install.** See [[Installation]].

`zetl watch` runs continuously, listens for filesystem events in your vault, and takes a silent snapshot on every meaningful change. If you want *every* edit captured — not just the ones you thought to commit — this is the command.

## The idea

When you're writing, you don't want to stop and version your thoughts. You want to keep typing. But later, you'll want to know what you wrote at 3 AM last Tuesday, or recover the paragraph you deleted in a moment of over-zealous editing.

`zetl watch` solves this by hooking into the same filesystem events your editor emits on save, debouncing them, and committing a snapshot under `.zetl/jj/`. No dialog, no prompt, no commit message. Just a quiet paper trail.

## Running it

```bash
zetl -d ~/notes watch
```

Leave it in a terminal tab (or under `tmux`, `systemd`, `launchd`, whatever you use). It prints a one-line NDJSON event per change by default; pipe to `jq` or `--quiet` it if you don't care.

```bash
zetl watch --quiet
zetl watch | jq -r '.event'
```

### Useful flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--debounce <MS>` | `150` | Coalesce rapid saves (your editor's autosave) into one snapshot. Min 10, max 5000. |
| `--exec <CMD>` | — | Run a shell command for each event; event JSON arrives on stdin. |
| `--exclude <PATTERN>` | — | Gitignore-syntax pattern, repeatable. Combines with `.zetlignore`. |
| `--include-hidden` | off | Also watch `.obsidian/`, `.claude/`, etc. `.git/`, `.zetl/`, `node_modules/` stay excluded. |

The `--exec` flag makes `watch` a general-purpose trigger: rebuild a preview, push notifications to another tool, fire a webhook. See [[Lifecycle Hooks]] for structured pre/post-build hooks — `watch --exec` is the lightweight sibling.

## When to run watch

**Good fit:**

- Your laptop is your vault. You write notes all day and you want every save preserved.
- You're in a research flow and you'll want to time-travel later ([[Time Travel]]).
- You're drafting long-form work and a rolling safety net saves you from destructive edits.

**Not a good fit:**

- CI or batch jobs — snapshots are for human editing sessions, not automation.
- Shared vaults on servers where many writers modify files concurrently — use `--collab` mode instead (see [[Running a Team Server]]).
- Vaults on network shares with flaky FS events — snapshotting may miss changes or fire spuriously.

## What actually happens on a snapshot

Each coalesced event triggers a silent [jj](https://jj-vcs.github.io/jj/) snapshot inside `.zetl/jj/` — separate from any `.git/` directory you keep alongside. Your `git log` stays clean; your zetl history fills up. See [[Snapshots Under the Hood]] for the full picture.

No commit messages, no author prompts, no staging area. The whole thing is designed to stay out of your way while still giving `--at` something to query.

## Interaction with `zetl index`

If you already run `zetl index` as part of a build pipeline, indexing *also* writes a snapshot when history is enabled. `watch` and `index` snapshots coexist: you'll see them interleaved in `zetl history timeline`. Running both is fine — duplicate states are collapsed.

## Stopping watch

`Ctrl-C`. No cleanup needed; the jj store is consistent on every snapshot boundary, not at shutdown.

## Related

- [[Time Travel]]
- [[Snapshots Under the Hood]]
- [[Page History]]
- [[Lifecycle Hooks]]

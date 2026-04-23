---
title: Co-editing
tags: [collaboration, crdt, editing]
---

# Co-editing

When two people open the same page in a `--collab` vault, their edits merge without conflicts. Under the hood ztl uses a **Peritext CRDT** engine over WebSocket: every keystroke is a small op that both clients apply in the same causal order, so the document converges no matter who's typing where. Think Google Docs, but local-first and committed to git.

## What you see as a user

Open a page in the web UI. If someone else is editing it, you see their cursor as a coloured caret with their display name floating above it. Your edits and theirs interleave in real time. There's no "save" button — everything is continuously streamed to the server.

A small idle period (a few seconds of quiet, or navigating away) triggers an **auto-commit**: ztl writes the page to disk and runs `git commit` with the author set to the person whose session made the last edits. If Alice and Bob were both editing, the commit message notes both contributors.

This means your vault's git log is an authentic editing history. You can `git blame` a line and find out who wrote it. You can revert a page with `git revert`. The CRDT layer is an editing affordance; the source of truth is still Markdown on disk, tracked by git.

## Crash recovery

Between auto-commits, in-flight edits live in a **write-ahead log** at `.ztl/collab/wal/`. If the server dies mid-edit — process killed, power cut, container restarted — the WAL is replayed on next startup so nothing is lost. You shouldn't ever need to think about this, but it's why `ztl serve --collab` can take an extra second to come up after a hard crash.

If you deploy in an ephemeral container where `.ztl/collab/wal/` vanishes on redeploy, your CRDT merge state is gone. Auto-commit frequency is your backstop: most "in-flight" edits are short-lived, and anything older than a few seconds is in git already. For long-lived containers you can persist the WAL directory on a volume.

## External edits still work

The server polls `git` every 30 seconds (tunable via `--git-poll-interval` on `serve`) for commits that landed outside the UI — a `git pull` in a terminal, a push from another machine, a hook that rewrote a page. Those changes show up in the editor without anyone refreshing. If you're actively editing when an external change lands on the same page, ztl merges at the CRDT layer just as it would for a second live user.

## Authorship and git

Every auto-commit carries the editor's display name and the email registered with their account. If Alice edits `Zettelkasten Method.md`, the commit shows:

```
Author: Alice <alice@example.com>
    edit: Zettelkasten Method
```

If multiple editors contributed between commits, `git log --format="%b"` will list them. This is plain git — every tool that knows how to read a git repo reads this correctly, including third-party hosts like Forgejo, GitLab, or GitHub.

## Hooks that fire on save

`on-save` lifecycle hooks run every time a page is committed by the collab server. This is how you wire the editor to external workflows: ping a Matrix channel when a project plan changes, regenerate a static site on a CDN, re-run your defeasible-reasoning build, sync to another store.

The hook receives the vault path, the page slug, the committing author, and the old/new SHAs on stdin as JSON. See [[Lifecycle Hooks]] for the full set of events and the payload schema.

## Limits in v0.5

A few things the co-editing engine doesn't do yet:

- **Selective undo** — the undo stack is local per-client; you can't undo someone else's last edit without editing around it.
- **Rich media comments** — inline comments are plain text, tied to a block by its content hash. See [[Blocks]] for how ztl addresses paragraphs.
- **Offline with later sync** — if your browser disconnects, your local edits pause. They'll send on reconnect, but if the server has moved on, you may see a merge nudge. For true offline work, edit on disk and let the git-poll loop pick it up.

## Related

- [[Running a Team Server]]
- [[Lifecycle Hooks]]
- [[Blocks]]
- [[Access Control]]
- [[Time Travel]]

---
title: Troubleshooting
tags: [help, troubleshooting, problems]
---

# Troubleshooting

Things that go wrong, and what to do. Each entry is **Symptom → Diagnosis → Fix**. Use the table of contents to jump; most issues resolve in one or two steps.

## Contents

- [Dead-link warnings for pages I haven't written yet](#dead-link-warnings-for-pages-i-havent-written-yet)
- [`zetl reason` prints an error about not being compiled in](#zetl-reason-prints-an-error-about-not-being-compiled-in)
- [`zetl --at ...` fails or is not recognised](#zetl---at--fails-or-is-not-recognised)
- [`.obsidian/` or `.claude/` contents aren't indexed](#obsidian-or-claude-contents-arent-indexed)
- [Terminal viewer shows a single pane instead of two](#terminal-viewer-shows-a-single-pane-instead-of-two)
- [`zetl serve` won't start on port 3000](#zetl-serve-wont-start-on-port-3000)
- [My hooks aren't running](#my-hooks-arent-running)
- [Safe-mode blocks my hooks](#safe-mode-blocks-my-hooks)
- [`zetl index` feels slow on a large vault](#zetl-index-feels-slow-on-a-large-vault)
- [Collaborative edits conflict or look stale](#collaborative-edits-conflict-or-look-stale)
- [I lost my passkey](#i-lost-my-passkey)
- [Capability-URL reader errors](#capability-url-reader-errors)
- [Getting a useful bug report](#getting-a-useful-bug-report)

---

## Dead-link warnings for pages I haven't written yet

**Symptom.** `zetl check` reports `[[Future Note]]` as a dead link, even though you intended to write that page later.

**Diagnosis.** Not a bug. zetl flags every `[[wikilink]]` whose target doesn't yet exist as a dead link — that's the definition. Many people use this as a "stub list" of pages to write.

**Fix.**

- Create the target page (even as a one-liner) when you're ready.
- If you want to suppress a cluster of planned stubs, put them under a folder that's ignored: add the folder to `.zetlignore` with gitignore-style syntax.
- Scope a check to ignore them for a single run: `zetl check --fail-on error` only fails on syntax/SPL errors, not dead links.

See [[Finding Orphans and Dead Links]] for the full workflow.

---

## `zetl reason` prints an error about not being compiled in

**Symptom.** Running `zetl reason status` prints a message explaining that the reasoning feature isn't compiled in, and exits non-zero.

**Diagnosis.** zetl's reasoning layer is gated behind a Cargo feature flag to keep the core binary small. Your install didn't enable it.

**Fix.** Reinstall with the `reason` feature:

```bash
cargo install --path . --features reason
```

See [[Installation]] for the complete feature matrix. The same pattern applies to the `history`, `semantic`, and `mcp` flags.

---

## `zetl --at ...` fails or is not recognised

**Symptom.** `zetl --at "3 days ago" links "Some Page"` fails or acts as if `--at` were ignored.

**Diagnosis.** Time-travel queries require the `history` feature.

**Fix.**

```bash
cargo install --path . --features history
```

History uses jj-backed snapshots stored in `.zetl/jj/` — not your repo's `.git/`. Once installed, `--at` works on every read-only subcommand. See [[Time Travel]].

---

## `.obsidian/` or `.claude/` contents aren't indexed

**Symptom.** Files under `.obsidian/`, `.claude/`, `.vscode/`, or similar dot-directories don't appear in search, backlinks, or the graph.

**Diagnosis.** zetl skips directories whose name starts with `.` by default. This keeps editor metadata, agent working files, and VCS internals out of your knowledge graph.

**Fix.** Two options:

- Include all hidden directories for one run: `zetl index --include-hidden`.
- Re-include a specific dotdir permanently by negating it in `.zetlignore`:

```gitignore
# .zetlignore at vault root
!.obsidian/
```

Run `zetl --verbose index` to see which paths were skipped and why. See [[Configuration]] for the full precedence stack.

---

## Terminal viewer shows a single pane instead of two

**Symptom.** `zetl view "Some Page"` opens a single-column layout instead of the two-pane reader with context cards on the right.

**Diagnosis.** Your terminal is narrower than 60 columns. The two-pane viewer falls back to single-pane in narrow windows.

**Fix.** Resize the terminal to at least 60 columns (80+ is more comfortable). You can also adjust the main-pane width:

```bash
zetl view "Some Page" --main-width 60
```

See [[Terminal Viewer]] for keybindings and layout options.

---

## `zetl serve` won't start on port 3000

**Symptom.** `zetl serve` exits with an address-in-use error.

**Diagnosis.** Another process is bound to port 3000.

**Fix.** Pick a different port:

```bash
zetl serve --port 8080
```

If you want to know which process is holding 3000: `lsof -i :3000` on macOS/Linux. See [[Web Server]].

---

## My hooks aren't running

**Symptom.** You added a script to `.zetl/hooks/post-build`, but `zetl build` completes without running it.

**Diagnosis.** Almost always one of three things:

1. The file isn't marked executable.
2. The file is named incorrectly (no extension — just the lifecycle name).
3. It's not in the right directory (`.zetl/hooks/` for vault, `.zetl/themes/<theme>/hooks/` for theme).

**Fix.**

```bash
chmod +x .zetl/hooks/post-build
zetl hook list            # verify discovery
zetl hook run post-build  # manually invoke with real context
```

`zetl hook list` prints every hook zetl sees, with the source (vault or theme) and the lifecycle name. If your hook isn't listed, zetl isn't seeing it. See [[Lifecycle Hooks]].

---

## Safe-mode blocks my hooks

**Symptom.** Running with `--safe-mode`, hooks that normally work are skipped.

**Diagnosis.** Expected. `--safe-mode` skips every vault hook unconditionally and only runs theme hooks that are explicitly declared in the theme's `[[theme.hooks]]` manifest table. It's the intended posture for previewing untrusted vaults.

**Fix.** Two paths:

- If you trust the vault, drop `--safe-mode`.
- If you're writing a theme that should work under safe-mode, declare each hook in `theme.toml`:

```toml
[[theme.hooks]]
stage = "transform"
extension_id = "callouts"
summary = "Render :::note blocks as styled callouts"
```

See [[Render Pipeline Hooks]] and `docs/hook-security.md` in the source tree for the full security model.

---

## `zetl index` feels slow on a large vault

**Symptom.** Index times climb as the vault grows.

**Diagnosis.** The cache is two-tier: mtime first, then BLAKE3 content hash. Unchanged files should be skipped almost instantly on subsequent runs. If every index is slow, either the cache is being bypassed or every file really has changed.

**Fix.**

- Don't use `--no-cache` unless you genuinely need a full rescan (e.g. after upgrading zetl across a cache-format bump). It forces every file to re-parse.
- Run with `-v` or `-vv` to see per-file scan decisions:

```bash
zetl --verbose index
# → [zetl] scan: skipped .obsidian reason=dotdir
# → [zetl] scan: cached hit Some Page.md
```

- Check for a tool that's rewriting mtimes on every file (some sync clients do this). Those invalidate the first-tier cache on every run; the hash tier still saves reparsing, but the file list scan grows.

---

## Collaborative edits conflict or look stale

**Symptom.** In `--collab` mode, your edits don't appear for another user, or you see a version from an hour ago.

**Diagnosis.** CRDT merges are eventual. If the WebSocket dropped (network blip, laptop sleep), the two sides resync on reconnection. A stale render usually means a browser tab missed reconnection.

**Fix.** Refresh the page. The CRDT state on disk is authoritative. If a refresh doesn't help, confirm the server is still running and the WebSocket reconnected (browser devtools → Network → WS).

For longer outages, every save in collab mode auto-commits to git with author attribution, so nothing is lost — you can `git log` the vault to reconstruct. See [[Co-editing]].

---

## I lost my passkey

**Symptom.** You've cleared browser data, changed devices, or lost your hardware key, and you can't sign in to the collab server.

**Diagnosis.** Passkeys are device-bound. The 12-word BIP39 mnemonic printed when you ran `--init-owner` is the recovery path.

**Fix.** Open `/auth/recovery` on the running server, enter your display name and the 12-word phrase, and register a new passkey when prompted. See [[Passkeys and Accounts]].

If you never saved the mnemonic: another admin on the same vault can re-invite you. If you were the only admin, the vault files are still on disk — you can stand up a fresh collab server on the same directory and re-bootstrap as owner.

---

## Capability-URL reader errors

**Symptom.** You opened a capability-URL static site and the shim renders a red error banner — "signature did not verify", "could not decrypt", etc.

**Diagnosis.** These errors are thrown by the in-browser shim before any decryption happens. They're described in detail in `docs/reader-troubleshooting.md` in the source tree. The short version: most of them mean *contact your wiki operator*, not *fix your browser*.

**Fix.** The single rule: never override a signature check or paste secrets into a console on a wiki operator's advice. Close the tab and contact them. Quote the error kind (the slug after `err-`) and the URL prefix before `#`. See [[Capability URLs]].

---

## Getting a useful bug report

When filing an issue, include:

```bash
zetl --version           # binary version
zetl check --json        # current vault health as structured data
zetl stats               # counts and cache stats
```

Also note:

- Which feature flags were enabled at install (`--features reason,history,...`). This is the single most useful piece of context — many "missing command" reports are feature-flag mismatches.
- OS and architecture (`uname -a` on macOS/Linux).
- Whether the bug reproduces from a fresh clone or only in your existing vault.
- The exact command you ran and its full output (stdout + stderr).

File issues at **<https://codeberg.org/anuna/zetl>**. For questions that aren't bugs — "does zetl do X?" — check [[FAQ]] first.

## Related

- [[FAQ]]
- [[Installation]]
- [[CLI Overview]]
- [[Configuration]]
- [[Lifecycle Hooks]]

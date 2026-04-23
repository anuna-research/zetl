---
id: TEST-FTU-001
title: First-Time User Test — pre-release 0.1.0
status: resolved
reporter: Claude Opus 4.6 (1M context)
detection-method: synthetic user simulation
date: 2026-04-14
resolved-date: 2026-04-14
binary: target/release/ztl (features reason,history,mcp)
vault: /tmp/ztl-firsttime (fresh, 10 markdown pages, 18 links, 3 dead, 1 orphan)
---

## Resolution Summary (added 2026-04-14)

| ID      | Outcome   | Notes                                                                                         |
|---------|-----------|-----------------------------------------------------------------------------------------------|
| BUG-001 | **fixed** | `src/main.rs cmd_stats`: join cached theory against live block keys before counting          |
| BUG-002 | **fixed** | `src/web/routes.rs page_handler`: wrap non-existent page response in `StatusCode::NOT_FOUND` |
| BUG-003 | withdrawn | False positive — `HEAD~1` resolved correctly; original test coincidentally matched default  |
| BUG-004 | withdrawn | False positive — prompt already goes to stderr; original test used `2>&1`                    |
| BUG-005 | **fixed** | `src/cli.rs Build`: added `short = 'o'` and `alias = "out"` to the `out_dir` arg             |

Verification (post-fix, on the same `/tmp/ztl-firsttime` vault):

```
BUG-001: $ ztl --no-cache index && ztl --json stats | jq '.spl_blocks,.grounded_spl_blocks'
         0
         0                                                                   # invariant holds
BUG-002: $ curl -o /dev/null -w "%{http_code}\n" http://127.0.0.1:.../NonExistent
         404                                                                 # was 200
BUG-005: $ ztl build --out /tmp/zbo   → succeeds
         $ ztl build -o   /tmp/zbo2   → succeeds
```



# First-Time User Test — pre-release 0.1.0

## Scope

Exercised every top-level `ztl` subcommand on a freshly-created vault containing
ten notes, eighteen wikilinks (three intentionally dead), one orphan, and one
fenced SPL block (removed partway through). Commands invoked: `index`, `stats`,
`list`, `links`, `backlinks`, `check`, `similar`, `search`, `path`, `export`,
`blocks`, `build`, `serve`, `watch`, `diff`, `history (timeline|log)`, `reason
(status)`, `hook list`, `theme list`, `invite`, `delegate`, `agent-token`,
`completions (bash|zsh)`, `man`. Skipped: `view` (interactive TUI), `mcp`
(long-lived stdio transport), `derive-ssh-key`, `agent run` (requires hooks).

Overall: the tool works — the core graph commands (index, links, backlinks,
stats, check, search, similar, path, export, watch, build) are fast, produce
clean JSON, and behave sensibly. Findings below are the rough edges a new user
is likely to hit in the first hour. Five defects, ranked by severity.

---

## BUG-001: `stats` reports `grounded_spl_blocks > spl_blocks`

**Severity:** S2 (Major — violates a numeric invariant; undermines confidence in stats)
**Priority:** P1
**Status:** new
**Reported by:** synthetic first-time user (Claude Opus 4.6)

### Specification Reference

- Violates: internal invariant — grounded blocks are a subset of SPL blocks, so
  `grounded_spl_blocks ≤ spl_blocks` must always hold.
- Related: no published REQ/TEST identified; this appears to be a specification
  gap as well as an implementation defect.

### Environment

- macOS Darwin 25.3.0, arm64
- ztl 0.1.0, release build with `--features "reason,history,mcp"`
- Vault: `/tmp/ztl-firsttime` with no current SPL content (one SPL block was
  added in `Rules.md`, later removed; file was deleted from disk and a fresh
  `--no-cache` index was run).

### Steps to Reproduce

1. Create an empty vault and populate it with markdown notes (no fenced SPL).
2. Add a file `Rules.md` with a fenced ` ```spl ` block; run `ztl index`.
3. Delete `Rules.md` from disk.
4. Run `ztl --no-cache index` (forces full rescan).
5. Run `ztl --json stats`.

### Expected Behaviour

With no SPL content in any file, `spl_blocks == 0` and
`grounded_spl_blocks == 0`. More generally, `grounded_spl_blocks` must never
exceed `spl_blocks`.

### Actual Behaviour

```json
{"pages": 10, "spl_blocks": 0, "grounded_spl_blocks": 1, "explicitly_grounded_facts": 0}
```

Both the JSON and table formatters emit the same inconsistent numbers, so the
defect is in the stats computation, not the presentation layer.

### Evidence

```
$ ztl --no-cache index >/dev/null && ztl --json stats | jq '.spl_blocks,.grounded_spl_blocks'
0
1
```

### Root Cause

*To be investigated.* Two plausible causes: (a) the grounded-block counter is
derived from the history cache rather than the live index and was not
invalidated when `Rules.md` was removed, or (b) the grounded-block counter is
computing something other than "grounded SPL blocks" (e.g. counting frontmatter
or headings with a grounding annotation) regardless of SPL presence. Category
currently: **implementation-error** pending investigation.

### Proposed Resolution

1. Add a runtime assertion / debug check `assert!(grounded <= total)` to fail
   fast in debug builds.
2. Add a regression TEST that sets up a vault with zero SPL blocks and asserts
   both counters are zero, plus a second case where the counter is driven from
   the live index after deletion.
3. Rename `grounded_spl_blocks` in docs/stats output to unambiguously state
   whether "grounded" means "has a `ground:` annotation" or "anchored to the
   link graph" — current wording allows both readings.

---

## BUG-002: Web server returns HTTP 200 for non-existent pages

**Severity:** S3 (Moderate — SEO/crawler hazard, monitoring hazard; feature works)
**Priority:** P2
**Status:** new

### Specification Reference

- Violates: HTTP semantics (RFC 9110 §15.5.5 — "404 Not Found"). No ztl REQ
  found addressing the HTTP status code policy for unknown slugs —
  **specification gap**.

### Steps to Reproduce

1. Run `ztl serve --port 18231`.
2. `curl -o /dev/null -w "%{http_code}\n" http://127.0.0.1:18231/NonExistent`
3. `curl -o /dev/null -w "%{http_code}\n" http://127.0.0.1:18231/api/stats`

### Expected Behaviour

For any slug that does not correspond to an existing page in the vault, the
server responds `404 Not Found`. If the UX intent is to show a "create this
page" affordance (Obsidian / Roam style), the response should still be `404`
with the helper UI in the body — never `200`, because uptime probes and search
engines use status codes as ground truth.

### Actual Behaviour

Both URLs return **`HTTP 200`** with a generic page stub. `/api/stats` renders
an HTML page titled `stats — ztl` — there is no JSON API at that path, but the
route is silently interpreted as "show me the page named `api/stats`".

### Evidence

```
=== /Missing Page ===
HTTP 200
=== /api/stats ===
<!DOCTYPE html>
<html lang="en" data-theme="default">
  <title>stats — ztl</title>
  <meta name="description" content="stats — a page in the ztl-firsttime knowledge vault.">
```

### Root Cause

*To be investigated.* Likely a catch-all route that renders a stub page for any
slug without consulting the index. Category: **design-error** — the policy has
not been specified.

### Proposed Resolution

1. Write a REQ that specifies the status code contract for unknown pages
   (recommended: `404` with a create-page affordance in the body).
2. If an `/api/*` namespace is reserved for future JSON endpoints, explicitly
   `404` unknown paths under it now to avoid accidental dependency by
   third-party tooling.
3. Add TEST-### asserting status codes for (known page, unknown page, nested
   unknown path, `/api/*`).

---

## BUG-003: `ztl diff --from <ref>` silently ignored in a non-git vault

**Severity:** S3 (Moderate — silent wrong answer; risk of misinforming the user)
**Priority:** P2
**Status:** new

### Specification Reference

- Violates: the `--help` contract for `ztl diff`. `--from` is described as
  "Git ref or jj change-ID / time expression to use as the diff baseline". In a
  vault with neither git nor the supplied jj ref, passing the flag must either
  succeed with that baseline or fail — not silently substitute.

### Steps to Reproduce

1. Create a vault in `/tmp/ztl-firsttime` with **no** `.git` directory.
2. Run `ztl index` twice with changes in between so a jj snapshot exists.
3. Run `ztl diff` and `ztl diff --from HEAD~1`.

### Expected Behaviour

`ztl diff --from HEAD~1` should either:
- resolve `HEAD~1` against an available VCS (error if no git repo is present), or
- print a clear error like `"no git repository found; --from HEAD~1 cannot be resolved"`.

It must not silently behave as though `--from` were omitted.

### Actual Behaviour

`ztl diff` and `ztl diff --from HEAD~1` return **byte-identical output**. The
`from.ref` field contains a jj change-ID (`lkmlknynstyv`) rather than
`HEAD~1`. There is no diagnostic indicating the supplied ref was not resolved.

### Root Cause

*To be investigated.* Category likely: **implementation-error** — silent
fallback to default baseline when the supplied ref fails to resolve.

### Proposed Resolution

1. When `--from` is provided, attempt resolution and fail with a clear error
   message if it cannot be resolved. Do not fall back silently.
2. Add TEST-### covering: valid git ref, invalid git ref, vault without git,
   jj change-ID prefix, malformed time expression.

---

## BUG-004: `index` leaks interactive prompt into stdout on non-TTY / declined flows

**Severity:** S3 (Moderate — machine-readable output contaminated on first run)
**Priority:** P2
**Status:** new

### Specification Reference

- Violates: implicit contract that JSON output on stdout is parseable. No
  explicit REQ found — **specification gap** on prompt/output separation.

### Steps to Reproduce

1. `rm -rf .ztl` in an empty directory so the semantic model cache is absent.
2. Run `ztl index` with stdin attached to a TTY (or pipe `N\n`).
3. Observe the prompt text and the JSON result emitted on the same stream.

### Expected Behaviour

Either:
- Interactive prompts go to **stderr** with a trailing newline; stdout remains
  pure JSON; or
- When stdin is not a TTY, the prompt is suppressed entirely and the tool
  proceeds with the documented default (currently `N`, which is fine).

### Actual Behaviour

The prompt `"Download now? [y/N] "` is written to stdout without a trailing
newline, immediately followed by the JSON result:

```
Download now? [y/N] {
  "files_scanned": 0,
  ...
}
```

Piping `ztl index | jq .` on a first run produces a JSON parse error. A user
who automates ztl in CI will hit this on any fresh workspace.

### Proposed Resolution

1. Move all interactive / informational prompts to **stderr**.
2. Add an `isatty(stdin)` check; suppress the prompt when non-interactive and
   default to "do not download".
3. Honour `--no-input` (already present on many subcommands) here too — make
   it a global flag that turns off all prompts.

---

## BUG-005: `ztl build` uses `--out-dir`; most tools use `--out` / `-o`

**Severity:** S4 (Minor — user friction; tool helpfully suggests correct name)
**Priority:** P3
**Status:** new

### Specification Reference

- Violates: convention, not specification. `ztl serve` accepts `--port`
  without an alias, and other CLIs in the space (hugo, zola, mdbook) use
  `-o` / `--output` / `--out`.

### Steps to Reproduce

```
$ ztl build --out /tmp/site
error: unexpected argument '--out' found
  tip: a similar argument exists: '--out-dir'
```

### Proposed Resolution

Add `--out` and `-o` as aliases to `--out-dir`. The `clap` `#[arg(long,
alias = "out", short = 'o')]` attribute suffices — no behaviour change, just
ergonomics. Confirmed low-risk: there is no other `-o` flag on `ztl build`.

---

## Additional Observations (not bugs — Phase-2 spec work)

These are ambiguities or friction points surfaced by the simulation. They are
not defects, but candidate specification amendments.

### Observation A: `ztl check` exit code contract is undocumented

`ztl check` exits **non-zero** when any dead link or orphan is found. This is
the right default for CI use, but:

- It is not mentioned in `ztl check --help`.
- A user running `ztl check && ztl search foo` in a shell will find the
  second command silently skipped on a healthy-but-orphaned vault.

**Proposed:** add a line to `--help`: `"Exits 0 if no findings; exits 1 if any
dead link / orphan / syntax error / SPL error is present. Use --ignore-orphans
to treat orphans as informational."` Introduce the `--ignore-*` flags if they
do not already exist.

### Observation B: SPL parser error messages paste the unparsed input blob into a single JSON string

With a malformed `spl` block, the diagnostic is:

```
"SPL parse error at line 1: Unparsed input remaining: (alice, project_alpha).\nfact owner(bob, project_beta).\n\nrule leads(X, Y) :- owner(X, Y).\n  | fact owner(alice, project_alpha)."
```

The `line` field correctly points at the bad line, but the message embeds
multiline content with escaped `\n` that is difficult to read in a terminal.
Prefer: `"unexpected token '(' after 'fact'; expected an identifier"` with
`line`/`column` doing the pointing.

### Observation C: `ztl history` uses `timeline`; other multi-verb commands use `list`

`ztl theme list`, `ztl hook list`, `ztl list` all use `list` as the
inventory verb. `ztl history timeline` breaks the pattern. Either add
`history list` as an alias or standardise on the more-expressive `timeline`
everywhere.

### Observation D: `ztl invite --as alice` fails with `"user 'alice' not found in this vault"` on a vault that has never had users configured

The error is correct but the new user has no signpost to "how do I add a user
to this vault?". Propose: include a hint in the error (`"Run 'ztl user add
<name>' first"` or equivalent) pointing at whatever the canonical add-user
flow is. If there is no CLI-driven add-user flow yet, document the
alternative in the error (file/config edit, serve-side self-registration).

### Observation E: First-run semantic model download is surprising

On an empty vault, the first `ztl index` prompts for a `~90 MB` model
download. The prompt text is clear, but this happens before any content has
been indexed — a user trying ztl for the first time with `mkdir demo && cd
demo && ztl index` may reasonably ask "why is a wikilink tool downloading
100 MB?" Consider deferring the prompt until the user first runs a command
that actually needs semantic search, or making it a one-line `ztl setup
semantic` bootstrap step.

---

## Summary Table

| ID      | Severity | Priority | Title                                                   |
|---------|----------|----------|---------------------------------------------------------|
| BUG-001 | S2       | P1       | `grounded_spl_blocks > spl_blocks` in stats output      |
| BUG-002 | S3       | P2       | Web server returns 200 for non-existent pages           |
| BUG-003 | S3       | P2       | `diff --from <ref>` silently ignored in non-git vault   |
| BUG-004 | S3       | P2       | Interactive prompt written to stdout, mingles with JSON |
| BUG-005 | S4       | P3       | `ztl build` lacks `--out` / `-o` alias                 |

## Release Recommendation

BUG-001 is release-blocking in my judgement: a statistics output that
violates a basic numeric invariant erodes trust in the whole tool.

BUG-004 is borderline release-blocking for anyone piping `ztl index` in CI
on a fresh workspace; a one-line `eprintln!` change should ship before 0.1.0
leaves alpha channel.

BUG-002, BUG-003, BUG-005 and all Observations are acceptable as known
issues in a 0.1.0 release provided they are tracked in the CHANGELOG under
"Known Issues" with links back to this document.

## AI Detection Context

- **Detecting model:** Claude Opus 4.6 (1M context)
- **Detection method:** synthetic user simulation — first-time user archetype
- **Confidence:** high (all bugs directly observed and reproduced)
- **Session context:** Interactive Claude Code session, 2026-04-14. Test
  vault preserved at `/tmp/ztl-firsttime` for reproduction.

## Adversarial Review Recommendation

Per USDD Constitutional Principle 12, this document was produced by the same
agent that would implement fixes. Before any of these BUG-### entries is
resolved, a second model (ideally a different family) should re-run the
simulation from a clean context with only the spec and this document as input,
to confirm that the findings reproduce and that none is a false positive.

---
title: "SPEC-016: ztl hooks — Git-Style Lifecycle Hooks for Vault Operations"
version: 0.1.0
status: implemented
audience: agent, human
date: 2026-03-02
---

# SPEC-016: ztl hooks — Git-Style Lifecycle Hooks for Vault Operations

## Information Table

| Field          | Value                                                          |
| -------------- | -------------------------------------------------------------- |
| Document ID    | SPEC-016                                                       |
| Title          | ztl hooks — Git-Style Lifecycle Hooks for Vault Operations    |
| Version        | 0.1.0                                                          |
| Status         | Implemented                                                    |
| Author         | Agent (USDD Protocol v1.3.0)                                   |
| Date           | 2026-03-02                                                     |
| Audience       | Agent, Human                                                   |
| Trace          | USDD Agent Protocol v1.3.0                                     |
| Parent         | SPEC-001: ztl — Bi-directional Link Graph CLI                 |
| Related        | SPEC-012: Named Themes, SPEC-014: Theme Distribution, SPEC-015: Fountain Theme |

---

## 1. Overview

ztl is a monolithic binary. Every feature — search, build, check, serve — is compiled in. This works well for the core graph engine, but users inevitably need vault-specific behaviour that doesn't belong in the binary: assembling a screenplay from chained scenes, generating an RSS feed, exporting citations, linting prose style, syncing to a CMS, notifying a Slack channel on save.

This specification adds **lifecycle hooks** — executable scripts that ztl invokes at defined points during its operations. The design follows git's hook model: named scripts in a well-known directory, receiving structured context, controlling flow via exit codes.

### 1.1 Core Insight

ztl already computes rich structured data at every lifecycle point — the link graph, parsed frontmatter, Merkle trees, diagnostics, search index. Today that data is only consumable via CLI JSON output (after the fact) or templates (at render time). Hooks expose the same data **during the operation**, enabling scripts to participate in the pipeline rather than merely react to its output.

### 1.2 Design Philosophy

1. **Git's model, not npm's** — hooks are local, executable files in `.ztl/hooks/`. No package manager, no dependency resolution, no registry. A hook is a script you can read, edit, and understand.
2. **Data in, artifacts out** — hooks receive structured JSON context on stdin and produce side effects (files, stdout, stderr). They do not modify ztl's internal state.
3. **Any language** — hooks are executables. Bash, Python, Deno, a compiled binary — anything with a shebang line (or `.exe` on Windows).
4. **Themes bundle hooks** — a theme's `hooks/` directory contains hooks that activate when the theme is selected. This is how domain-specific build logic (e.g., screenplay assembly) ships with the theme rather than being hardcoded in ztl.
5. **Fail-safe defaults** — missing hooks are silently skipped. Failing hooks emit warnings but do not abort the parent operation by default (`pre-` hooks are the exception — they can abort).

### 1.3 Scope

**In scope:**

- `.ztl/hooks/` directory for vault-level hooks
- Theme-bundled hooks in `.ztl/themes/<name>/hooks/` (or `themes/<name>/hooks/` for bundled themes)
- Lifecycle hook points: `pre-build`, `post-build`, `post-index`, `post-check`, `on-save`, `pre-serve`
- Structured JSON context passed on stdin to each hook
- Environment variables for common fields (`ztl_VAULT_ROOT`, `ztl_OUT_DIR`, `ztl_THEME`, etc.)
- Exit code semantics: 0 = success, non-zero = failure (pre-hooks abort; post-hooks warn)
- Hook discovery and execution order (vault hooks run after theme hooks)
- `ztl hook list` command to show active hooks
- `ztl hook run <name>` command to manually invoke a hook with current vault context

**Out of scope:**

- Dynamic plugin loading (shared libraries, WASM modules)
- Hook-to-hook communication or dependency ordering between hooks
- Hook configuration files (hooks are self-contained executables)
- Hook package manager or registry
- Hooks that modify ztl's in-memory state (hooks are side-effect-only)
- Windows `.bat`/`.cmd` support (deferred — Unix executable model first)
- Parallel hook execution (hooks run sequentially per lifecycle point)

---

## 2. User Profiles

### 2.1 Screenwriter (Theme Hook Consumer)

```
Role: Writer using the fountain theme to write screenplays
Goals:
  - Install the fountain theme, which bundles a post-build hook for screenplay assembly
  - Get assembled .fountain files automatically when running ztl build
  - Not need to understand hooks — it just works with the theme
Constraints:
  - Non-technical; should not need to write or configure hooks
  - Expects the theme to handle domain-specific output
Daily workflow:
  1. ztl serve --theme fountain → writes scenes, sees prev/next navigation
  2. ztl build --theme fountain → dist/ contains per-scene HTML AND screenplay.fountain
  3. The .fountain file appeared because the theme's post-build hook assembled it
```

**Happy Path: Theme Hook Runs Transparently**

```
Preconditions: Vault with fountain theme active. Theme has hooks/post-build.
Steps:
  1. ztl build --theme fountain
  2. ztl runs build pipeline (scan, render, static assets)
  3. ztl discovers themes/fountain/hooks/post-build
  4. ztl invokes post-build hook with vault context JSON on stdin
  5. Hook walks scene chains, strips frontmatter/wikilinks, writes screenplay.fountain to out_dir
  6. Build completes. dist/ contains both site HTML and screenplay.fountain
Postconditions: screenplay.fountain exists in build output.
Failure modes:
  - Hook script not executable → warning, build still succeeds
  - Hook exits non-zero → warning logged, build output is still valid (just missing .fountain)
  - Hook not present → silently skipped, build is normal
```

### 2.2 Documentation Engineer (Custom Hook Author)

```
Role: Engineer publishing internal documentation via ztl
Goals:
  - Add a post-build hook that generates an RSS feed from recent pages
  - Add an on-save hook that pings a Slack channel when docs are updated
  - Add a post-check hook that enforces custom linting rules
Constraints:
  - Comfortable writing scripts
  - Wants hooks to be version-controlled alongside the vault
  - Needs access to the full link graph and frontmatter data
Daily workflow:
  1. Write hooks in .ztl/hooks/ as shell/Python scripts
  2. ztl build → post-build hook generates feed.xml
  3. ztl serve → on-save hook sends Slack notification
  4. ztl check → post-check hook runs prose linter
```

**Happy Path: Write a Custom Post-Build Hook**

```
Preconditions: Vault with .ztl/ directory. jq installed.
Steps:
  1. Create .ztl/hooks/post-build:
     #!/bin/bash
     # Generate RSS feed from pages with "date" frontmatter
     jq -r '.pages[] | select(.frontmatter.date) | ...' < /dev/stdin > "$ztl_OUT_DIR/feed.xml"

  2. chmod +x .ztl/hooks/post-build
  3. ztl build → hook runs, feed.xml appears in output
  4. ztl hook list → shows "post-build  .ztl/hooks/post-build  (vault)"

Postconditions: feed.xml generated on every build.
Failure modes:
  - Hook not executable → clear warning with chmod hint
  - jq not installed → hook fails, warning logged, build still succeeds
```

### 2.3 Theme Author (Hook Publisher)

```
Role: Developer creating a ztl theme that needs custom build output
Goals:
  - Bundle hooks with the theme so they activate automatically
  - Ship a post-build hook that produces domain-specific artifacts
  - Ensure hooks work across platforms (bash + portable tools)
Constraints:
  - Cannot assume user has specific tools installed (beyond sh and common Unix utils)
  - Hook must receive enough context to do useful work
  - Hook failures must not break the user's build
```

**Happy Path: Ship a Theme with Hooks**

```
Preconditions: Theme author developing themes/fountain/.
Steps:
  1. Create themes/fountain/hooks/post-build:
     #!/bin/sh
     # Assemble .fountain screenplay from scene chains
     # Reads vault context JSON from stdin
     ...

  2. ztl build --theme fountain
     → ztl finds themes/fountain/hooks/post-build (bundled theme hook)
     → invokes it with vault context
     → screenplay.fountain written to out_dir

  3. User installs theme via ztl theme install, hooks come along automatically.

Postconditions: Theme hooks run for any user who activates the theme.
Failure modes:
  - Theme installed from git, hook not marked executable → ztl makes it executable at install time (or warns)
```

---

## 3. Requirements

### 3.1 Hook Directory and Discovery

REQ-016-001: Hook Directory Structure

The system SHALL discover hooks from two locations, in order:

1. **Theme hooks:** `<theme-dir>/hooks/<hook-name>` — from the active theme's directory (disk-installed themes at `.ztl/themes/<name>/hooks/`, bundled themes at compile-time embed)
2. **Vault hooks:** `.ztl/hooks/<hook-name>` — vault-local hooks

Both locations are checked for each lifecycle point. If both a theme hook and a vault hook exist for the same lifecycle point, both run — theme hook first, then vault hook.

Hook files MUST be executable (Unix `+x` permission). Non-executable hook files SHALL produce a warning suggesting `chmod +x`.

Trace:
- TEST-016-001

---

REQ-016-002: Hook Names

Hook names correspond to lifecycle points. The system SHALL recognise the following hook names:

| Hook Name | Trigger | Can Abort? |
| --- | --- | --- |
| `pre-build` | Before `ztl build` renders any pages | Yes (non-zero exit skips build) |
| `post-build` | After `ztl build` completes all rendering and asset copying | No (warning only) |
| `post-index` | After `ztl index` completes scanning and indexing | No |
| `post-check` | After `ztl check` collects all diagnostics | No |
| `on-save` | After a page is saved in `ztl serve` and the vault is reindexed | No |
| `pre-serve` | Before `ztl serve` starts the HTTP server | Yes |

Unrecognised files in the hooks directory SHALL be silently ignored. This allows hooks directories to contain supporting files (libraries, config, README) alongside the hook executables.

Trace:
- TEST-016-002

---

### 3.2 Hook Execution

REQ-016-003: Execution Model

When a lifecycle point is reached, the system SHALL:

1. Discover all hooks for that point (theme + vault), in order.
2. For each hook:
   a. Verify the file is executable. If not, log a warning and skip.
   b. Spawn the hook as a child process.
   c. Write the hook context JSON to the process's stdin, then close stdin.
   d. Set environment variables (REQ-016-005).
   e. Set the working directory to the vault root.
   f. Wait for the process to exit (with a timeout per REQ-016-006).
   g. Capture stdout and stderr.
3. After all hooks for a lifecycle point complete, continue the parent operation.

Hooks run sequentially within a lifecycle point — theme hook before vault hook. No parallelism.

Trace:
- TEST-016-003

---

REQ-016-004: Exit Code Semantics

| Exit Code | Meaning |
| --- | --- |
| 0 | Success. Hook output (stdout) is logged at verbose level. |
| Non-zero | Failure. Hook stderr is logged as a warning. |

For **pre-** hooks (`pre-build`, `pre-serve`): a non-zero exit code SHALL abort the parent operation. The hook's stderr is printed as the error message. This allows pre-hooks to act as gates (e.g., "don't build if linting fails").

For all other hooks: a non-zero exit code SHALL produce a warning but SHALL NOT abort the parent operation. The build/index/check/serve continues normally.

Trace:
- TEST-016-004

---

REQ-016-005: Environment Variables

The system SHALL set the following environment variables for every hook invocation:

| Variable | Value | Example |
| --- | --- | --- |
| `ztl_HOOK` | The hook name being invoked | `post-build` |
| `ztl_VAULT_ROOT` | Absolute path to the vault root | `/Users/jane/screenplay` |
| `ztl_THEME` | Active theme name | `fountain` |
| `ztl_VERSION` | ztl version string | `0.9.0` |

Additional variables for specific hooks:

| Variable | Hooks | Value |
| --- | --- | --- |
| `ztl_OUT_DIR` | `pre-build`, `post-build` | Absolute path to the build output directory |
| `ztl_SAVED_FILE` | `on-save` | Relative path of the saved file |
| `ztl_SAVED_PAGE` | `on-save` | Page name of the saved file |
| `ztl_PORT` | `pre-serve` | Port number for the server |

Trace:
- TEST-016-005

---

REQ-016-006: Timeout

Hook execution SHALL have a default timeout of 30 seconds. If a hook does not exit within the timeout, the system SHALL kill the process and log a warning.

The timeout is not configurable in this version (future: `.ztl/config.toml` hook timeout setting).

Trace:
- TEST-016-006

---

### 3.3 Hook Context (stdin JSON)

REQ-016-007: Context Schema

The system SHALL write a JSON object to each hook's stdin containing the vault context relevant to the lifecycle point. The schema varies by hook:

**All hooks receive a common base:**

```json
{
  "hook": "post-build",
  "vault_root": "/Users/jane/screenplay",
  "theme": "fountain",
  "ztl_version": "0.9.0",
  "pages": [
    {
      "name": "Scene 1 - Apartment",
      "path": "scenes/scene-1-apartment.md",
      "slug": "scenes/scene-1-apartment",
      "frontmatter": { "title": "...", "next": "[[Scene 2]]", ... },
      "outlinks": ["Scene 2", "Character Bible"],
      "backlinks": ["FADE IN"],
      "is_orphan": false
    }
  ],
  "stats": {
    "total_pages": 42,
    "total_links": 128,
    "dead_links": 3,
    "orphans": 5
  }
}
```

**`post-build` and `pre-build` additionally receive:**

```json
{
  "out_dir": "/Users/jane/screenplay/dist",
  "pages_rendered": 42
}
```

**`post-check` additionally receives:**

```json
{
  "diagnostics": {
    "dead_links": [ { "source": "...", "target": "...", "line": 10 } ],
    "orphans": [ { "page": "...", "forward_links": 3 } ],
    "syntax_errors": [ { "file": "...", "line": 5, "message": "..." } ]
  }
}
```

**`on-save` additionally receives:**

```json
{
  "saved": {
    "file": "scenes/scene-1-apartment.md",
    "page": "Scene 1 - Apartment",
    "content_length": 2048
  }
}
```

The context SHALL include enough data for hooks to perform useful work without needing to re-scan the vault or call ztl subcommands.

Trace:
- TEST-016-007
- CON-016-001

---

### 3.4 Theme-Bundled Hooks

REQ-016-008: Theme Hook Activation

When a theme is active (`--theme <name>`), the system SHALL discover hooks in the theme's `hooks/` subdirectory:

- **Disk-installed themes:** `.ztl/themes/<name>/hooks/<hook-name>`
- **Bundled themes:** Embedded at compile time from `themes/<name>/hooks/<hook-name>`

For bundled themes, hook files cannot be executed directly from the embedded binary. The system SHALL extract bundled hook files to a temporary directory (or `.ztl/cache/hooks/<theme>/`) before execution. Extracted hooks are refreshed when the ztl binary version changes.

Trace:
- TEST-016-008

---

REQ-016-009: Theme Hook Security

Theme hooks installed via `ztl theme install` (SPEC-014) SHALL be made executable (`chmod +x`) during installation. The system SHALL log the hooks being installed:

```
  theme: installed hooks/post-build (executable)
```

The system SHALL NOT execute hooks from themes that were installed without the user's explicit action (i.e., no auto-downloaded hooks). Only hooks from:
- Bundled themes (shipped with the binary, trusted)
- Explicitly installed themes (`ztl theme install`)
- Vault-local hooks (`.ztl/hooks/`, user-created)

Trace:
- TEST-016-009

---

### 3.5 Hook Management Commands

REQ-016-010: `ztl hook list`

The system SHALL provide a `ztl hook list` subcommand that lists all active hooks for the current vault and theme. Default output is JSON; `-f table` produces a human-readable table.

**JSON schema:**

```json
{
  "hooks": [
    {
      "name": "post-build",
      "source": "theme",
      "path": ".ztl/themes/fountain/hooks/post-build",
      "executable": true
    },
    {
      "name": "on-save",
      "source": "vault",
      "path": ".ztl/hooks/on-save",
      "executable": true
    }
  ]
}
```

**Table output:**

```
 Hook         Source  Path                                      Executable
 post-build   theme  .ztl/themes/fountain/hooks/post-build    yes
 on-save      vault  .ztl/hooks/on-save                       yes
```

Trace:
- TEST-016-010

---

REQ-016-011: `ztl hook run <name>`

The system SHALL provide a `ztl hook run <name>` subcommand that manually invokes a hook with the current vault context. This is useful for testing hooks during development.

The command SHALL:
1. Run the vault pipeline (scan, parse, graph) to build current context.
2. Invoke the named hook(s) — both theme and vault if both exist — with the appropriate context for that hook type.
3. Print the hook's stdout to the terminal.
4. Print the hook's stderr to stderr.
5. Exit with the hook's exit code.

For hooks that require additional context (e.g., `on-save` needs a saved file), the command SHALL accept `--` followed by extra JSON fields that are merged into the context.

Trace:
- TEST-016-011

---

### 3.6 Non-Functional Requirements

NFR-016-001: Hook Overhead

Hook discovery (checking for executable files in hooks directories) SHALL add ≤ 5 ms to operations when no hooks are present.

Trace:
- TEST-016-NFR-001

---

NFR-016-002: Hook Isolation

Hooks SHALL NOT be able to modify ztl's in-memory state. They are separate processes with no shared memory. They can only affect the world through their own I/O (writing files, network calls, stdout/stderr).

Trace:
- TEST-016-NFR-002

---

NFR-016-003: Graceful Degradation

If a hook crashes (segfault, panic) or times out, the parent operation SHALL continue normally (except for pre-hooks, which abort). No hook failure shall corrupt vault data, the link graph, or the build output produced by ztl itself.

Trace:
- TEST-016-NFR-003

---

## 4. Architecture Decisions

### ADR-016-001: Git-Style Executable Hooks over Plugin API

**Context:** Extensibility can be achieved through: (a) executable hooks (git model), (b) a plugin API with shared libraries (editors like Neovim), (c) WASM plugins (Zed, Figma), (d) scripting language embedding (Lua, JS).

**Decision:** Executable hooks in `.ztl/hooks/`, invoked as child processes.

**Rationale:**

- **Zero coupling** — hooks are separate processes. They cannot corrupt ztl's memory, deadlock its threads, or introduce undefined behaviour. A crashing hook is just a non-zero exit.
- **Any language** — no FFI, no ABI compatibility, no SDK. A hook is a script with a shebang. The ecosystem of available tools (jq, python, node, compiled binaries) is the "plugin API."
- **Universally understood** — every developer knows executable scripts. Git, husky, pre-commit, and similar tools have normalised this pattern.
- **Version-controllable** — hooks are files in `.ztl/hooks/`, committed alongside the vault. No external state to manage.
- **Debuggable** — `ztl hook run post-build` invokes the hook with real data. Print-debugging works. No plugin framework to understand.

**Trade-offs:**

- **Performance** — process spawn per hook (~5-10 ms) plus JSON serialisation. Acceptable for lifecycle hooks that run at most once per operation.
- **No ztl API access** — hooks cannot query the search index or traverse the graph incrementally. They receive a snapshot of data. For deep integration, the hook would need to shell out to `ztl search` or parse the JSON context. This is by design — hooks are for side effects, not for extending the core engine.
- **No Windows `.bat` support initially** — Unix executable model. Windows users need WSL or a compiled binary as their hook.

**Alternatives rejected:**

- WASM plugins: powerful but requires an SDK, a compilation step, and a runtime. Overkill for "run a script after build."
- Lua/JS embedding: ties ztl to a specific runtime, increases binary size, creates a maintenance burden.
- Shared library plugins: ABI instability, safety concerns, platform-specific builds.

---

### ADR-016-002: Theme-Bundled Hooks over Core Feature Flags

**Context:** Domain-specific build outputs (screenplay assembly, RSS feeds, slide decks) could be implemented as: (a) core features behind feature flags, (b) theme-bundled hooks, (c) standalone plugins.

**Decision:** Theme-bundled hooks. Themes ship `hooks/` alongside `templates/` and `static/`.

**Rationale:**

- **Cohesion** — the fountain theme's post-build hook and its templates are part of the same concern (screenplay workflow). Shipping them together means installing one theme gets the complete experience.
- **No binary bloat** — screenplay assembly doesn't increase ztl's binary size. It's a script in the theme.
- **Swappable** — users can override a theme's hook by placing their own version in `.ztl/hooks/` with the same name (vault hooks run after theme hooks, and can override side effects).
- **Discoverable** — `ztl hook list` shows what hooks a theme provides. No hidden feature flags.

**Trade-off:** Theme hooks are scripts, not compiled Rust. They may be slower than a native implementation. For the expected use cases (assembling a few hundred files, generating an XML feed), script performance is adequate.

---

### ADR-016-003: JSON on Stdin over Temp Files or Arguments

**Context:** Hook context could be passed via: (a) JSON on stdin, (b) a temporary JSON file path as an argument, (c) environment variables only, (d) a Unix socket / named pipe.

**Decision:** JSON on stdin, supplemented by environment variables for the most common fields.

**Rationale:**

- **Streaming** — stdin allows large contexts without hitting argument length limits or cluttering `/tmp`.
- **Composable** — hooks can pipe stdin through `jq`, `python -c`, or any tool that reads stdin. This is the Unix way.
- **No cleanup** — unlike temp files, stdin doesn't leave artifacts to clean up.
- **Environment variables for convenience** — `ztl_OUT_DIR`, `ztl_VAULT_ROOT` etc. are available without parsing JSON, making trivial hooks simpler.

**Trade-off:** Hooks that need to read stdin twice must buffer it (e.g., `INPUT=$(cat); echo "$INPUT" | jq ...`). This is a standard shell pattern and well-understood.

---

## 5. Contracts

### CON-016-001: Hook Context JSON Schema (Base)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["hook", "vault_root", "theme", "ztl_version", "pages", "stats"],
  "properties": {
    "hook": { "type": "string", "description": "Hook name (e.g., post-build)" },
    "vault_root": { "type": "string", "description": "Absolute path to vault root" },
    "theme": { "type": "string", "description": "Active theme name" },
    "ztl_version": { "type": "string", "description": "ztl version" },
    "pages": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["name", "path", "slug"],
        "properties": {
          "name": { "type": "string" },
          "path": { "type": "string" },
          "slug": { "type": "string" },
          "frontmatter": { "type": "object" },
          "outlinks": { "type": "array", "items": { "type": "string" } },
          "backlinks": { "type": "array", "items": { "type": "string" } },
          "is_orphan": { "type": "boolean" }
        }
      }
    },
    "stats": {
      "type": "object",
      "properties": {
        "total_pages": { "type": "integer" },
        "total_links": { "type": "integer" },
        "dead_links": { "type": "integer" },
        "orphans": { "type": "integer" }
      }
    }
  }
}
```

**Hook-specific extensions** are merged into the base object:

- `post-build` / `pre-build`: adds `out_dir` (string), `pages_rendered` (integer)
- `post-check`: adds `diagnostics` (object with `dead_links`, `orphans`, `syntax_errors` arrays)
- `on-save`: adds `saved` (object with `file`, `page`, `content_length`)
- `pre-serve`: adds `port` (integer)

Implements:
- REQ-016-007

Verified by:
- TEST-016-007

---

### CON-016-002: Hook Execution Protocol

```
Invocation:
  1. Set environment variables (ztl_HOOK, ztl_VAULT_ROOT, ztl_THEME, ztl_VERSION, ...)
  2. Set working directory to vault root
  3. Spawn process: <hook-path>
  4. Write JSON context to stdin, close stdin
  5. Wait for exit (timeout: 30s)
  6. Read stdout (logged at verbose level)
  7. Read stderr (logged as warning on non-zero exit)

Exit codes:
  0         → success
  1-125     → failure (pre-hooks abort; post-hooks warn)
  126       → hook not executable (ztl detects this before spawn)
  127       → hook interpreter not found
  128+      → killed by signal (e.g., timeout → SIGKILL)

Discovery order per lifecycle point:
  1. Active theme hooks (bundled → extracted; installed → direct)
  2. Vault hooks (.ztl/hooks/)

Both run if both exist. Theme hook runs first.
```

Implements:
- REQ-016-003
- REQ-016-004

Verified by:
- TEST-016-003
- TEST-016-004

---

## 6. Test Specifications

### TEST-016-001: Hook Discovery

Scenario: `.ztl/hooks/post-build` exists and is executable.
Expected: Discovered and listed by `ztl hook list`.

Scenario: `.ztl/hooks/post-build` exists but is NOT executable.
Expected: Warning logged. Not invoked.

Scenario: Theme has `hooks/post-build`, vault also has `.ztl/hooks/post-build`.
Expected: Both discovered. Theme hook listed first.

Scenario: No hooks directory exists.
Expected: No warnings. Operations proceed normally.

---

### TEST-016-002: Hook Names

Scenario: `.ztl/hooks/post-build` — recognised lifecycle point.
Expected: Invoked after build.

Scenario: `.ztl/hooks/my-custom-thing` — unrecognised name.
Expected: Silently ignored. No warning.

Scenario: `.ztl/hooks/README.md` — supporting file.
Expected: Silently ignored.

---

### TEST-016-003: Execution Model

Scenario: `post-build` hook reads stdin, writes a file to `$ztl_OUT_DIR/custom.txt`.
Expected: File exists after build completes.

Scenario: Hook's working directory.
Expected: `pwd` inside hook equals vault root.

Scenario: Hook receives JSON on stdin.
Expected: Valid JSON parseable by `jq .hook` → returns "post-build".

---

### TEST-016-004: Exit Code Semantics

Scenario: `pre-build` hook exits with code 1.
Expected: Build aborted. Hook's stderr printed as error.

Scenario: `post-build` hook exits with code 1.
Expected: Warning logged. Build output is intact.

Scenario: `post-index` hook exits with code 0.
Expected: Hook stdout logged at verbose level only.

---

### TEST-016-005: Environment Variables

Scenario: `post-build` hook prints `$ztl_VAULT_ROOT`.
Expected: Output matches the vault's absolute path.

Scenario: `post-build` hook prints `$ztl_OUT_DIR`.
Expected: Output matches the build output directory.

Scenario: `on-save` hook prints `$ztl_SAVED_FILE`.
Expected: Output matches the relative path of the saved file.

---

### TEST-016-006: Timeout

Scenario: Hook contains `sleep 60`.
Expected: Killed after 30 seconds. Warning logged. Parent operation continues.

---

### TEST-016-007: Context JSON

Scenario: `post-build` hook parses stdin JSON, extracts `pages[0].frontmatter.title`.
Expected: Returns the correct frontmatter value.

Scenario: `post-check` hook parses stdin JSON, counts `diagnostics.dead_links`.
Expected: Count matches `ztl check` output.

Scenario: `on-save` hook parses stdin JSON, reads `saved.page`.
Expected: Returns the saved page's name.

---

### TEST-016-008: Theme-Bundled Hooks

Scenario: Active theme has `hooks/post-build`. Run `ztl build --theme <name>`.
Expected: Theme's post-build hook is invoked.

Scenario: Bundled theme has `hooks/post-build` (embedded in binary).
Expected: Hook extracted to cache, made executable, invoked.

Scenario: Switch to `--theme default` (no hooks).
Expected: Previous theme's hooks no longer run.

---

### TEST-016-009: Theme Hook Security

Scenario: `ztl theme install user/repo` where theme contains `hooks/post-build`.
Expected: Hook installed with executable permission. Installation log mentions the hook.

Scenario: Theme hook exists but is not executable after install.
Expected: Warning with `chmod +x` hint. Hook not invoked.

---

### TEST-016-010: Hook List Command

Scenario: Theme has `post-build` hook, vault has `on-save` hook.
Expected: `ztl hook list` shows both with correct source and path.

Scenario: No hooks.
Expected: Empty list. No error.

---

### TEST-016-011: Hook Run Command

Scenario: `ztl hook run post-build`.
Expected: Runs the post-build hook with current vault context. Stdout printed.

Scenario: `ztl hook run on-save -- '{"saved":{"file":"test.md","page":"Test","content_length":100}}'`.
Expected: Runs on-save hook with merged context.

Scenario: `ztl hook run nonexistent`.
Expected: Error: no hook named "nonexistent" found.

---

## 7. Observability

OBS-016-001: Hook Execution Logging

When `--verbose` is active, the system SHALL log for each hook invocation:
- Hook name and source (theme or vault)
- Hook path
- Execution duration (ms)
- Exit code
- Stdout (truncated to 1 KB if longer)
- Stderr on failure

---

OBS-016-002: Hook Discovery Logging

When `--verbose` is active, the system SHALL log during hook discovery:
- Which directories were checked
- Which hooks were found (name, path, executable status)
- Which hooks were skipped and why

---

## 8. Phased Implementation

### Phase 1: Core Hook Execution Engine

**Goal:** Implement hook discovery, execution, and context passing for `post-build` and `post-index`. These are the simplest lifecycle points — no abort semantics, no serve-mode complexity.

**Changes:**
- Add hook discovery module: scan `.ztl/hooks/` and theme hooks directories
- Add hook execution: spawn process, write JSON stdin, read stdout/stderr, handle exit codes
- Add context builder: serialise vault data (pages, frontmatter, stats, links) to JSON
- Wire `post-build` into `build_static()` and `post-index` into `cmd_index()`
- Add `ztl_*` environment variables

**Verification:** Create a post-build hook that writes a file. Verify it runs and the file appears. Verify non-executable hooks produce warnings. Verify failing hooks don't break the build.

### Phase 2: Pre-Hooks, On-Save, and Pre-Serve

**Goal:** Add remaining lifecycle points including abort-capable pre-hooks and the serve-mode on-save hook.

**Changes:**
- Implement `pre-build` and `pre-serve` with abort-on-failure semantics
- Implement `on-save` hook in the save handler (after reindex)
- Add `post-check` hook after diagnostics collection
- Add timeout enforcement (30s)
- Add hook-specific context extensions (diagnostics, saved file info)

**Verification:** Pre-build hook that exits 1 aborts the build. On-save hook fires after page edit in serve mode. Timeout kills a sleeping hook.

### Phase 3: Theme-Bundled Hooks

**Goal:** Support hooks shipped inside themes — both disk-installed and bundled (compile-time embedded).

**Changes:**
- Add theme hook discovery (check active theme's hooks/ directory)
- For bundled themes: extract embedded hooks to `.ztl/cache/hooks/<theme>/` before execution
- Update `ztl theme install` to make hook files executable
- Log theme hooks during installation
- Run theme hooks before vault hooks at each lifecycle point

**Verification:** Install a theme with hooks via `ztl theme install`. Verify hooks run on build. Verify bundled theme hooks are extracted and executed. Verify theme + vault hooks both run in correct order.

### Phase 4: Management Commands and Polish

**Goal:** Add `ztl hook list` and `ztl hook run`, polish logging and error messages.

**Changes:**
- Add `Hook` subcommand group to `cli.rs` with `List` and `Run` variants
- Implement `ztl hook list` with JSON and table output
- Implement `ztl hook run <name>` with optional context merging
- Add verbose logging (OBS-016-001, OBS-016-002)
- Polish all warning/error messages

**Verification:** `ztl hook list` shows all hooks with metadata. `ztl hook run post-build` invokes the hook with real data. Verbose output is informative.

---

## 9. Open Questions

1. **Should hooks be able to produce diagnostics that appear in `ztl check` output?**
   A `post-check` hook could print JSON diagnostics to stdout that ztl merges into the check output. This would let hooks act as custom linters whose findings appear alongside native diagnostics. Recommendation: defer — hooks can print their own warnings to stderr for now. Structured diagnostic integration is a future enhancement.

2. **Should `on-save` hooks block the HTTP response?**
   If an on-save hook takes 5 seconds, the user's browser waits. Options: (a) run hooks synchronously (simple, might feel slow), (b) run hooks asynchronously (fast response, but hook errors are invisible), (c) configurable. Recommendation: run asynchronously — the save response returns immediately, hooks run in a background task. Hook errors are logged to the server's stderr.

3. **Should there be a `post-serve` hook (server shutdown)?**
   Useful for cleanup tasks. Recommendation: defer — limited use case, and cleanup is better handled by the shell (trap/finally in the calling script).

4. **Should hook context include file content?**
   Including raw markdown for every page in the JSON context could be large for big vaults. Currently only frontmatter, links, and metadata are included. Hooks that need content can read files from disk using the `path` field. Recommendation: do not include content in the base context. Add a `--with-content` flag to `ztl hook run` for development/debugging.

5. **Should vault hooks be able to override theme hooks?**
   Currently both run. A user might want to replace a theme's post-build hook entirely. Options: (a) both always run (current design), (b) vault hook replaces theme hook if same name, (c) vault hook can declare `skip-theme: true`. Recommendation: both run — vault hooks can check `ztl_HOOK_SOURCE` or similar to decide whether to act. Full replacement semantics are a future enhancement.

6. **Should bundled theme hooks be embedded as scripts or as compiled helpers?**
   For the fountain theme's screenplay assembler, a compiled Rust binary would be faster than a shell script. But embedding compiled binaries per-platform is complex. Recommendation: ship hooks as shell scripts. If performance is insufficient, the hook can call out to a compiled binary that the user installs separately (or that ships in the theme's `static/` directory).

---

## 10. Future Considerations

| Feature | Description |
| --- | --- |
| Hook configuration | `.ztl/config.toml` settings for hook timeout, enable/disable per hook |
| Structured diagnostics | Hooks emit JSON diagnostics to stdout that merge into `ztl check` output |
| Watch-mode hooks | `on-change` hook triggered by file watcher (SPEC-008 integration) |
| Hook templates | `ztl hook init <name>` scaffolds a hook from a template |
| Async on-save | Explicit async flag for on-save hooks to control blocking vs background |
| Windows support | `.cmd` / `.ps1` hook support for native Windows execution |
| Hook marketplace | Standalone hooks (not bundled with themes) distributed via git, similar to SPEC-014 |
| Hook chaining | Pipe one hook's stdout into the next hook's stdin for composition |

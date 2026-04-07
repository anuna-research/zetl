---
title: "SPEC-022: OSC 8 Editor Hyperlinks in CLI Output"
version: 0.1.0
status: draft
date: 2026-04-07
audience: agent, human
parent: SPEC-001
related:
  - SPEC-003  # Agent Ergonomics & Robustness
  - SPEC-009  # zetl view (TUI)
  - SPEC-013  # zetl search
---

| Field        | Value                                        |
|--------------|----------------------------------------------|
| Document     | SPEC-022                                     |
| Title        | OSC 8 Editor Hyperlinks in CLI Output        |
| Version      | 0.1.0                                        |
| Status       | Draft                                        |
| Author       | Agent (USDD Protocol v1.3.0)                 |
| Date         | 2026-04-07                                   |
| Audience     | agent, human                                 |
| Trace        | USDD §2 (Vision → Specification)             |
| Parent       | SPEC-001                                     |
| Related      | SPEC-003, SPEC-009, SPEC-013                 |

---

## 1. Overview

### 1.1 Problem

zetl CLI output is rich with file references. Search results include `path/to/note.md:42`, dead-link diagnostics print `source.md:17: broken link [[Missing]]`, and link listings show source locations for every backlink and forward link. In every case, the user sees a file path and line number but cannot act on it — they must manually copy the path, switch to their editor, open the file, and navigate to the line. This context switch is a friction tax paid dozens of times per session.

Modern terminals (iTerm2, WezTerm, Ghostty, Windows Terminal, kitty, foot, GNOME Terminal ≥3.50, macOS Terminal.app ≥15.0) support OSC 8 hyperlinks — the terminal equivalent of HTML `<a href>`. An OSC 8 hyperlink wraps visible text in invisible escape sequences that make the text clickable. The terminal renders the text normally but opens a URI when the user clicks (or Cmd-clicks) it. If the URI uses an editor protocol (`vscode://file/...`), the click opens the file at the exact line in the user's editor.

### 1.2 Core Insight

Every file:line reference zetl already prints is a latent hyperlink. The data is present; only the escape sequence wrapper is missing. Adding OSC 8 requires no changes to output content, no new commands, and no new flags. The feature is purely additive: terminals that support OSC 8 gain clickable links; terminals that do not see identical output (the escape sequences are invisible no-ops in legacy terminals, and zetl suppresses them entirely when stdout is not a TTY).

### 1.3 Design Philosophy

- **Zero configuration for the common case.** If `$EDITOR` is `code`, links use `vscode://file/` automatically. The user clicks and it works.
- **Full control for the uncommon case.** `ZETL_EDITOR_URI` overrides everything with an arbitrary URI template. Any editor with a URI protocol handler is supported.
- **No output pollution.** OSC 8 sequences are suppressed when stdout is piped, redirected, or when `NO_COLOR` is set. JSON output is never affected. The feature is invisible unless the user is at an interactive terminal.
- **No new dependencies.** OSC 8 is a simple byte sequence (`\x1b]8;;{uri}\x1b\\{text}\x1b]8;;\x1b\\`). It requires no terminal library support beyond raw write.

### 1.4 Scope

**In scope:**

- OSC 8 escape sequence wrapping of file:line references in CLI text output
- `ZETL_EDITOR_URI` environment variable for custom editor URI templates
- Built-in presets for VS Code, Cursor, Zed, Sublime Text, and Neovim
- Auto-detection of `$EDITOR` / `$VISUAL` to select a sensible default preset
- TTY detection: only emit OSC 8 when stdout is a terminal
- Respect `NO_COLOR` convention (already supported by zetl)
- Coverage across all commands that print file references: `search`, `links`, `backlinks`, `check`, `diff`, `graph`, `blocks`
- `--no-hyperlinks` flag to explicitly disable

**Out of scope:**

- Hyperlinks in TUI mode (`zetl view`) — the TUI uses ratatui and crossterm for rendering; OSC 8 in TUI cells is a separate concern (future work)
- Hyperlinks in JSON output — JSON consumers parse structured data, not terminal escapes
- Hyperlinks in `zetl serve` web output — the browser has its own link model
- Custom URI schemes beyond the built-in presets (users handle this via `ZETL_EDITOR_URI`)
- OSC 8 in log/debug output on stderr

---

## 2. User Profiles

### UP-022-001: Terminal Power User

**Goals:** Click file references in zetl output to jump directly to the relevant line in their editor. Reduce the copy-paste-navigate cycle to a single click.

**Constraints:** Uses a modern terminal (iTerm2, WezTerm, Ghostty, kitty, or similar). Has VS Code, Cursor, or Zed as their primary editor. Expects things to work without configuration.

**Happy path:**
1. User has `$EDITOR=code` in their shell profile
2. Runs `zetl check` — sees dead-link diagnostics with file:line references
3. Cmd-clicks a file reference → VS Code opens at that exact line
4. No configuration was needed; zetl detected `code` and used `vscode://file/` automatically

### UP-022-002: Neovim User with Custom Terminal

**Goals:** Open file references in a running Neovim instance via `nvim --remote` or a custom URI handler.

**Constraints:** Uses Neovim in a terminal multiplexer (tmux/zellij). May need a custom URI template because Neovim's remote protocol varies by setup.

**Happy path:**
1. Sets `ZETL_EDITOR_URI="nvim://open?file={path}&line={line}"` in `.zshrc`
2. Registers a system URI handler for `nvim://` that invokes `nvr --remote +{line} {path}`
3. Runs `zetl search "TODO"` — clicks a result → Neovim jumps to the line

### UP-022-003: CI / Scripting Agent

**Goals:** Consume zetl output programmatically. Hyperlink escapes must never appear in piped or redirected output.

**Constraints:** Non-interactive. Parses text output with grep/awk or uses `--format json`.

**Happy path:**
1. Runs `zetl check --format json | jq '.dead_links[]'` — JSON output, no escapes
2. Runs `zetl check 2>&1 | grep "broken"` — stdout is a pipe, no OSC 8 emitted
3. Runs `zetl check > report.txt` — stdout is a file, no OSC 8 emitted

### UP-022-004: User on Legacy Terminal

**Goals:** Use zetl normally. Not affected by the hyperlink feature.

**Constraints:** Terminal does not support OSC 8 (e.g., older xterm, raw Linux console).

**Happy path:**
1. Runs `zetl check` — output looks identical to pre-SPEC-022 behavior
2. OSC 8 sequences are present in the byte stream but the terminal ignores them (they are defined as no-ops for unsupporting terminals per the OSC 8 specification)
3. If the user finds the invisible bytes problematic (e.g., a terminal that renders them as garbage), they set `--no-hyperlinks` or `NO_COLOR=1`

---

## 3. Requirements

### 3.1 OSC 8 Emission

#### REQ-120: OSC 8 Wrapping of File References

The system SHALL wrap file:line references in OSC 8 hyperlink escape sequences when all of the following conditions are true:

1. stdout is a TTY (as determined by `std::io::stdout().is_terminal()`)
2. `NO_COLOR` environment variable is not set
3. `--no-hyperlinks` flag is not passed
4. Output format is not JSON (`--format json` suppresses all escape sequences)

The OSC 8 escape sequence format SHALL be:

```
\x1b]8;;<uri>\x1b\\<visible_text>\x1b]8;;\x1b\\
```

Where `<uri>` is the resolved editor URI and `<visible_text>` is the file:line reference as it would appear without this feature.

Trace: TEST-143, TEST-144, TEST-148

#### REQ-121: Editor URI Template

The system SHALL resolve editor URIs using a template string containing the following placeholders:

| Placeholder | Expansion                                    |
|-------------|----------------------------------------------|
| `{path}`    | Absolute file path (resolved from vault root)|
| `{line}`    | 1-based line number                          |
| `{col}`     | 1-based column number (default: 1)           |

Example template: `vscode://file/{path}:{line}:{col}`

The template SHALL be sourced from (in priority order):

1. `ZETL_EDITOR_URI` environment variable (highest priority)
2. Auto-detected from `$VISUAL` or `$EDITOR` (see REQ-122)
3. Built-in default: `vscode://file/{path}:{line}:{col}`

Trace: TEST-145

#### REQ-122: Editor Auto-Detection

The system SHALL inspect `$VISUAL` (first) and `$EDITOR` (second) to select a built-in URI template. Detection SHALL match the basename of the environment variable value against the following presets:

| Basename match       | URI template                                         |
|----------------------|------------------------------------------------------|
| `code`, `code-insiders` | `vscode://file/{path}:{line}:{col}`               |
| `cursor`             | `cursor://file/{path}:{line}:{col}`                 |
| `zed`                | `zed://file/{path}:{line}:{col}`                    |
| `subl`, `sublime_text` | `subl://open?url=file://{path}&line={line}&column={col}` |
| `nvim`, `vim`, `vi`  | `file://{path}`                                     |
| `emacs`, `emacsclient` | `emacs://open?url=file://{path}&line={line}&column={col}` |

If neither `$VISUAL` nor `$EDITOR` matches a known preset, the system SHALL fall back to `vscode://file/{path}:{line}:{col}`.

Rationale: VS Code is the most widely-used editor with URI protocol support. A wrong default is corrected by setting `ZETL_EDITOR_URI`.

Trace: TEST-145, TEST-146

#### REQ-123: Path Resolution

All `{path}` expansions SHALL be resolved to absolute paths using the vault root as the base directory. Paths SHALL be percent-encoded per RFC 3986 for characters that are not unreserved (`A-Z a-z 0-9 - . _ ~`) or path separators (`/`). Spaces SHALL be encoded as `%20`.

Trace: TEST-147

#### REQ-124: Command Coverage

OSC 8 hyperlinks SHALL be applied to file:line references in the text output of the following commands:

| Command           | Wrapped references                                      |
|-------------------|---------------------------------------------------------|
| `zetl search`     | File path and line number in each search result         |
| `zetl links`      | Source file:line for each forward link                  |
| `zetl backlinks`  | Source file:line for each backlink                      |
| `zetl check`      | File:line in dead-link, orphan, and ambiguous-link diagnostics |
| `zetl diff`       | File paths in added/removed/changed entries             |
| `zetl graph`      | File paths in graph node listings                       |
| `zetl blocks`     | File:line for each block reference                      |

Commands that do not print file references (`zetl index`, `zetl serve`, `zetl build`, `zetl view`, `zetl reason`) are unaffected.

Trace: TEST-143, TEST-149

#### REQ-125: --no-hyperlinks Flag

The system SHALL accept a `--no-hyperlinks` global flag that disables OSC 8 emission regardless of TTY detection and environment variables.

```
zetl --no-hyperlinks check
```

This flag SHALL be exposed as a clap argument on the top-level CLI struct.

Trace: TEST-148

#### REQ-126: NO_COLOR Interaction

When the `NO_COLOR` environment variable is set (to any value, including empty string), the system SHALL suppress all OSC 8 hyperlink sequences. This is consistent with zetl's existing `NO_COLOR` behavior which disables ANSI color codes.

Rationale: `NO_COLOR` signals "do not emit terminal escapes." OSC 8 is a terminal escape. Suppressing it is both semantically correct and practically useful for users whose terminals mishandle OSC 8.

Trace: TEST-148

---

## 4. Architecture

### 4.1 Module Structure

```
src/
  hyperlink.rs          # New module: OSC 8 utilities
    - EditorPreset      # Enum of known editors
    - HyperlinkConfig   # Resolved configuration (template, enabled/disabled)
    - resolve_config()  # Reads env vars, detects editor, returns HyperlinkConfig
    - wrap_path()       # Wraps a file:line string in OSC 8 escape sequence
    - format_uri()      # Expands template with path/line/col
  cli.rs                # Add --no-hyperlinks flag
  main.rs               # Thread HyperlinkConfig through output paths
```

### 4.2 Data Flow

```
CLI invocation
  │
  ├─ parse --no-hyperlinks flag (cli.rs)
  ├─ check stdout.is_terminal()
  ├─ check NO_COLOR
  │
  ▼
resolve_config() → HyperlinkConfig { enabled: bool, template: String }
  │
  ├─ If !enabled → pass-through (no wrapping)
  │
  ▼
Command execution produces file:line references
  │
  ▼
wrap_path(config, display_text, abs_path, line, col) → String
  │
  ├─ enabled=false  → "{display_text}"
  ├─ enabled=true   → "\x1b]8;;{uri}\x1b\\{display_text}\x1b]8;;\x1b\\"
  │
  ▼
stdout
```

### 4.3 Integration Points

Each command that prints file references already has a formatting step where the path and line number are interpolated into the output string. The integration is a wrapper call at that interpolation point:

```rust
// Before (current):
format!("{}:{}: {}", path, line, message)

// After (SPEC-022):
format!("{}:{}: {}", hyperlink.wrap(path, line, 1, &format!("{}:{}", path, line)), message)
```

When `HyperlinkConfig.enabled` is false, `wrap()` returns the display text unchanged — zero overhead beyond a branch prediction.

### 4.4 Interaction with Existing Color System

OSC 8 hyperlinks are orthogonal to ANSI color codes. A file reference can be both colored and hyperlinked:

```
\x1b]8;;vscode://file/abs/path:42:1\x1b\\\x1b[36mpath.md:42\x1b[0m\x1b]8;;\x1b\\
```

The OSC 8 open/close sequences wrap the entire colored span. This ordering ensures that:
- The hyperlink target is the URI (not affected by color codes)
- The visible text retains its color
- Terminals that ignore OSC 8 still see the colored text

---

## 5. Contracts

### CON-022-001: HyperlinkConfig

```rust
pub struct HyperlinkConfig {
    /// Whether OSC 8 emission is enabled.
    pub enabled: bool,
    /// Editor URI template with {path}, {line}, {col} placeholders.
    /// Only meaningful when enabled=true.
    pub template: String,
}
```

### CON-022-002: resolve_config Signature

```rust
/// Resolve hyperlink configuration from environment and flags.
///
/// Priority:
/// 1. If no_hyperlinks is true → disabled
/// 2. If stdout is not a TTY → disabled
/// 3. If NO_COLOR is set → disabled
/// 4. If format is JSON → disabled
/// 5. Template from ZETL_EDITOR_URI, or auto-detected, or default
pub fn resolve_config(no_hyperlinks: bool, is_json: bool) -> HyperlinkConfig;
```

### CON-022-003: wrap_path Signature

```rust
/// Wrap display text in an OSC 8 hyperlink.
///
/// Returns display_text unchanged if config.enabled is false.
/// The path is resolved to absolute and percent-encoded.
/// Line and col are 1-based.
pub fn wrap_path(
    config: &HyperlinkConfig,
    display_text: &str,
    abs_path: &Path,
    line: u32,
    col: u32,
) -> String;
```

### CON-022-004: OSC 8 Byte Sequence

The exact byte sequence for a hyperlink SHALL be:

```
ESC ] 8 ; ; <uri> ESC \ <visible_text> ESC ] 8 ; ; ESC \
```

Where:
- `ESC` = `0x1B`
- `]` = `0x5D`
- `\` = `0x5C` (ST — String Terminator, per ECMA-48)
- `<uri>` = percent-encoded URI per RFC 3986
- `<visible_text>` = the file:line reference as rendered without this feature

The `params` field between the two semicolons in the opening sequence SHALL be empty (no `id=` parameter). This maximizes terminal compatibility.

### CON-022-005: URI Template Grammar

```
template     = *( literal / placeholder )
literal      = <any character except "{">
placeholder  = "{" name "}"
name         = "path" / "line" / "col"
```

Unknown placeholders SHALL be left unexpanded (treated as literal text). This allows forward-compatible templates if future placeholders are added.

---

## 6. Non-Functional Requirements

### NFR-047: Hyperlink Overhead

The per-reference overhead of OSC 8 wrapping (template expansion + escape sequence construction) SHALL be < 1 microsecond. The feature SHALL not measurably impact the wall-clock time of any command.

Rationale: OSC 8 wrapping is string concatenation and a single branch. It must not regress performance for commands that may print thousands of references (e.g., `zetl search` on a large vault).

Trace: REQ-120

### NFR-048: Terminal Compatibility

OSC 8 output SHALL be compatible with the following terminals without visual artifacts:

| Terminal          | Platform       | OSC 8 support |
|-------------------|----------------|---------------|
| iTerm2 ≥ 3.1     | macOS          | Full          |
| Terminal.app ≥ 15 | macOS          | Full          |
| WezTerm           | macOS/Linux    | Full          |
| Ghostty           | macOS/Linux    | Full          |
| kitty             | macOS/Linux    | Full          |
| foot              | Linux (Wayland)| Full          |
| Windows Terminal  | Windows        | Full          |
| GNOME Terminal ≥ 3.50 | Linux      | Full          |
| xterm (legacy)    | Linux          | Ignored (no artifacts) |
| tmux ≥ 3.4        | Any            | Passthrough   |
| screen            | Any            | Stripped (no artifacts) |

Terminals that do not support OSC 8 SHALL either ignore the escape sequences (rendering only the visible text) or strip them silently. The specification relies on the OSC 8 standard's design guarantee that non-supporting terminals treat the sequences as no-ops.

Trace: REQ-120

---

## 7. Architecture Decision Records

### ADR-059: OSC 8 Over Alternative Approaches

**Context:** zetl needs clickable file references in terminal output. Several approaches exist for connecting terminal output to editors.

**Decision:** Use OSC 8 hyperlinks with editor-specific URI schemes.

**Rationale:**
- OSC 8 is a terminal standard (originated by iTerm2, adopted by all major terminals) — not a proprietary extension
- URI schemes for editors (vscode://, cursor://, zed://) are established and widely supported
- No daemon, no socket, no IPC — the terminal handles the click entirely
- Graceful degradation: non-supporting terminals show identical output (the sequences are invisible)
- Zero dependencies: OSC 8 is a byte sequence, not a library call

**Trade-offs:**
- Terminals inside tmux < 3.4 may not pass through OSC 8 (mitigated: tmux 3.4+ supports passthrough; older tmux strips sequences cleanly)
- Some editors (vanilla vim/neovim in terminal) lack native URI handlers — users must configure a system-level handler or set `ZETL_EDITOR_URI` to `file://`
- Column information is rarely available from zetl's output (most references are file:line only) — `{col}` defaults to 1

**Alternatives rejected:**
- **ANSI hyperlinks via terminal-link crate:** This is OSC 8 — the crate just wraps the same byte sequence. No value in adding a dependency for string concatenation.
- **Editor-specific IPC (LSP, nvim --remote, emacsclient):** Requires detecting running editor instances, managing sockets, handling timeouts. Far more complex and fragile than URI schemes.
- **File path copying to clipboard on click:** Non-standard behavior; requires terminal-specific APIs; not what users expect from clickable text.
- **No hyperlinks (status quo):** Every other modern CLI tool that prints file references (cargo, ripgrep via hyperlink flag, gh, delta) is adopting OSC 8. zetl should not lag.

---

## 8. Test Specifications

### TEST-143: OSC 8 Wrapping in Search Results

**Scenario:** `zetl search` output contains OSC 8 hyperlinks around file:line references.
**Precondition:** Vault with at least one note containing the search term. `ZETL_EDITOR_URI` is unset. `$EDITOR=code`.
**Steps:**
1. Simulate TTY stdout (set `is_terminal` to true in test harness)
2. Run `zetl search "test_term"` and capture raw bytes
3. Verify output contains `\x1b]8;;vscode://file/` followed by the absolute path, line, and col
4. Verify the visible text between the open and close sequences is the file:line reference
5. Verify the close sequence `\x1b]8;;\x1b\\` follows the visible text

### TEST-144: OSC 8 Suppressed When Piped

**Scenario:** OSC 8 sequences are absent when stdout is not a TTY.
**Precondition:** Same vault as TEST-143.
**Steps:**
1. Run `zetl search "test_term"` with stdout piped (not a TTY)
2. Capture raw output bytes
3. Verify output contains zero `\x1b]8` sequences
4. Verify file:line references are still present as plain text

### TEST-145: ZETL_EDITOR_URI Override

**Scenario:** Custom editor URI template overrides auto-detection.
**Precondition:** `ZETL_EDITOR_URI="custom://open?file={path}&line={line}"`, `$EDITOR=code`.
**Steps:**
1. Simulate TTY stdout
2. Run `zetl check` on a vault with dead links
3. Verify OSC 8 URIs use `custom://open?file=...&line=...` (not vscode://)
4. Verify `{path}` is expanded to the absolute file path
5. Verify `{line}` is expanded to the correct line number

### TEST-146: Editor Auto-Detection

**Scenario:** Each known editor basename maps to the correct URI template.
**Steps (unit test on `resolve_config`):**
1. For each preset pair `(basename, expected_template)`:
   a. Set `$VISUAL` to `/usr/bin/{basename}`
   b. Call `resolve_config(false, false)` with simulated TTY
   c. Assert `config.template` matches `expected_template`
2. Test fallback: unset `$VISUAL` and `$EDITOR` → template is VS Code default
3. Test `$EDITOR` fallback: unset `$VISUAL`, set `$EDITOR=zed` → template is Zed

### TEST-147: Path Percent-Encoding

**Scenario:** File paths with special characters are correctly percent-encoded in URIs.
**Steps (unit test on `format_uri`):**
1. Path: `/vault/my notes/page one.md`, line: 10, col: 1
2. Template: `vscode://file/{path}:{line}:{col}`
3. Expected URI: `vscode://file//vault/my%20notes/page%20one.md:10:1`
4. Verify spaces encoded as `%20`
5. Test path with unicode: `/vault/notizen/Ubersicht.md` → properly encoded
6. Test path with parentheses: `/vault/notes/(archive)/old.md` → `%28archive%29`

### TEST-148: Suppression Conditions

**Scenario:** OSC 8 is suppressed under each disabling condition independently.
**Steps:**
1. `--no-hyperlinks` flag set, stdout is TTY → no OSC 8 sequences in output
2. `NO_COLOR=1` set, stdout is TTY, no `--no-hyperlinks` → no OSC 8 sequences
3. `NO_COLOR=""` (empty string) set, stdout is TTY → no OSC 8 sequences (any value disables)
4. `--format json` specified, stdout is TTY → no OSC 8 sequences (JSON is never escaped)
5. All conditions absent, stdout is TTY → OSC 8 sequences present

### TEST-149: Coverage Across Commands

**Scenario:** All specified commands emit OSC 8 hyperlinks for file references.
**Precondition:** Vault with notes that exercise each command's file-reference output. TTY simulated. Default editor preset.
**Steps:**
1. `zetl search "term"` → verify OSC 8 in output
2. `zetl links "SomePage"` → verify OSC 8 in source file:line references
3. `zetl backlinks "SomePage"` → verify OSC 8 in source file:line references
4. `zetl check` (with dead links present) → verify OSC 8 in diagnostic file:line references
5. `zetl diff` (with changes) → verify OSC 8 in file path references
6. `zetl blocks "SomePage"` → verify OSC 8 in block file:line references
7. For each command, also verify `--format json` output contains zero OSC 8 sequences

---

## 9. Traceability Matrix

| Requirement | Tests                  | Contracts              | NFRs    | ADRs    |
|-------------|------------------------|------------------------|---------|---------|
| REQ-120     | TEST-143, TEST-144, TEST-148 | CON-022-001, CON-022-004 | NFR-047, NFR-048 | ADR-059 |
| REQ-121     | TEST-145, TEST-147     | CON-022-003, CON-022-005 | —       | —       |
| REQ-122     | TEST-145, TEST-146     | CON-022-002             | —       | ADR-059 |
| REQ-123     | TEST-147               | CON-022-003             | —       | —       |
| REQ-124     | TEST-143, TEST-149     | —                       | —       | —       |
| REQ-125     | TEST-148               | CON-022-002             | —       | —       |
| REQ-126     | TEST-148               | CON-022-002             | —       | ADR-059 |

---

## 10. Future Work

- **TUI integration (zetl view):** Ratatui cells can embed OSC 8 via raw spans. This requires changes to the view module's rendering pipeline and is deferred to a future spec.
- **Terminal capability detection:** Probing whether the terminal actually supports OSC 8 (via `TERM_PROGRAM` or DA1 queries) could allow zetl to suppress sequences on known-incompatible terminals. Deferred because the graceful-degradation guarantee of OSC 8 makes this low-priority.
- **`id=` parameter for multi-line links:** OSC 8 supports an `id=` parameter that groups multiple hyperlink spans as a single logical link (useful for wrapped lines). If users report issues with long paths wrapping across terminal lines, this can be added without breaking changes.
- **Config file support:** `ZETL_EDITOR_URI` could also be set in `.zetl/config.toml` for per-vault configuration. Deferred until zetl has a general config file mechanism.

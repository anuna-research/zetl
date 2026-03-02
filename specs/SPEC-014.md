---
title: "SPEC-014: zetl theme — Theme Distribution and Installation"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-03-02
---

# SPEC-014: zetl theme — Theme Distribution and Installation

## Information Table

| Field          | Value                                                          |
| -------------- | -------------------------------------------------------------- |
| Document ID    | SPEC-014                                                       |
| Title          | zetl theme — Theme Distribution and Installation               |
| Version        | 0.1.0                                                          |
| Status         | Draft                                                          |
| Author         | Agent (USDD Protocol v1.3.0)                                   |
| Date           | 2026-03-02                                                     |
| Audience       | Agent, Human                                                   |
| Trace          | USDD Agent Protocol v1.3.0                                     |
| Parent         | SPEC-012: zetl — Named Themes for Serve and Build              |
| Related        | SPEC-001: zetl — Bi-directional Link Graph CLI                 |

---

## 1. Overview

SPEC-012 established named themes under `.zetl/themes/<name>/` with a two-tier template resolution (user theme → built-in default). That specification explicitly deferred theme distribution:

> **Out of scope:** Theme packaging or distribution (future SPEC — theme marketplace, `zetl theme install`)

This specification addresses that gap. It defines how themes are authored, discovered, installed, and managed — giving users a way to share and reuse themes without manually copying directories.

### 1.1 Core Insight

Themes are small collections of Minijinja templates and static assets. They are naturally version-controlled artifacts. Git is already universally available to zetl users (the tool targets knowledge workers who publish vaults). Rather than inventing a theme registry or package format, zetl uses **git references** as the distribution mechanism — any git repository (or subdirectory within one) can be a theme source. A curated set of **bundled themes** ships inside the zetl binary for zero-network-required defaults.

### 1.2 Design Philosophy

1. **Git is the registry** — no proprietary hosting, no API keys, no accounts. A theme is a git repo (or a path within one). Users install themes the same way they install anything else in their workflow.
2. **Bundled themes are first-class** — the zetl binary ships with a small set of official themes beyond `default`. These are always available, even offline, and serve as reference implementations for theme authors.
3. **Themes are inert files** — a theme is templates + static assets + a manifest. No executable hooks, no build steps, no post-install scripts. This eliminates supply-chain risk.
4. **Offline-first** — once installed, a theme works without network access. The install step is the only operation that touches the network.
5. **Vault-local** — themes install into `.zetl/themes/<name>/`, exactly where SPEC-012 already looks for them. No global theme store, no cross-vault interference.

### 1.3 Scope

**In scope:**

- `theme.toml` manifest file for theme metadata (name, version, author, description, zetl compatibility)
- `zetl theme install <source>` command to install themes from git references
- `zetl theme list` command to enumerate available themes (bundled + installed)
- `zetl theme remove <name>` command to uninstall a theme
- `zetl theme export <name>` command to extract a bundled theme to `.zetl/themes/` for customisation
- Bundled theme system: official themes embedded in the zetl binary via a `themes/` directory in the source repository
- Theme source resolution: git URL, GitHub shorthand (`user/repo`), and optional ref/path specifiers

**Out of scope:**

- Theme marketplace or central registry (discovery is via documentation, GitHub search, or word of mouth)
- Theme compilation or build steps (Sass, TypeScript, etc. — users bring pre-compiled assets)
- Theme dependencies on other themes (no theme inheritance chain beyond Minijinja `{% extends %}`)
- Automatic theme updates (`zetl theme install` is explicit; users re-install to update)
- Theme configuration variables (deferred — `theme.toml` carries metadata, not user-configurable variables)

---

## 2. User Profiles

### 2.1 Vault Publisher

```
Role: Knowledge worker publishing a digital garden or documentation site
Goals:
  - Browse available themes and install one that matches their aesthetic
  - Switch between themes without losing content
  - Update to a newer version of an installed theme
Constraints:
  - May not be deeply technical; expects simple CLI commands
  - Works offline frequently (airplane, commute)
  - Uses git daily but does not want to manage submodules
```

**Happy Path: Install and Use a Community Theme**

```
Preconditions: Vault exists with .zetl/ directory. Network available.
Steps:
  1. zetl theme list → sees bundled themes (default, minimal, docs)
  2. zetl theme install aesop-themes/garden → installs from GitHub
  3. zetl serve --theme garden → previews the theme
  4. zetl build --theme garden → builds static site with the theme
Postconditions: Theme files exist in .zetl/themes/garden/
Failure modes:
  - Network unavailable → clear error with hint to check connectivity
  - Repo not found → clear error with the resolved URL
  - Theme missing theme.toml → warning (not fatal), installs as bare template directory
```

### 2.2 Theme Author

```
Role: Developer who creates themes for the zetl community
Goals:
  - Create a theme as a standalone git repository
  - Include metadata (name, description, compatibility) for discoverability
  - Test the theme locally before publishing
  - Support users installing via a single command
Constraints:
  - Wants minimal ceremony — create repo, add templates, push
  - Does not want to register with a central authority
  - May host themes in a monorepo with multiple themes
```

**Happy Path: Publish a Theme**

```
Preconditions: Theme author has a working theme in .zetl/themes/my-theme/
Steps:
  1. Create theme.toml with name, version, description, author
  2. git init, commit templates + static/ + theme.toml
  3. Push to GitHub as my-org/zetl-theme-clean
  4. Users install via: zetl theme install my-org/zetl-theme-clean
Postconditions: Theme is installable by anyone with the git URL
Failure modes:
  - Missing theme.toml → installs but zetl theme list shows "(no manifest)"
  - Incompatible zetl version → warning at install time, not a hard block
```

### 2.3 Monorepo Theme Author

```
Role: Developer maintaining multiple themes in a single repository
Goals:
  - Publish several themes from one repo (e.g., a "themes" collection)
  - Let users install individual themes by path
Constraints:
  - Each theme is a subdirectory with its own theme.toml
  - Does not want to maintain separate repos per theme
```

**Happy Path: Install from a Monorepo**

```
Preconditions: Monorepo at github.com/acme/zetl-themes with themes/garden/ and themes/minimal/
Steps:
  1. zetl theme install acme/zetl-themes --path themes/garden
  2. zetl theme install acme/zetl-themes --path themes/minimal
Postconditions: .zetl/themes/garden/ and .zetl/themes/minimal/ exist
Failure modes:
  - Path does not exist in repo → clear error listing available subdirectories
```

---

## 3. Requirements

### 3.1 Theme Manifest

REQ-014-001: Theme Manifest File

The system SHALL recognise a `theme.toml` file in the root of a theme directory as the theme manifest. The manifest is OPTIONAL — themes without a manifest are valid but limited in metadata.

```toml
# theme.toml — theme manifest
[theme]
name = "garden"                     # Required. Display name. Must match directory name when installed.
version = "1.0.0"                   # Required. SemVer.
description = "A warm, organic theme for digital gardens"  # Optional.
author = "Jane Doe <jane@example.com>"                     # Optional.
license = "MIT"                     # Optional.
homepage = "https://github.com/jane/zetl-garden"           # Optional.
min_zetl_version = "0.9.0"         # Optional. Minimum zetl version for compatibility.

[theme.templates]
# Declares which templates this theme overrides. Informational only — the
# template engine still uses its two-tier resolution regardless of this list.
overrides = ["base.html", "page.html"]
```

Trace:
- TEST-014-001
- CON-014-001

---

REQ-014-002: Manifest Validation

The system SHALL validate `theme.toml` at install time:

- `theme.name` MUST be a non-empty string matching `^[a-z0-9][a-z0-9_-]*$` (lowercase, hyphens, underscores, no leading special chars).
- `theme.version` MUST be valid SemVer if present.
- `theme.min_zetl_version`, if present, SHALL be compared against the running zetl version. If the running version is older, the system SHALL print a warning but SHALL NOT block installation.
- Unknown keys SHALL be ignored (forward compatibility).

Trace:
- TEST-014-002

---

### 3.2 Bundled Themes

REQ-014-003: Bundled Theme Directory

The zetl source repository SHALL contain a `themes/` directory at the repository root. Each subdirectory is a complete theme with the same structure as `.zetl/themes/<name>/`:

```
themes/
  default/           # The current built-in templates, extracted into theme form
    theme.toml
    base.html
    index.html
    page.html
    folder.html
  minimal/           # A stripped-down theme with minimal styling
    theme.toml
    base.html
    index.html
    page.html
    folder.html
  docs/              # A documentation-oriented theme
    theme.toml
    base.html
    ...
```

Trace:
- TEST-014-003

---

REQ-014-004: Bundled Theme Embedding

The zetl binary SHALL embed all themes from the `themes/` source directory at compile time. The template engine's fallback chain (SPEC-012 Tier 2) SHALL resolve built-in templates from the active bundled theme rather than only from hardcoded `include_str!()` calls.

The resolution order becomes:

1. **Tier 1 (User Theme):** `.zetl/themes/<active-theme>/<template>.html` on disk
2. **Tier 2 (Bundled Theme):** Embedded templates for `<active-theme>` from the `themes/` compile-time directory
3. **Tier 3 (Default Fallback):** Embedded templates from `themes/default/`

When `--theme default` (or no `--theme`), Tier 1 is skipped and Tier 2 = Tier 3.

Trace:
- TEST-014-004
- ADR-014-001

---

REQ-014-005: Bundled Theme Listing

`zetl theme list` SHALL distinguish between bundled themes, installed themes (in `.zetl/themes/`), and themes that shadow a bundled theme (an installed theme with the same name as a bundled one). The output SHALL indicate the source of each theme.

Trace:
- TEST-014-005

---

### 3.3 Git-Based Installation

REQ-014-006: Install from Git URL

`zetl theme install <source>` SHALL accept the following source formats:

| Format | Example | Resolution |
| --- | --- | --- |
| GitHub shorthand | `user/repo` | `https://github.com/user/repo.git` |
| HTTPS URL | `https://git.example.com/repo.git` | Used directly |
| SSH URL | `git@github.com:user/repo.git` | Used directly |

The system SHALL clone the repository (shallow, depth 1) into a temporary directory, then copy the theme files into `.zetl/themes/<name>/`.

Trace:
- TEST-014-006
- CON-014-002

---

REQ-014-007: Git Ref Specifier

The source MAY include a ref specifier using `#<ref>` syntax:

```
zetl theme install user/repo#v2.0.0      # tag
zetl theme install user/repo#main        # branch
zetl theme install user/repo#abc1234     # commit SHA
```

When no ref is specified, the system SHALL use the repository's default branch.

Trace:
- TEST-014-007

---

REQ-014-008: Subdirectory Path

The `--path <dir>` flag SHALL specify a subdirectory within the cloned repository to use as the theme root. This enables monorepo workflows where multiple themes live in a single repository.

```
zetl theme install acme/zetl-themes --path themes/garden
```

The system SHALL verify the path exists within the cloned repository. If not, the system SHALL list available subdirectories (up to 1 level deep) that contain a `theme.toml` or template files, as a hint.

Trace:
- TEST-014-008

---

REQ-014-009: Theme Name Resolution

The installed theme name SHALL be determined by the following precedence:

1. `--name <name>` flag (explicit override)
2. `theme.name` from `theme.toml` in the resolved theme directory
3. The last path component of `--path`, if specified
4. The repository name from the source URL

The name MUST pass the same validation as REQ-014-002 (`^[a-z0-9][a-z0-9_-]*$`). If it does not, the system SHALL suggest a sanitised alternative and require `--name` to proceed.

Trace:
- TEST-014-009

---

REQ-014-010: Install Overwrites

If `.zetl/themes/<name>/` already exists, the system SHALL refuse to overwrite unless `--force` is passed. With `--force`, the existing directory is replaced entirely (not merged).

Trace:
- TEST-014-010

---

REQ-014-011: Provenance Tracking

After installation, the system SHALL write a `.zetl-source.toml` file inside the installed theme directory recording the installation provenance:

```toml
# .zetl-source.toml — auto-generated, do not edit
[source]
url = "https://github.com/user/repo.git"
ref = "v2.0.0"
commit = "abc1234def5678..."
path = "themes/garden"
installed_at = "2026-03-02T14:30:00Z"
zetl_version = "0.9.0"
```

This file enables `zetl theme list` to show where a theme came from and supports future `zetl theme update` functionality (out of scope for this spec).

Trace:
- TEST-014-011

---

### 3.4 Theme Management Commands

REQ-014-012: `zetl theme list`

The system SHALL provide a `zetl theme list` subcommand that outputs all available themes. Default output is JSON; `-f table` produces a human-readable table.

**JSON schema:**

```json
{
  "themes": [
    {
      "name": "default",
      "source": "bundled",
      "version": "0.9.0",
      "description": "The default zetl theme",
      "active": true
    },
    {
      "name": "garden",
      "source": "installed",
      "version": "1.0.0",
      "description": "A warm, organic theme",
      "origin": "https://github.com/jane/zetl-garden.git#v1.0.0",
      "active": false
    }
  ]
}
```

**Table output:**

```
 Name     Source     Version  Description
 default  bundled    0.9.0    The default zetl theme
 garden   installed  1.0.0    A warm, organic theme (github.com/jane/zetl-garden)
 minimal  bundled    0.9.0    A minimal, typography-focused theme
```

The `source` field SHALL be one of: `bundled`, `installed`, `installed (shadows bundled)`.

Trace:
- TEST-014-012
- CON-014-003

---

REQ-014-013: `zetl theme remove <name>`

The system SHALL provide a `zetl theme remove <name>` subcommand that deletes `.zetl/themes/<name>/` and all its contents. The system SHALL refuse to remove bundled themes (they are embedded in the binary and cannot be deleted). If the theme shadows a bundled theme, the system SHALL warn that the bundled version will become active after removal.

Trace:
- TEST-014-013

---

REQ-014-014: `zetl theme export <name>`

The system SHALL provide a `zetl theme export <name>` subcommand that copies a bundled theme's files into `.zetl/themes/<name>/`. This allows users to customise a bundled theme as a starting point. If the target directory already exists, the system SHALL refuse unless `--force` is passed.

When used with a non-bundled theme name, the system SHALL return an error explaining that only bundled themes can be exported (installed themes are already on disk).

Trace:
- TEST-014-014

---

### 3.5 Security

REQ-014-015: No Executable Content

Theme installation SHALL NOT execute any scripts, hooks, or build steps from the theme source. The install process is strictly: clone → copy files → write provenance. If a theme directory contains executable files (`.sh`, `.py`, etc.), they SHALL be copied as inert files and never executed by zetl.

Trace:
- TEST-014-015

---

REQ-014-016: Path Traversal Protection

Theme names and `--path` values SHALL be validated to prevent path traversal. Names containing `/`, `\`, `..`, or null bytes SHALL be rejected. This extends the existing validation in `validate_theme()`.

Trace:
- TEST-014-016

---

### 3.6 Non-Functional Requirements

NFR-014-001: Install Latency

Theme installation SHALL complete in ≤ 10 seconds for a theme repository under 10 MB on a 10 Mbps connection, using shallow clone (depth 1).

Trace:
- TEST-014-NFR-001

---

NFR-014-002: Binary Size Impact

Bundled themes SHALL add ≤ 100 KB to the compressed binary size. Themes consist of HTML templates and small CSS/JS files; images and fonts are not bundled.

Trace:
- TEST-014-NFR-002

---

NFR-014-003: Offline Operation

All theme operations except `zetl theme install` SHALL work without network access. Bundled themes and installed themes are fully local.

Trace:
- TEST-014-NFR-003

---

## 4. Architecture Decisions

### ADR-014-001: Git References over Package Registry

**Context:** Theme distribution requires a mechanism for users to discover, download, and install themes. Options include: (a) a central registry (npm-style), (b) git repositories as the distribution unit, (c) tarballs/zips from URLs, (d) a zetl-specific package format.

**Decision:** Use git repositories as the sole external distribution mechanism, with GitHub shorthand as syntactic sugar.

**Rationale:**

- **Zero infrastructure** — no registry to host, secure, or maintain. No API keys or accounts.
- **Versioning is built-in** — git refs (tags, branches, commits) provide immutable version pinning.
- **Familiar workflow** — zetl users already use git. `zetl theme install user/repo#v2` reads naturally.
- **Monorepo support** — the `--path` flag handles collections elegantly.
- **Precedent** — Hugo (`hugo mod get`), Zola (git submodules), Go modules, and Terraform providers all use git-based distribution successfully.

**Trade-offs:**

- No dependency resolution (themes cannot declare dependencies on other themes). Acceptable because themes are self-contained by design.
- No search/discovery built into the CLI. Acceptable because GitHub search, awesome-lists, and documentation fill this role.
- Requires git on the user's machine. Acceptable because zetl's target audience universally has git installed.

**Alternatives rejected:**

- Central registry: high maintenance burden, single point of failure, overkill for the ecosystem size.
- Tarball URLs: no versioning, no provenance, harder to update.
- Git submodules: too complex for end users, pollutes vault's `.gitmodules`, awkward for non-git vaults.

---

### ADR-014-002: Bundled Themes as Compile-Time Embedded Files

**Context:** The current `default` theme is embedded via `include_str!()` in `engine.rs`. To support multiple bundled themes, we need a scalable embedding strategy.

**Decision:** Move built-in templates from `src/web/templates/` into `themes/default/` at the repository root. Use a `build.rs` script (or `include_dir` crate) to embed all themes under `themes/` at compile time. The template engine resolves bundled theme templates from this embedded directory tree.

**Rationale:**

- **Single source of truth** — bundled themes live as normal theme directories, editable and testable like any user theme.
- **Extractable** — `zetl theme export default` can write the exact files that ship in the binary, no desync risk.
- **Scalable** — adding a new bundled theme is: create directory, add files, rebuild.
- **Testable** — integration tests can use the real bundled theme files.

**Trade-offs:**

- Adds a build-time step (the `build.rs` or `include_dir!` macro). Minor complexity increase.
- Bundled themes increase binary size. Mitigated by NFR-014-002 (≤ 100 KB budget).

---

### ADR-014-003: Shallow Clone for Installation

**Context:** Cloning a full repository with history is wasteful when we only need the latest snapshot of template files.

**Decision:** Use `git clone --depth 1 [--branch <ref>]` for installation. If a commit SHA is specified (not a branch/tag), fall back to a full clone followed by `git checkout <sha>` (shallow clone does not support arbitrary SHAs on all servers).

**Rationale:**

- Shallow clone is dramatically faster and uses less disk/bandwidth.
- Commit SHA fallback ensures all ref types work, at the cost of a slower clone for that case.

---

## 5. Contracts

### CON-014-001: theme.toml Schema

```toml
[theme]
name = "string"               # Required. ^[a-z0-9][a-z0-9_-]*$
version = "string"            # Required. Valid SemVer.
description = "string"        # Optional.
author = "string"             # Optional.
license = "string"            # Optional. SPDX identifier.
homepage = "string"           # Optional. URL.
min_zetl_version = "string"   # Optional. SemVer — minimum compatible zetl version.

[theme.templates]
overrides = ["string"]        # Optional. List of template filenames this theme overrides.
```

Implements:
- REQ-014-001
- REQ-014-002

Verified by:
- TEST-014-001
- TEST-014-002

---

### CON-014-002: `zetl theme install` CLI Interface

```
zetl theme install <source> [--path <dir>] [--name <name>] [--force]

Arguments:
  <source>    Theme source. One of:
              - GitHub shorthand: user/repo
              - HTTPS git URL:   https://host/repo.git
              - SSH git URL:     git@host:user/repo.git
              May include #<ref> suffix for branch, tag, or commit.

Options:
  --path <dir>   Subdirectory within the repository to use as theme root.
  --name <name>  Override the installed theme directory name.
  --force        Overwrite existing theme directory without prompting.

Exit codes:
  0   Success
  1   Installation failed (network error, repo not found, validation error)

Stdout (JSON):
  {
    "installed": {
      "name": "garden",
      "version": "1.0.0",
      "source": "https://github.com/jane/zetl-garden.git",
      "ref": "v1.0.0",
      "path": ".zetl/themes/garden"
    }
  }
```

Pre-conditions:
- `.zetl/` directory exists in the vault root (or is created automatically)
- `git` is available on `$PATH`

Post-conditions:
- `.zetl/themes/<name>/` contains the theme files
- `.zetl/themes/<name>/.zetl-source.toml` records provenance

Error model:
- Network unreachable: exit 1, stderr message with hint
- Repository not found: exit 1, stderr message with resolved URL
- Path not found in repo: exit 1, stderr lists available theme directories
- Name collision without --force: exit 1, stderr suggests --force
- Invalid theme name: exit 1, stderr suggests sanitised alternative with --name

Implements:
- REQ-014-006
- REQ-014-007
- REQ-014-008
- REQ-014-009
- REQ-014-010

Verified by:
- TEST-014-006 through TEST-014-010

---

### CON-014-003: `zetl theme list` CLI Interface

```
zetl theme list

Output (JSON, default):
  {
    "themes": [
      {
        "name": "string",
        "source": "bundled" | "installed" | "installed (shadows bundled)",
        "version": "string | null",
        "description": "string | null",
        "origin": "string | null"
      }
    ]
  }

Output (table, with -f table):
  Name     Source     Version  Description
  <name>   <source>  <ver>    <desc> [(<origin>)]

Exit codes:
  0   Always (listing cannot fail)
```

Implements:
- REQ-014-012

Verified by:
- TEST-014-012

---

## 6. Test Specifications

### TEST-014-001: Manifest Parsing

Scenario: Parse a valid `theme.toml` with all fields populated.
Expected: All fields are extracted correctly. Unknown keys are ignored.

Scenario: Parse a `theme.toml` with only required fields (`name`, `version`).
Expected: Optional fields default to None/empty.

Scenario: Theme directory with no `theme.toml`.
Expected: Theme is usable. Metadata fields report as unknown/none.

---

### TEST-014-002: Manifest Validation

Scenario: `theme.name` contains uppercase, spaces, or special characters.
Expected: Validation fails with a descriptive error.

Scenario: `theme.version` is not valid SemVer (e.g., "1.0" or "latest").
Expected: Validation fails.

Scenario: `theme.min_zetl_version` is newer than running zetl.
Expected: Warning printed, installation proceeds.

---

### TEST-014-003: Bundled Theme Directory Structure

Scenario: `themes/default/` in the source repo contains `base.html`, `index.html`, `page.html`, `folder.html`, and `theme.toml`.
Expected: All files present and parseable.

---

### TEST-014-004: Three-Tier Template Resolution

Scenario: `--theme garden` where `garden` is bundled. No `.zetl/themes/garden/` on disk.
Expected: Templates resolve from bundled `themes/garden/`.

Scenario: `--theme garden` where `garden` is bundled AND `.zetl/themes/garden/page.html` exists on disk.
Expected: `page.html` resolves from disk (Tier 1), other templates from bundled (Tier 2).

Scenario: `--theme custom` where `custom` is not bundled. `.zetl/themes/custom/page.html` exists.
Expected: `page.html` from disk (Tier 1), all others from `themes/default/` (Tier 3).

---

### TEST-014-005: Theme Listing

Scenario: No `.zetl/themes/` directory exists.
Expected: Only bundled themes listed, all with `source: "bundled"`.

Scenario: `.zetl/themes/garden/` exists with `theme.toml`.
Expected: Listed as `source: "installed"` with metadata from manifest.

Scenario: `.zetl/themes/minimal/` exists and `minimal` is also bundled.
Expected: Listed as `source: "installed (shadows bundled)"`.

---

### TEST-014-006: Install from GitHub Shorthand

Scenario: `zetl theme install user/repo` where repo contains `theme.toml` at root.
Expected: Theme installed to `.zetl/themes/<name>/`. Provenance file written.

---

### TEST-014-007: Install with Git Ref

Scenario: `zetl theme install user/repo#v2.0.0` where tag `v2.0.0` exists.
Expected: Theme installed from that tag. Provenance records the ref and resolved commit SHA.

---

### TEST-014-008: Install with Subdirectory Path

Scenario: `zetl theme install user/monorepo --path themes/garden`.
Expected: Only `themes/garden/` contents copied to `.zetl/themes/garden/`.

Scenario: `zetl theme install user/monorepo --path nonexistent`.
Expected: Error with hint listing available subdirectories.

---

### TEST-014-009: Name Resolution Precedence

Scenario: `--name my-garden` flag provided.
Expected: Installed as `.zetl/themes/my-garden/` regardless of manifest name.

Scenario: No `--name`, manifest says `name = "garden"`.
Expected: Installed as `.zetl/themes/garden/`.

Scenario: No `--name`, no manifest, `--path themes/garden`.
Expected: Installed as `.zetl/themes/garden/`.

Scenario: No `--name`, no manifest, no `--path`, source is `user/zetl-garden`.
Expected: Installed as `.zetl/themes/zetl-garden/`.

---

### TEST-014-010: Install Overwrite Protection

Scenario: `.zetl/themes/garden/` already exists, `zetl theme install user/repo` resolves to `garden`.
Expected: Error, existing directory not modified.

Scenario: Same as above with `--force`.
Expected: Existing directory replaced. New provenance file written.

---

### TEST-014-011: Provenance File

Scenario: Install a theme, read `.zetl-source.toml` from the installed directory.
Expected: Contains `url`, `commit` (full SHA), `installed_at` (ISO 8601), `zetl_version`.

---

### TEST-014-012: List Command Output

Scenario: `zetl theme list` with both bundled and installed themes.
Expected: JSON output matches CON-014-003 schema. Table output is human-readable.

---

### TEST-014-013: Remove Command

Scenario: `zetl theme remove garden` where `garden` is installed.
Expected: `.zetl/themes/garden/` deleted. Success message.

Scenario: `zetl theme remove default`.
Expected: Error — cannot remove a bundled theme.

---

### TEST-014-014: Export Command

Scenario: `zetl theme export default`.
Expected: Bundled `default` theme files written to `.zetl/themes/default/`.

Scenario: `zetl theme export default` when `.zetl/themes/default/` already exists.
Expected: Error, suggests `--force`.

Scenario: `zetl theme export garden` where `garden` is installed (not bundled).
Expected: Error — only bundled themes can be exported.

---

### TEST-014-015: No Executable Execution

Scenario: Theme repo contains `postinstall.sh` alongside templates.
Expected: File is copied but never executed. No shell invocation during install.

---

### TEST-014-016: Path Traversal Rejection

Scenario: `zetl theme install user/repo --name "../../../etc"`.
Expected: Error — invalid theme name.

Scenario: `zetl theme install user/repo --path "../../secrets"`.
Expected: Error — path traversal detected.

---

## 7. Observability

OBS-014-001: Theme Install Metrics

When `--verbose` is active, theme installation SHALL log:
- Resolved git URL
- Ref and resolved commit SHA
- Clone duration (ms)
- Number of files copied
- Total size of installed theme (bytes)

---

OBS-014-002: Theme Resolution Logging

When `--verbose` is active, the template engine SHALL log which tier each template was resolved from:

```
  theme: base.html  ← .zetl/themes/garden/base.html (disk)
  theme: index.html ← bundled:garden/index.html
  theme: page.html  ← bundled:default/page.html (fallback)
```

---

## 8. Phased Implementation

### Phase 1: Bundled Themes and `theme list`

**Goal:** Move existing templates into `themes/default/`, add a build-time embedding mechanism, create at least one additional bundled theme (`minimal`), and implement `zetl theme list`.

**Changes:**
- Create `themes/default/` with current templates + `theme.toml`
- Create `themes/minimal/` with a stripped-down theme
- Add `build.rs` or `include_dir!` for compile-time embedding
- Refactor `engine.rs` template loader to support three-tier resolution
- Add `zetl theme list` subcommand to `cli.rs`
- Update `validate_theme()` to accept bundled theme names

**Verification:** Existing tests pass. `zetl theme list` shows bundled themes. `zetl serve --theme minimal` renders correctly.

### Phase 2: Git-Based Install and Remove

**Goal:** Implement `zetl theme install` and `zetl theme remove`.

**Changes:**
- Add `theme` subcommand group to `cli.rs`
- Implement git clone + file copy logic
- Implement source format parsing (GitHub shorthand, URLs, #ref, --path)
- Implement name resolution (REQ-014-009)
- Write `.zetl-source.toml` provenance
- Implement `zetl theme remove`

**Verification:** Install from a real GitHub repo. Verify provenance file. Remove and verify cleanup.

### Phase 3: Export and Polish

**Goal:** Implement `zetl theme export`, add a `docs` bundled theme, polish error messages.

**Changes:**
- Implement `zetl theme export`
- Create `themes/docs/` bundled theme
- Improve error messages for all failure modes
- Add verbose logging (OBS-014-001, OBS-014-002)

**Verification:** Export default theme, modify it, use as custom theme. End-to-end workflow test.

---

## 9. Open Questions

1. **Should bundled themes include static assets?**
   Bundled themes that include CSS or JS beyond what's in the Tailwind CDN would increase binary size. The `minimal` theme could be pure template overrides (no extra assets), staying within the 100 KB budget. A `docs` theme might need a small CSS file. Recommendation: allow small static assets in bundled themes but enforce the size budget in CI.

2. **Should `zetl theme install` support non-git sources (tarball URL, local path)?**
   A local path (`zetl theme install ./path/to/theme`) would be useful for development. A tarball URL would support non-git hosting. Recommendation: add `--local <path>` as a convenience alias for `cp -r`, defer tarball support.

3. **Should there be a `zetl theme update` command?**
   With provenance tracking (`.zetl-source.toml`), a future `zetl theme update` could re-install from the same source at a newer ref. This spec defers the command but establishes the provenance data it would need.

4. **Should the `--theme` flag accept a git reference directly?**
   E.g., `zetl serve --theme github:user/repo#v2` that auto-installs on first use. This would blur the line between install and use. Recommendation: keep install explicit — `zetl theme install` then `zetl serve --theme name`.

---

## 10. Future Considerations

| Feature | Description |
| --- | --- |
| `zetl theme update` | Re-install from provenance source at latest ref or specified ref |
| Theme variables | `theme.toml` `[variables]` section exposing configurable values to templates |
| Theme preview | `zetl theme preview <source>` — install to temp dir, serve, clean up |
| Theme init | `zetl theme init <name>` — scaffold a new theme directory with boilerplate |
| Awesome list | Curated list of community themes in zetl documentation |
| Theme CI validation | GitHub Action that validates theme structure and renders test pages |

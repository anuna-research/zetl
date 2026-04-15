# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.5] - 2026-04-15

### Fixed

- CI: macOS cross-compile was failing one level deeper than 0.2.4 —
  `libgit2-sys` defaults to Apple Secure Transport (`stransport.c`)
  for HTTPS on darwin targets, which requires a complete
  `Security.framework` that osxcross doesn't ship. The
  `vendored-openssl` feature now also enables `git2/vendored-openssl`,
  routing libgit2's HTTPS through the already-vendored OpenSSL.

## [0.2.4] - 2026-04-15

### Fixed

- CI: macOS (arm64 + x86_64) and Windows release builds were failing
  in `openssl-sys` because no host sysroot OpenSSL exists for those
  cross-compile targets. Adds an opt-in `vendored-openssl` cargo
  feature that statically links a self-contained OpenSSL into the
  binary; enabled in the macOS and Windows release jobs. Linux builds
  continue to dynamic-link against the system libssl.

## [0.2.3] - 2026-04-15

### Fixed

- CI: arm64 (`aarch64-unknown-linux-gnu`) release build was failing in
  the `openssl-sys` build script because the cross-compile step
  installed only the gcc toolchain, not the aarch64 OpenSSL headers.
  Now enables Debian arm64 multiarch + `libssl-dev:arm64` and
  configures `pkg-config` for cross queries. Re-enables shipping arm64
  Linux binaries to `files.anuna.io/zetl/v0.2.3/`.

## [0.2.2] - 2026-04-15

### Changed

- Internal: `cargo fmt` pass across the 0.2.0 / 0.2.1 changesets. No
  behaviour change.

## [0.2.1] - 2026-04-15

### Fixed

- `zetl hook run` / in-process hook execution: retry `spawn()` on Linux
  `ETXTBSY` ("Text file busy") up to 20× with 10ms backoff. Defeats a
  kernel-level race when a hook script is written and immediately
  executed from the same process (common in tests, theme installers,
  and agent tooling that generates hooks on the fly).

## [0.2.0] - 2026-04-15

### Changed (breaking)

- **Vault scan now skips dotdirs by default.** Directories whose name starts with
  `.` (e.g. `.claude/`, `.obsidian/`, `.vscode/`, `.cache/`, `.venv/`,
  `.terraform/`) are no longer walked by `zetl build`, `zetl serve`,
  `zetl index`, `zetl search`, or `zetl watch`. Previously these were scanned
  unless explicitly ignored, causing tool-state and AI-agent scratchpads to
  leak into `dist/` and pollute the link graph and search index. The
  hardcoded force-ignores (`.git/`, `.zetl/`, `node_modules/`) behave as
  before. Dotfiles at the vault root (e.g. `.hidden-note.md`,
  `.zetlignore`, `.gitignore`) are still walked. (SPEC-026)

  **Migration:** if you intentionally publish a dotdir, either pass
  `--include-hidden` or add a negated pattern to a `.zetlignore` file at
  the vault root (e.g. `!.archive/`).

### Added

- **History UI surfaces on the default theme.** Every page now shows an
  inline metadata strip with `Last changed`, a humanised `stable` label
  (e.g. `3d`, `2w`, `9mo`), and a `history` link. A new `/_history`
  route (and `_history.html` under `zetl build`) surfaces vault-wide
  recent changes with a link-count trend sparkline and a
  reverse-chronological list of added / modified / removed pages. The
  sidebar gains a "Recent changes" link. All surfaces degrade silently
  when history is absent (SPEC-027 / REQ-305). Static builds now emit
  `pages/<slug>/_history.html` alongside `index.html` so deployed vaults
  get the same temporal affordances as `zetl serve`. Requires the
  `history` feature (enabled in default builds).
- `--exclude PATTERN` (repeatable) and `--include-hidden` flags on
  `zetl build`, `zetl index`, `zetl serve`, `zetl search`, and `zetl watch`.
  `--exclude` accepts gitignore-syntax patterns; `--include-hidden` disables
  the new dotdir default while preserving the level-1 `.git/`/`.zetl/`/
  `node_modules/` force-ignore. (SPEC-026)
- `.zetlignore` is now a documented first-class feature. Patterns use
  gitignore syntax and are evaluated relative to the vault root. Negated
  patterns (`!foo`) override the default dotdir exclusion.
- With `--verbose`, the scanner prints one stderr line per skipped path
  with a `reason=` tag (`hardcoded`, `nested-vault`, or `dotdir`) for
  debugging unexpected omissions.

### Fixed

- `zetl stats`: `grounded_spl_blocks` could exceed `spl_blocks` when the theory
  cache outlived deleted SPL blocks. Grounded / grounding counts are now joined
  against the live pipeline so only currently-present blocks are counted.
  (BUG-001)
- `zetl serve`: unknown pages now respond `404 Not Found` instead of `200 OK`.
  The "create this page" body is preserved — only the status code changes —
  so uptime probes, crawlers and monitoring see the correct signal. (BUG-002)
- `zetl build`: accept `--out` and `-o` as aliases for `--out-dir`. (BUG-005)

### Added

- `zetl completions <shell>` — generate shell completion scripts for bash, zsh, fish, elvish, and powershell.
- `zetl man` — generate a roff(7) man page on stdout. `make install` places it at `$(PREFIX)/share/man/man1/zetl.1` so `man zetl` works out of the box.
- `--no-input` global flag for unattended / CI usage; disables interactive prompts such as the `zetl view` page picker.
- Release profile tuning: `lto = true`, `codegen-units = 1`, `strip = true` (cuts binary size ~40%).
- Release pipeline: `release.sh`, `install.sh`, and `.woodpecker/release.yaml` for cross-platform binary distribution via Cloudflare R2.

### Changed

- Global flags (`--json`, `--format`, `--dir`, `--quiet`, `--verbose`, `--no-color`, `--no-cache`, `--at`) now propagate to subcommands — `zetl list --json` works, not just `zetl --json list`.
- JSON error output now goes to stderr instead of stdout, so `zetl … | jq` consumers get clean stdout on both success and failure.
- README: regrouped feature bullets into six themed sections; tagline broadened from "personal knowledge management" to "knowledge management, solo or team" to reflect the multi-user collab feature set.
- `make install` now also installs the man page and bash/zsh/fish completions under `$(PREFIX)/share/`.

### Fixed

- `Cargo.toml` license field corrected from `MIT` to `AGPL-3.0-or-later` (LICENSE file has always been AGPL).
- `Makefile` SPDX header corrected from `MIT` to `AGPL-3.0-or-later`.
- Stale `github.com/anuna/zetl` link in `--help` footer now points at `codeberg.org/anuna/zetl`.

## [0.1.0] — unreleased

Initial public release.

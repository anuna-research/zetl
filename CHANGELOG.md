# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

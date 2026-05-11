# zetl mobile

Tauri Mobile shell for [SPEC-040](../specs/SPEC-040-zetl-mobile.md) —
embeds `zetl serve` (single-user) inside an iOS / Android app and
serves the existing web UI through a WebView. v0.1 supports **multiple
vaults** with a picker UI, a derived BIP39 recovery phrase, optional
vault subpath selection, PAT-based private-HTTPS clones, and a Mobile
section injected into the default theme's sidebar.

> **Status:** v0.1.0-strawman. The desktop dev shell is end-to-end
> working today; iOS / Android builds compile through the standard
> `cargo tauri android|ios` flow but live device testing is the
> user's responsibility (we have no CI on real hardware yet).

## What works today

| Surface | Status |
|---|---|
| Tauri shell boots → embedded `zetl serve` → WebView | ✅ |
| Auto-generate per-device SSH key from a fresh BIP39 mnemonic | ✅ |
| Persist `ssh_key.json` (mnemonic + derived keys, 0o600) | ✅ |
| Restore on subsequent launches | ✅ |
| Onboarding wizard with SSH-key display + direct host links | ✅ |
| Suggest the SSH form when an HTTPS URL is pasted (GitHub/GitLab/Codeberg/Gitea) | ✅ |
| Clone via SSH (BIP39-derived key) | ✅ |
| Clone via HTTPS (public, no auth) | ✅ |
| Clone via HTTPS (private, optional Personal Access Token) | ✅ |
| Vault working trees at `app_data_dir/vaults/<owner-repo>/` | ✅ |
| Active vault via `app_data_dir/vault` symlink (Unix) | ✅ |
| Multi-vault picker (`/_mobile/vaults`) with switch + remove | ✅ |
| Add another vault from picker (`/_mobile/onboarding?add=1`) | ✅ |
| Vault subpath detection + picker (`/_mobile/vaults/pick`) | ✅ |
| Capture (`/_mobile/capture`): atomic write + commit + auto-title | ✅ |
| Capture form: `✕ Cancel` to return to / | ✅ |
| Sync (`/_mobile/sync`): manual Pull (FF-only) + Push | ✅ |
| Sync: per-button busy state (`Pulling…` / `Pushing…` / `Cloning…`) | ✅ |
| Sync: Reset active vault (other vaults preserved) | ✅ |
| Remove vault: in-page modal confirm, server POST wipes local dir | ✅ |
| BIP39 recovery phrase view (`/_mobile/recovery`) — tap-to-reveal | ✅ |
| Sidebar **Mobile** section in default theme (Vaults / Capture / Sync / Recovery) | ✅ |
| Outbound link routing via `tauri-plugin-opener` (OS browser) | ✅ |
| Legacy single-vault → multi-vault layout migration on startup | ✅ |
| Read / edit / search / backlinks / graph | delegated to existing `zetl serve` UI ✅ |

## Architecture

```
WebView ── http://127.0.0.1:23423/<path> ─→ embedded zetl serve
                                              │
                                              ├── existing serve UI (Minijinja, themes, CodeMirror 6)
                                              └── /_mobile/{onboarding, vaults, capture, sync, recovery}
                                              │
                                              ▼
                                            app_data_dir/
                                              ssh_key.json                  ← BIP39 + derived keys (0600)
                                              vault → vaults/<active>       ← symlink (Unix)
                                              vaults/
                                                anuna-zetl/                 ← per-vault working tree
                                                  .git/
                                                  ...
                                                <owner>-<repo>/
                                                  ...
```

The Tauri shell is intentionally tiny: `src/lib.rs` registers
`tauri-plugin-opener`, spawns the embedded serve, programmatically
navigates the WebView to `http://127.0.0.1:23423/_mobile/vaults`
once the listener is bound. `src/serve_lifecycle.rs` calls
`zetl::web::launch_default` which builds a minimal `WebState` and
runs `axum::serve`.

All UI rendering happens in the embedded `zetl serve` — the Mobile
sidebar links, all `/_mobile/*` routes, and the auto-generated
BIP39 management. Tauri-specific glue is limited to the navigate
call and the opener plugin.

## Desktop dev shell

The fastest iteration loop is the desktop build of the Tauri
project — no NDK or simulator setup required.

```bash
make mobile-run             # cargo run --release -p zetl-mobile
# or
cd mobile && cargo run --release
```

The window opens; the WebView programmatically navigates to
`/_mobile/vaults` once the embedded serve binds (typically
<1s). On a fresh install you'll see the empty vaults picker
with an **+ Add another vault** button; click it to enter
onboarding.

App data lives at:

- macOS: `~/Library/Application Support/io.anuna.zetl.mobile/`
- Linux: `~/.local/share/io.anuna.zetl.mobile/`
- Windows: `%APPDATA%\io.anuna.zetl.mobile\`

Wipe it with `make mobile-wipe` to start from a fresh install
state.

## Onboarding flow

1. **First launch:** the Tauri shell auto-generates a fresh BIP39
   mnemonic, derives an ed25519 SSH key from it (SLIP-0010 path
   `m/44'/2'/0'`), and persists everything to `ssh_key.json`. You
   can view + write down the phrase any time via the sidebar's
   **🔑 Recovery** link.
2. Add the displayed `ssh-ed25519 AAAA…` public-key line to your
   git host's SSH-keys page. Direct links to GitHub / GitLab /
   Codeberg are provided on the onboarding screen and open in the
   OS browser via `tauri-plugin-opener`.
3. Paste your vault's git remote URL. **SSH is recommended**
   (`git@host:owner/repo.git`) — the auto-generated key handles
   both clone and ongoing pull/push with no extra setup. If you
   paste an HTTPS URL of a known host, a yellow hint suggests the
   SSH equivalent and offers a one-click rewrite.
4. *(Optional)* For private HTTPS clones, expand **Advanced:
   private HTTPS** and paste a Personal Access Token. The PAT is
   used once and never written to disk; ongoing pull/push for that
   vault will re-prompt (until v0.1.x adds keychain-backed
   credential storage).
5. Tap **Clone vault →**. The repo is cloned into
   `app_data_dir/vaults/<owner-repo>/`. If multiple subdirectories
   look like plausible vaults (each with `.md` files, optional
   `.zetl/` config), you're routed to the subpath picker.
   Otherwise the symlink auto-points at the best candidate (or the
   repo root) and you land on `/` (the vault page list).

### Recovery

`/_mobile/recovery` (sidebar **🔑 Recovery**) shows the 12-word
BIP39 phrase under a tap-to-reveal blur. Write it down somewhere
safe — it's the master secret that lets you re-derive the same
SSH identity on another device via the *Advanced: use my
desktop's BIP39 seed* path in onboarding. Without it, a lost
device means a new identity and re-adding the new pubkey to your
git host.

## Multi-vault

Once cloned, every vault lives under
`app_data_dir/vaults/<label>/` where `<label>` is derived from
the remote URL (`anuna-cooperative-agent-comms-wiki` for
`https://github.com/anuna-cooperative/agent-comms-wiki.git`).

The `/_mobile/vaults` picker (sidebar **⇄ Vaults**) lists every
cloned vault with:

- `● active` tag on the current one
- **Switch** button on every other → repoints the symlink + reindexes
- **Remove** button on every row → opens an in-page modal confirming
  the local wipe (the git remote is untouched; you can re-clone any
  time)

The active-vault symlink (`app_data_dir/vault`) is followed by
the embedded `zetl serve`'s `vault_root`, so switching is a
zero-restart operation — the reindex call swaps the in-memory
`VaultData` and the next request to `/` reflects the new working
tree.

`+ Add another vault` from the picker routes to
`/_mobile/onboarding?add=1` which bypasses the
already-onboarded redirect; the rest of the wizard is identical.

### Subpath picker

Some repos hold the vault content in a subdirectory (`notes/`,
`docs/wiki/`, etc.). After clone, the handler scans the working
tree for plausible vault dirs (any directory with at least one
`.md` file or a `.zetl/` config dir, excluding `node_modules/`,
`target/`, `.git/`, etc.). If the top scorer is unambiguous, the
symlink points there automatically. If multiple candidates
qualify, you're routed to **`/_mobile/vaults/pick?label=<label>`**:

```
Which folder is the vault?
( ) / (repo root) — 3 markdown files
(•) / notes      — 12 markdown files
( ) / docs/wiki  — 8 markdown files ● .zetl/
[ Use this folder → ]
```

Symlink moves to the chosen subpath + reindex.

## Capture

`/_mobile/capture` (sidebar **+ Capture**) is a single textarea
form. Title is optional — auto-filled from the first non-empty
line of content, falling back to `Inbox YYYY-MM-DD-HHMM` (UTC).

On submit: atomic write to the active vault, git commit
(`capture: <slug>`), best-effort push (queued for next online
event when offline). Redirects to the new page after save.

Slug collisions are resolved silently by appending `-2`, `-3`…
to the filename; the title stays verbatim.

The **✕ Cancel** pill in the top-right returns to `/` without
saving anything.

## Sync controls

`/_mobile/sync` (sidebar **⇅ Sync**):

- **Pull (fast-forward only)** — `git fetch` + `merge --ff-only`.
  Non-FF surfaces a "resolve on desktop" banner; push is blocked
  until the next FF pull succeeds.
- **Push** — `git push` with the SSH credential callback. Errors
  bubble up to a banner.
- Both buttons show an immediate busy state (`Pulling…` /
  `Pushing…` + spinner) on click.
- **Switch vault (N other)** — link to `/_mobile/vaults` if any
  other vaults exist; otherwise **Manage vaults**.
- **Reset active vault** (collapsed `<details>`) — wipes the
  active vault's working tree + forgets the SSH key. Other
  vaults are preserved. Confirmed via JS `confirm()` (browser
  default) since this is the destructive escape hatch.

## Android build

### Prerequisites

| Tool                          | Purpose                                       | Verified install                                                  |
| ----------------------------- | --------------------------------------------- | ----------------------------------------------------------------- |
| Android SDK command-line tools| `sdkmanager`, `adb`                           | `brew install --cask android-commandlinetools`                    |
| Android NDK                   | Native cross-compile                          | `"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" 'ndk;27.2.12479018'` |
| Android platform + build-tools| Runtime + APK packaging                       | `sdkmanager 'platforms;android-34' 'build-tools;34.0.0'`          |
| JDK 17                        | Gradle 8.14 max; JDK 25 is rejected           | `brew install openjdk@17`                                         |
| `cargo-tauri` 2.10+           | Build driver                                  | Already installed (`which cargo-tauri`)                           |
| Rust Android targets          | Cross-compile targets                         | Added automatically by `cargo tauri android init`                 |

`mobile/scripts/android-env.sh` auto-detects all of the above and is
sourced by every `make mobile-android-*` recipe — no manual env vars
needed when the tools are at their default Homebrew locations.

#### Accept the SDK licenses (one time)

```bash
yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses
```

### Build

```bash
make mobile-android-init                    # one-time: generates mobile/gen/android/
make mobile-android-build                   # arm64 release APK (default)
make mobile-android-build TARGET=universal  # all four ABIs
make mobile-android-build-debug             # unstripped debug APK (~400 MB)
make mobile-android-dev                     # debug build + run on connected device
```

`mobile-android-build` produces `zetl-mobile-release.apk` at the repo
root, signed with the Android debug keystore (auto-created on first
run). The keystore is the standard sideload-test key — fine for `adb
install` and personal devices. **Without signing, Android 7+ rejects
the install with "App not installed as a package, appears to be
invalid"** — that's why the Makefile chains the sign step in.

For Play Store distribution, configure `gen/android/app/build.gradle.kts`
with a release signing config and skip `mobile-android-sign`.

### Cross-compile quirks the env script handles

1. **`openssl-sys` cross-compile** — the Cargo `mobile` feature enables
   `vendored-openssl` so both `git2`'s HTTPS path and `webauthn-rs`'s
   crypto get a statically-linked OpenSSL.
2. **NDK clang shims** — OpenSSL's vendored `Configure` invokes
   `<target>-clang` / `<target>-ranlib` *without* an API level, but the
   NDK ships only API-suffixed binaries. The script symlinks unsuffixed
   names into `mobile/.android-shims/` (gitignored) and prepends that
   directory to PATH. The default API level is 24 — override with
   `ZETL_NDK_API=29 make mobile-android-build`.
3. **JDK pinning** — Gradle 8.14.3 (current Tauri scaffold) rejects
   class file major version 69 (Java 25). The env script forces
   `JAVA_HOME` to `openjdk@17` when one is installed.

### Size budgets (current)

| Build                | Size  | Notes                                            |
| -------------------- | ----- | ------------------------------------------------ |
| Debug, universal-arm64 | ~436 MB | Unstripped, debug symbols                      |
| Release, universal-arm64 | ~27 MB | `strip = true`, `lto = true`, `codegen-units = 1` |

Further reduction would require pruning desktop features from the
`mobile` Cargo feature set — tantivy, webauthn-rs, the full LSP/MCP
server, history (`jj-lib`), and ratatui-view are pulled in transitively
today and aren't all needed on the phone.

## iOS build

Same shape, requires Xcode + an Apple Developer account for
signed device builds.

```bash
make mobile-ios-init
make mobile-ios-dev
make mobile-ios-build
```

## Known limitations (v0.1-strawman)

- **No share-extension targets** (REQ-4007). iOS Share Extension
  and Android `ACTION_SEND` activity stubs are not yet present;
  capture is FAB-only.
- **No platform secure-element storage.** The persisted
  `ssh_key.json` (now schema v2 carrying the BIP39 mnemonic) is
  on disk with `0o600` perms. iOS Keychain / Android Keystore
  integration is the next slice; behind the same `KeyStore`
  API so no caller changes when it lands.
- **PAT not persisted.** HTTPS PATs are used once for the clone
  and never written to disk — pull/push for HTTPS-cloned vaults
  re-prompts until keychain storage lands. SSH-cloned vaults
  don't hit this issue.
- **Inline HTML for `/_mobile/*`.** The mobile pages are built
  with `format!()` rather than Minijinja templates. They work
  and pick up the same fonts via the OS, but don't yet inherit
  theme typography / colour customisations the way the rest of
  serve does.
- **Hardcoded port 23423.** Loopback-bound. A future slice
  could randomise the port and pass it to the WebView at
  runtime.
- **Symlink layout is Unix-only.** macOS / iOS / Android / Linux
  fine; Windows mobile would need a JSON pointer file instead
  of a symlink. v0.2 problem.

## Test counts (this branch)

```bash
make mobile-test
# →  33 unit + 16 integration = 49 mobile-specific tests
```

Plus 4 existing-route tests cover the non-mobile serve path.

## Implementation map (SPEC-040 trace)

| Component                       | File                              | Status |
| ------------------------------- | --------------------------------- | ------ |
| Tauri shell entry               | `src/lib.rs`                      | ✅ |
| Embed lifecycle                 | `src/serve_lifecycle.rs`          | ✅ |
| WebView dist loader             | `dist/index.html`                 | ✅ (programmatic navigate from Rust) |
| Keys (BIP39 → ed25519)          | `../src/mobile_state.rs`          | ✅ generate + import + persist + restore + mnemonic |
| Git ops (clone/pull/push)       | `../src/mobile_git.rs`            | ✅ git2-rs + SSH + HTTPS PAT |
| Capture (write + commit)        | `../src/mobile_capture.rs`        | ✅ |
| Multi-vault (list/switch/etc.)  | `../src/mobile_state.rs`          | ✅ symlink layout + migration |
| Subpath detection               | `../src/mobile_state.rs`          | ✅ |
| `/_mobile/onboarding{,/seed,/clone}` | `../src/web/mobile.rs`       | ✅ |
| `/_mobile/capture` GET / POST   | `../src/web/mobile.rs`            | ✅ |
| `/_mobile/sync{,/pull,/push}`   | `../src/web/mobile.rs`            | ✅ |
| `/_mobile/reset`                | `../src/web/mobile.rs`            | ✅ |
| `/_mobile/vaults{,/switch,/add,/remove,/pick}` | `../src/web/mobile.rs` | ✅ |
| `/_mobile/recovery`             | `../src/web/mobile.rs`            | ✅ |
| Default-theme sidebar block     | `../themes/default/base.html`     | ✅ (Mobile section) |
| Outbound URL opener             | `tauri-plugin-opener`             | ✅ |
| Keychain (platform)             | not yet                           | ⬜ iOS Keychain / Android Keystore |
| Share-ext intake                | not yet                           | ⬜ iOS / Android platform glue |
| Minijinja templates for `/_mobile/*` | not yet                      | ⬜ theme-aware mobile pages |

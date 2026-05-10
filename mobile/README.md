# zetl mobile

Tauri Mobile shell for [SPEC-040](../specs/SPEC-040-zetl-mobile.md) —
embeds `zetl serve` (single-user) inside an iOS / Android app and
serves the existing web UI through a WebView.

> **Status:** v0.1.0-strawman. Working today, end to end on the
> desktop dev shell:
>
> - Tauri shell boots → embeds `zetl serve` → WebView loads it.
> - Onboarding wizard: paste BIP39 seed → derive ed25519 SSH key →
>   user adds pubkey to git host → paste remote URL → clone runs
>   → vault populated. SSH key is persisted to
>   `{app_data_dir}/ssh_key.json` (0o600 on Unix) so subsequent
>   launches skip the seed step.
> - Capture: `/_mobile/capture` form writes atomically + commits +
>   best-effort pushes. Auto-titles fall back to first-line or
>   `Inbox YYYY-MM-DD-HHMM`.
> - Sync: `/_mobile/sync` has manual Pull (FF-only) and Push
>   buttons. Non-FF pull surfaces a "resolve on desktop" conflict
>   banner.
> - Read / edit: delegated to the existing serve UI verbatim —
>   themes, [[wikilinks]], backlinks, transclusion, CodeMirror 6
>   editor, search, graph (collapsed below the responsive
>   breakpoint).
>
> **Still TODO** before full SPEC-040 v0.1 acceptance: real
> Minijinja templates for `/_mobile/*` (currently inline HTML);
> platform secure-element key storage (currently JSON file);
> iOS Share Extension + Android share-target intake (REQ-4007).

## Architecture

```
WebView ── http://127.0.0.1:23423/ ─→ embedded zetl serve
                                       │
                                       ├── existing serve UI (Minijinja, themes)
                                       └── /_mobile/{onboarding,capture,sync}
```

The shell is intentionally tiny — see `src/lib.rs` (~70 lines) and
`src/serve_lifecycle.rs` (~30 lines). All UI rendering happens in the
embedded `zetl serve`.

## Desktop dev shell

The fastest iteration loop is the desktop build of the same Tauri
project — no NDK or simulator setup required. Builds for Mac, Linux,
or Windows depending on the host.

```bash
cd mobile
cargo run -p zetl-mobile
```

A blue-ish app window opens. The first paint is the loader in
`dist/index.html`, which polls `http://127.0.0.1:23423/` until the
embedded server is reachable, then redirects the WebView to it. The
vault data dir is at `~/Library/Application Support/io.anuna.zetl.mobile/vault`
on macOS (or the platform-equivalent), created empty on first run.

Until the onboarding POST handlers ship, the page list is empty and
nothing is committable from inside the app — but the existing serve
routes (`/`, `/_static/*`, `/api/pages`, etc.) all work against the
empty vault.

## Android build

### Prerequisites

| Tool                   | Purpose                                       |
| ---------------------- | --------------------------------------------- |
| Android Studio         | SDK manager, emulator, JDK                    |
| Android NDK (r25+)     | Native code cross-compile                     |
| Android SDK (API 33+)  | Runtime                                       |
| `cargo-tauri` 2.10+    | Already installed (`which cargo-tauri`)       |
| Rust Android targets   | `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android` |

Set the standard env vars (Android Studio's SDK Manager prints the
right paths):

```bash
export ANDROID_HOME=$HOME/Library/Android/sdk          # macOS Android Studio default
export NDK_HOME=$ANDROID_HOME/ndk/<version>
export PATH="$ANDROID_HOME/platform-tools:$PATH"
```

### One-time init

Generates `mobile/gen/android/` with a Gradle project, manifest,
icons, and signing config templates. Re-runnable; safe to commit
the generated `gen/android/` only if you want to override defaults
(otherwise leave it gitignored — see `mobile/.gitignore`).

```bash
cd mobile
cargo tauri android init
```

### Run on an emulator or device

```bash
cd mobile
cargo tauri android dev          # debug build, hot-reload, attaches to first running emulator/device
```

### Build a signed APK

```bash
cd mobile
cargo tauri android build        # release APK at gen/android/app/build/outputs/apk/release/
```

The default debug-signed APK is fine for sideloading onto a personal
device. For Play Store distribution, configure
`gen/android/app/build.gradle.kts` with your signing config — that's
out of scope for v0.1-strawman.

## iOS build

Mirrors the Android path but requires Xcode + an Apple Developer
account for signed device builds. See the
[Tauri Mobile iOS guide](https://v2.tauri.app/distribute/mobile/ios/)
once SPEC-040 v0.2 promotes the iOS surfaces past strawman.

```bash
cd mobile
cargo tauri ios init
cargo tauri ios dev
cargo tauri ios build
```

## Known limitations (v0.1-strawman)

- **No share-extension targets.** iOS Share Extension and Android
  `ACTION_SEND` activity stubs are not yet present, so "share to zetl
  mobile" from another app does nothing. Capture is FAB-only.
- **No platform secure-element key storage.** The persisted SSH key
  is a plaintext JSON file in the app data dir (0o600 on Unix). iOS
  and Android sandbox app data per-app, but a jailbroken / rooted
  device + forensics would surface the key. iOS Keychain / Android
  Keystore integration is the next slice.
- **Inline HTML for /_mobile/* routes.** The onboarding, capture,
  and sync pages are inline `format!()`-built HTML rather than
  rendered through the active Minijinja theme. They work but don't
  pick up theme typography / colour overrides.
- **Hardcoded port 23423.** Loopback-bound. A future slice can
  randomize the port and pass it to the WebView via a Tauri command
  rather than `tauri.conf.json` `devUrl`.
- **Empty vault until onboarding completes.** Fresh install with no
  cloned vault → empty page list. The user navigates to
  `/_mobile/onboarding` to clone.

## Implementation map (SPEC-040 trace)

| Component                | File                            | Status                       |
| ------------------------ | ------------------------------- | ---------------------------- |
| Tauri shell entry        | `src/lib.rs`                    | ✅ Boots, spawns serve        |
| Embed lifecycle          | `src/serve_lifecycle.rs`        | ✅ Calls launch_default       |
| WebView loader           | `dist/index.html`               | ✅ Polls + redirects          |
| Keys (BIP39→ed25519)     | `../src/mobile_state.rs`        | ✅ In-memory + JSON persist   |
| Git ops (clone/pull/push)| `../src/mobile_git.rs`          | ✅ git2-rs + SSH callback     |
| Capture (write+commit)   | `../src/mobile_capture.rs`      | ✅ Atomic write + auto-title  |
| `/_mobile/onboarding`    | `../src/web/mobile.rs`          | ✅ Two-step wizard            |
| `/_mobile/capture`       | `../src/web/mobile.rs`          | ✅ GET form + POST handler    |
| `/_mobile/sync`          | `../src/web/mobile.rs`          | ✅ Pull/Push buttons          |
| Keychain (platform)      | not yet                         | ⬜ iOS/Android secure-element |
| Share-ext intake         | not yet                         | ⬜ iOS/Android platform glue  |
| Minijinja templates      | not yet                         | ⬜ Theme-aware /_mobile/*     |

**Test counts:** 19 unit (`cargo test --features mobile --lib mobile_`)
+ 12 integration (`cargo test --features mobile --test mobile_integration`)
+ 4 existing-route tests = **35 passing** on this branch.

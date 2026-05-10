# zetl mobile

Tauri Mobile shell for [SPEC-040](../specs/SPEC-040-zetl-mobile.md) —
embeds `zetl serve` (single-user) inside an iOS / Android app and
serves the existing web UI through a WebView.

> **Status:** v0.1.0-strawman. The Rust shell boots, embeds serve
> via `zetl::web::launch_default`, and the WebView loads pages from
> the embedded server. Onboarding (BIP39 seed → SSH key derivation
> → git clone), capture / save / push, and share-extension intake
> are scaffolded as `/_mobile/*` routes but the POST handlers are
> not yet wired — see SPEC-040 §1.5 and the open ADR-* items.

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

- **No onboarding wiring.** `/_mobile/onboarding` returns a placeholder
  page; the BIP39 → SSH-key → keychain → git-clone path is the next
  scheduled slice.
- **No share-extension targets.** iOS Share Extension and Android
  `ACTION_SEND` activity stubs are not yet present.
- **No git pull/push.** `/_mobile/sync` is a placeholder.
- **No keychain glue.** SSH key storage relies on the platform secure
  element, which lands with the keys module.
- **Empty vault on first run.** Until clone is wired, the page list is
  empty. The embedded server still starts and the WebView still loads
  it — useful for verifying the pipeline.
- **Hardcoded port 23423.** Loopback-bound. A future slice can
  randomize the port and pass it to the WebView via a Tauri command
  rather than `tauri.conf.json` `devUrl`.

## Implementation map (SPEC-040 trace)

| Component              | File                           | Status                           |
| ---------------------- | ------------------------------ | -------------------------------- |
| Tauri shell entry      | `src/lib.rs`                   | ✅ Boots, spawns serve            |
| Embed lifecycle        | `src/serve_lifecycle.rs`       | ✅ Calls launch_default           |
| WebView loader         | `dist/index.html`              | ✅ Polls + redirects              |
| `/_mobile/onboarding`  | `../src/web/mobile.rs`         | 🟡 GET placeholder; POST pending  |
| `/_mobile/capture`     | `../src/web/mobile.rs`         | 🟡 GET placeholder; POST pending  |
| `/_mobile/sync`        | `../src/web/mobile.rs`         | 🟡 GET placeholder; POST pending  |
| Keys (`BIP39`→ed25519) | not yet                        | ⬜ Reuses `zetl derive-ssh-key`   |
| Git ops (clone/pull/push) | not yet                     | ⬜ git2-rs wrapper                |
| Keychain bridge        | not yet                        | ⬜ Tauri plugin                   |
| Share-ext intake       | not yet                        | ⬜ iOS/Android platform glue      |

#!/usr/bin/env bash
# SPEC-040 post-init patcher for the Tauri-generated Android project.
#
# `cargo tauri android init` regenerates `mobile/gen/android/` from
# scratch and the directory is gitignored, so any project-local tweaks
# need to be re-applied after init. This script is idempotent — safe to
# run from `mobile-android-build` on every invocation.
#
# Current patches:
#
# 1. Cleartext loopback. The embedded `zetl serve` listens on
#    `http://127.0.0.1:23423`. Android 9+ blocks cleartext HTTP by
#    default, so the release WebView returns
#    `net::ERR_CLEARTEXT_NOT_PERMITTED` when the user opens any
#    `/_mobile/*` page. The fix: drop a network-security-config that
#    allows cleartext for `127.0.0.1` / `localhost` only, and wire it
#    into the manifest. Cleartext to public hosts stays blocked.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ANDROID_PROJECT="$(cd "$SCRIPT_DIR/.." && pwd)/gen/android"

if [ ! -d "$ANDROID_PROJECT" ]; then
  echo "patch-android-project: $ANDROID_PROJECT does not exist — run 'make mobile-android-init' first" >&2
  exit 1
fi

# ── network_security_config.xml ──────────────────────────────────────────────
NSC_DIR="$ANDROID_PROJECT/app/src/main/res/xml"
NSC_FILE="$NSC_DIR/network_security_config.xml"
mkdir -p "$NSC_DIR"
cat > "$NSC_FILE" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<!-- SPEC-040: the embedded zetl serve runs at http://127.0.0.1:23423.
     Android 9+ blocks cleartext HTTP by default; this config allows it
     for loopback only. All other hosts still require TLS. -->
<network-security-config>
    <domain-config cleartextTrafficPermitted="true">
        <domain includeSubdomains="false">127.0.0.1</domain>
        <domain includeSubdomains="false">localhost</domain>
    </domain-config>
</network-security-config>
XML

# ── AndroidManifest.xml: add android:networkSecurityConfig ──────────────────
MANIFEST="$ANDROID_PROJECT/app/src/main/AndroidManifest.xml"
if ! grep -q 'networkSecurityConfig' "$MANIFEST"; then
  # Use perl rather than sed -i (BSD sed wants a different -i syntax).
  perl -i -pe 's|(android:usesCleartextTraffic="\$\{usesCleartextTraffic\}")|android:networkSecurityConfig="\@xml/network_security_config"\n        $1|' "$MANIFEST"
fi

# ── MainActivity.kt: mixed-content override + edge-to-edge wiring ───────────
# Tauri loads the WebView from `https://tauri.localhost/` (the bundled
# dist/) and then the Rust setup() navigates it to
# `http://127.0.0.1:23423/_mobile/...`. The HTTPS → HTTP hop is treated
# as mixed content by Android WebView, which defaults to
# `MIXED_CONTENT_NEVER_ALLOW` on API 21+ — the resulting error surfaces
# as `net::ERR_CLEARTEXT_NOT_PERMITTED` even when NSC explicitly allows
# cleartext for 127.0.0.1.
#
# Patching the auto-generated `RustWebView.kt` does not survive: every
# Gradle build re-runs `cargo tauri android android-studio-script`
# which regenerates the `generated/` directory before kotlinCompile.
# MainActivity.kt is the documented user-editable entrypoint that
# `cargo tauri android init` only generates if missing, so it is the
# safe place to put the override. We keep the canonical version under
# `mobile/android-overrides/MainActivity.kt` and copy it in here.
OVERRIDES="$(cd "$SCRIPT_DIR/.." && pwd)/android-overrides"
MAIN_ACTIVITY="$ANDROID_PROJECT/app/src/main/java/io/anuna/zetl/mobile/MainActivity.kt"
if [ -f "$OVERRIDES/MainActivity.kt" ] && [ -f "$MAIN_ACTIVITY" ]; then
  cp "$OVERRIDES/MainActivity.kt" "$MAIN_ACTIVITY"
fi

# ── build.gradle.kts: flip release usesCleartextTraffic to "true" ────────────
# Some Android WebView versions consult the manifest's
# `usesCleartextTraffic` flag *before* falling through to the
# network-security-config. Set it to true so the WebView doesn't bail
# out at the manifest layer; the NSC above is what actually scopes
# cleartext to 127.0.0.1 / localhost.
BUILD_GRADLE="$ANDROID_PROJECT/app/build.gradle.kts"
if [ -f "$BUILD_GRADLE" ]; then
  perl -i -pe 's|(getByName\("release"\)\s*\{[^}]*?manifestPlaceholders\["usesCleartextTraffic"\]\s*=\s*)"false"|$1"true"|gs' "$BUILD_GRADLE"
  # Also handle the case where the placeholder is only set in defaultConfig:
  # leave defaultConfig alone (debug already overrides to "true") but make
  # sure release has an explicit override. If the release block does not
  # already mention it, inject one.
  if ! grep -A 10 'getByName("release")' "$BUILD_GRADLE" | grep -q 'usesCleartextTraffic'; then
    perl -i -pe 's|(getByName\("release"\)\s*\{)|$1\n            manifestPlaceholders["usesCleartextTraffic"] = "true"|' "$BUILD_GRADLE"
  fi
fi

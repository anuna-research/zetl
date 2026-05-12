#!/usr/bin/env bash
# SPEC-040 Android build environment.
#
# Source from a Makefile recipe or shell to set up everything `cargo
# tauri android …` needs on macOS:
#
#   . mobile/scripts/android-env.sh
#
# Detects the SDK / NDK / JDK 17 locations and creates a per-repo
# `mobile/.android-shims/` directory holding unsuffixed wrapper symlinks
# (e.g. `aarch64-linux-android-clang` → the NDK's API-suffixed binary).
# The shims are required because `openssl-sys`'s vendored build invokes
# `<target>-clang` / `<target>-ranlib` without an API level, and the NDK
# ships only the API-suffixed names.
#
# Why JDK 17: Gradle 8.14.3 (current Tauri Android scaffold) does not
# yet support class file major version 69 (Java 25). Java 17 is the
# stable LTS that Gradle has supported since 7.6.
#
# Exits the calling shell when sourced if any prerequisite is missing —
# with an actionable hint, never just a stack trace.

set -e

# Resolve repo root from this script's location (mobile/scripts/).
ANDROID_ENV_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZETL_REPO_ROOT="$(cd "$ANDROID_ENV_SCRIPT_DIR/../.." && pwd)"
MOBILE_DIR="$ZETL_REPO_ROOT/mobile"

# ── Android SDK ──────────────────────────────────────────────────────────────
if [ -z "${ANDROID_HOME:-}" ]; then
  for candidate in \
    /opt/homebrew/share/android-commandlinetools \
    /usr/local/share/android-commandlinetools \
    "$HOME/Library/Android/sdk" \
    "$HOME/Android/Sdk"; do
    if [ -d "$candidate/cmdline-tools" ]; then
      export ANDROID_HOME="$candidate"
      break
    fi
  done
fi
if [ -z "${ANDROID_HOME:-}" ] || [ ! -d "$ANDROID_HOME" ]; then
  echo "android-env: ANDROID_HOME not set and no SDK found in common locations." >&2
  echo "  Install: brew install --cask android-commandlinetools" >&2
  echo "  Then: export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools" >&2
  return 1 2>/dev/null || exit 1
fi

# ── Android NDK (pick highest installed) ─────────────────────────────────────
if [ -z "${NDK_HOME:-}" ]; then
  if [ -d "$ANDROID_HOME/ndk" ]; then
    NDK_LATEST="$(ls -1 "$ANDROID_HOME/ndk" 2>/dev/null | sort -V | tail -n 1)"
    if [ -n "$NDK_LATEST" ]; then
      export NDK_HOME="$ANDROID_HOME/ndk/$NDK_LATEST"
    fi
  fi
fi
if [ -z "${NDK_HOME:-}" ] || [ ! -d "$NDK_HOME" ]; then
  echo "android-env: no NDK installed under $ANDROID_HOME/ndk/" >&2
  echo "  Install: \"\$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager\" 'ndk;27.2.12479018'" >&2
  return 1 2>/dev/null || exit 1
fi
export ANDROID_NDK_HOME="$NDK_HOME"
export ANDROID_NDK_ROOT="$NDK_HOME"

# Locate the NDK toolchain bin (darwin-x86_64 even on Apple Silicon —
# Apple's Rosetta translates).
NDK_TOOLCHAIN_BIN=""
for host in darwin-x86_64 darwin-arm64 linux-x86_64; do
  if [ -d "$NDK_HOME/toolchains/llvm/prebuilt/$host/bin" ]; then
    NDK_TOOLCHAIN_BIN="$NDK_HOME/toolchains/llvm/prebuilt/$host/bin"
    break
  fi
done
if [ -z "$NDK_TOOLCHAIN_BIN" ]; then
  echo "android-env: NDK toolchain bin not found under $NDK_HOME/toolchains/llvm/prebuilt/" >&2
  return 1 2>/dev/null || exit 1
fi

# ── JDK 17 (Gradle 8.14 max) ─────────────────────────────────────────────────
if [ -z "${JAVA_HOME:-}" ] || ! "$JAVA_HOME/bin/java" -version 2>&1 | grep -qE 'version "(17|21)\.'; then
  for candidate in \
    /opt/homebrew/opt/openjdk@17 \
    /usr/local/opt/openjdk@17 \
    /Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home \
    /Library/Java/JavaVirtualMachines/zulu-17.jdk/Contents/Home; do
    if [ -d "$candidate" ]; then
      export JAVA_HOME="$candidate"
      break
    fi
  done
fi
if [ -z "${JAVA_HOME:-}" ] || [ ! -d "$JAVA_HOME" ]; then
  echo "android-env: JDK 17 not found. Gradle 8.14 does not support JDK 25." >&2
  echo "  Install: brew install openjdk@17" >&2
  return 1 2>/dev/null || exit 1
fi

# ── NDK clang/ar/ranlib shims (drop unsuffixed names for openssl-sys) ────────
# openssl-sys's vendored Configure invokes <target>-clang / <target>-ranlib
# without an API level. The NDK ships only API-suffixed binaries, so we
# create local symlinks pinned to API level 24 (Android 7.0 / 95%+ market).
ZETL_NDK_SHIMS="$MOBILE_DIR/.android-shims"
ZETL_NDK_API="${ZETL_NDK_API:-24}"
ZETL_NDK_SHIMS_STAMP="$ZETL_NDK_SHIMS/.stamp-$ZETL_NDK_API-$(basename "$NDK_HOME")"
if [ ! -f "$ZETL_NDK_SHIMS_STAMP" ]; then
  mkdir -p "$ZETL_NDK_SHIMS"
  rm -f "$ZETL_NDK_SHIMS"/*-clang "$ZETL_NDK_SHIMS"/*-clang++ \
        "$ZETL_NDK_SHIMS"/*-ar "$ZETL_NDK_SHIMS"/*-ranlib \
        "$ZETL_NDK_SHIMS"/*-strip "$ZETL_NDK_SHIMS"/*-objcopy \
        "$ZETL_NDK_SHIMS"/.stamp-* 2>/dev/null || true
  for arch in aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android; do
    # armv7 wants armv7a- prefix for clang specifically.
    case "$arch" in
      armv7-linux-androideabi) clang_arch=armv7a-linux-androideabi ;;
      *) clang_arch="$arch" ;;
    esac
    ln -sf "$NDK_TOOLCHAIN_BIN/${clang_arch}${ZETL_NDK_API}-clang"   "$ZETL_NDK_SHIMS/${arch}-clang"
    ln -sf "$NDK_TOOLCHAIN_BIN/${clang_arch}${ZETL_NDK_API}-clang++" "$ZETL_NDK_SHIMS/${arch}-clang++"
    ln -sf "$NDK_TOOLCHAIN_BIN/llvm-ar"      "$ZETL_NDK_SHIMS/${arch}-ar"
    ln -sf "$NDK_TOOLCHAIN_BIN/llvm-ranlib"  "$ZETL_NDK_SHIMS/${arch}-ranlib"
    ln -sf "$NDK_TOOLCHAIN_BIN/llvm-strip"   "$ZETL_NDK_SHIMS/${arch}-strip"
    ln -sf "$NDK_TOOLCHAIN_BIN/llvm-objcopy" "$ZETL_NDK_SHIMS/${arch}-objcopy"
  done
  # Also drop the armv7a-prefixed clang name some build scripts use.
  ln -sf "$NDK_TOOLCHAIN_BIN/armv7a-linux-androideabi${ZETL_NDK_API}-clang"   "$ZETL_NDK_SHIMS/armv7a-linux-androideabi-clang"
  ln -sf "$NDK_TOOLCHAIN_BIN/armv7a-linux-androideabi${ZETL_NDK_API}-clang++" "$ZETL_NDK_SHIMS/armv7a-linux-androideabi-clang++"
  touch "$ZETL_NDK_SHIMS_STAMP"
fi

# ── PATH ─────────────────────────────────────────────────────────────────────
# Order matters: shims first (so unsuffixed names resolve to them), JDK
# 17 next (so `java` is the right version), then the SDK / NDK bins.
export PATH="$ZETL_NDK_SHIMS:$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$NDK_TOOLCHAIN_BIN:$PATH"

set +e

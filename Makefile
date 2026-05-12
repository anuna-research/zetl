# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Anuna Research

PREFIX ?= $(HOME)/.local
MANDIR ?= $(PREFIX)/share/man/man1
BASHCOMPDIR ?= $(PREFIX)/share/bash-completion/completions
ZSHCOMPDIR  ?= $(PREFIX)/share/zsh/site-functions
FISHCOMPDIR ?= $(PREFIX)/share/fish/vendor_completions.d

.PHONY: all build test test-reason test-history test-all test-nfr test-nfr-install test-nfr-build nfr-gates nfr-gates-strict nfr-gates-033 nfr-gates-033-strict check lint clippy fmt fmt-fix install uninstall clean doc doc-open release ast-reference ast-reference-check ext-golden ext-golden-update helper-js-install helper-js-build helper-js-test helper-contracts eco-features-check eco-matrix-check translator-roundtrip audit-corpus dist dist-macos-arm64 dist-macos-x86_64 dist-windows dist-metadata dist-upload dist-clean help mobile-build mobile-run mobile-test mobile-clean mobile-wipe mobile-android-init mobile-android-dev mobile-android-build mobile-ios-init mobile-ios-dev mobile-ios-build

all: build

build:
	cargo build --release --all-features

test:
	cargo test

test-reason:
	cargo test --features reason

test-history:
	cargo test --features history

test-all:
	cargo test --features "reason,history"

# NFR harness (SPEC-028): headless-browser timing checks on a built dist/.
# First run needs `make test-nfr-install` to fetch Playwright's Chromium.
test-nfr-install:
	cd tests/nfr && npm install && npm run install-browsers

test-nfr-build:
	cd tests/nfr && npm run build:2k

test-nfr: test-nfr-build
	cd tests/nfr && npm test

check: test lint

lint:
	cargo fmt --check
	cargo clippy -- -D warnings

clippy:
	cargo clippy -- -D warnings

fmt:
	cargo fmt --check

fmt-fix:
	cargo fmt

# Rebuild themes/default/static/theme.css from the Tailwind + DaisyUI pipeline
# under tools/theme-build/default/. Runs Tailwind's content scan so unused
# classes get purged based on the current templates + JS under themes/default/.
# npm install is idempotent — the first run pulls ~120 MB of tailwind + plugins
# into tools/theme-build/default/node_modules and subsequent runs are fast.
theme-css:
	cd tools/theme-build/default && npm install --silent && npm run build

# Regenerate docs/zetl-ast-reference.md from tools/zetl-ast-schema-v1.json
# (SPEC-032 REQ-3202). `ast-reference-check` is the CI diff gate — it exits
# nonzero if the on-disk file is stale.
ast-reference:
	cargo run -p zetl-ast-reference-gen

ast-reference-check:
	cargo run -p zetl-ast-reference-gen -- --check

# zetl-ast-js helper library (SPEC-032 REQ-3210). `helper-js-build` emits
# dist/{esm,cjs,types}/ so tests/helper_js_integration.rs can spawn the
# helper against zetl's persistent-protocol driver.
helper-js-install:
	cd tools/zetl-ast-js && npm install

helper-js-build:
	cd tools/zetl-ast-js && npm run build

helper-js-test:
	cd tools/zetl-ast-js && node --experimental-strip-types --test test/*.test.ts

# SPEC-032 REQ-3210 / CON-3210 cross-implementation contract gate.
# Runs the fixture corpus under tests/fixtures/helper-contracts/ against
# the Rust, Python, and JavaScript helper identity transforms. Requires
# `python3` + `node` on PATH and a fresh helper-js dist.
helper-contracts: helper-js-build
	cargo test --test helper_contracts_integration -- --nocapture

# SPEC-032 CON-3212 canonical-extension golden-HTML gate. Runs every
# fixture under tests/extension-fixtures/ through its registered runner
# and compares against expected.html. Failures surface a unified diff
# plus the exact `cargo xtask update-golden <name>` command to accept
# the change after a deliberate review.
ext-golden:
	cargo test --test ext_golden_html_integration -- --nocapture

# Regenerate every extension fixture's expected.html from its runner's
# current output. Run after a deliberate change to a runner or input.md
# — the same code path the gate uses, so a write here implies a pass
# there on the next invocation.
ext-golden-update:
	cargo xtask update-golden

# SPEC-033 task-eco-feature-flags acceptance: each per-ecosystem cargo
# feature must compile in isolation, every combination must compile
# together, and the no-feature build must still succeed. The
# `ecosystems-v1` umbrella was retired in SPEC-033 §12 Phase F; we
# still exercise the all-three combination explicitly because that is
# the configuration release binaries ship. Runs as a CI gate via
# `.woodpecker/ci.yaml`.
eco-features-check:
	cargo check --no-default-features
	cargo check --no-default-features --features ecosystem-pandoc
	cargo check --no-default-features --features ecosystem-mdbook
	cargo check --no-default-features --features ecosystem-remark
	cargo check --no-default-features --features "ecosystem-pandoc ecosystem-mdbook ecosystem-remark"

# SPEC-033 REQ-3311 / TEST-3311 ecosystem-matrix structural gate. Walks
# every row in tools/zetl-ecosystem-matrix.toml, asserts required
# columns, semver-range shape, fixture-path existence, tier→contract
# coupling, and runs the tier-downgrade-without-rationale simulation.
# Runs as a CI gate via `.woodpecker/ci.yaml`; local contributors can
# invoke it directly when editing the matrix.
eco-matrix-check:
	cargo test --test ecosystem_matrix_integration -- --nocapture

# SPEC-032 NFR-3201..NFR-3208 performance + determinism gates. The
# default-on tests (selector P95, exit-code policy, AST schema pin,
# memory-default, etc.) are coarse and host-stable — they run in CI
# under `cargo test`. `nfr-gates` makes the surface explicit and lets
# regressions print one-line per-NFR telemetry on local runs.
nfr-gates:
	cargo test --test nfr_gates_integration -- --nocapture

# Strict-budget arms (e.g. NFR-3201 P95 on 1k samples; NFR-3207 round
# trip on 200 samples). Marked `#[ignore]` in the harness because they
# are host-dependent; this target unlocks them on demand and runs in
# release mode so the budgets bind to the fast path zetl ships.
nfr-gates-strict:
	cargo test --release --test nfr_gates_integration -- --ignored --nocapture

# SPEC-033 NFR-3301..NFR-3308 ecosystem performance + lifecycle gates.
# Default-on arms (constants pinned to spec, lifecycle table parity with
# the registry, canonicalise idempotence, in-process translation
# headroom) run in CI; strict arms (cold-start probe of every runtime,
# release-binary size sanity ceiling) are `#[ignore]`-gated and unlock
# via `make nfr-gates-033-strict`.
nfr-gates-033:
	cargo test --test nfr_gates_033_integration -- --nocapture

nfr-gates-033-strict:
	cargo test --release --test nfr_gates_033_integration -- --ignored --nocapture

# SPEC-033 NFR-3305 / TEST-3305-fidelity translator round-trip property.
# Drives `arb_document()` through each registered translator and asserts
# canonical-form equivalence after a zetl→foreign→zetl round trip.
# Bump PROPTEST_CASES for the nightly / release sweep (NFR-3305 cites
# 10,000 as the release-gate corpus).
translator-roundtrip:
	PROPTEST_CASES=$${PROPTEST_CASES:-256} cargo test --lib -p zetl translators::roundtrip -- --nocapture

# SPEC-034 REQ-3424 / ADR-3410 malicious-author PR gate. Walks every
# fixture under tools/audit-diff-corpus/ and asserts that the expected
# finding-kind markers fire. A miss means the adversary's sample got
# past `zetl cap audit-diff` — an immediate CI failure. The same
# corpus fixtures are also driven via the library API by
# tests/cap_audit_diff_integration.rs; this target surfaces the
# per-fixture PASS/MISS log on the CI console.
audit-corpus:
	cargo run --quiet --bin zetl -- cap audit-diff --corpus-root tools/audit-diff-corpus

# SPEC-034 REQ-3413 / OBS-3407 referrer-leak canary. Runs the TEST-3413
# integration suite that builds a mixed-link page through the
# capability driver, decrypts the emitted envelope, and asserts:
#   • external <a> carries rel="noopener noreferrer"
#   • internal <a> is byte-identical to the sanitiser output
#   • the capability HTML shell carries
#     <meta name="referrer" content="no-referrer">
#   • [access] rel_noreferrer = false opts out of the per-link
#     rewrite but leaves the shell meta tag in place
# Miss == the path-cap leaks into outgoing Referer headers, so
# treat any red here as a spec regression.
ref-leak-test:
	cargo test --test cap_referrer_scrubbing_integration -- --nocapture

install: build
	install -d $(PREFIX)/bin $(MANDIR) $(BASHCOMPDIR) $(ZSHCOMPDIR) $(FISHCOMPDIR)
	install -m 755 target/release/zetl $(PREFIX)/bin/zetl
	target/release/zetl man        > $(MANDIR)/zetl.1
	target/release/zetl completions bash > $(BASHCOMPDIR)/zetl
	target/release/zetl completions zsh  > $(ZSHCOMPDIR)/_zetl
	target/release/zetl completions fish > $(FISHCOMPDIR)/zetl.fish
	@echo ""
	@echo "Installed zetl to $(PREFIX)/bin/zetl"
	@echo "Man page:       $(MANDIR)/zetl.1  (run 'man zetl')"
	@echo "Completions:    bash, zsh, fish installed under $(PREFIX)/share/"
	@echo ""
	@echo "Ensure these are on your paths:"
	@echo "  PATH     includes $(PREFIX)/bin"
	@echo "  MANPATH  includes $(PREFIX)/share/man"

uninstall:
	rm -f $(PREFIX)/bin/zetl
	rm -f $(MANDIR)/zetl.1
	rm -f $(BASHCOMPDIR)/zetl
	rm -f $(ZSHCOMPDIR)/_zetl
	rm -f $(FISHCOMPDIR)/zetl.fish

clean:
	cargo clean

release:
	./release.sh $(VERSION)

# Local release artifact builder — use when CI upload times out.
# Builds macOS arm64 + x86_64 + Windows from the current tag, packages
# them, and writes metadata. Linux builds are done in CI only (Docker).
#
# Usage:
#   make dist                  # build all three platforms
#   make dist-macos-arm64      # single platform
#   make dist-upload           # upload dist-release/ to R2 via wrangler
#   make dist-clean            # remove dist-release/
#
# Windows requires: brew install mingw-w64
DIST_DIR      ?= dist-release
DIST_FEATURES  = reason,history,mcp,vendored-openssl
DIST_TAG      := $(shell git describe --tags --abbrev=0)
DIST_VERSION  := $(shell echo $(DIST_TAG) | sed 's/^v//')
R2_BUCKET      = anuna-files
R2_PREFIX      = zetl
# Use rustup-managed cargo + rustc for cross-compilation. Homebrew installs
# its own rustc earlier on PATH; explicitly pinning both binaries prevents
# cargo from picking up the wrong compiler when cross-targeting.
CARGO_RUSTUP  := $(shell rustup which cargo 2>/dev/null || echo $(HOME)/.cargo/bin/cargo)
RUSTC_RUSTUP  := $(shell rustup which rustc 2>/dev/null || echo $(HOME)/.cargo/bin/rustc)

dist: dist-macos-arm64 dist-macos-x86_64 dist-windows dist-metadata
	@echo ""
	@ls -lh $(DIST_DIR)/

dist-macos-arm64:
	rustup target add aarch64-apple-darwin
	RUSTC=$(RUSTC_RUSTUP) $(CARGO_RUSTUP) build --release --features "$(DIST_FEATURES)" --target aarch64-apple-darwin
	mkdir -p $(DIST_DIR)
	cp target/aarch64-apple-darwin/release/zetl $(DIST_DIR)/zetl
	tar czf $(DIST_DIR)/zetl-macos-arm64.tar.gz -C $(DIST_DIR) zetl
	rm $(DIST_DIR)/zetl
	@echo "Packaged: $(DIST_DIR)/zetl-macos-arm64.tar.gz"

dist-macos-x86_64:
	rustup target add x86_64-apple-darwin
	RUSTC=$(RUSTC_RUSTUP) $(CARGO_RUSTUP) build --release --features "$(DIST_FEATURES)" --target x86_64-apple-darwin
	mkdir -p $(DIST_DIR)
	cp target/x86_64-apple-darwin/release/zetl $(DIST_DIR)/zetl
	tar czf $(DIST_DIR)/zetl-macos-x86_64.tar.gz -C $(DIST_DIR) zetl
	rm $(DIST_DIR)/zetl
	@echo "Packaged: $(DIST_DIR)/zetl-macos-x86_64.tar.gz"

dist-windows:
	rustup target add x86_64-pc-windows-gnu
	RUSTC=$(RUSTC_RUSTUP) \
	CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
	CARGO_PROFILE_RELEASE_LTO=thin \
	CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
		$(CARGO_RUSTUP) build --release --features "$(DIST_FEATURES)" --target x86_64-pc-windows-gnu
	mkdir -p $(DIST_DIR)
	cp target/x86_64-pc-windows-gnu/release/zetl.exe $(DIST_DIR)/zetl.exe
	zip -j $(DIST_DIR)/zetl-windows-x86_64.zip $(DIST_DIR)/zetl.exe
	rm $(DIST_DIR)/zetl.exe
	@echo "Packaged: $(DIST_DIR)/zetl-windows-x86_64.zip"

dist-metadata:
	mkdir -p $(DIST_DIR)
	printf '{\n  "version": "%s",\n  "download_url": "https://files.anuna.io/zetl/v%s",\n  "features": "reason,history,mcp"\n}\n' \
		"$(DIST_VERSION)" "$(DIST_VERSION)" > $(DIST_DIR)/version.json
	cp install.sh $(DIST_DIR)/install.sh
	shasum -a 256 $(DIST_DIR)/zetl-*.tar.gz $(DIST_DIR)/zetl-*.zip > $(DIST_DIR)/SHA256SUMS
	@echo "Metadata written to $(DIST_DIR)/"

dist-upload:
	@echo "Uploading $(DIST_DIR)/ → R2 $(R2_BUCKET)/$(R2_PREFIX)/$(DIST_TAG)/ + latest/"
	@for f in $(DIST_DIR)/zetl-macos-arm64.tar.gz \
	           $(DIST_DIR)/zetl-macos-x86_64.tar.gz \
	           $(DIST_DIR)/zetl-windows-x86_64.zip \
	           $(DIST_DIR)/version.json \
	           $(DIST_DIR)/install.sh \
	           $(DIST_DIR)/SHA256SUMS; do \
	  [ -f "$$f" ] || continue; \
	  fname=$$(basename "$$f"); \
	  echo "  $$fname"; \
	  wrangler r2 object put $(R2_BUCKET)/$(R2_PREFIX)/$(DIST_TAG)/$$fname --file "$$f" --remote; \
	  wrangler r2 object put $(R2_BUCKET)/$(R2_PREFIX)/latest/$$fname --file "$$f" --remote; \
	done
	@echo "Done — https://files.anuna.io/$(R2_PREFIX)/$(DIST_TAG)/"

dist-clean:
	rm -rf $(DIST_DIR)

doc:
	cargo doc --no-deps

doc-open:
	cargo doc --no-deps --open

# ─── SPEC-040 mobile (Tauri Mobile) ─────────────────────────────────────────
#
# `mobile-run` is the fastest iteration loop: builds the desktop dev shell
# of the Tauri Mobile project and runs it. Window opens; embedded zetl serve
# boots on 127.0.0.1:23423; WebView lands on /_mobile/onboarding.
#
# `mobile-wipe` clears the app's data directory so you can re-test the
# fresh-install onboarding flow without manually rm-ing platform-specific
# paths. Use before `mobile-run` to start clean.
#
# Android + iOS targets wrap `cargo tauri android` / `ios` subcommands —
# the cargo-tauri-cli must be on PATH, plus Android NDK/SDK or Xcode
# respectively. The `init` targets are one-time and generate
# `mobile/gen/{android,apple}/` (gitignored).

# Best-guess app data dir per platform. macOS default below; override on
# Linux / Windows by passing MOBILE_APP_DATA=... to mobile-wipe.
MOBILE_APP_DATA ?= $(HOME)/Library/Application Support/io.anuna.zetl.mobile

mobile-build:
	cargo build --release -p zetl-mobile

mobile-run:
	cargo run --release -p zetl-mobile

mobile-test:
	cargo test --features mobile --lib mobile_
	cargo test --features mobile --test mobile_integration

mobile-clean:
	cargo clean -p zetl-mobile

mobile-wipe:
	@echo "Wiping $(MOBILE_APP_DATA)/"
	@rm -rf "$(MOBILE_APP_DATA)"
	@echo "Done — next launch starts at /_mobile/onboarding step 1."

# Android targets source mobile/scripts/android-env.sh, which detects
# the SDK / NDK / JDK-17 locations and creates the per-repo NDK clang
# shims that openssl-sys's vendored build needs. See mobile/README.md
# §Android for the prerequisite install commands.
mobile-android-init:
	bash -c '. mobile/scripts/android-env.sh && cd mobile && cargo tauri android init' && \
	  bash mobile/scripts/patch-android-project.sh

mobile-android-dev:
	bash mobile/scripts/patch-android-project.sh && \
	  bash -c '. mobile/scripts/android-env.sh && cd mobile && cargo tauri android dev'

# Default: arm64 release APK (covers >95% of modern devices, smallest
# universal bundle). Override TARGET=universal for all four ABIs.
#
# Always sign the release APK with the standard Android debug
# keystore — Tauri's release path produces an unsigned APK, which
# Android 7+ rejects with "App not installed as a package, appears
# to be invalid". The debug keystore is fine for sideload-testing;
# Play Store distribution should use a real signing config in
# `gen/android/app/build.gradle.kts`.
TARGET ?= aarch64
# Always wipe Gradle's APK outputs + intermediates before building.
# Gradle's up-to-date check has been observed to skip
# `mergeUniversalReleaseNativeLibs` when the cargo-produced .so changes
# under a symlink in `jniLibs/`, leaving a stale APK in
# `build/outputs/apk/universal/release/`. The wipe forces a full repack.
mobile-android-build:
	rm -rf mobile/gen/android/app/build/intermediates mobile/gen/android/app/build/outputs zetl-mobile-release.apk
	bash mobile/scripts/patch-android-project.sh && \
	  bash -c '. mobile/scripts/android-env.sh && cd mobile && cargo tauri android build --apk --target $(TARGET) && $(MAKE) -C .. mobile-android-sign'

mobile-android-build-debug:
	rm -rf mobile/gen/android/app/build/intermediates mobile/gen/android/app/build/outputs
	bash mobile/scripts/patch-android-project.sh && \
	  bash -c '. mobile/scripts/android-env.sh && cd mobile && cargo tauri android build --apk --debug --target $(TARGET)'

# Sign the most recent release APK with the Android debug keystore.
# Creates the keystore on first use. Output: zetl-mobile-release.apk
# at the repo root, ready for `adb install`.
mobile-android-sign:
	bash -c '. mobile/scripts/android-env.sh && \
	  KEYSTORE=$$HOME/.android/debug.keystore && \
	  if [ ! -f "$$KEYSTORE" ]; then \
	    mkdir -p "$$HOME/.android" && \
	    keytool -genkey -v -keystore "$$KEYSTORE" -storepass android \
	      -alias androiddebugkey -keypass android \
	      -keyalg RSA -keysize 2048 -validity 10000 \
	      -dname "CN=Android Debug,O=Android,C=US"; \
	  fi && \
	  APKSIGNER=$$(find $$ANDROID_HOME/build-tools -name apksigner | sort -V | tail -1) && \
	  IN=mobile/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk && \
	  OUT=zetl-mobile-release.apk && \
	  "$$APKSIGNER" sign --ks "$$KEYSTORE" --ks-pass pass:android \
	    --key-pass pass:android --out "$$OUT" "$$IN" && \
	  "$$APKSIGNER" verify "$$OUT" && \
	  echo "Signed APK: $$OUT"'

mobile-ios-init:
	cd mobile && cargo tauri ios init

mobile-ios-dev:
	cd mobile && cargo tauri ios dev

mobile-ios-build:
	cd mobile && cargo tauri ios build

help:
	@echo "zetl - Bi-directional wikilink graph CLI"
	@echo ""
	@echo "Targets:"
	@echo "  make build        - Build release binary"
	@echo "  make test         - Run core test suite"
	@echo "  make test-reason  - Run tests with reason feature"
	@echo "  make test-history - Run tests with history feature"
	@echo "  make test-all     - Run tests with all features"
	@echo "  make test-nfr-install - Install Playwright + Chromium for NFR harness"
	@echo "  make test-nfr-build   - Seed + build the 2k-page NFR fixture"
	@echo "  make test-nfr         - Run headless NFR harness (SPEC-028)"
	@echo "  make check        - Run tests and lint"
	@echo "  make lint         - Run fmt check and clippy"
	@echo "  make clippy       - Run clippy lints"
	@echo "  make fmt          - Check formatting"
	@echo "  make fmt-fix      - Auto-fix formatting"
	@echo "  make helper-js-install   - npm install for tools/zetl-ast-js"
	@echo "  make helper-js-build     - Build ESM/CJS/types for zetl-ast-js"
	@echo "  make helper-js-test      - Run zetl-ast-js unit tests"
	@echo "  make helper-contracts    - Run cross-impl (py+js+rust) fixture corpus"
	@echo "  make ast-reference       - Regenerate docs/zetl-ast-reference.md"
	@echo "  make ast-reference-check - CI gate: fail if the reference is stale"
	@echo "  make ext-golden          - Run CON-3212 canonical-extension golden-HTML gate"
	@echo "  make ext-golden-update   - Regenerate expected.html for every extension fixture"
	@echo "  make eco-features-check  - Compile-in-isolation gate for every ecosystem feature flag"
	@echo "  make eco-matrix-check    - SPEC-033 REQ-3311 / TEST-3311 ecosystem-matrix structural gate"
	@echo "  make translator-roundtrip - NFR-3305 property-test gate: zetl→foreign→zetl canonical equivalence"
	@echo "  make nfr-gates           - SPEC-032 NFR-3201..NFR-3208 performance + determinism gates"
	@echo "  make nfr-gates-strict    - Run the #[ignore]-gated strict-budget arms (release mode)"
	@echo "  make nfr-gates-033       - SPEC-033 NFR-3301..NFR-3308 ecosystem performance + lifecycle gates"
	@echo "  make nfr-gates-033-strict- Run the #[ignore]-gated SPEC-033 strict arms (release mode)"
	@echo "  make install      - Install binary, man page, and shell completions"
	@echo "  make uninstall    - Remove binary, man page, and completions"
	@echo "  make clean        - Remove build artifacts"
	@echo "  make doc          - Generate documentation"
	@echo "  make doc-open     - Generate and open documentation"
	@echo "  make release      - Tag and push a new release (VERSION=X.Y.Z optional)"
	@echo "  make dist         - Build macOS arm64/x86_64 + Windows artifacts locally"
	@echo "  make dist-upload  - Upload dist-release/ to R2 via wrangler"
	@echo "  make dist-clean   - Remove dist-release/"
	@echo ""
	@echo "SPEC-040 mobile (Tauri Mobile):"
	@echo "  make mobile-build         - Build the desktop dev shell"
	@echo "  make mobile-run           - Build + run desktop dev shell (opens window)"
	@echo "  make mobile-test          - Run mobile unit + integration tests"
	@echo "  make mobile-clean         - cargo clean -p zetl-mobile (forces dist/ rebundle)"
	@echo "  make mobile-wipe          - Wipe app data dir so onboarding restarts fresh"
	@echo "  make mobile-android-init        - One-time: cargo tauri android init (after SDK is set up)"
	@echo "  make mobile-android-dev         - Build+run debug APK on emulator/device"
	@echo "  make mobile-android-build       - Build arm64 release APK, sign with debug key (TARGET=universal for all ABIs)"
	@echo "  make mobile-android-build-debug - Build debug APK (large; unstripped)"
	@echo "  make mobile-android-sign        - Sign the last release APK with the Android debug keystore"
	@echo "  make mobile-ios-init      - One-time: cargo tauri ios init"
	@echo "  make mobile-ios-dev       - Build+run on simulator/device"
	@echo "  make mobile-ios-build     - Build release IPA"
	@echo ""
	@echo "Options:"
	@echo "  PREFIX=<path>     - Install prefix (default: ~/.local)"
	@echo "  VERSION=<ver>     - Version for release target (e.g. VERSION=0.1.1)"
	@echo "  MOBILE_APP_DATA=<path> - Override app data dir for mobile-wipe (macOS default shown)"

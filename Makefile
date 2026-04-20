# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Anuna Research

PREFIX ?= $(HOME)/.local
MANDIR ?= $(PREFIX)/share/man/man1
BASHCOMPDIR ?= $(PREFIX)/share/bash-completion/completions
ZSHCOMPDIR  ?= $(PREFIX)/share/zsh/site-functions
FISHCOMPDIR ?= $(PREFIX)/share/fish/vendor_completions.d

.PHONY: all build test test-reason test-history test-all test-nfr test-nfr-install test-nfr-build nfr-gates nfr-gates-strict nfr-gates-033 nfr-gates-033-strict check lint clippy fmt fmt-fix install uninstall clean doc doc-open release ast-reference ast-reference-check ext-golden ext-golden-update helper-js-install helper-js-build helper-js-test helper-contracts eco-features-check eco-matrix-check translator-roundtrip audit-corpus help

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

doc:
	cargo doc --no-deps

doc-open:
	cargo doc --no-deps --open

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
	@echo ""
	@echo "Options:"
	@echo "  PREFIX=<path>     - Install prefix (default: ~/.local)"
	@echo "  VERSION=<ver>     - Version for release target (e.g. VERSION=0.1.1)"

# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Anuna Research

PREFIX ?= $(HOME)/.local
MANDIR ?= $(PREFIX)/share/man/man1
BASHCOMPDIR ?= $(PREFIX)/share/bash-completion/completions
ZSHCOMPDIR  ?= $(PREFIX)/share/zsh/site-functions
FISHCOMPDIR ?= $(PREFIX)/share/fish/vendor_completions.d

.PHONY: all build test test-reason test-history test-all check lint clippy fmt fmt-fix install uninstall clean doc doc-open release help

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
	@echo "  make check        - Run tests and lint"
	@echo "  make lint         - Run fmt check and clippy"
	@echo "  make clippy       - Run clippy lints"
	@echo "  make fmt          - Check formatting"
	@echo "  make fmt-fix      - Auto-fix formatting"
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

# SPDX-License-Identifier: MIT
# Copyright (c) 2026 Anuna Research

PREFIX ?= $(HOME)/.local

.PHONY: all build test test-reason test-history test-all check lint clippy fmt fmt-fix install uninstall clean doc doc-open help

all: build

build:
	cargo build --release

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
	install -d $(PREFIX)/bin
	install -m 755 target/release/zetl $(PREFIX)/bin/zetl

uninstall:
	rm -f $(PREFIX)/bin/zetl

clean:
	cargo clean

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
	@echo "  make install      - Install to $(PREFIX)/bin"
	@echo "  make uninstall    - Remove from $(PREFIX)/bin"
	@echo "  make clean        - Remove build artifacts"
	@echo "  make doc          - Generate documentation"
	@echo "  make doc-open     - Generate and open documentation"
	@echo ""
	@echo "Options:"
	@echo "  PREFIX=<path>     - Install prefix (default: ~/.local)"

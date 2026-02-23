.PHONY: all build test test-reason clippy fmt check clean install release

# Default target
all: check build test

# Build
build:
	cargo build

# Build release
release:
	cargo build --release

# Run all tests
test:
	cargo test

# Run all tests with reason feature
test-reason:
	cargo test --features reason

# Run clippy lints
clippy:
	cargo clippy -- -D warnings

# Check formatting
fmt:
	cargo fmt -- --check

# Format code
fmt-fix:
	cargo fmt

# Full CI check (fmt + clippy + test)
check: fmt clippy

# Install CLI
install:
	cargo install --path .

# Clean build artifacts
clean:
	cargo clean

# Generate documentation
doc:
	cargo doc --no-deps

# Open documentation in browser
doc-open:
	cargo doc --no-deps --open

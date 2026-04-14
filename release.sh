#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Anuna Research

set -e

# zetl release script
# Usage: ./release.sh [version]
# Example: ./release.sh 0.1.1

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info() { echo -e "${GREEN}[INFO]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
  CURRENT=$(grep '^version' Cargo.toml | head -1 | grep -o '"[^"]*"' | tr -d '"')
  echo "Current version: $CURRENT"
  read -p "Enter new version (without 'v' prefix): " VERSION
fi

[[ -z "$VERSION" ]] && error "Version is required"

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  error "Invalid version format. Use semver: X.Y.Z or X.Y.Z-suffix"
fi

TAG="v$VERSION"

info "Preparing release $TAG"

if ! git diff --quiet || ! git diff --cached --quiet; then
  error "You have uncommitted changes. Commit or stash them first."
fi

if git rev-parse "$TAG" >/dev/null 2>&1; then
  error "Tag $TAG already exists"
fi

info "Updating version in Cargo.toml..."
if [[ "$(uname)" == "Darwin" ]]; then
  sed -i '' "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" Cargo.toml
else
  sed -i "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" Cargo.toml
fi

info "Running tests..."
cargo test --quiet

info "Running clippy..."
cargo clippy --quiet -- -D warnings

info "Committing version bump..."
git add Cargo.toml Cargo.lock 2>/dev/null || git add Cargo.toml
git commit -m "chore: bump version to $VERSION"

info "Creating tag $TAG..."
git tag -a "$TAG" -m "Release $VERSION"

info "Pushing to origin..."
git push origin main
git push origin "$TAG"

echo ""
info "Release $TAG published!"
echo ""
echo "Woodpecker release pipeline triggered."
echo "Release will be available at:"
echo "  https://files.anuna.io/zetl/$TAG/"
echo "  https://files.anuna.io/zetl/latest/"

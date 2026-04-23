#!/bin/bash
set -e

# ztl installer
# Usage: curl -fsSL https://files.anuna.io/ztl/latest/install.sh | bash

TOOL_NAME="ztl"
VERSION="${VERSION:-latest}"
BASE_URL="https://files.anuna.io/ztl"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
MAN_DIR="${MAN_DIR:-$HOME/.local/share/man/man1}"
COMP_BASE="${COMP_BASE:-$HOME/.local/share}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info() { echo -e "${GREEN}[INFO]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

detect_platform() {
  local detected_os detected_arch
  detected_os="$(uname -s 2>/dev/null | tr '[:upper:]' '[:lower:]')"
  detected_arch="$(uname -m 2>/dev/null)"

  case "$detected_os" in
    linux*)  OS="linux" ;;
    darwin*) OS="macos" ;;
    *)       error "Unsupported OS: $detected_os. For Windows, download from: $BASE_URL/latest/ztl-windows-x86_64.zip" ;;
  esac

  case "$detected_arch" in
    x86_64|amd64)  ARCH="x86_64" ;;
    arm64|aarch64) ARCH="arm64" ;;
    *)             error "Unsupported architecture: $detected_arch" ;;
  esac

  PLATFORM="${OS}-${ARCH}"
  info "Detected platform: $PLATFORM"
}

get_version() {
  if [ "$VERSION" = "latest" ]; then
    info "Fetching latest version..."
    VERSION=$(curl -fsSL "$BASE_URL/latest/version.json" 2>/dev/null \
      | grep -o '"version"[[:space:]]*:[[:space:]]*"[^"]*"' | cut -d'"' -f4)
    [ -z "$VERSION" ] && error "Could not determine latest version"
    info "Latest version: $VERSION"
  fi
}

install_binary() {
  info "Installing $TOOL_NAME v$VERSION for $PLATFORM..."

  ARCHIVE_NAME="ztl-${PLATFORM}.tar.gz"
  DOWNLOAD_URL="$BASE_URL/v$VERSION/$ARCHIVE_NAME"

  TMP_DIR=$(mktemp -d)
  trap 'rm -rf "$TMP_DIR"' EXIT

  info "Downloading from $DOWNLOAD_URL..."
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/ztl.tar.gz" || error "Download failed"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$DOWNLOAD_URL" -O "$TMP_DIR/ztl.tar.gz" || error "Download failed"
  else
    error "curl or wget is required"
  fi

  tar -xzf "$TMP_DIR/ztl.tar.gz" -C "$TMP_DIR"

  mkdir -p "$INSTALL_DIR"
  mv "$TMP_DIR/ztl" "$INSTALL_DIR/ztl"
  chmod +x "$INSTALL_DIR/ztl"

  info "Installed binary → $INSTALL_DIR/ztl"
}

install_man_and_completions() {
  ztl_BIN="$INSTALL_DIR/ztl"

  # Man page
  if mkdir -p "$MAN_DIR" 2>/dev/null; then
    if "$ztl_BIN" man > "$MAN_DIR/ztl.1" 2>/dev/null; then
      info "Man page   → $MAN_DIR/ztl.1  (run 'man ztl')"
    else
      warn "Could not generate man page"
    fi
  fi

  # Completions
  mkdir -p "$COMP_BASE/bash-completion/completions" "$COMP_BASE/zsh/site-functions" "$COMP_BASE/fish/vendor_completions.d" 2>/dev/null || true
  "$ztl_BIN" completions bash > "$COMP_BASE/bash-completion/completions/ztl" 2>/dev/null && \
    info "Bash completion → $COMP_BASE/bash-completion/completions/ztl"
  "$ztl_BIN" completions zsh  > "$COMP_BASE/zsh/site-functions/_ztl" 2>/dev/null && \
    info "Zsh completion  → $COMP_BASE/zsh/site-functions/_ztl"
  "$ztl_BIN" completions fish > "$COMP_BASE/fish/vendor_completions.d/ztl.fish" 2>/dev/null && \
    info "Fish completion → $COMP_BASE/fish/vendor_completions.d/ztl.fish"
}

check_path() {
  if echo "$PATH" | tr ':' '\n' | grep -q "^${INSTALL_DIR}$"; then
    : # already in PATH
  else
    warn "$INSTALL_DIR is not in your PATH"
    echo "  Add it: export PATH=\"$INSTALL_DIR:\$PATH\""
  fi
}

main() {
  echo "================================"
  echo "  ztl Installer"
  echo "================================"
  echo ""

  detect_platform
  get_version
  install_binary
  install_man_and_completions
  check_path

  echo ""
  info "Installation complete!"
  echo ""
  echo "Quick start:"
  echo "  ztl --help"
  echo "  ztl -d ./my-vault index"
  echo "  ztl -d ./my-vault serve    # http://localhost:3000"
  echo ""
  echo "Documentation: https://codeberg.org/anuna/ztl"
}

main "$@"

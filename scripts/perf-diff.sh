#!/usr/bin/env bash
#
# perf-diff.sh — verify that two consecutive `zetl build` runs produce a
# byte-identical dist tree (modulo timestamps in sitemap.xml).
#
# Catches the kinds of nondeterminism that the PERF-BUILD-2026-05-12
# parallel-page-render and parallel-scanner refactors are most at risk
# of introducing — HashMap iteration order, rayon work-stealing
# permuting some side effect, etc.
#
# Usage:
#   scripts/perf-diff.sh                    # uses target/perf/vault-1k
#   scripts/perf-diff.sh <vault-dir>        # use a custom vault
#
# Exit codes:
#   0  identical (modulo sitemap.xml + brotli .br files)
#   1  diverged — see stdout for the offending paths
#   2  setup error (vault missing, binary missing, etc.)

set -euo pipefail

VAULT_DIR="${1:-target/perf/vault-1k}"
BIN="./target/release/zetl"
DIST_A="$(mktemp -d -t zetl-perf-diff-a-XXXXXX)"
DIST_B="$(mktemp -d -t zetl-perf-diff-b-XXXXXX)"

cleanup() {
    rm -rf "$DIST_A" "$DIST_B"
}
trap cleanup EXIT

if [[ ! -d "$VAULT_DIR" ]]; then
    echo "perf-diff: vault dir '$VAULT_DIR' not found." >&2
    echo "Generate one with:" >&2
    echo "  cargo run --release --bin gen-vault -- --pages 1000 --avg-links 12 --seed 42 --out '$VAULT_DIR'" >&2
    exit 2
fi

if [[ ! -x "$BIN" ]]; then
    echo "perf-diff: release binary '$BIN' not found. Run \`cargo build --release\` first." >&2
    exit 2
fi

echo "perf-diff: vault=$VAULT_DIR"

echo "perf-diff: build A → $DIST_A"
"$BIN" --dir "$VAULT_DIR" build --no-cache --out "$DIST_A" >/dev/null

echo "perf-diff: build B → $DIST_B"
"$BIN" --dir "$VAULT_DIR" build --no-cache --out "$DIST_B" >/dev/null

# Files whose contents legitimately differ between runs:
#   * sitemap.xml — embeds <lastmod> from the build clock
#   * graph-index.json — embeds a `generated_at` ISO timestamp
#   * *.br — brotli-precompressed twins of HTML/CSS/JS depend on
#     wall-clock-keyed encoder state
# Compare every other file byte-for-byte.

EXCLUDES=(
    --exclude='sitemap.xml'
    --exclude='graph-index.json'
    --exclude='*.br'
)

echo "perf-diff: comparing dist trees …"
DIFF_OUT=$(diff -rq "${EXCLUDES[@]}" "$DIST_A" "$DIST_B" 2>&1 || true)

if [[ -z "$DIFF_OUT" ]]; then
    echo "perf-diff: ✓ dist trees are byte-identical (modulo timestamped + brotli files)"
    exit 0
fi

echo "perf-diff: ✗ dist trees diverged. Files that differ:"
echo "$DIFF_OUT" | head -50
DIVERGED_COUNT=$(echo "$DIFF_OUT" | wc -l | tr -d ' ')
echo "perf-diff: total diverged paths = $DIVERGED_COUNT"
exit 1

#!/usr/bin/env bash
# 🦞 Cross-compile Claw for all supported platforms.
#
# Requires: cargo, cross (cargo install cross --git https://github.com/cross-rs/cross)
#
# Usage:
#   ./scripts/cross-build.sh              # build all, version "dev"
#   ./scripts/cross-build.sh 0.1.0        # build all, version 0.1.0
#   ./scripts/cross-build.sh 0.1.0 linux  # build only linux targets
#
set -euo pipefail

VERSION="${1:-dev}"
FILTER="${2:-all}"
OUT_DIR="dist"
mkdir -p "$OUT_DIR"

# ── All supported targets ──────────────────────────────────────
# Format: TARGET:GROUP (group used for filtering)
TARGETS=(
    "x86_64-unknown-linux-gnu:linux"
    "aarch64-unknown-linux-gnu:linux"
    "armv7-unknown-linux-gnueabihf:linux"
    "x86_64-unknown-linux-musl:linux"
    "aarch64-unknown-linux-musl:linux"
    "x86_64-apple-darwin:macos"
    "aarch64-apple-darwin:macos"
    "x86_64-pc-windows-msvc:windows"
    "aarch64-linux-android:android"
)

BUILT=0
FAILED=0

echo "🦞 Building Claw v${VERSION}"
echo "   Filter: ${FILTER}"
echo ""

for entry in "${TARGETS[@]}"; do
    TARGET="${entry%%:*}"
    GROUP="${entry##*:}"

    # Filter by group if specified
    if [[ "$FILTER" != "all" && "$GROUP" != "$FILTER" ]]; then
        continue
    fi

    echo "━━━ ${TARGET} ━━━"

    # Determine build command
    BUILD_CMD="cargo"
    if command -v cross &> /dev/null; then
        # Use cross for non-native targets
        case "$TARGET" in
            # Don't use cross for native macOS targets when running on macOS
            *-apple-darwin)
                if [[ "$(uname -s)" == "Darwin" ]]; then
                    BUILD_CMD="cargo"
                else
                    BUILD_CMD="cross"
                fi
                ;;
            *)
                BUILD_CMD="cross"
                ;;
        esac
    fi

    if $BUILD_CMD build --release --target "$TARGET" --bin claw 2>&1; then
        # Copy binary to dist/
        EXT=""
        if [[ "$TARGET" == *"windows"* ]]; then EXT=".exe"; fi

        SRC="target/${TARGET}/release/claw${EXT}"
        DST="${OUT_DIR}/claw-v${VERSION}-${TARGET}${EXT}"

        if [ -f "$SRC" ]; then
            cp "$SRC" "$DST"
            SIZE=$(du -h "$DST" | cut -f1)
            echo "  ✓ ${DST} (${SIZE})"
            BUILT=$((BUILT + 1))
        else
            echo "  ⚠ Output not found: $SRC"
            FAILED=$((FAILED + 1))
        fi
    else
        echo "  ✗ Build failed for ${TARGET}"
        FAILED=$((FAILED + 1))
    fi
    echo ""
done

# Generate checksums
if [ -n "$(ls -A "$OUT_DIR"/claw-* 2>/dev/null)" ]; then
    (cd "$OUT_DIR" && sha256sum claw-* > SHA256SUMS.txt 2>/dev/null || shasum -a 256 claw-* > SHA256SUMS.txt)
    echo "📋 Checksums written to ${OUT_DIR}/SHA256SUMS.txt"
fi

echo ""
echo "✅ Done: ${BUILT} built, ${FAILED} failed"
echo ""
ls -lh "$OUT_DIR/" 2>/dev/null || true

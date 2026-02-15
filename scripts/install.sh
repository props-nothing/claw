#!/usr/bin/env sh
# 🦞 Claw — Universal Install Script
#
# Usage:
#   curl -sSL https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | sh
#   wget -qO- https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | sh
#
# Options (env vars):
#   CLAW_VERSION=0.1.0   Install a specific version (default: latest)
#   CLAW_DIR=~/.claw      Install directory (default: ~/.claw)
#   CLAW_NO_MODIFY_PATH=1 Don't touch shell profile
#
set -eu

REPO="props-nothing/claw"
CLAW_DIR="${CLAW_DIR:-$HOME/.claw}"
BIN_DIR="${CLAW_DIR}/bin"
VERSION="${CLAW_VERSION:-latest}"

# ── Colors ──────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { printf "${CYAN}info${NC}  %s\n" "$1"; }
ok()    { printf "${GREEN}  ✓${NC}  %s\n" "$1"; }
warn()  { printf "${YELLOW}warn${NC}  %s\n" "$1"; }
err()   { printf "${RED}error${NC} %s\n" "$1" >&2; }
die()   { err "$1"; exit 1; }

# ── Detect platform ────────────────────────────────────────────
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            # Check if running in Termux (Android) or iSH (iOS)
            if [ -n "${TERMUX_VERSION:-}" ] || [ -d "/data/data/com.termux" ]; then
                PLATFORM="android"
            elif [ -f "/proc/ish/version" ] || (uname -a | grep -qi "ish"); then
                PLATFORM="ios"
            else
                PLATFORM="linux"
            fi
            ;;
        Darwin)   PLATFORM="macos" ;;
        MINGW*|MSYS*|CYGWIN*) PLATFORM="windows" ;;
        *)        die "Unsupported OS: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64)   ARCH="x86_64" ;;
        aarch64|arm64)   ARCH="aarch64" ;;
        armv7l|armhf)    ARCH="armv7" ;;
        *)               die "Unsupported architecture: $ARCH" ;;
    esac
}

# ── Map platform+arch → release target triple ──────────────────
get_target() {
    case "${PLATFORM}-${ARCH}" in
        linux-x86_64)    TARGET="x86_64-unknown-linux-gnu" ;;
        linux-aarch64)   TARGET="aarch64-unknown-linux-gnu" ;;
        linux-armv7)     TARGET="armv7-unknown-linux-gnueabihf" ;;
        macos-x86_64)    TARGET="x86_64-apple-darwin" ;;
        macos-aarch64)   TARGET="aarch64-apple-darwin" ;;
        windows-x86_64)  TARGET="x86_64-pc-windows-msvc" ;;
        android-aarch64) TARGET="aarch64-linux-android" ;;
        android-armv7)   TARGET="armv7-linux-androideabi" ;;
        ios-aarch64)     TARGET="aarch64-unknown-linux-musl" ;;  # iSH runs Alpine Linux userland
        *)               die "No prebuilt binary for ${PLATFORM}-${ARCH}" ;;
    esac
}

# ── Resolve latest version from GitHub ─────────────────────────
resolve_version() {
    if [ "$VERSION" = "latest" ]; then
        info "Fetching latest release..."
        VERSION=$(curl -sS "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
            | grep '"tag_name"' | head -1 | sed 's/.*"v\?\([^"]*\)".*/\1/' || true)
        if [ -z "$VERSION" ]; then
            warn "No published releases found. Will build from source."
            VERSION=""
            return
        fi
    fi
    # Strip leading 'v' if present
    VERSION="${VERSION#v}"
}

# ── Download + install ─────────────────────────────────────────
install_binary() {
    EXT=""
    if [ "$PLATFORM" = "windows" ]; then EXT=".exe"; fi

    FILENAME="claw-v${VERSION}-${TARGET}${EXT}"
    URL="https://github.com/${REPO}/releases/download/v${VERSION}/${FILENAME}"

    info "Downloading ${FILENAME}..."
    mkdir -p "$BIN_DIR"

    TMPFILE="$(mktemp)"
    HTTP_CODE=$(curl -sSL -o "$TMPFILE" -w '%{http_code}' "$URL" 2>/dev/null || echo "000")

    if [ "$HTTP_CODE" != "200" ]; then
        rm -f "$TMPFILE"
        echo ""
        warn "Pre-built binary not available for ${TARGET} (HTTP ${HTTP_CODE})."
        echo ""
        try_build_from_source
        return
    fi

    mv "$TMPFILE" "${BIN_DIR}/claw${EXT}"
    chmod +x "${BIN_DIR}/claw${EXT}"
    ok "Installed claw v${VERSION} → ${BIN_DIR}/claw${EXT}"
}

# ── Fallback: build from source ────────────────────────────────
try_build_from_source() {
    info "Attempting to build from source..."

    if ! command -v cargo >/dev/null 2>&1; then
        warn "Rust not installed. Installing via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --quiet
        . "$HOME/.cargo/env"
    fi

    if ! command -v cargo >/dev/null 2>&1; then
        die "Failed to install Rust. Install manually: https://rustup.rs"
    fi

    # Install build deps on Linux/Termux
    if [ "$PLATFORM" = "linux" ]; then
        if command -v apt-get >/dev/null 2>&1; then
            info "Installing build dependencies (apt)..."
            sudo apt-get update -qq && sudo apt-get install -y -qq build-essential pkg-config libssl-dev >/dev/null 2>&1 || true
        elif command -v dnf >/dev/null 2>&1; then
            info "Installing build dependencies (dnf)..."
            sudo dnf install -y gcc pkg-config openssl-devel >/dev/null 2>&1 || true
        fi
    elif [ "$PLATFORM" = "android" ]; then
        if command -v pkg >/dev/null 2>&1; then
            info "Installing build dependencies (Termux)..."
            pkg install -y rust openssl 2>/dev/null || true
        fi
    elif [ "$PLATFORM" = "ios" ]; then
        if command -v apk >/dev/null 2>&1; then
            info "Installing build dependencies (iSH/Alpine)..."
            apk add --no-cache build-base openssl-dev rust cargo 2>/dev/null || true
        fi
    fi

    info "Building claw (this may take a few minutes)..."
    TMPDIR="$(mktemp -d)"
    git clone --depth 1 "https://github.com/${REPO}.git" "$TMPDIR/claw" 2>/dev/null \
        || die "Failed to clone repository"

    (cd "$TMPDIR/claw" && cargo build --release --bin claw) \
        || die "Build failed. Check the errors above."

    mkdir -p "$BIN_DIR"
    cp "$TMPDIR/claw/target/release/claw" "${BIN_DIR}/claw"
    chmod +x "${BIN_DIR}/claw"
    rm -rf "$TMPDIR"
    ok "Built and installed claw from source → ${BIN_DIR}/claw"
}

# ── Add to PATH ────────────────────────────────────────────────
setup_path() {
    if [ "${CLAW_NO_MODIFY_PATH:-}" = "1" ]; then return; fi

    EXPORT_LINE="export PATH=\"${BIN_DIR}:\$PATH\""

    # Check if already in PATH
    case ":${PATH}:" in
        *":${BIN_DIR}:"*) return ;;
    esac

    # Detect shell and modify profile
    SHELL_NAME="$(basename "${SHELL:-/bin/sh}")"
    case "$SHELL_NAME" in
        zsh)  PROFILE="$HOME/.zshrc" ;;
        bash)
            if [ -f "$HOME/.bash_profile" ]; then
                PROFILE="$HOME/.bash_profile"
            else
                PROFILE="$HOME/.bashrc"
            fi
            ;;
        fish)
            FISH_CONF="$HOME/.config/fish/conf.d"
            mkdir -p "$FISH_CONF"
            echo "set -gx PATH ${BIN_DIR} \$PATH" > "$FISH_CONF/claw.fish"
            ok "Added to PATH (fish)"
            return
            ;;
        *)    PROFILE="$HOME/.profile" ;;
    esac

    if [ -f "$PROFILE" ] && grep -qF "$BIN_DIR" "$PROFILE" 2>/dev/null; then
        return
    fi

    echo "" >> "$PROFILE"
    echo "# Claw AI agent" >> "$PROFILE"
    echo "$EXPORT_LINE" >> "$PROFILE"
    ok "Added to PATH in ${PROFILE}"
    warn "Run: source ${PROFILE}  (or open a new terminal)"
}

# ── Initialize config if first install ─────────────────────────
init_config() {
    CONFIG_FILE="${CLAW_DIR}/claw.toml"
    if [ ! -f "$CONFIG_FILE" ]; then
        info "Creating default config..."
        "${BIN_DIR}/claw" init 2>/dev/null || true
        ok "Config created at ${CONFIG_FILE}"
    fi
}

# ── Main ───────────────────────────────────────────────────────
main() {
    printf "\n${BOLD}🦞 Claw Installer${NC}\n\n"

    detect_platform
    get_target
    info "Platform: ${PLATFORM} (${ARCH})"
    info "Target:   ${TARGET}"

    resolve_version

    if [ -n "$VERSION" ]; then
        info "Version:  ${VERSION}"
        echo ""
        install_binary
    else
        echo ""
        try_build_from_source
    fi

    setup_path
    init_config

    echo ""
    printf "${BOLD}${GREEN}🦞 Claw is ready!${NC}\n"
    echo ""
    echo "  Next steps:"
    echo "    1. Add your API key to ${CLAW_DIR}/claw.toml"
    echo "       [services]"
    echo "       anthropic_api_key = \"sk-ant-...\""
    echo ""
    echo "    2. Start the agent:"
    echo "       claw start"
    echo ""
    echo "    3. Or chat directly:"
    echo "       claw chat"
    echo ""
}

main "$@"

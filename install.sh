#!/usr/bin/env sh
# Mena installer script
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/sxwedo/mena/main/install.sh | sh
# Or with custom options:
#   curl -fsSL https://raw.githubusercontent.com/sxwedo/mena/main/install.sh | MENA_INSTALL_DIR=/usr/local/bin sh

set -e

REPO="sxwedo/mena"
BINARY_NAME="mena"
INSTALL_DIR="${MENA_INSTALL_DIR:-$HOME/.local/bin}"

# Colors for output
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    BOLD=''
    NC=''
fi

info() {
    printf "${BLUE}info:${NC} %s\n" "$1"
}

success() {
    printf "${GREEN}${BOLD}success:${NC} %s\n" "$1"
}

warn() {
    printf "${YELLOW}warning:${NC} %s\n" "$1"
}

error() {
    printf "${RED}error:${NC} %s\n" "$1" >&2
    exit 1
}

# 1. Detect OS and architecture
detect_target() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            TARGET_OS="unknown-linux-gnu"
            ;;
        Darwin)
            TARGET_OS="apple-darwin"
            ;;
        *)
            error "Unsupported operating system: $OS. Please build from source using 'cargo install --git https://github.com/$REPO'."
            ;;
    esac

    case "$ARCH" in
        x86_64|amd64)
            TARGET_ARCH="x86_64"
            ;;
        arm64|aarch64)
            TARGET_ARCH="aarch64"
            ;;
        *)
            error "Unsupported architecture: $ARCH. Please build from source using 'cargo install --git https://github.com/$REPO'."
            ;;
    esac

    TARGET="${TARGET_ARCH}-${TARGET_OS}"
}

# 2. Check downloader (curl or wget)
check_downloader() {
    if command -v curl >/dev/null 2>&1; then
        DOWNLOADER="curl"
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOADER="wget"
    else
        error "Neither curl nor wget was found. Please install one of them to proceed."
    fi
}

download_file() {
    URL="$1"
    OUTPUT="$2"
    if [ "$DOWNLOADER" = "curl" ]; then
        curl -fsSL "$URL" -o "$OUTPUT"
    elif [ "$DOWNLOADER" = "wget" ]; then
        wget -qO "$OUTPUT" "$URL"
    fi
}

fetch_string() {
    URL="$1"
    if [ "$DOWNLOADER" = "curl" ]; then
        curl -fsSL "$URL"
    elif [ "$DOWNLOADER" = "wget" ]; then
        wget -qO- "$URL"
    fi
}

# 3. Resolve version
resolve_version() {
    if [ -n "$MENA_VERSION" ]; then
        TAG="$MENA_VERSION"
        # Ensure tag starts with v if not specified
        case "$TAG" in
            v*) ;;
            *) TAG="v$TAG" ;;
        esac
        info "Using specified version: $TAG"
    else
        info "Resolving latest release for $REPO..."
        LATEST_JSON=$(fetch_string "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null || true)
        TAG=$(echo "$LATEST_JSON" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)

        if [ -z "$TAG" ]; then
            warn "Could not resolve latest version via GitHub API. Falling back to default v0.1.0."
            TAG="v0.1.0"
        else
            info "Found latest release: $TAG"
        fi
    fi
}

# 4. Download and install binary
install_binary() {
    detect_target
    check_downloader
    resolve_version

    ARCHIVE_NAME="mena-${TAG}-${TARGET}.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${ARCHIVE_NAME}"

    TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'mena-install')"
    cleanup() {
        rm -rf "$TMP_DIR"
    }
    trap cleanup EXIT

    info "Downloading $BINARY_NAME from $DOWNLOAD_URL..."
    if ! download_file "$DOWNLOAD_URL" "$TMP_DIR/$ARCHIVE_NAME"; then
        warn "Failed to download pre-built binary for $TARGET."
        if command -v cargo >/dev/null 2>&1; then
            info "Cargo detected. Attempting to build and install from source..."
            cargo install --git "https://github.com/$REPO" --locked
            success "Successfully installed $BINARY_NAME via cargo!"
            exit 0
        else
            error "Binary download failed and cargo is not installed. Please check https://github.com/$REPO for releases."
        fi
    fi

    info "Extracting $ARCHIVE_NAME..."
    tar -xzf "$TMP_DIR/$ARCHIVE_NAME" -C "$TMP_DIR"

    # Find the binary in extracted files
    if [ -f "$TMP_DIR/$BINARY_NAME" ]; then
        BIN_SRC="$TMP_DIR/$BINARY_NAME"
    elif [ -f "$TMP_DIR/mena-${TAG}-${TARGET}/$BINARY_NAME" ]; then
        BIN_SRC="$TMP_DIR/mena-${TAG}-${TARGET}/$BINARY_NAME"
    else
        BIN_SRC="$(find "$TMP_DIR" -type f -name "$BINARY_NAME" | head -n 1)"
    fi

    if [ -z "$BIN_SRC" ] || [ ! -f "$BIN_SRC" ]; then
        error "Could not find $BINARY_NAME executable in the downloaded archive."
    fi

    mkdir -p "$INSTALL_DIR"
    chmod +x "$BIN_SRC"
    mv "$BIN_SRC" "$INSTALL_DIR/$BINARY_NAME"

    success "$BINARY_NAME $TAG has been installed to $INSTALL_DIR/$BINARY_NAME"
}

# 5. Check PATH and provide instructions
check_path() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*)
            ;;
        *)
            printf "\n"
            warn "$INSTALL_DIR is not in your PATH."
            printf "Add the following line to your shell configuration profile:\n\n"
            
            CURRENT_SHELL="$(basename "$SHELL" 2>/dev/null || echo "sh")"
            case "$CURRENT_SHELL" in
                zsh)
                    printf "  ${BOLD}echo 'export PATH=\"\$PATH:%s\"' >> ~/.zshrc${NC}\n" "$INSTALL_DIR"
                    printf "  ${BOLD}source ~/.zshrc${NC}\n\n"
                    ;;
                bash)
                    printf "  ${BOLD}echo 'export PATH=\"\$PATH:%s\"' >> ~/.bashrc${NC}\n" "$INSTALL_DIR"
                    printf "  ${BOLD}source ~/.bashrc${NC}\n\n"
                    ;;
                fish)
                    printf "  ${BOLD}fish_add_path %s${NC}\n\n" "$INSTALL_DIR"
                    ;;
                *)
                    printf "  ${BOLD}export PATH=\"\$PATH:%s\"${NC}\n\n" "$INSTALL_DIR"
                    ;;
            esac
            ;;
    esac
}

install_binary
check_path

printf "\nRun ${BOLD}mena --help${NC} or ${BOLD}mena ag${NC} to get started!\n"

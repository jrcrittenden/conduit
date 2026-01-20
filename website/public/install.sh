#!/bin/sh
# Conduit Installer
# Usage: curl -fsSL https://getconduit.sh/install.sh | sh
#
# This script detects your OS and architecture, downloads the appropriate
# Conduit binary, and installs it to ~/.local/bin (or /usr/local/bin with sudo).

set -e

REPO="conduit-cli/conduit"
INSTALL_DIR="${CONDUIT_INSTALL_DIR:-$HOME/.local/bin}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() {
    printf "${BLUE}==>${NC} %s\n" "$1"
}

success() {
    printf "${GREEN}==>${NC} %s\n" "$1"
}

warn() {
    printf "${YELLOW}Warning:${NC} %s\n" "$1"
}

error() {
    printf "${RED}Error:${NC} %s\n" "$1" >&2
    exit 1
}

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        *)       error "Unsupported operating system: $(uname -s)" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        arm64|aarch64) echo "aarch64" ;;
        *)             error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Show instructions for building from source
build_from_source() {
    echo ""
    warn "Pre-built binary not available for your platform."
    echo ""
    echo "You can build Conduit from source:"
    echo ""
    echo "  ${BLUE}# Install Rust (if not already installed)${NC}"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo ""
    echo "  ${BLUE}# Clone and build${NC}"
    echo "  git clone https://github.com/${REPO}.git"
    echo "  cd conduit"
    echo "  cargo build --release"
    echo ""
    echo "  ${BLUE}# Install the binary${NC}"
    echo "  cp target/release/conduit ~/.local/bin/"
    echo ""
    echo "For more details, see: https://github.com/${REPO}#build-from-source"
    echo ""
    exit 0
}

# Get the download URL for the latest release
get_download_url() {
    local os="$1"
    local arch="$2"
    local target=""
    local ext=""

    case "$os" in
        linux)
            if [ "$arch" = "x86_64" ]; then
                target="x86_64-unknown-linux-musl"
                ext="tar.gz"
            else
                build_from_source
            fi
            ;;
        macos)
            if [ "$arch" = "aarch64" ]; then
                target="aarch64-apple-darwin"
                ext="tar.gz"
            else
                build_from_source
            fi
            ;;
        windows)
            if [ "$arch" = "x86_64" ]; then
                target="x86_64-pc-windows-msvc"
                ext="zip"
            else
                build_from_source
            fi
            ;;
    esac

    # Get latest release tag
    local latest_tag
    latest_tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

    if [ -z "$latest_tag" ]; then
        error "Failed to fetch latest release. Please check your internet connection."
    fi

    echo "https://github.com/${REPO}/releases/download/${latest_tag}/conduit-${target}.${ext}"
}

# Download and extract
download_and_install() {
    local url="$1"
    local os="$2"
    local tmpdir

    tmpdir=$(mktemp -d)
    trap "rm -rf '$tmpdir'" EXIT

    info "Downloading Conduit..."

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$tmpdir/conduit-archive"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$url" -O "$tmpdir/conduit-archive"
    else
        error "Neither curl nor wget found. Please install one of them."
    fi

    info "Extracting..."

    case "$url" in
        *.tar.gz)
            tar -xzf "$tmpdir/conduit-archive" -C "$tmpdir"
            ;;
        *.zip)
            unzip -q "$tmpdir/conduit-archive" -d "$tmpdir"
            ;;
    esac

    # Create install directory if it doesn't exist
    if [ ! -d "$INSTALL_DIR" ]; then
        info "Creating $INSTALL_DIR..."
        mkdir -p "$INSTALL_DIR"
    fi

    # Install the binary
    info "Installing to $INSTALL_DIR/conduit..."

    if [ "$os" = "windows" ]; then
        mv "$tmpdir/conduit.exe" "$INSTALL_DIR/conduit.exe"
    else
        mv "$tmpdir/conduit" "$INSTALL_DIR/conduit"
        chmod +x "$INSTALL_DIR/conduit"
    fi
}

# Check if directory is in PATH
check_path() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return 0 ;;
        *) return 1 ;;
    esac
}

# Main installation
main() {
    echo ""
    printf "  ${GREEN} ░██████                               ░██            ░██   ░██${NC}\n"
    printf "  ${GREEN}░██   ░██                              ░██                  ░██${NC}\n"
    printf "  ${GREEN}░██        ░███████  ░████████   ░████████ ░██    ░██ ░██░████████${NC}\n"
    printf "  ${GREEN}░██       ░██    ░██ ░██    ░██ ░██    ░██ ░██    ░██ ░██   ░██${NC}\n"
    printf "  ${GREEN}░██       ░██    ░██ ░██    ░██ ░██    ░██ ░██    ░██ ░██   ░██${NC}\n"
    printf "  ${GREEN}░██   ░██ ░██    ░██ ░██    ░██ ░██   ░███ ░██   ░███ ░██   ░██${NC}\n"
    printf "  ${GREEN} ░██████   ░███████  ░██    ░██  ░█████░██  ░█████░██ ░██    ░████${NC}\n"
    echo ""
    echo "  Multi-Agent TUI for AI Coding Assistants"
    echo ""

    local os arch url

    os=$(detect_os)
    arch=$(detect_arch)

    info "Detected: $os ($arch)"

    url=$(get_download_url "$os" "$arch")

    info "Release URL: $url"

    download_and_install "$url" "$os"

    success "Conduit installed successfully!"
    echo ""

    # Check if install directory is in PATH
    if ! check_path; then
        warn "$INSTALL_DIR is not in your PATH"
        echo ""
        echo "Add it to your shell configuration:"
        echo ""
        case "$(basename "$SHELL")" in
            zsh)
                echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc"
                echo "  source ~/.zshrc"
                ;;
            bash)
                echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
                echo "  source ~/.bashrc"
                ;;
            fish)
                echo "  fish_add_path ~/.local/bin"
                ;;
            *)
                echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
                ;;
        esac
        echo ""
    fi

    echo "Get started:"
    echo ""
    echo "  conduit"
    echo ""
    echo "For help:"
    echo ""
    echo "  conduit --help"
    echo ""
    echo "Documentation: https://getconduit.sh/docs"
    echo "GitHub: https://github.com/${REPO}"
    echo ""
}

main "$@"

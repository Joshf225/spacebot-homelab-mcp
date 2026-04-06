#!/usr/bin/env bash
# Install script for spacebot-homelab-mcp
# Usage: curl -fsSL https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/master/install.sh | bash

set -euo pipefail

# Detect Windows environments and redirect to PowerShell installer
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    echo "Detected Windows environment. Use install.ps1 instead:"
    echo "  irm https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/master/install.ps1 | iex"
    exit 1
    ;;
esac

REPO="Joshf225/spacebot-homelab-mcp"
BINARY="spacebot-homelab-mcp"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

info() { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
error() { printf "\033[1;31merror:\033[0m %s\n" "$1" >&2; exit 1; }

detect_platform() {
  local os arch

  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)  os="unknown-linux-gnu" ;;
    Darwin) os="apple-darwin" ;;
    *)      error "Unsupported OS: $os" ;;
  esac

  case "$arch" in
    x86_64|amd64)  arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *)             error "Unsupported architecture: $arch" ;;
  esac

  echo "${arch}-${os}"
}

get_latest_version() {
  local url="https://api.github.com/repos/${REPO}/releases/latest"

  if command -v curl &>/dev/null; then
    curl -fsSL "$url" | grep '"tag_name"' | sed -E 's/.*"v([^"]+)".*/\1/'
  elif command -v wget &>/dev/null; then
    wget -qO- "$url" | grep '"tag_name"' | sed -E 's/.*"v([^"]+)".*/\1/'
  else
    error "Neither curl nor wget found. Please install one."
  fi
}

download() {
  local url="$1" dest="$2"

  if command -v curl &>/dev/null; then
    curl -fsSL "$url" -o "$dest"
  elif command -v wget &>/dev/null; then
    wget -qO "$dest" "$url"
  fi
}

main() {
  local version="${VERSION:-}"
  local platform

  platform="$(detect_platform)"

  if [ -z "$version" ]; then
    info "Fetching latest release..."
    version="$(get_latest_version)"
  fi

  if [ -z "$version" ]; then
    error "Could not determine latest version. Set VERSION=x.y.z manually."
  fi

  info "Installing ${BINARY} v${version} for ${platform}"

  local archive="${BINARY}-${version}-${platform}.tar.gz"
  local url="https://github.com/${REPO}/releases/download/v${version}/${archive}"

  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT

  info "Downloading ${url}..."
  download "$url" "${tmpdir}/${archive}"

  info "Extracting..."
  tar xzf "${tmpdir}/${archive}" -C "$tmpdir"

  if [ ! -f "${tmpdir}/${BINARY}" ]; then
    error "Binary not found in archive"
  fi

  chmod +x "${tmpdir}/${BINARY}"

  # Try direct install, fall back to sudo
  if [ -w "$INSTALL_DIR" ]; then
    mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
  else
    info "Elevated permissions needed to install to ${INSTALL_DIR}"
    sudo mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
  fi

  info "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

  # Verify
  if command -v "$BINARY" &>/dev/null; then
    info "Verify: $(command -v "$BINARY")"
  else
    printf "\n"
    info "Binary installed but not on PATH. Add to your shell config:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
  fi

  # Offer interactive setup wizard, or print manual next steps
  printf "\n"
  if [ -t 0 ] && [ -t 1 ]; then
    info "Setup wizard available!"
    echo "  The setup wizard will walk you through configuring SSH hosts,"
    echo "  Docker connections, confirmation rules, and rate limits."
    printf "\n"
    read -r -p "  Start the setup wizard now? [Y/n] " answer </dev/tty
    case "${answer:-Y}" in
      [nN]*)
        print_next_steps
        ;;
      *)
        printf "\n"
        "${INSTALL_DIR}/${BINARY}" setup </dev/tty
        ;;
    esac
  else
    print_next_steps
  fi
}

print_next_steps() {
  info "Next steps:"
  echo "  1. Run the setup wizard:"
  echo "     ${BINARY} setup"
  echo ""
  echo "  Or configure manually:"
  echo "  1. Create config: mkdir -p ~/.config/spacebot-homelab-mcp && cp example.config.toml ~/.config/spacebot-homelab-mcp/config.toml"
  echo "  2. Edit config with your Docker/SSH hosts"
  echo "  3. Validate: ${BINARY} doctor --config ~/.config/spacebot-homelab-mcp/config.toml"
  echo "  4. Add to Spacebot config.toml:"
  echo ""
  echo "     [[mcp_servers]]"
  echo "     name = \"homelab\""
  echo "     transport = \"stdio\""
  echo "     command = \"${BINARY}\""
  echo "     args = [\"server\", \"--config\", \"~/.config/spacebot-homelab-mcp/config.toml\"]"
}

main

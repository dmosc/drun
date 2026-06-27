#!/usr/bin/env bash
set -euo pipefail

REPO="dmosc/drun"
VERSION="${1:-latest}"

# ── platform detection ────────────────────────────────────────────────────────

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os-$arch" in
    Darwin-arm64)  ASSET="drun-mcp-macos-arm64" ;;
    Darwin-x86_64) ASSET="drun-mcp-macos-x86_64" ;;
    Linux-x86_64)  ASSET="drun-mcp-linux-x86_64" ;;
    *)
      echo "Unsupported platform: $os-$arch"
      exit 1
      ;;
  esac
}

# ── binary update ─────────────────────────────────────────────────────────────

update_binary() {
  if ! BIN="$(command -v drun-mcp 2>/dev/null)"; then
    echo "drun-mcp is not installed. Run the install script first:"
    echo "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash"
    exit 1
  fi

  local url
  if [[ "$VERSION" == "latest" ]]; then
    url="https://github.com/$REPO/releases/latest/download/$ASSET"
  else
    url="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
  fi

  echo "Updating drun-mcp to $VERSION..."

  if [[ -w "$BIN" ]]; then
    curl -fsSL "$url" -o "$BIN"
  else
    sudo curl -fsSL "$url" -o "$BIN"
  fi
  chmod +x "$BIN" 2>/dev/null || sudo chmod +x "$BIN"

  echo "Done. drun-mcp updated at $BIN."
}

# ── main ──────────────────────────────────────────────────────────────────────

detect_platform
update_binary

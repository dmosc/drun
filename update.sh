#!/usr/bin/env bash
set -euo pipefail

REPO="dmosc/drun"
VERSION="${1:-latest}"

# ── Find existing install ──────────────────────────────────────────────────────

if ! BIN="$(command -v drun-mcp 2>/dev/null)"; then
  echo "drun-mcp is not installed. Run the install script first:"
  echo "  curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash"
  exit 1
fi

# ── Platform detection ────────────────────────────────────────────────────────

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS-$ARCH" in
  Darwin-arm64)  ASSET="drun-mcp-macos-arm64" ;;
  Darwin-x86_64) ASSET="drun-mcp-macos-x86_64" ;;
  Linux-x86_64)  ASSET="drun-mcp-linux-x86_64" ;;
  *)
    echo "Unsupported platform: $OS-$ARCH"
    exit 1
    ;;
esac

# ── Download ──────────────────────────────────────────────────────────────────

if [[ "$VERSION" == "latest" ]]; then
  URL="https://github.com/$REPO/releases/latest/download/$ASSET"
else
  URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
fi

echo "Updating drun-mcp to $VERSION..."

if [[ -w "$BIN" ]]; then
  curl -fsSL "$URL" -o "$BIN"
else
  sudo curl -fsSL "$URL" -o "$BIN"
fi
chmod +x "$BIN" 2>/dev/null || sudo chmod +x "$BIN"

echo "Done. drun-mcp updated at $BIN."

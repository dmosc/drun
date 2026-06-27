#!/usr/bin/env bash
set -euo pipefail

REPO="dmosc/drun"
BIN_DIR="/usr/local/bin"
BIN="$BIN_DIR/drun-mcp"

if command -v drun-mcp &>/dev/null; then
  echo "drun-mcp is already installed. To upgrade, remove it first:"
  echo "  sudo rm $(command -v drun-mcp)"
  exit 0
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

# ── drun-mcp binary ───────────────────────────────────────────────────────────

echo "Downloading drun-mcp..."
LATEST_URL="https://github.com/$REPO/releases/latest/download/$ASSET"

if [[ -w "$BIN_DIR" ]]; then
  curl -fsSL "$LATEST_URL" -o "$BIN"
else
  sudo curl -fsSL "$LATEST_URL" -o "$BIN"
fi
chmod +x "$BIN" 2>/dev/null || sudo chmod +x "$BIN"

echo "drun-mcp installed to $BIN."

# ── Claude Code MCP registration ─────────────────────────────────────────────

if command -v claude &>/dev/null; then
  claude mcp add drun -- "$BIN"
  echo "drun added to Claude Code."
else
  echo ""
  echo "Claude Code CLI not found. Add drun manually:"
  echo "  claude mcp add drun -- $BIN"
fi

echo "Done! drun is ready."

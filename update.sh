#!/usr/bin/env bash
set -euo pipefail

REPO="dmosc/drun"
VERSION="${1:-latest}"
LAUNCHD_PLIST="$HOME/Library/LaunchAgents/com.drun.mcp-server.plist"

# ── platform detection ────────────────────────────────────────────────────────

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os-$arch" in
    Darwin-arm64)  ASSET="drun-mcp-macos-arm64" ;;
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

  local tmp="$(dirname "$BIN")/.drun-mcp.tmp.$$"
  trap 'rm -f "$tmp"' EXIT

  if [[ -w "$(dirname "$BIN")" ]]; then
    curl -fsSL "$url" -o "$tmp"
    chmod +x "$tmp"
    mv -f "$tmp" "$BIN"
  else
    sudo curl -fsSL "$url" -o "$tmp"
    sudo chmod +x "$tmp"
    sudo mv -f "$tmp" "$BIN"
  fi
  trap - EXIT

  echo "drun-mcp updated at $BIN."
}

# ── daemon restart ────────────────────────────────────────────────────────────

restart_daemon() {
  case "$(uname -s)" in
    Darwin)
      if [[ -f "$LAUNCHD_PLIST" ]]; then
        launchctl unload "$LAUNCHD_PLIST" 2>/dev/null || true
        launchctl load -w "$LAUNCHD_PLIST"
        echo "drun-mcp daemon restarted — any active sessions were lost."
      else
        pkill -f "drun-mcp\$" 2>/dev/null || true
        echo "No launchd agent found — killed any running drun-mcp daemon process."
      fi
      ;;
    Linux)
      if systemctl --user is-active drun-mcp.service &>/dev/null; then
        systemctl --user restart drun-mcp.service
        echo "drun-mcp daemon restarted — any active sessions were lost."
      else
        pkill -f "drun-mcp\$" 2>/dev/null || true
      fi
      ;;
  esac
}

# ── main ──────────────────────────────────────────────────────────────────────

detect_platform
update_binary
restart_daemon

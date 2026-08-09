#!/usr/bin/env bash
set -euo pipefail

REPO="dmosc/drun"
BIN_DIR="/usr/local/bin"
BIN="$BIN_DIR/drun-mcp"
DRUN_HOME="$HOME/.drun"
DRUN_CONFIG="$DRUN_HOME/config.toml"
LAUNCHD_LABEL="com.drun.mcp-server"
LAUNCHD_PLIST="$HOME/Library/LaunchAgents/$LAUNCHD_LABEL.plist"
SYSTEMD_SERVICE="$HOME/.config/systemd/user/drun-mcp.service"
MCP_PORT=7273
MCP_URL="http://127.0.0.1:$MCP_PORT/sse"
SETUP_PORT=7275
SETUP_URL="http://127.0.0.1:$SETUP_PORT"

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

# ── binary installation ───────────────────────────────────────────────────────

install_binary() {
  if command -v drun-mcp &>/dev/null; then
    echo "drun-mcp already installed at $(command -v drun-mcp), skipping."
    return
  fi

  echo "Downloading drun-mcp..."
  local url="https://github.com/$REPO/releases/latest/download/$ASSET"
  local tmp="$BIN_DIR/.drun-mcp.tmp.$$"
  trap 'rm -f "$tmp"' EXIT

  if [[ -w "$BIN_DIR" ]]; then
    curl -fsSL "$url" -o "$tmp"
    chmod +x "$tmp"
    mv -f "$tmp" "$BIN"
  else
    sudo curl -fsSL "$url" -o "$tmp"
    sudo chmod +x "$tmp"
    sudo mv -f "$tmp" "$BIN"
  fi
  trap - EXIT

  echo "drun-mcp installed to $BIN."
}

# ── global daemon config ──────────────────────────────────────────────────────

create_config() {
  mkdir -p "$DRUN_HOME"

  if [[ -f "$DRUN_CONFIG" ]]; then
    echo "Existing config kept at $DRUN_CONFIG."
    return
  fi

  cat > "$DRUN_CONFIG" <<EOF
# Domains agents may reach via session_fetch.
domain_allowlist = []

# Timeout for session_fetch HTTP requests (full response), in milliseconds.
fetch_timeout_ms = 60_000

# Timeout for establishing a TCP connection during session_fetch, in milliseconds.
connect_timeout_ms = 30_000

# Maximum workspace size per session, in megabytes.
max_workspace_mb = 512

# Maximum number of concurrent sessions.
max_sessions = 50

# Maximum number of checkpoints per session.
max_checkpoints = 200

# Seconds of inactivity before a session is considered abandoned.
session_idle_timeout_secs = 3600

# Host path prefixes available to drun sessions for read and write operations.
mount_allowlist = []

# Directory names that session_mount registers as read-only host overlays
# instead of loading into the workspace. Symlinked at execution time, never
# checkpointed. Set to [] to disable.
mount_overlay_paths = ["node_modules", ".venv", "venv", "target", "__pycache__", ".git"]

# Directory where session_snapshot writes .drun files.
snapshots_dir = "$DRUN_HOME/snapshots"

# Automatically snapshot when session_close is called.
snapshot_on_close = false

# Host environment variable names exposed to agents via session_get_env.
env_allowlist = []

# Timeout for session_bash calls, in milliseconds.
bash_timeout_ms = 30_000

# Enables session_package_install (pip/npm installs). Disabled by default since
# it gives the sandboxed install process network access.
package_install_enabled = false

# Timeout for session_package_install calls, in milliseconds.
package_install_timeout_ms = 180_000

# Shell command substrings that are always denied.
bash_command_denylist = []

# Shell command substrings that are permitted (empty = all allowed except denylist).
bash_command_allowlist = []

# TCP port for the embedded trajectory viewer web UI. Set to 0, or remove the field, to disable.
web_port = 7274
EOF

  echo "Created config at $DRUN_CONFIG."
}

# ── background daemon ─────────────────────────────────────────────────────────

install_launchd_agent() {
  mkdir -p "$(dirname "$LAUNCHD_PLIST")"
  cat > "$LAUNCHD_PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$LAUNCHD_LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>$BIN</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>DRUN_CONFIG</key>
        <string>$DRUN_CONFIG</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$DRUN_HOME/drun-mcp.log</string>
    <key>StandardErrorPath</key>
    <string>$DRUN_HOME/drun-mcp.log</string>
</dict>
</plist>
EOF

  launchctl unload "$LAUNCHD_PLIST" 2>/dev/null || true
  launchctl load -w "$LAUNCHD_PLIST"
  echo "drun-mcp daemon started via launchd (auto-restarts on login)."
}

install_systemd_service() {
  mkdir -p "$(dirname "$SYSTEMD_SERVICE")"
  cat > "$SYSTEMD_SERVICE" <<EOF
[Unit]
Description=drun MCP server
After=network.target

[Service]
ExecStart=$BIN
Environment=DRUN_CONFIG=$DRUN_CONFIG
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
EOF

  systemctl --user daemon-reload
  systemctl --user enable --now drun-mcp.service
  echo "drun-mcp daemon started via systemd user service."
}

install_daemon() {
  case "$(uname -s)" in
    Darwin) install_launchd_agent ;;
    Linux)  install_systemd_service ;;
  esac
}

# ── setup wizard ───────────────────────────────────────────────────────────

launch_setup_wizard() {
  echo "Opening the setup wizard in your browser to finish installing dependencies."
  nohup "$BIN" setup >"$DRUN_HOME/setup.log" 2>&1 &
  disown
}

# ── main ──────────────────────────────────────────────────────────────────────

detect_platform
install_binary
create_config
install_daemon
launch_setup_wizard

echo ""
echo "Done! drun is ready."
echo "  MCP     → $MCP_URL"
echo "  UI      → http://127.0.0.1:7274"
echo "  Wizard  → $SETUP_URL"
echo ""
echo "Run 'drun-mcp bridges list' to integrate drun with different providers."
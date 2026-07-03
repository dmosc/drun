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

  if [[ -w "$BIN_DIR" ]]; then
    curl -fsSL "$url" -o "$BIN"
  else
    sudo curl -fsSL "$url" -o "$BIN"
  fi
  chmod +x "$BIN" 2>/dev/null || sudo chmod +x "$BIN"

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
# drun configuration — all fields are optional; these are the defaults.

# Domains agents may reach via session_fetch. PyPI domains are always added
# on top of whatever you list here.
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

# Host path prefixes agents may mount into a session. Empty = all paths allowed.
mount_allowlist = []

# Directory names that session_mount registers as read-only host overlays
# instead of loading into the workspace. Symlinked at execution time, never
# checkpointed. Set to [] to disable.
mount_overlay_paths = ["node_modules", ".venv", "venv", "target", "__pycache__", ".git"]

# Directory where session_export writes files.
export_root = "$DRUN_HOME/exports"

# Directory where session_snapshot writes .drun files.
snapshots_dir = "$DRUN_HOME/snapshots"

# Automatically snapshot when session_close is called.
snapshot_on_close = false

# Host environment variable names exposed to agents via session_get_env.
env_allowlist = []

# Timeout for session_bash calls, in milliseconds.
bash_timeout_ms = 30_000

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

# ── Claude Code project settings ─────────────────────────────────────────────

create_claude_settings() {
  local settings_dir="$PWD/.claude"
  local settings_file="$settings_dir/settings.json"

  mkdir -p "$settings_dir"

  if [[ -f "$settings_file" ]]; then
    echo "Existing .claude/settings.json kept at $settings_file."
    return
  fi

  cat > "$settings_file" <<'EOF'
{
  "permissions": {
    "deny": [
      "Bash", "BashOutput", "KillBash",
      "Edit", "Write", "NotebookEdit",
      "Read", "Glob", "Grep",
      "WebFetch", "WebSearch",
      "Task"
    ],
    "allow": ["mcp__drun__*"]
  }
}
EOF

  echo "Created .claude/settings.json."
}

create_claude_md() {
  local claude_md="$PWD/CLAUDE.md"

  if [[ -f "$claude_md" ]]; then
    echo "Existing CLAUDE.md kept at $claude_md."
    return
  fi

  cat > "$claude_md" <<EOF
# Agent instructions

This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.
Native file, shell, and network tools (\`Bash\`, \`Edit\`, \`Write\`,
\`NotebookEdit\`, \`Read\`, \`Glob\`, \`Grep\`, \`WebFetch\`, \`WebSearch\`,
\`Task\`) are disabled for this workspace — they would otherwise read or write
the host directly, bypassing the sandbox. Use the drun MCP tools (prefixed
\`mcp__drun__\`) for everything.

## Getting started

1. Call \`create_session\` — sessions start with an empty workspace.
2. Call \`session_mount\` with path \`$PWD\` to load this project's files into
   the session. Re-mount any other host paths you need the same way.
3. From there, work entirely through drun tools — there is no host file or
   shell access outside of them.

## Core tools

- **\`session_bash\`** — run shell commands in the sandboxed workspace
  (also covers listing/searching files — e.g. \`ls\`, \`grep\`, \`find\`)
- **\`session_read_file\`** / **\`session_write_file\`** / **\`session_delete_file\`**
  — read, write, and delete files in the session
- **\`session_mount\`** — load a host file or directory into the session
- **\`session_fetch\`** — make HTTP requests from the sandbox (subject to
  the server's domain_allowlist)
- **\`session_export\`** — write session files back out to the host
- **\`session_diff\`** / **\`session_rollback\`** / **\`session_fork\`**
  — inspect and navigate checkpoint history (session_rollback is
  destructive past the rollback point once you continue the session — use
  session_fork first if you need to keep that history)
EOF

  echo "Created CLAUDE.md."
}

# ── Claude Code MCP registration ──────────────────────────────────────────────

register_mcp() {
  local registry="$DRUN_HOME/projects"
  grep -qxF "$PWD" "$registry" 2>/dev/null || echo "$PWD" >> "$registry"

  if ! command -v claude &>/dev/null; then
    echo ""
    echo "Claude Code CLI not found. Add drun manually:"
    echo "  claude mcp add --transport sse drun $MCP_URL"
    return
  fi

  if claude mcp list 2>/dev/null | grep -q "^drun"; then
    echo "drun already registered in Claude Code, skipping."
    return
  fi

  claude mcp add --transport sse drun "$MCP_URL"
  echo "drun added to Claude Code (SSE → $MCP_URL)."
}

# ── main ──────────────────────────────────────────────────────────────────────

detect_platform
install_binary
create_config
install_daemon
create_claude_settings
create_claude_md
register_mcp

echo "Done! drun is ready."
echo "  MCP  → $MCP_URL"
echo "  UI   → http://127.0.0.1:7274"

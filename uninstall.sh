#!/usr/bin/env bash
# Thin bootstrap script for uninstalling drun without requiring a working binary.
# Prefer `drun uninstall` for normal use.
set -euo pipefail

DRUN_REGISTRY="$HOME/.drun/projects"
LAUNCHD_PLIST="$HOME/Library/LaunchAgents/com.drun.mcp-server.plist"
SYSTEMD_SERVICE="$HOME/.config/systemd/user/drun-mcp.service"

# ── daemon removal ────────────────────────────────────────────────────────────

remove_daemon() {
  case "$(uname -s)" in
    Darwin)
      if [[ -f "$LAUNCHD_PLIST" ]]; then
        launchctl unload "$LAUNCHD_PLIST" 2>/dev/null || true
        rm -f "$LAUNCHD_PLIST"
        echo "Removed launchd agent."
      fi
      ;;
    Linux)
      if [[ -f "$SYSTEMD_SERVICE" ]]; then
        systemctl --user disable --now drun-mcp.service 2>/dev/null || true
        rm -f "$SYSTEMD_SERVICE"
        systemctl --user daemon-reload
        echo "Removed systemd user service."
      fi
      ;;
  esac
  pkill -f drun 2>/dev/null || true
}

# ── Claude Code MCP deregistration ───────────────────────────────────────────

deregister_mcp() {
  if ! command -v claude &>/dev/null; then
    return
  fi

  claude mcp remove --scope user drun 2>/dev/null && echo "Removed drun from Claude Code." || true
}

# ── per-project cleanup ───────────────────────────────────────────────────────

cleanup_project_settings() {
  [[ ! -f "$DRUN_REGISTRY" ]] && return

  while IFS= read -r project_dir; do
    [[ -z "$project_dir" ]] && continue
    local settings_file="$project_dir/.claude/settings.json"
    if [[ -f "$settings_file" ]]; then
      rm -f "$settings_file"
      rmdir "$project_dir/.claude" 2>/dev/null || true
      echo "Removed .claude/settings.json from $project_dir."
    fi
    [[ -f "$project_dir/CLAUDE.md" ]] && \
      echo "Left CLAUDE.md at $project_dir/CLAUDE.md — run \`drun deinit\` to remove it."
  done < "$DRUN_REGISTRY"

  rm -f "$DRUN_REGISTRY"
}

# ── binary removal ────────────────────────────────────────────────────────────

remove_binary() {
  if ! BIN="$(command -v drun 2>/dev/null)"; then
    echo "drun is not installed."
    return
  fi

  if [[ -w "$BIN" ]]; then
    rm "$BIN"
  else
    sudo rm "$BIN"
  fi

  echo "drun removed from $BIN."
}

# ── main ──────────────────────────────────────────────────────────────────────

remove_daemon
deregister_mcp
cleanup_project_settings
remove_binary

echo "Done."
echo "Preserved: ~/.drun/config.toml, exports/, snapshots/ — remove manually if not needed."

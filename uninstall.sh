#!/usr/bin/env bash
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
  pkill -f "drun-mcp\$" 2>/dev/null || true
}

# ── Bridge deregistration ─────────────────────────────────────────────────────

deregister_bridges() {
  if command -v drun-mcp &>/dev/null; then
    drun-mcp bridges deregister-all 2>/dev/null || true
  fi
}

# ── per-project cleanup ───────────────────────────────────────────────────────
BRIDGE_SETTINGS_FILES=(".claude/settings.json" ".gemini/settings.json")
BRIDGE_CONTEXT_FILES=("CLAUDE.md" "GEMINI.md" "AGENTS.md" "HERMES.md")

cleanup_project_settings() {
  [[ ! -f "$DRUN_REGISTRY" ]] && return

  while IFS= read -r project_dir; do
    [[ -z "$project_dir" ]] && continue

    for settings_file in "${BRIDGE_SETTINGS_FILES[@]}"; do
      local full_path="$project_dir/$settings_file"
      if [[ -f "$full_path" ]]; then
        rm -f "$full_path"
        rmdir "$(dirname "$full_path")" 2>/dev/null || true
        echo "Removed $settings_file from $project_dir."
      fi
    done

    for context_file in "${BRIDGE_CONTEXT_FILES[@]}"; do
      [[ -f "$project_dir/$context_file" ]] && \
        echo "Left $context_file at $project_dir/$context_file — delete manually if not needed."
    done
  done < "$DRUN_REGISTRY"

  rm -f "$DRUN_REGISTRY"
}

# ── binary removal ────────────────────────────────────────────────────────────

remove_binary() {
  if ! BIN="$(command -v drun-mcp 2>/dev/null)"; then
    echo "drun-mcp is not installed."
    return
  fi

  if [[ -w "$BIN" ]]; then
    rm "$BIN"
  else
    sudo rm "$BIN"
  fi

  echo "drun-mcp removed from $BIN."
}

# ── main ──────────────────────────────────────────────────────────────────────

remove_daemon
deregister_bridges
cleanup_project_settings
remove_binary

echo "Done."
echo "Preserved: ~/.drun/config.toml, exports/, snapshots/ — remove manually if not needed."

#!/usr/bin/env bash
set -euo pipefail

DRUN_REGISTRY="$HOME/.drun/projects"

# ── Claude Code MCP deregistration ───────────────────────────────────────────

deregister_mcp() {
  if ! command -v claude &>/dev/null; then
    return
  fi

  claude mcp remove drun -s user 2>/dev/null && echo "Removed drun from user scope." || true

  if [[ ! -f "$DRUN_REGISTRY" ]]; then
    return
  fi

  while IFS= read -r project_dir; do
    [[ -z "$project_dir" ]] && continue
    for scope in local project; do
      if (cd "$project_dir" && claude mcp remove drun -s "$scope" 2>/dev/null); then
        echo "Removed drun from $project_dir ($scope scope)."
      fi
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

deregister_mcp
remove_binary

echo "Done."

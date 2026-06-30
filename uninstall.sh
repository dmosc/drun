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

# ── Claude Code project settings cleanup ──────────────────────────────────────

cleanup_project_files() {
 if [[ ! -f "$DRUN_REGISTRY" ]]; then
   return
 fi


 while IFS= read -r project_dir; do
   [[ -z "$project_dir" ]] && continue

   local settings_file="$project_dir/.claude/settings.json"
   if [[ -f "$settings_file" ]]; then
     rm -f "$settings_file"
     rmdir "$project_dir/.claude" 2>/dev/null || true
     echo "Removed .claude/settings.json from $project_dir."
   fi

   if [[ -f "$project_dir/CLAUDE.md" ]]; then
     echo "Left CLAUDE.md at $project_dir/CLAUDE.md — delete manually if not needed."
   fi
 done < "$DRUN_REGISTRY"
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
cleanup_project_files
remove_binary

echo "Done."

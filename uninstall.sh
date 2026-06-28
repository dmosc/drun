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

# ── Remove session guard hooks ────────────────────────────────────────────────

python3 - << 'PYEOF'
import json, os

settings_path = os.path.expanduser("~/.claude/settings.json")
hooks_script  = os.path.expanduser("~/.drun/session_hooks.sh")

if os.path.exists(settings_path):
    with open(settings_path) as f:
        settings = json.load(f)
    hooks = settings.get("hooks", {})
    for phase in ("PostToolUse", "PreToolUse"):
        if phase in hooks:
            hooks[phase] = [
                e for e in hooks[phase]
                if not any(hooks_script in h.get("command", "") for h in e.get("hooks", []))
            ]
            if not hooks[phase]:
                del hooks[phase]
    if not hooks:
        settings.pop("hooks", None)
    with open(settings_path, "w") as f:
        json.dump(settings, f, indent=2)
        f.write("\n")
    print(f"drun hooks removed from {settings_path}.")

for path in (hooks_script, os.path.expanduser("~/.drun/session_count")):
    if os.path.exists(path):
        os.remove(path)
        print(f"Removed {path}.")
PYEOF

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

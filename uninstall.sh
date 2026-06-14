#!/usr/bin/env bash
set -euo pipefail

# ── Deregister from Claude Code (all scopes) ─────────────────────────────────

if command -v claude &>/dev/null; then
  removed=false
  for scope in local user project; do
    if claude mcp remove drun -s "$scope" 2>/dev/null; then
      removed=true
    fi
  done
  if $removed; then
    echo "drun removed from Claude Code."
  fi
fi

# ── Remove binary ─────────────────────────────────────────────────────────────

if ! BIN="$(command -v drun-mcp 2>/dev/null)"; then
  echo "drun-mcp is not installed."
  exit 0
fi

if [[ -w "$BIN" ]]; then
  rm "$BIN"
else
  sudo rm "$BIN"
fi

echo "drun-mcp removed from $BIN."

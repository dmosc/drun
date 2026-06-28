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

# ── drun config ───────────────────────────────────────────────────────────────

DRUN_DIR="$HOME/.drun"
DRUN_CONFIG_FILE="$DRUN_DIR/drun.toml"
SAMPLE_TOML_URL="https://raw.githubusercontent.com/$REPO/main/sample.toml"

mkdir -p "$DRUN_DIR"

if [[ ! -f "$DRUN_CONFIG_FILE" ]]; then
  if curl -fsSL "$SAMPLE_TOML_URL" -o "$DRUN_CONFIG_FILE"; then
    echo "Created default config at $DRUN_CONFIG_FILE."
  else
    echo "Warning: could not download sample.toml; created empty config at $DRUN_CONFIG_FILE."
    touch "$DRUN_CONFIG_FILE"
  fi
else
  echo "Existing config kept at $DRUN_CONFIG_FILE."
fi

# ── Claude Code MCP registration ─────────────────────────────────────────────

if command -v claude &>/dev/null; then
  claude mcp add drun -e "DRUN_CONFIG=$DRUN_CONFIG_FILE" -- "$BIN"
  echo "drun added to Claude Code."
else
  echo ""
  echo "Claude Code CLI not found. Add drun manually:"
  echo "  claude mcp add drun -e DRUN_CONFIG=$DRUN_CONFIG_FILE -- $BIN"
fi

# ── Session guard hooks ───────────────────────────────────────────────────────

HOOKS_SCRIPT="$DRUN_DIR/session_hooks.sh"

cat > "$HOOKS_SCRIPT" << 'HOOKS_EOF'
#!/bin/sh
COUNTER_FILE="$HOME/.drun/session_count"
count() { cat "$COUNTER_FILE" 2>/dev/null || echo 0; }
case "$1" in
  increment) echo $(( $(count) + 1 )) > "$COUNTER_FILE" ;;
  decrement) c=$(count); echo $(( c > 0 ? c - 1 : 0 )) > "$COUNTER_FILE" ;;
  guard)
    if [ "$(count)" -gt 0 ]; then
      echo "drun session active — use drun session tools instead of host tools."
      exit 2
    fi ;;
esac
HOOKS_EOF

chmod +x "$HOOKS_SCRIPT"

python3 - << PYEOF
import json, os

settings_path = os.path.expanduser("~/.claude/settings.json")
hooks_script = "$HOOKS_SCRIPT"

if os.path.exists(settings_path):
    with open(settings_path) as f:
        settings = json.load(f)
else:
    settings = {}

hooks = settings.setdefault("hooks", {})
post  = hooks.setdefault("PostToolUse", [])
pre   = hooks.setdefault("PreToolUse",  [])

new_post = [
    {"matcher": "mcp__drun__create_session", "hooks": [{"type": "command", "command": f"{hooks_script} increment"}]},
    {"matcher": "mcp__drun__session_close",  "hooks": [{"type": "command", "command": f"{hooks_script} decrement"}]},
]
new_pre = [
    {"matcher": "Bash|Edit|Write|NotebookEdit", "hooks": [{"type": "command", "command": f"{hooks_script} guard"}]},
]

existing_post = {e["matcher"] for e in post}
existing_pre  = {e["matcher"] for e in pre}

for entry in new_post:
    if entry["matcher"] not in existing_post:
        post.append(entry)
for entry in new_pre:
    if entry["matcher"] not in existing_pre:
        pre.append(entry)

os.makedirs(os.path.dirname(os.path.abspath(settings_path)), exist_ok=True)
with open(settings_path, "w") as f:
    json.dump(settings, f, indent=2)
    f.write("\n")

print(f"Session guard hooks registered in {settings_path}.")
PYEOF

echo "Done! drun is ready."

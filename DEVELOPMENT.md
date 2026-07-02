# Development

## Build

```bash
cargo build -p drun-mcp
```

## Run locally

If you installed drun via `install.sh`, the launchd agent (macOS) or systemd
service (Linux) will restart the installed binary within milliseconds of any
`pkill`. Suspend it first so the local build can bind the ports:

```bash
# macOS — suspend the launchd agent
launchctl unload ~/Library/LaunchAgents/com.drun.mcp-server.plist 2>/dev/null

# Linux — suspend the systemd service
systemctl --user stop drun-mcp.service

# Kill any process still holding the ports, then start the local build
pkill -f drun-mcp 2>/dev/null; sleep 0.3
DRUN_CONFIG="$PWD/.drun/config.toml" ./target/debug/drun-mcp
```

Expected startup output:

```
drun: MCP → http://127.0.0.1:7273/mcp (streamable HTTP)
drun: MCP → http://127.0.0.1:7273/sse (SSE)
drun: web UI → http://127.0.0.1:7274
```

## Register with Claude Code

```bash
# Remove the previously installed entry, if any
claude mcp remove drun 2>/dev/null

# Point Claude Code at the local daemon
claude mcp add --transport sse drun http://127.0.0.1:7273/sse

# Confirm
claude mcp list
```

## Validate

```bash
# SSE endpoint (Ctrl-C to exit)
curl -N http://127.0.0.1:7273/sse

# Streamable HTTP endpoint
curl http://127.0.0.1:7273/mcp

# Sessions API (returns empty JSON tree before any sessions exist)
curl -s http://127.0.0.1:7274/api/sessions/tree

# Confirm exactly one process is running
ps aux | grep drun-mcp
```

Open a new chat tab in VSCode — it connects to the running daemon via SSE. Call
`create_session` from it, then check `http://127.0.0.1:7274` in a browser to see
the session appear. Repeat in a second chat tab — both sessions surface in the
same UI, confirming the shared `SessionMap`.

## Reload after changes

`~/.drun/config.toml` (or whichever path `DRUN_CONFIG` points to) is read once
at startup. The web UI HTML is compiled into the binary. Both require a rebuild
and restart to take effect:

```bash
cargo build -p drun-mcp
pkill -f drun-mcp 2>/dev/null; sleep 0.3; DRUN_CONFIG="$PWD/.drun/config.toml" ./target/debug/drun-mcp &
```

When done, restore the installed daemon:

```bash
# macOS
launchctl load -w ~/Library/LaunchAgents/com.drun.mcp-server.plist

# Linux
# systemctl --user start drun-mcp.service
```

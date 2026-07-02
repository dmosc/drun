# Development

## Build

```bash
cargo build -p drun-mcp
```

## Run locally

```bash
# Kill any existing instance on ports 7273 / 7274
pkill -f drun-mcp

# Start the daemon against the repo's own config
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

## Reload after config changes

`~/.drun/config.toml` (or whichever path `DRUN_CONFIG` points to) is read once
at startup. To apply edits, restart the process:

```bash
# Rebuild binary.
cargo build -p drun-mcp

# Kill current process.
pkill -f drun-mcp

# Launch a new one.
DRUN_CONFIG="$PWD/.drun/config.toml" ./target/debug/drun-mcp &
```

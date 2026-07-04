# Development

## Build

```bash
cargo build -p drun-mcp
```

## Test

```bash
cargo test --workspace
```

## Coverage

Coverage is measured with
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov). One-time setup:

```bash
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
```

Then, from the repo root:

```bash
# Per-file summary in the terminal
cargo llvm-cov --workspace --summary-only

# Full line-by-line HTML report
cargo llvm-cov --workspace --html --open
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

## Test drun init

`drun init` is a subcommand on the binary (not the MCP server). It can be tested
without suspending the daemon — it exits immediately after writing files and
does not bind any ports.

```bash
# Run init against the local build from any project directory
cd ~/path/to/some-project
/path/to/drun/target/debug/drun-mcp init
```

Expected output:

```
drun: created .claude/settings.json
drun: created CLAUDE.md
drun: initialized for /path/to/some-project
```

Running it a second time in the same directory should skip the files that
already exist:

```
drun: .claude/settings.json already exists, skipping
drun: CLAUDE.md already exists, skipping
drun: initialized for /path/to/some-project
```

The project path is appended to `~/.drun/projects` (checked for duplicates
before writing).

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

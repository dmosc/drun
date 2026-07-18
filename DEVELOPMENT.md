# Development

## Build

```bash
cargo build -p drun-mcp
```

## Test

Test drun core libraries.

```bash
cargo test --workspace
```

Test the `drun chat` CLI service.

```bash
cd crates/drun-py
pip install -e '.[chat,test]'
pytest
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

Run a local build. First create a dedicated `config.toml` with at the very least
a `web_port` configured:

```toml
# .drun/config.toml
web_port = 1234
```

Then, compile the binary and run the server.

```bash
cargo build -p drun-mcp
DRUN_CONFIG="$PWD/.drun/config.toml" DRUN_MCP_PORT=8273 ./target/debug/drun-mcp
```

- `DRUN_MCP_PORT` — MCP port.
- `DRUN_CONFIG` — path to a config.toml.

If you're using Claude, register the MCP under a dedicated name:

```bash
claude mcp add --transport sse drun-dev http://127.0.0.1:8273/sse
```

Confirm it's up:

```bash
curl -s http://127.0.0.1:8274/api/status
```

### Cleanup

Kill the terminal where you have your MCP running and also remove the entry from
your Claude MCPs list if you registered it.

```bash
claude mcp remove drun-dev
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

## Test the installed binary with a local build

The workflow above runs a throwaway debug build directly and never touches
`/usr/local/bin/drun-mcp`. Sometimes you actually need to test against the
_installed_ path and service manager instead — e.g. verifying `install.sh`/
`update.sh` themselves, or reproducing something that only shows up under
launchd/systemd supervision.

**Never `cp`/`curl -o` directly onto `/usr/local/bin/drun-mcp` while the daemon
is running from that path.** Truncating a binary in place while a process is
actively executing it can corrupt the kernel's code-signing validation for that
file on macOS (`OS_REASON_CODESIGNING`) and wedge the daemon into a crash loop
that not even `kill -9` can recover from — this is exactly what `update.sh` was
fixed to avoid, and it's a real failure mode, not a theoretical one. Always swap
the binary in via a temp file + atomic rename, the same way `update.sh` does:

```bash
# 1. Build a release binary — matches what install.sh actually ships
cargo build --release -p drun-mcp

# 2. Stop the managed daemon
launchctl unload ~/Library/LaunchAgents/com.drun.mcp-server.plist   # macOS
# systemctl --user stop drun-mcp.service                            # Linux

# 3. Swap the binary in via a temp file + atomic rename — not a direct overwrite
tmp="/usr/local/bin/.drun-mcp.tmp.$$"
sudo cp target/release/drun-mcp "$tmp"
sudo mv -f "$tmp" /usr/local/bin/drun-mcp

# 4. Restart the managed daemon so it picks up the new binary
launchctl load -w ~/Library/LaunchAgents/com.drun.mcp-server.plist  # macOS
# systemctl --user start drun-mcp.service                           # Linux
```

Then confirm it actually came back up healthy — not stuck at zero processes, not
crash-looping — with the checks from
[docs/troubleshooting.md's Health check section](docs/troubleshooting.md#health-check--is-drun-actually-running):

```bash
pgrep -fl "drun-mcp$"
launchctl print gui/$(id -u)/com.drun.mcp-server | grep -E "state|pid|runs|last exit reason"
```

To drop your local build and go back to the last released version:

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash
```

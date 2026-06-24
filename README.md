# drun

<div align="center">

![drun architecture](assets/architecture.png)

</div>

**Safe-by-design agentic code execution**

Isolated by design. Every execution is a checkpoint: mistakes are undoable,
experiments are forkable, and nothing reaches your host until you approve it.
You control what agents can access — network, files, secrets — and what they can
do to the outside world.

---

## Table of contents

1. [Architecture](#architecture)
2. [What drun installs on your host](#what-drun-installs-on-your-host)
3. [Dependencies](#dependencies)
4. [Installation](#installation)
5. [Upgrading](#upgrading)
6. [Uninstalling](#uninstalling)
7. [Configuration](#configuration)
8. [User journeys](#user-journeys)
9. [How it works](#how-it-works)
10. [MCP tools reference](#mcp-tools-reference)
11. [Python SDK reference](#python-sdk-reference)
12. [Typical workflows](#typical-workflows)
13. [Claude Code integration](#claude-code-integration)
14. [Examples](#examples)

---

## Architecture

drun has three components that can be used independently or together:

**drun-core** — the Rust library at the center of everything. It manages
sessions, checkpoints, the Python runner subprocess, the egress proxy, and
snapshot serialization.

**drun-mcp** — a lightweight MCP server that wraps drun-core and exposes it to
any MCP-compatible client (Claude Code, Cursor, etc.) as a set of tool calls.
Runs as a long-lived process alongside your LLM client.

**drun-py** — Python bindings (via PyO3) that expose the same session API
directly to Python scripts without an MCP layer. Used for scripted workflows and
the `drun chat` CLI.

```
┌────────────────┐       ┌──────────────────────┐
│  Claude Code   │       │  Python script       │
│  Cursor, etc.  │       │  / drun chat CLI     │
└──────┬─────────┘       └──────────┬───────────┘
       │ MCP (stdio)                 │ PyO3 bindings
       ▼                             ▼
┌────────────────────────────────────────────────┐
│                   drun-core                    │
│                                                │
│  Session  ──► Runner ──► Python subprocess     │
│  (state)        │             │                │
│                 │         http_proxy env var   │
│                 ▼             ▼                │
│           EgressProxy ◄── all outbound HTTP    │
│           (TCP proxy)    from Python code      │
└────────────────────────────────────────────────┘
```

---

## What drun installs on your host

| What                  | Location                                         | Notes                                                                 |
| --------------------- | ------------------------------------------------ | --------------------------------------------------------------------- |
| MCP binary            | `/usr/local/bin/drun-mcp` (one-liner installer)  | Or wherever you copy it manually                                      |
| Python runner script  | `$TMPDIR/drun_runner_<pid>.py`                   | Written at startup, deleted when the process exits                    |
| pip package cache     | `$TMPDIR/drun-packages/`                         | Shared across all sessions; overridable with `packages_dir` in config |
| Session snapshots     | `./drun-snapshots/` (relative to cwd)            | Overridable with `snapshots_dir` in config                            |
| Session exports       | `./drun-export/<session_id>/`                    | Overridable with `export_root` in config                              |
| Claude Code MCP entry | `~/.claude/settings.json` (via `claude mcp add`) | One line added under `"mcpServers"`                                   |

drun does not install global Python packages, does not modify system paths, and
does not run any background services outside of the process you start.

---

## Dependencies

### Required

**Python 3.9 or later.** drun launches `python3` from your PATH as the execution
subprocess. It does not bundle a Python runtime.

```bash
python3 --version   # must be 3.9 or later
```

### Optional

**Ollama** — for running local models with `drun chat` or the Python SDK. No API
key needed.

Install from [ollama.com](https://ollama.com), then pull a model:

```bash
ollama pull qwen2.5:14b
```

`qwen2.5:14b` and `qwen2.5:7b` are the most reliable local models for multi-turn
structured tool calling. Avoid reasoning/thinking variants (`deepseek-r1`,
`qwen3.*`) — they emit tool calls as plain text rather than structured JSON and
do not interoperate reliably with standard tool-use loops.

When using Ollama with drun, use the `openai/<model>` prefix and point to the
`/v1` endpoint:

```bash
drun chat --model openai/qwen2.5:14b \
          --base-url http://localhost:11434/v1 \
          "your prompt"
```

**Rust toolchain** — only needed if building from source. Install from
[rustup.rs](https://rustup.rs).

**bubblewrap (Linux only)** — required for `session_bash` on Linux.
`sandbox-exec` is used on macOS (built in). On Linux:

```bash
apt install bubblewrap      # Debian / Ubuntu
dnf install bubblewrap      # Fedora
```

**API keys** — only for cloud LLM providers:

| Provider  | Environment variable |
| --------- | -------------------- |
| Anthropic | `ANTHROPIC_API_KEY`  |
| OpenAI    | `OPENAI_API_KEY`     |
| Gemini    | `GEMINI_API_KEY`     |

---

## Installation

### MCP server (Claude Code integration)

**One-liner (recommended)** — detects your platform, downloads the binary to
`/usr/local/bin`, and registers drun with Claude Code:

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

**Manual binary download:**

```bash
# macOS Apple Silicon
curl -fsSL https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-arm64 \
  -o /usr/local/bin/drun-mcp && chmod +x /usr/local/bin/drun-mcp

# macOS Intel
curl -fsSL https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-x86_64 \
  -o /usr/local/bin/drun-mcp && chmod +x /usr/local/bin/drun-mcp

# Linux x86-64
curl -fsSL https://github.com/dmosc/drun/releases/latest/download/drun-mcp-linux-x86_64 \
  -o /usr/local/bin/drun-mcp && chmod +x /usr/local/bin/drun-mcp

# Then register with Claude Code:
claude mcp add drun -- /usr/local/bin/drun-mcp
```

**Build from source:**

```bash
git clone https://github.com/dmosc/drun.git && cd drun
cargo build --release -p drun-mcp
claude mcp add drun -- "$(pwd)/target/release/drun-mcp"
```

### Python SDK

Requires Python 3.9+. Prebuilt wheels are available for macOS (arm64, x86_64)
and Linux (x86_64, aarch64) — no Rust toolchain needed:

```bash
pip install 'drun-sandbox[chat]'
```

The `[chat]` extra installs [litellm](https://github.com/BerriAI/litellm), which
routes requests to any provider — Ollama, Anthropic, OpenAI, Google, or any
OpenAI-compatible endpoint — from a single interface.

**Build from source** (requires a [Rust toolchain](https://rustup.rs)):

```bash
git clone https://github.com/dmosc/drun.git
cd drun/crates/drun-py
pip install '.[chat]'
```

---

## Upgrading

```bash
# MCP binary
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash

# Update to a specific version
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash -s -- v0.1.1

# Python SDK
pip install --upgrade 'drun-sandbox[chat]'
```

No re-registration needed after updating the binary — Claude Code keeps pointing
to the same path.

**Reload after upgrading the MCP binary:**

- VSCode extension: `Cmd+Shift+P` → **Developer: Reload Window**
- Terminal `claude`: exit and relaunch

---

## Uninstalling

```bash
# Remove the MCP binary and deregister from Claude Code
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/uninstall.sh | bash

# Python SDK
pip uninstall drun-sandbox

# Pip cache (optional — safe to delete)
rm -rf "$(python3 -c 'import tempfile; print(tempfile.gettempdir())')/drun-packages"
```

---

## Configuration

drun is configured through a TOML file pointed to by the `DRUN_CONFIG`
environment variable. The file is read once at process startup. Without it,
built-in defaults apply.

### Where to set DRUN_CONFIG

The right place depends on which workflow you use.

#### Python SDK / `drun chat` CLI

Pass it inline or export it in your shell:

```bash
# Inline (recommended — explicit per-run)
DRUN_CONFIG=/path/to/your.toml python your_script.py
DRUN_CONFIG=/path/to/your.toml drun chat --model claude-sonnet-4-6 "..."

# Or export for the session
export DRUN_CONFIG=/path/to/your.toml
python your_script.py
```

**Reload:** restart the process — the config is read once at startup.

#### Claude Code VSCode extension

VSCode is typically launched from the Dock or Finder, which does **not** source
`.zshrc` or `.zshenv`. Shell exports have no effect on the MCP server. Set
`DRUN_CONFIG` inside Claude Code's settings instead.

**Global** (applies to every project):

```json
// ~/.claude/settings.json
{
  "env": {
    "DRUN_CONFIG": "/absolute/path/to/your.toml"
  }
}
```

**Project-level** (overrides global for this repo only):

```json
// .claude/settings.json  (committed or gitignored as you prefer)
{
  "env": {
    "DRUN_CONFIG": "/absolute/path/to/your.toml"
  }
}
```

**Reload:** `Cmd+Shift+P` → **Developer: Reload Window**. The MCP server
restarts and reads the updated config.

#### Claude Code terminal

When Claude Code is launched from a terminal that already has `DRUN_CONFIG` set,
the MCP server inherits it:

```bash
export DRUN_CONFIG=/path/to/your.toml
claude
```

**Reload:** exit and relaunch `claude`. Opening a new chat window does not
restart the MCP server — the server persists across windows within the same
Claude Code process.

#### Standalone `drun-mcp`

```bash
DRUN_CONFIG=/path/to/your.toml drun-mcp
```

**Reload:** kill and restart the process.

---

### Field reference

All fields are optional. Omitting a field applies the default shown below.

| Field                       | Default                 | Description                                                                                                                                                                                                                                        |
| --------------------------- | ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `domain_allowlist`          | `[]`                    | Additional domains reachable via `session_fetch` and Python outbound HTTP. `pypi.org`, `files.pythonhosted.org`, and `cdn.jsdelivr.net` are always permitted and cannot be removed. Use `["*"]` to allow all, or `"*.example.com"` for subdomains. |
| `fetch_timeout_ms`          | `60000`                 | Timeout for the full `session_fetch` response in milliseconds.                                                                                                                                                                                     |
| `connect_timeout_ms`        | `30000`                 | TCP connection timeout for `session_fetch` in milliseconds.                                                                                                                                                                                        |
| `exec_timeout_ms`           | `60000`                 | Maximum wall time for a single `session_execute_python` call. The runner is killed when exceeded; the session auto-rebuilds and remains usable.                                                                                                    |
| `install_timeout_ms`        | `120000`                | Maximum wall time for `session_install_package` (pip download + install).                                                                                                                                                                          |
| `bash_timeout_ms`           | `30000`                 | Maximum wall time for a single `session_bash` call.                                                                                                                                                                                                |
| `max_workspace_mb`          | `512`                   | Maximum workspace size per session in megabytes. Checked before each new checkpoint is appended.                                                                                                                                                   |
| `max_sessions`              | `50`                    | Maximum number of concurrent sessions.                                                                                                                                                                                                             |
| `max_checkpoints`           | `200`                   | Maximum checkpoints stored per session. When the limit is reached, squash or drop old checkpoints.                                                                                                                                                 |
| `session_idle_timeout_secs` | `3600`                  | Seconds of inactivity before a session is considered abandoned and rejected.                                                                                                                                                                       |
| `mount_allowlist`           | `[]`                    | Host path prefixes that `session_mount` may read from. Empty means all paths are permitted. Non-empty restricts mounts to the listed prefixes.                                                                                                     |
| `export_root`               | `"drun-export"`         | Directory that `session_export` must write into. Relative paths are resolved from the current working directory.                                                                                                                                   |
| `snapshots_dir`             | `"drun-snapshots"`      | Directory where `session_snapshot` writes `.drun` files.                                                                                                                                                                                           |
| `snapshot_on_close`         | `false`                 | When `true`, automatically write a snapshot when `session_close` is called.                                                                                                                                                                        |
| `env_allowlist`             | `[]`                    | Host environment variable names exposed to agents via `session_get_env`. Empty means no variables are exposed.                                                                                                                                     |
| `package_allowlist`         | `[]`                    | Package names the agent may install via `session_install_package`. Empty means all packages are allowed. Enforced at the MCP layer only — the Python SDK bypasses this check.                                                                      |
| `bash_command_denylist`     | `[]`                    | Command substrings always rejected by `session_bash` before execution.                                                                                                                                                                             |
| `bash_command_allowlist`    | `[]`                    | Command substrings permitted by `session_bash`. Empty means all commands are allowed (subject to the denylist).                                                                                                                                    |
| `packages_dir`              | `$TMPDIR/drun-packages` | Directory where pip installs packages. Shared across all sessions as a persistent cache.                                                                                                                                                           |

### Example

```toml
# PyPI domains (pypi.org, files.pythonhosted.org, cdn.jsdelivr.net) are always
# reachable and do not need to be listed here.
domain_allowlist = ["api.example.com"]

max_workspace_mb = 256
max_sessions = 10
exec_timeout_ms = 60_000
install_timeout_ms = 120_000
session_idle_timeout_secs = 1800

# Only paths under this prefix may be mounted.
mount_allowlist = ["/home/user/projects/myapp"]

# Exports land here.
export_root = "/tmp/drun-outputs"

# Agent can only install these packages.
package_allowlist = ["pandas", "numpy", "matplotlib"]

# Expose this secret to agents that call session_get_env.
env_allowlist = ["DATABASE_URL"]

# Reject these commands before they reach the sandbox.
bash_command_denylist = ["curl", "wget", "nc"]

# Sessions are ephemeral — no snapshots written on close.
snapshot_on_close = false
```

See [`examples/financial_analysis.toml`](examples/financial_analysis.toml),
[`examples/data_science.toml`](examples/data_science.toml), and
[`examples/heavy_workloads.toml`](examples/heavy_workloads.toml) for fully
annotated real-world recipes. See
[`examples/fibonacci_benchmark.toml`](examples/fibonacci_benchmark.toml) for a
tight-allowlist example that stress-tests every constraint.

---

## User journeys

### Python SDK — scripted agentic workflow

Use this path to drive a drun session from your own Python script, without
Claude Code or any MCP client. The SDK exposes the full session API directly.

**Install:**

```bash
pip install 'drun-sandbox[chat]'
```

**Quickstart (no LLM needed):**

```python
from drun import Session

session = Session()                                    # reads DRUN_CONFIG if set
cp = session.execute_python("print('hello')")
print(cp.stdout)

session.write_file("data.txt", b"version A")
cp_a = session.execute_python("print(open('data.txt').read())")

session.execute_python("open('data.txt', 'w').write('version B')")
session.rollback(cp_a.id)                             # revert to version A

session.install("tabulate")
cp_bash = session.execute_bash("ls -1 /workspace")
print(cp_bash.stdout)

patch = session.diff(0, session.current.id)
session.export("/tmp/my-outputs")
```

**LLM-driven loop:**

```python
from drun import Session
from drun.chat import run

session = Session()
session.mount("/path/to/data")

run(
    session,
    "clean the data and compute summary statistics",
    model="claude-sonnet-4-6",        # or any litellm model string
    max_iterations=30,
)
```

```bash
ANTHROPIC_API_KEY=sk-ant-... \
DRUN_CONFIG=/path/to/your.toml \
    python your_script.py
```

**Reload config:** restart the Python process.

---

### `drun chat` — agentic CLI

Drive a session from the command line. Requires the `[chat]` extra.

**Cloud providers:**

```bash
# Anthropic
ANTHROPIC_API_KEY=sk-ant-... \
    drun chat --model claude-sonnet-4-6 \
              --mount ./src \
              "refactor the parser module"

# OpenAI
OPENAI_API_KEY=sk-... \
    drun chat --model gpt-4o "analyze this dataset" --mount ./data.csv

# Google Gemini
GEMINI_API_KEY=... \
    drun chat --model gemini/gemini-2.0-flash "write and run a quicksort"
```

**Local models via Ollama (no API key needed):**

```bash
# Pull the model once
ollama pull qwen2.5:14b

# Run
DRUN_CONFIG=/path/to/your.toml \
    drun chat --model openai/qwen2.5:14b \
              --base-url http://localhost:11434/v1 \
              --mount ./src \
              "summarize what each module does"
```

Use the `openai/<model>` prefix with `--base-url http://localhost:11434/v1`
(Ollama's OpenAI-compatible endpoint). The `/v1` endpoint handles tool call IDs
more reliably across multi-turn conversations than the native Ollama endpoint.

**`drun chat` options:**

| Flag                 | Default              | Description                                     |
| -------------------- | -------------------- | ----------------------------------------------- |
| `--model MODEL`      | `ollama/qwen2.5:14b` | litellm model identifier                        |
| `--base-url URL`     | —                    | API base URL override                           |
| `--mount PATH`       | —                    | Mount a host path into the session (repeatable) |
| `--system PROMPT`    | built-in             | Override the system prompt                      |
| `--max-iterations N` | `30`                 | Maximum agent loop iterations                   |

**Reload config:** restart `drun chat`. Config is read once at startup.

---

### Claude Code + VSCode extension

1. Install the MCP binary and register it:

   ```bash
   curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
   ```

2. Set `DRUN_CONFIG` in `~/.claude/settings.json`:

   ```json
   {
     "env": {
       "DRUN_CONFIG": "/absolute/path/to/your.toml"
     }
   }
   ```

3. Reload: `Cmd+Shift+P` → **Developer: Reload Window**.

4. Verify: ask Claude to call `session_list`. If it returns an empty list, the
   server is up. If the tool is not available, check **Output → Claude Code
   MCP** for startup errors.

**Reload config:** reload the window again. The MCP server restarts and reads
the updated TOML.

**Updating the TOML path (switching projects):** update the `DRUN_CONFIG` value
in `settings.json` and reload the window. Opening a new chat window is not
enough — the MCP server persists across windows until the VSCode window itself
is reloaded.

---

### Claude Code + terminal

```bash
# Install and register once
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash

# Launch with config
DRUN_CONFIG=/path/to/your.toml claude
```

**Reload config:** exit and relaunch `claude`.

---

### Loading a snapshot

Snapshots are `.drun` files that capture a session's full checkpoint history,
installed packages, and workspace. They let you pause long sessions and resume
them later.

**Via MCP (Claude Code):**

```
session_snapshot(session_id)          → writes ./drun-snapshots/<id>.drun
list_snapshots()                      → shows available .drun files
session_restore(path)                 → loads the snapshot into a new session
```

**Via Python SDK:**

There is no direct snapshot API in the Python SDK — snapshots are a server-side
feature of drun-mcp. The Python SDK equivalent is to keep the `Session` object
alive in memory, or to export outputs with `session.export()`.

---

## How it works

### Egress proxy

When drun starts (either as `drun-mcp` or via the Python SDK), it binds a TCP
proxy to a random port on `127.0.0.1`. This proxy is injected into the Python
runner subprocess as `http_proxy` and `https_proxy` environment variables. Every
outbound HTTP and HTTPS request from Python code — including calls from
`urllib`, `requests`, `httpx`, and any other library that respects the standard
proxy environment variables — is routed through this proxy.

**HTTPS (CONNECT tunneling):** when the Python code opens an HTTPS connection,
it first sends `CONNECT hostname:443 HTTP/1.1` to the proxy. The proxy parses
the hostname, checks it against `domain_allowlist`, and either returns
`403 Forbidden` or establishes the upstream TCP connection and starts relaying
bytes bidirectionally. The TLS handshake happens end-to-end between Python and
the remote server — the proxy only routes raw bytes and never reads the
encrypted payload.

**Plain HTTP:** the proxy parses the request-line, extracts the hostname, checks
the allowlist, and if permitted rewrites the request path and forwards it to the
upstream server.

The proxy runs in its own threads on the host, outside the Python subprocess.
This means domain enforcement cannot be bypassed from inside the sandbox by
modifying proxy settings or using raw socket syscalls at the Python level.

**When `domain_allowlist = ["*"]`** — the proxy is not started at all. The
Python subprocess gets no proxy configuration and connects directly.

**PyPI domains** (`pypi.org`, `files.pythonhosted.org`, `cdn.jsdelivr.net`) are
always merged into the effective allowlist at startup, regardless of what the
TOML sets. Package installation always works.

**Bash commands** do not go through the egress proxy. `session_bash` uses the
OS-level sandbox (`sandbox-exec` on macOS, `bubblewrap` on Linux) to block all
network access at the kernel level. Use `session_fetch` to pull external data
into the workspace first, then process it with `session_bash`.

### Sandbox layers

drun applies two complementary isolation layers:

**Egress proxy (Python subprocess):** routes and filters outbound TCP via
`http_proxy`/`https_proxy`. Enforces `domain_allowlist`. Operates at the
application layer.

**OS sandbox (bash subprocess):** for `session_bash`, drun wraps the shell
command in a kernel-enforced sandbox profile:

- **macOS** — `sandbox-exec` with an SBPL ("Apple Sandbox Profile Language")
  profile. The profile denies everything by default, then explicitly permits
  file reads everywhere, file writes only within the workspace temp directory,
  and basic process/signal operations. Network is denied by the default policy
  since no network rule is added.

- **Linux** — `bubblewrap` (`bwrap`). The sandbox gets a read-only view of the
  host filesystem (for PATH and shared libraries), a writable bind-mount of the
  workspace directory, an isolated `/tmp`, and `--unshare-net` to remove all
  network access. It also sets `--die-with-parent` to prevent orphaned
  processes.

### Checkpoint mechanics

Every session starts with checkpoint 0 — an empty workspace, or the files loaded
by any `session_mount` calls. Each operation that changes the workspace appends
a new checkpoint:

| Operation                          | Creates checkpoint    |
| ---------------------------------- | --------------------- |
| `session_execute_python`           | Yes                   |
| `session_bash`                     | Yes                   |
| `session_write_file`               | Yes                   |
| `session_delete_file`              | Yes                   |
| `session_mount`                    | Modifies checkpoint 0 |
| `session_merge`                    | Yes                   |
| `session_rollback`, `session_fork` | No                    |

A checkpoint stores:

- An integer ID (its index in the history, starting from 0)
- The stdout and stderr from the operation that produced it
- A complete snapshot of the workspace: every file path mapped to its content

**Content-addressed deduplication.** File contents are stored as
reference-counted (`Arc`) pointers. Before storing, content is hashed. If the
same bytes already exist in the session's intern table (from any prior
checkpoint or a fork), the new entry reuses the existing pointer. A session with
100 checkpoints where only one file changed holds roughly 99 shared pointers and
1 new allocation — unchanged files cost nothing extra.

**HEAD pointer.** The session maintains a `checkpoint_idx` cursor. All read
operations and new writes operate relative to this cursor.

**Rollback.** Moving the cursor backwards does not delete anything. History is
fully preserved. The next write after a rollback truncates the "future"
checkpoints (those ahead of the new cursor) and branches from there.

**Fork.** Creates a new independent session whose checkpoint 0 is a copy of the
source session's files at the forked checkpoint. The new session's packages are
reinstalled from scratch into a fresh Python subprocess. After forking, both
sessions are completely independent.

**Squash.** `session_checkpoint_squash(from, to, label)` replaces the range
`[from..=to]` (inclusive on both ends) with a single checkpoint. The squashed
checkpoint takes the **terminal file state** (from checkpoint `to`), and its
stdout/stderr is the concatenation of all squashed steps. All checkpoint IDs are
re-indexed afterward. If the current HEAD was within the squashed range, it
moves to `from`.

**Drop.** `session_checkpoint_drop(from, to)` permanently deletes checkpoints
`[from..=to]` from history to free memory or stay under `max_checkpoints`. The
current checkpoint cannot be dropped. IDs are re-indexed after the deletion.

**Labels.** Any checkpoint can be given a human-readable label
(`session_checkpoint_label`). Labels appear in `session_history` and
`session_tree` and can be used in place of numeric IDs in `session_rollback`,
`session_diff`, `session_fork`, and `session_merge`.

**Workspace size limit.** Before appending any checkpoint, drun sums all file
sizes in the new file map. If the total exceeds `max_workspace_mb`, the
operation is rejected and the session stays at its current checkpoint.

**Checkpoint limit.** When `max_checkpoints` is reached, drun rejects new
operations with an error. Use `session_checkpoint_squash` or
`session_checkpoint_drop` to reclaim space, or close and snapshot the session
and start fresh.

### Snapshot format

`session_snapshot` serializes a session to a `.drun` file:

1. The checkpoint history is encoded with a deduplicated blob store: file
   contents are collected into a flat `blobs` array, and each checkpoint records
   a map of `{path → blob_index}` instead of storing raw bytes. This means files
   shared across checkpoints are stored once.
2. The structure is JSON-serialized and then compressed with zstd at level 3.
3. A 4-byte magic header (`DRUN`) is prepended.
4. A lightweight sidecar `.drun.meta` file (plain JSON) is written alongside,
   containing only the session label, package list, and checkpoint count.
   `list_snapshots` reads only the `.meta` sidecar — it never decompresses the
   full snapshot.

`session_restore` decompresses the `.drun` file, re-hydrates checkpoints from
the blob array, and reinstalls all recorded packages into a new Python
subprocess.

### Python runner lifecycle

The Python runner is a long-lived subprocess (`runner.py` written to a temp file
at startup). It communicates with drun-core over stdin/stdout using JSON
line-delimited messages:

- **Execute:** `{"code": "...", "files": {...}}` — the runner materializes the
  file map into a temp directory, sets it as the working directory, executes the
  code, and returns `{"stdout": "...", "stderr": "...", "files": {...}}`.
- **Install:** `{"package": "numpy"}` — the runner calls pip and returns `{}` on
  success or `{"error": "..."}` on failure.
- **Progress lines:** during long operations, the runner emits
  `{"progress": "..."}` lines that are forwarded to the caller as streaming
  output.

A watchdog thread starts for every operation. If the runner doesn't respond
within the configured timeout, the thread kills the subprocess and marks the
operation as timed out. After a crash or timeout, drun automatically spawns a
fresh runner subprocess so the session remains usable.

---

## MCP tools reference

### Lifecycle

**`create_session`** — Create a new sandbox session. Returns `session_id`.

No parameters.

---

**`session_list`** — List all active sessions with checkpoint count, installed
packages, and labels.

No parameters.

---

**`session_close`** — Terminate a session and free all resources. If
`snapshot_on_close` is `true` in config, writes a `.drun` snapshot first.

| Parameter    | Type   | Required | Description      |
| ------------ | ------ | -------- | ---------------- |
| `session_id` | string | Yes      | Session to close |

---

**`session_tree`** — Return the full session/checkpoint tree. Root sessions are
top-level; forked sessions are nested under the checkpoint they branched from.
Each checkpoint is flagged with `is_current`.

No parameters.

---

**`session_label`** — Attach a human-readable name to a session (visible in
`session_list` and `session_tree`). Pass an empty string to clear.

| Parameter    | Type   | Required | Description               |
| ------------ | ------ | -------- | ------------------------- |
| `session_id` | string | Yes      |                           |
| `label`      | string | Yes      | Empty string clears label |

---

### Execution

**`session_execute_python`** — Run Python code in the session. File changes and
stdout are captured as a new checkpoint.

| Parameter    | Type   | Required | Description               |
| ------------ | ------ | -------- | ------------------------- |
| `session_id` | string | Yes      |                           |
| `code`       | string | Yes      | Python source code to run |

Returns: `stdout`, `checkpoint_id`.

---

**`session_bash`** — Run a shell command in the session workspace. On macOS,
uses `sandbox-exec`; on Linux, uses `bubblewrap`. Network is blocked. File
changes are captured as a new checkpoint.

| Parameter    | Type   | Required | Description                       |
| ------------ | ------ | -------- | --------------------------------- |
| `session_id` | string | Yes      |                                   |
| `command`    | string | Yes      | Shell command (passed to `sh -c`) |

Returns: `stdout`, `checkpoint_id`.

---

**`session_install_package`** — Install a Python package into the session via
pip. The package is available in all subsequent `session_execute_python` calls.

| Parameter    | Type   | Required | Description                        |
| ------------ | ------ | -------- | ---------------------------------- |
| `session_id` | string | Yes      |                                    |
| `package`    | string | Yes      | Package name as it appears on PyPI |

---

**`session_cancel`** — Interrupt an in-progress `session_execute_python` or
`session_bash` call. The sandbox subprocess is killed; the session recovers
automatically. Returns immediately if nothing is executing.

| Parameter    | Type   | Required | Description |
| ------------ | ------ | -------- | ----------- |
| `session_id` | string | Yes      |             |

---

**`session_get_env`** — Read a host environment variable by name. Only variables
listed in `env_allowlist` can be read.

| Parameter    | Type   | Required | Description               |
| ------------ | ------ | -------- | ------------------------- |
| `session_id` | string | Yes      |                           |
| `name`       | string | Yes      | Environment variable name |

---

### Navigation

**`session_rollback`** — Move the session HEAD to a prior checkpoint without
discarding history. Subsequent writes branch from the new HEAD. Provide
`checkpoint_id` or `checkpoint_label`; label takes precedence.

| Parameter          | Type   | Required | Description                    |
| ------------------ | ------ | -------- | ------------------------------ |
| `session_id`       | string | Yes      |                                |
| `checkpoint_id`    | int    | No       | Checkpoint to restore          |
| `checkpoint_label` | string | No       | Label takes precedence over ID |

---

**`session_fork`** — Create a new session branching from an existing session at
a given checkpoint. The fork inherits workspace files and installed packages.
Returns a new `session_id`.

| Parameter          | Type   | Required | Description                       |
| ------------------ | ------ | -------- | --------------------------------- |
| `session_id`       | string | Yes      | Session to fork from              |
| `checkpoint_id`    | int    | No       | Branch point; defaults to current |
| `checkpoint_label` | string | No       | Label takes precedence over ID    |

---

**`session_merge`** — Overlay files from another session's checkpoint onto the
current session, creating a new checkpoint. Pass `keys` to merge only specific
files; omit to merge all.

| Parameter                 | Type     | Required | Description                                  |
| ------------------------- | -------- | -------- | -------------------------------------------- |
| `session_id`              | string   | Yes      | Target session (receives the files)          |
| `source_session_id`       | string   | Yes      | Source session (provides the files)          |
| `source_checkpoint_id`    | int      | No       | Defaults to source's current checkpoint      |
| `source_checkpoint_label` | string   | No       | Takes precedence over `source_checkpoint_id` |
| `keys`                    | string[] | No       | File paths to merge; omit for all            |

---

**`session_history`** — List every checkpoint with its stdout and the file delta
relative to the previous checkpoint.

| Parameter    | Type   | Required | Description |
| ------------ | ------ | -------- | ----------- |
| `session_id` | string | Yes      |             |

---

**`get_session_state`** — Get current workspace files, installed packages, and
checkpoint info for a session.

| Parameter    | Type   | Required | Description |
| ------------ | ------ | -------- | ----------- |
| `session_id` | string | Yes      |             |

---

### Files

**`session_read_file`** — Read a file from the current checkpoint. Use `offset`
and `limit` to page through large files without flooding context. The response
includes `total_bytes` and `has_more`.

| Parameter    | Type   | Required | Description                        |
| ------------ | ------ | -------- | ---------------------------------- |
| `session_id` | string | Yes      |                                    |
| `path`       | string | Yes      | File path relative to `/workspace` |
| `offset`     | int    | No       | Byte offset to start reading from  |
| `limit`      | int    | No       | Maximum bytes to return            |

---

**`session_write_file`** — Create or overwrite a file in the workspace. Creates
a new checkpoint. Set `is_base64 = true` for binary files.

| Parameter    | Type    | Required | Description                            |
| ------------ | ------- | -------- | -------------------------------------- |
| `session_id` | string  | Yes      |                                        |
| `path`       | string  | Yes      | Path relative to `/workspace`          |
| `content`    | string  | Yes      | File content (plain text or base64)    |
| `is_base64`  | boolean | No       | Decode `content` from base64 if `true` |

---

**`session_delete_file`** — Delete a file from the workspace. Creates a new
checkpoint.

| Parameter    | Type   | Required | Description                   |
| ------------ | ------ | -------- | ----------------------------- |
| `session_id` | string | Yes      |                               |
| `path`       | string | Yes      | Path relative to `/workspace` |

---

**`session_mount`** — Copy a file or directory from the host filesystem into the
workspace. Files land at `/workspace/<filename>` (or
`/workspace/<relative-path>` for directories).

| Parameter    | Type   | Required | Description                          |
| ------------ | ------ | -------- | ------------------------------------ |
| `session_id` | string | Yes      |                                      |
| `path`       | string | Yes      | Absolute path on the host filesystem |

---

**`session_diff`** — Compute a unified diff between two checkpoints. Defaults to
comparing checkpoint 0 (the mounted state) against the current checkpoint. Each
endpoint accepts an ID or a label; label takes precedence.

| Parameter               | Type   | Required | Description                                |
| ----------------------- | ------ | -------- | ------------------------------------------ |
| `session_id`            | string | Yes      |                                            |
| `from_checkpoint_id`    | int    | No       | Defaults to 0                              |
| `from_checkpoint_label` | string | No       | Takes precedence over `from_checkpoint_id` |
| `to_checkpoint_id`      | int    | No       | Defaults to current                        |
| `to_checkpoint_label`   | string | No       | Takes precedence over `to_checkpoint_id`   |

---

### Host I/O

**`session_fetch`** — The designated gateway for all outbound HTTP. Makes an
HTTP request from the host process and saves the response body as a workspace
file. The target domain must be in `domain_allowlist`. Use `session_read_file`
with `offset`/`limit` to read the saved file.

| Parameter    | Type              | Required | Description                           |
| ------------ | ----------------- | -------- | ------------------------------------- |
| `session_id` | string            | Yes      |                                       |
| `url`        | string            | Yes      | Fully-qualified URL                   |
| `method`     | string            | No       | HTTP method; defaults to `GET`        |
| `headers`    | `{name, value}[]` | No       | Request headers                       |
| `body`       | string            | No       | Request body for `POST`/`PUT`/`PATCH` |
| `save_to`    | string            | No       | Workspace path for the response body  |

---

**`get_fetch_allowlist`** — Return the list of domains permitted for
`session_fetch` and Python outbound HTTP.

No parameters.

---

**`get_allowed_packages`** — Return the list of packages permitted for
`session_install_package`. Empty means all packages are allowed.

No parameters.

---

**`session_export`** — Write sandbox-generated files to the host filesystem. By
default exports all files that were created inside the sandbox (not mounted from
the host). Pass `keys` to select specific files.

| Parameter    | Type     | Required | Description                                                         |
| ------------ | -------- | -------- | ------------------------------------------------------------------- |
| `session_id` | string   | Yes      |                                                                     |
| `output_dir` | string   | No       | Absolute host path; defaults to `<export_root>/<session_id>`        |
| `keys`       | string[] | No       | Specific file paths to export; omit for all sandbox-generated files |

---

**`session_commit`** — Write changed mounted files back to their original host
paths. Only files that were mounted and have changed since mounting are written.

| Parameter    | Type     | Required | Description                                    |
| ------------ | -------- | -------- | ---------------------------------------------- |
| `session_id` | string   | Yes      |                                                |
| `keys`       | string[] | No       | Specific mounted files to commit; omit for all |

---

### Snapshots

**`session_snapshot`** — Serialize a session's full checkpoint history to a
`.drun` file. Also writes a `.drun.meta` sidecar for `list_snapshots`.

| Parameter    | Type   | Required | Description                                                  |
| ------------ | ------ | -------- | ------------------------------------------------------------ |
| `session_id` | string | Yes      |                                                              |
| `path`       | string | No       | Output path; defaults to `<snapshots_dir>/<session_id>.drun` |

---

**`session_restore`** — Load a session from a `.drun` snapshot file. Reinstalls
packages and restores all checkpoint history. Returns a new `session_id`.

| Parameter | Type   | Required | Description                       |
| --------- | ------ | -------- | --------------------------------- |
| `path`    | string | Yes      | Absolute path to the `.drun` file |

---

**`list_snapshots`** — List all `.drun` snapshot files in the server's
`snapshots_dir`. Returns path, size, label, checkpoint count, and installed
packages for each file (read from the lightweight `.meta` sidecar).

No parameters.

---

### Checkpoint housekeeping

**`session_checkpoint_label`** — Attach a human-readable label to a checkpoint.
Labels appear in `session_history` and `session_tree` and can be used in place
of IDs.

| Parameter       | Type   | Required | Description                        |
| --------------- | ------ | -------- | ---------------------------------- |
| `session_id`    | string | Yes      |                                    |
| `checkpoint_id` | int    | No       | Defaults to the current checkpoint |
| `label`         | string | Yes      | Empty string clears the label      |

---

**`session_checkpoint_squash`** — Collapse a range of checkpoints into one. The
squashed checkpoint takes the terminal file state; stdout/stderr from all
squashed steps are concatenated. The range is inclusive on both ends. Checkpoint
IDs are re-indexed after the operation.

| Parameter            | Type   | Required | Description                                 |
| -------------------- | ------ | -------- | ------------------------------------------- |
| `session_id`         | string | Yes      |                                             |
| `from_checkpoint_id` | int    | Yes      | First checkpoint in range (inclusive)       |
| `to_checkpoint_id`   | int    | Yes      | Last checkpoint in range (inclusive)        |
| `label`              | string | No       | Optional label for the resulting checkpoint |

---

**`session_checkpoint_drop`** — Permanently delete a range of checkpoints to
free memory or stay under `max_checkpoints`. Cannot drop the current checkpoint.
IDs are re-indexed after the operation.

| Parameter            | Type   | Required | Description                          |
| -------------------- | ------ | -------- | ------------------------------------ |
| `session_id`         | string | Yes      |                                      |
| `from_checkpoint_id` | int    | Yes      | First checkpoint to drop (inclusive) |
| `to_checkpoint_id`   | int    | Yes      | Last checkpoint to drop (inclusive)  |

---

## Python SDK reference

```python
from drun import Session
from drun.chat import run
```

**`Session()`** — Create a session. Reads `DRUN_CONFIG` from the environment if
set.

**Methods:**

| Method                                  | Returns          | Description                                            |
| --------------------------------------- | ---------------- | ------------------------------------------------------ |
| `execute_python(code: str)`             | `DrunCheckpoint` | Run Python code; captures file changes                 |
| `execute_bash(command: str)`            | `DrunCheckpoint` | Run a shell command; captures file changes             |
| `install(package: str)`                 | `None`           | Install a pip package into the session                 |
| `write_file(path: str, content: bytes)` | `None`           | Write a file into the workspace; creates a checkpoint  |
| `delete_file(path: str)`                | `DrunCheckpoint` | Delete a file from the workspace; creates a checkpoint |
| `mount(path: str)`                      | `list[str]`      | Copy a host file or directory into the workspace       |
| `rollback(checkpoint_id: int)`          | `None`           | Move HEAD to a prior checkpoint                        |
| `diff(from_id=0, to_id=None)`           | `str`            | Unified diff between two checkpoints                   |
| `export(output_dir: str, keys=None)`    | `list[str]`      | Write sandbox-generated files to a host directory      |
| `commit(keys=None)`                     | `list[str]`      | Write changed mounted files back to their host paths   |
| `set_label(label: str)`                 | `None`           | Attach a label to the session                          |
| `set_checkpoint_label(id, label: str)`  | `None`           | Attach a label to a checkpoint                         |

**Properties:**

| Property   | Type                   | Description                       |
| ---------- | ---------------------- | --------------------------------- |
| `.current` | `DrunCheckpoint`       | The current checkpoint            |
| `.history` | `list[DrunCheckpoint]` | All checkpoints from 0 to current |

**`DrunCheckpoint` fields:**

| Field     | Type               | Description                              |
| --------- | ------------------ | ---------------------------------------- |
| `.id`     | `int`              | Checkpoint index                         |
| `.stdout` | `str`              | Captured stdout from the operation       |
| `.stderr` | `str`              | Captured stderr from the operation       |
| `.files`  | `dict[str, bytes]` | All files in the workspace at this point |

**`run(session, prompt, model, base_url=None, max_iterations=30)`** — Drive a
session with an LLM agent loop. The agent can call `execute_python`,
`execute_bash`, `install_package`, and `write_file` as tools. Blocks until the
agent finishes or `max_iterations` is reached.

---

## Typical workflows

**Data analysis with rollback:**

```
create_session
  → session_install_package("pandas")
  → session_mount("/data/sales.csv")
  → session_execute_python(load + clean data)       # checkpoint 1
  → session_execute_python(compute summary)         # checkpoint 2 — looks wrong
  → session_rollback(1)                             # back to clean data
  → session_execute_python(corrected approach)      # checkpoint 3
  → session_export                                  # write output to host
```

**Parallel hypothesis testing:**

```
create_session → session_execute_python(load data)  # checkpoint 1
→ session_checkpoint_label(1, "data loaded")

session_fork("data loaded")  →  session B

session_execute_python(session A, strategy 1)
session_execute_python(session B, strategy 2)   # both start from the same base

session_merge(session B → session A, keys=["results.csv"])   # take best output
session_close(session B)
```

**Safe host file editing:**

```
session_mount("/path/to/script.py")
session_execute_python(refactor the code)
session_diff(0, current)          # review changes before touching the host
session_commit                    # write only the changed mounted files back
```

**Long session housekeeping:**

```
session_checkpoint_label(current, "model trained")     # mark milestone
session_checkpoint_squash(0, 5, "setup")               # collapse noisy setup steps
session_checkpoint_drop(6, 10)                         # drop abandoned branch
session_snapshot                                       # save to disk before closing
```

**Resuming a paused session:**

```
list_snapshots                          # find the .drun file
session_restore("/path/to/session.drun") # restore into a new session_id
session_execute_python(continue work)   # picks up exactly where it left off
```

---

## Claude Code integration

Add this to `~/.claude/CLAUDE.md` to route all code execution through drun:

```markdown
## Code execution

Always use drun MCP tools for all code execution. Never run Python or shell
commands directly via Bash.

- `create_session` — start every coding task
- `session_install_package` — before importing third-party packages
- `session_execute_python` — run Python code
- `session_bash` — run shell commands (git, npm, make, etc.)
- `session_fork` — explore alternative approaches without losing the original
- `session_rollback` — recover from mistakes
- `session_read_file` / `session_export` — retrieve outputs
- `session_commit` — write changes back to host files after review
- `session_fetch` — retrieve external data (NOT curl or requests directly)

## Network access

Do not use curl, wget, or Python HTTP libraries to fetch external data directly.
Both session_execute_python and session_bash have restricted or no network
access by design. Always use session_fetch to retrieve external data — it saves
the response as a workspace file that subsequent calls can read immediately.
```

### Enforcing drun usage

CLAUDE.md instructions guide but do not hard-constrain — Claude can still choose
the built-in Bash tool. Add explicit prohibitions to steer it:

```markdown
Never use Bash to run Python or shell commands that modify files — no
`python script.py`, no `pip install`, no `npm install`, no build tool
invocations. Use drun MCP tools for all code execution.
```

---

## Examples

See [`examples/`](examples/) for step-by-step guides and runnable scripts:

| Example                                                         | What it demonstrates                                                       |
| --------------------------------------------------------------- | -------------------------------------------------------------------------- |
| [`quickstart.py`](examples/quickstart.py)                       | Core SDK operations: execute, write, rollback, install, bash, diff, export |
| [`financial_analysis.py`](examples/financial_analysis.py)       | LLM agent fetching SEC EDGAR data and building a revenue table             |
| [`data_science.py`](examples/data_science.py)                   | LLM agent training three classifiers on the Iris dataset                   |
| [`fibonacci_benchmark.py`](examples/fibonacci_benchmark.py)     | Constraint probes + multi-algorithm benchmark with pyperf                  |
| [`financial_analysis.toml`](examples/financial_analysis.toml)   | Config: SEC/Yahoo domains, financial package allowlist                     |
| [`data_science.toml`](examples/data_science.toml)               | Config: large workspace, extended timeouts for model training              |
| [`heavy_workloads.toml`](examples/heavy_workloads.toml)         | Config: 4 GB workspace, 10-minute timeouts, persistent pip cache           |
| [`fibonacci_benchmark.toml`](examples/fibonacci_benchmark.toml) | Config: tight allowlist that stress-tests every policy layer               |

# drun

**An ephemeral, fully-sealed execution runtime for AI agents.**

drun gives agents a safe place to think with code. Every execution runs in a
fully isolated, stateful sandbox — no host filesystem access, no side effects,
no blast radius. Agents can explore, mutate state, generate files, roll back
mistakes, and iterate freely without touching the machine they run on.

---

## Why drun

Agentic systems that execute code face a hard tradeoff: either sandbox
everything and lose developer ergonomics, or give agents full access and accept
the risk. drun rejects the tradeoff.

- **Ephemeral by default.** Nothing persists to the host machine unless
  explicitly exported. Agents can write files, install packages, and mutate
  state — all contained inside the session.
- **Checkpointing and rollback.** Every execution step is a checkpoint. Agents
  can explore a branch of execution, decide it was wrong, and roll back to any
  prior state — like `git` for runtime.
- **Forking.** Spin up a new session branching from any checkpoint. Run two
  approaches in parallel, compare results, discard the worse one.
- **Frozen dependency surface.** The runtime bundles its own execution
  environment. Agents can't accidentally pull in a compromised or outdated
  package at the system level — the core sandbox is immutable and auditable.
- **Persistent sessions.** State accumulates across steps within a session.
  Packages installed in step one are available in step ten. No re-importing, no
  cold starts between tool calls.
- **Native artifact output.** Files generated inside the sandbox — images,
  reports, datasets — are returned in the right format. Images render inline.
  Text and binary are handled automatically.
- **Two consumption models.** Use it as an MCP server (Claude Code, any
  MCP-compatible client) or embed it as a library (`pip install drun-sandbox`).

---

## Language support

drun currently supports **Python**, executed via [Pyodide](https://pyodide.org)
— a WebAssembly port of CPython. Support for additional languages is on the
roadmap.

---

## Installation

### As an MCP server (recommended for Claude Code)

The one-liner installs Deno if needed, downloads the right binary for your
platform, and registers drun with Claude Code:

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

**Supported platforms:** macOS (Apple Silicon, Intel), Linux (x86_64, arm64)

**Dependencies:** [Deno](https://deno.land) — installed automatically if not
present.

To install manually:

```bash
# macOS Apple Silicon
curl -L https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-arm64 -o drun-mcp
chmod +x drun-mcp
claude mcp add drun -- /path/to/drun-mcp

# macOS Intel
curl -L https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-x86_64 -o drun-mcp

# Linux x86_64
curl -L https://github.com/dmosc/drun/releases/latest/download/drun-mcp-linux-x86_64 -o drun-mcp
```

Or via Cargo if you have Rust installed:

```bash
cargo install drun-mcp
claude mcp add drun -- $(which drun-mcp)
```

### As a library

```bash
pip install drun-sandbox
```

Requires Python ≥ 3.9. Deno must be installed separately:

```bash
curl -fsSL https://deno.land/install.sh | sh
```

---

## MCP tools

Once registered, drun exposes 19 tools to any MCP-compatible client, organized
below by function.

### Session lifecycle

| Tool             | Description                                                                                                                                                                                                                                 |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create_session` | Start a new sandbox session. Accepts `allowed_hosts` (list of hostnames the sandbox may reach — omit to inherit the server's fetch allowlist) and `timeout_ms` (per-execution wall-clock limit, default 60 000 ms). Returns a `session_id`. |
| `session_list`   | List all active sessions with checkpoint count, installed packages, and resource limits.                                                                                                                                                    |
| `session_close`  | Terminate a session and free all associated resources.                                                                                                                                                                                      |
| `session_tree`   | Return the full session-and-fork tree in one call. Shows which checkpoint every session currently heads, with forks nested under the checkpoint they branched from.                                                                         |

### Execution

| Tool                      | Description                                                                                                            |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `session_execute`         | Run Python code in a session, building on the current checkpoint. Returns stdout, stderr, and the new `checkpoint_id`. |
| `session_install_package` | Install a PyPI package into the session. Available in all subsequent `session_execute` calls.                          |

### Navigation & inspection

| Tool                | Description                                                                                                                                                    |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `session_rollback`  | Move the session head to a prior checkpoint without discarding history. Subsequent writes branch from the new head.                                            |
| `session_fork`      | Create a new independent session branching from an existing session at a given checkpoint. The fork inherits workspace, packages, network policy, and timeout. |
| `session_history`   | List every checkpoint with its stdout and the file delta relative to the previous checkpoint. Use this to decide which checkpoint to roll back to.             |
| `get_session_state` | Get the current state of a session: workspace files, installed packages, and checkpoint info.                                                                  |

### File operations

| Tool                  | Description                                                                                               |
| --------------------- | --------------------------------------------------------------------------------------------------------- |
| `session_read_file`   | Read the contents of a file from the current checkpoint. Handles text, JSON, images, and binary formats.  |
| `session_write_file`  | Create or overwrite a file in the session workspace. Creates a new checkpoint.                            |
| `session_delete_file` | Delete a file from the session workspace. Creates a new checkpoint.                                       |
| `session_mount`       | Copy a file or directory from the host filesystem into the session workspace at `/workspace`.             |
| `session_diff`        | Compute a unified diff between two checkpoints. Defaults to initial mounted state vs. current checkpoint. |

### Export & host I/O

| Tool                  | Description                                                                                                                                                                                                                                    |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `session_export`      | Write sandbox-generated files to the host filesystem. Exports only files created inside the sandbox (not mounted ones) unless specific keys are given.                                                                                         |
| `session_commit`      | Write changed mounted files back to their original host paths. Only files that were mounted and have changed are written.                                                                                                                      |
| `session_fetch`       | Make an HTTP request from the host and return the response body. Bypasses the WASM networking boundary so agents can reach external APIs. Requires the target URL to match the server's fetch allowlist — see [Configuration](#configuration). |
| `get_fetch_allowlist` | Return the list of domains the server permits for `session_fetch` calls. Read-only — the agent cannot modify the allowlist.                                                                                                                    |

---

## Configuration

drun reads a TOML config file at the path set by the `DRUN_CONFIG` environment
variable. If the variable is not set, drun starts with defaults — which means
`session_fetch` is blocked entirely.

```toml
# ~/drun-config.toml

[fetch]
# Domains permitted for session_fetch calls and Python outbound HTTP.
# Use ["*"] to allow all. Omit or leave empty to block all external fetches.
allowlist = [
    "data.sec.gov",
    "efts.sec.gov",
    "query1.finance.yahoo.com",
]

[session]
# Maximum workspace size in megabytes per session. Unset means no limit.
# max_workspace_mb = 512
```

Point drun at this file when registering it as an MCP server:

```json
// ~/.claude.json
{
  "mcpServers": {
    "drun": {
      "command": "/path/to/drun-mcp",
      "env": {
        "DRUN_CONFIG": "/Users/you/drun-config.toml"
      }
    }
  }
}
```

The allowlist is enforced by the server process — the agent cannot modify it or
grant itself access to unlisted URLs.

---

## Usage

### Typical agent flows

**Data analysis:**

```
create_session
session_install_package(pandas)
session_mount(/path/to/data.csv)
session_execute(load and analyze data)
session_execute(generate summary chart)
session_read_file(chart.png)
session_export               ← writes chart.png to ./drun-export/<session>/
```

**Parallel hypothesis testing:**

```
create_session               → session A
session_execute(load data)   → checkpoint 1

session_fork(session_A, checkpoint_1)  → session B

session_execute(session_A, approach 1)
session_execute(session_B, approach 2)

session_read_file(session_A, results.json)
session_read_file(session_B, results.json)
session_close(loser)
```

**Editing host files safely:**

```
create_session
session_mount(/path/to/script.py)
session_execute(refactor the code)
session_diff                 ← review changes before writing back
session_commit               ← writes only changed mounted files to host
```

**Fetching external data and producing a report:**

```
create_session(network: "none")
session_fetch(https://data.sec.gov/submissions/CIK....json)
                             ← host makes the request; WASM boundary not involved
session_write_file(filing.json, <response body>)
session_install_package(pandas)
session_execute(parse filings, compute metrics, write report.md)
session_read_file(report.md)
session_export               ← writes report.md to ./drun-export/<session>/
```

**Recovering from a mistake:**

```
session_history              ← review all checkpoints
session_rollback(checkpoint_id)
session_execute(corrected approach)
```

### Python library

```python
from drun import Session

s = Session()

# Execute code — nothing writes to your disk
result = s.execute("x = 42\nprint(x)")
print(result.stdout)  # "42"

# Packages survive across steps
s.execute("import micropip\nawait micropip.install('faker')")
result = s.execute("from faker import Faker\nprint(Faker().name())")

# Checkpoint and rollback
checkpoint_a = s.execute("data = [1, 2, 3]")
s.execute("data.append(4)")
s.rollback(checkpoint_a.id)  # data is [1, 2, 3] again

# Explicitly export a file to disk when you want it
s.execute("open('/workspace/output.csv', 'w').write('a,b\\n1,2')")
s.export("output.csv", dest="~/Downloads/output.csv")
```

### File isolation

All files written during execution live in `/workspace` inside the sandbox. They
are snapshotted at each checkpoint and never written to your host filesystem
automatically.

```python
# Writes to the sandbox, not your machine
s.execute("open('/workspace/report.txt', 'w').write('done')")

# Export explicitly when you want it locally
s.export("report.txt")               # writes to ./report.txt
s.export_all(dest_dir="./outputs")   # exports everything in /workspace
```

---

## Claude Code integration

After running the installer, drun is immediately available in Claude Code. To
make Claude always route code execution through drun — rather than running code
directly on your machine — add this to your `~/.claude/CLAUDE.md`:

```markdown
## Code execution

Always use drun MCP tools for code execution:

- Use `create_session` at the start of any coding task
- Use `session_install_package` before importing third-party packages
- Use `session_execute` to run code
- Use `session_read_file` to inspect output files and images
- Use `session_fork` to explore alternative approaches in parallel
- Use `session_rollback` to recover from mistakes
- Use `session_commit` to write changes back to host files after review
- Use `session_fetch` to retrieve data from external URLs (requires server
  allowlist)
- Never run code directly on the host machine
```

This instruction is picked up by Claude Code at the start of every conversation.

---

## How it works

drun runs code via [Pyodide](https://pyodide.org) — a WebAssembly port of
CPython — inside a [Deno](https://deno.land) subprocess. The Deno process stays
alive for the lifetime of a session, communicating with the Rust host over
stdin/stdout. Pyodide's in-memory filesystem provides an ephemeral workspace;
drun snapshots it at each step to power checkpointing, rollback, diffing, and
forking.

The isolation is structural, not policy-based. Because execution happens inside
WebAssembly, the sandbox cannot make arbitrary system calls, access the host
filesystem, or spawn processes. There is no escape hatch to configure wrong —
the boundary is the architecture.

This also means the dependency surface is frozen at the Deno + Pyodide layer. A
compromised or malicious package installed inside a session cannot affect the
host runtime, and the core execution environment itself is version-pinned and
auditable.

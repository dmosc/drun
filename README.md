# drun

![drun architecture](assets/architecture.png)

**Safe-by-design agentic code execution**

Isolated by design. Every execution is a checkpoint: mistakes are undoable,
experiments are forkable, and nothing reaches your host until you approve it.
You control what agents can access — network, files, secrets — and what they can
do to the outside world.

---

## Checkpoint model

Every time an agent runs code, the full workspace state is snapshotted before
execution begins. Think of it like `git` for your runtime: the entire history of
what ran, what changed, and what the environment looked like at each step is
preserved until the session is closed.

From any point in that history you can go back, branch off into a parallel
exploration without losing the original, compare what changed between any two
moments, or tag a milestone to return to later. Agents can try things that might
not work — because the cost of a wrong turn is a single rollback, not a broken
environment.

The operator decides the boundaries up front: which directories agents can read
from, which domains they can reach, which packages they can install, which
secrets they can see. Agents operate freely within those bounds and cannot
expand them at runtime.

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

### Updating

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

### Uninstalling

```bash
# MCP binary
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/uninstall.sh | bash

# Python SDK
pip uninstall drun-sandbox
```

---

## Configuration

drun is configured through a TOML file. Point `DRUN_CONFIG` at the file and the
server or SDK picks it up at startup. Without it, built-in defaults apply: PyPI
and jsDelivr are reachable, workspace is capped at 512 MB per session, and
active sessions are capped at 50.

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

| Field                       | Default                            | Description                                                                                                                                                                                                                                     |
| --------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `domain_allowlist`          | `[]` (only package infrastructure) | Additional domains reachable via `session_fetch` and Python outbound HTTP. `pypi.org`, `files.pythonhosted.org`, and `cdn.jsdelivr.net` are always allowed and cannot be removed. Use `["*"]` to allow all, or `"*.example.com"` for wildcards. |
| `fetch_timeout_ms`          | `60000`                            | Timeout for the full `session_fetch` response in milliseconds.                                                                                                                                                                                  |
| `connect_timeout_ms`        | `30000`                            | TCP connection timeout for `session_fetch` in milliseconds.                                                                                                                                                                                     |
| `exec_timeout_ms`           | `60000`                            | Maximum wall time for a single `session_execute_python` call. The runner is killed when exceeded; the session auto-rebuilds and remains usable.                                                                                                 |
| `install_timeout_ms`        | `120000`                           | Maximum wall time for `session_install_package` (pip download + install).                                                                                                                                                                       |
| `bash_timeout_ms`           | `30000`                            | Maximum wall time for a single `session_bash` call.                                                                                                                                                                                             |
| `max_workspace_mb`          | `512`                              | Maximum workspace size per session in megabytes.                                                                                                                                                                                                |
| `max_sessions`              | `50`                               | Maximum number of concurrent sessions.                                                                                                                                                                                                          |
| `max_checkpoints`           | `200`                              | Maximum checkpoints stored per session.                                                                                                                                                                                                         |
| `session_idle_timeout_secs` | `3600`                             | Seconds of inactivity before a session is abandoned.                                                                                                                                                                                            |
| `mount_allowlist`           | `[]`                               | Host path prefixes that `session_mount` may read from. Empty means all paths are permitted. Non-empty restricts mounts to the listed prefixes.                                                                                                  |
| `export_root`               | `"drun-export"`                    | Directory that `session_export` must write into.                                                                                                                                                                                                |
| `snapshots_dir`             | `"drun-snapshots"`                 | Directory where `session_snapshot` writes `.drun` files.                                                                                                                                                                                        |
| `snapshot_on_close`         | `false`                            | When true, automatically write a snapshot when `session_close` is called.                                                                                                                                                                       |
| `env_allowlist`             | `[]`                               | Host environment variable names exposed to agents via `session_get_env`. Empty means no variables are exposed.                                                                                                                                  |
| `package_allowlist`         | `[]`                               | Package names the agent may install via `session_install_package`. Empty means all packages are allowed. Enforced at the MCP layer only — the Python SDK bypasses this check.                                                                   |
| `bash_command_denylist`     | `[]`                               | Command substrings always rejected by `session_bash` before execution. Checked before the sandbox runs, so the error is an application-level rejection rather than a sandbox error.                                                             |
| `bash_command_allowlist`    | `[]`                               | Command substrings permitted by `session_bash`. Empty means all commands are allowed (subject to the denylist).                                                                                                                                 |
| `packages_dir`              | OS temp dir                        | Directory where pip installs packages. Shared across all sessions as a cache.                                                                                                                                                                   |

### Example

```toml
# Note: Domains for major package managers like PyPi are injected under the
# hood to allow for package installs.
domain_allowlist = [
    "api.example.com",
]

max_workspace_mb = 256
max_sessions = 10
exec_timeout_ms = 60_000
install_timeout_ms = 120_000
session_idle_timeout_secs = 1800

# Only the listed prefixes can be mounted into a session.
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

See [`examples/financial_analysis.toml`](examples/financial_analysis.toml) and
[`examples/data_science.toml`](examples/data_science.toml) for fully annotated
real-world recipes. See
[`examples/fibonacci_benchmark.toml`](examples/fibonacci_benchmark.toml) for a
tight-allowlist example that stress-tests every constraint.

---

## Onboarding workflows

### Python SDK — scripted agentic workflow

Use this path to drive a drun session from your own Python script, without
Claude Code or any MCP client. The SDK exposes the full session API directly.

```python
from drun import Session
from drun.chat import run

session = Session()
session.mount("/path/to/data")

# Direct API
cp = session.execute_python("import os; print(os.listdir('.'))")
print(cp.stdout)
session.rollback(cp.id - 1)

# LLM-driven loop (4 tools: execute_python, execute_bash, install_package, write_file)
run(
    session,
    "clean the data and compute summary statistics",
    model="claude-sonnet-4-6",
)
```

```bash
ANTHROPIC_API_KEY=sk-ant-... \
DRUN_CONFIG=/path/to/your.toml \
    python your_script.py
```

### `drun chat` — agentic CLI

Drive a session from the command line. Supports cloud providers and local
models.

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

**Local models via [Ollama](https://ollama.com) — no API key needed:**

```bash
ollama pull qwen2.5:14b

drun chat --model openai/qwen2.5:14b \
          --base-url http://localhost:11434/v1 \
          --mount ./src \
          "summarize what each module does"
```

Use the `openai/<model>` prefix with `--base-url http://localhost:11434/v1`
(Ollama's OpenAI-compatible endpoint) rather than `ollama/<model>`. The `/v1`
endpoint threads tool call IDs more reliably across turns.

`qwen2.5:14b` and `qwen2.5:7b` are the most reliable local models for multi-turn
structured tool calling. Avoid reasoning/thinking variants (`deepseek-r1`,
`qwen3.*`) — they emit tool calls as plain text rather than structured JSON and
do not interoperate reliably with standard tool-use loops.

**Options:**

| Flag                 | Default              | Description                                     |
| -------------------- | -------------------- | ----------------------------------------------- |
| `--model MODEL`      | `ollama/qwen2.5:14b` | litellm model identifier                        |
| `--base-url URL`     | —                    | API base URL override                           |
| `--mount PATH`       | —                    | Mount a host path into the session (repeatable) |
| `--system PROMPT`    | built-in             | Override the system prompt                      |
| `--max-iterations N` | `30`                 | Maximum agent loop iterations                   |

### Claude Code + VSCode extension

After installing the MCP binary and setting `DRUN_CONFIG` in
`~/.claude/settings.json` (see
[Where to set DRUN_CONFIG](#where-to-set-drun_config)), reload the VSCode
window. drun tools are then available in every Claude chat within VSCode.

Verify the server is connected by asking Claude to run `session_list`. If it
responds with an empty list the server is up. If the tool is missing, check the
Output panel under **Claude Code MCP** for errors.

### Claude Code + terminal

```bash
# Install and register once
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash

# Launch with config
DRUN_CONFIG=/path/to/your.toml claude
```

---

## Typical flows

**Data analysis with rollback:**

```
create_session
  → session_install_package(pandas)
  → session_mount(/data/sales.csv)
  → session_execute_python(load + clean data)       # checkpoint 1
  → session_execute_python(compute summary)         # checkpoint 2 — something looks off
  → session_rollback(1)                             # back to clean data
  → session_execute_python(corrected approach)      # checkpoint 3
  → session_export                                  # write output to host
```

**Parallel hypothesis testing:**

```
create_session → session_execute_python(load data) → checkpoint 1

session_fork(checkpoint_1) → session B

session_execute_python(session A, strategy 1)
session_execute_python(session B, strategy 2)       # both run from the same base

session_merge(winner into A)                        # bring best results together
session_close(session B)
```

**Safe host file editing:**

```
session_mount(/path/to/script.py)
session_execute_python(refactor the code)
session_diff                                        # review before touching the host
session_commit                                      # writes only changed mounted files back
```

**Checkpoint housekeeping:**

```
session_checkpoint_label(cp, "baseline")            # tag a milestone
session_checkpoint_squash(start, end, "setup")      # collapse setup steps into one
session_checkpoint_drop(cp)                         # delete a checkpoint permanently
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

## Network access

Never use curl, wget, or Python HTTP libraries to fetch external data directly.
Both session_execute_python and session_bash have restricted or no network
access by design. Always use session_fetch to retrieve external data — it saves
the response as a workspace file that subsequent session_execute_python and
session_bash calls can read immediately.
```

### Enforcing drun usage

CLAUDE.md instructions guide but do not hard-constrain — Claude can still choose
to use the built-in `Bash` tool. Add explicit prohibitions to steer it toward
drun:

```markdown
Never use Bash to run Python or shell commands that modify files — no
`python script.py`, no `pip install`, no `npm install`, no build tool
invocations. Use drun MCP tools for all code execution.
```

For shell commands that need to run in the workspace, use `session_bash` instead
of the built-in Bash tool. It executes in the same sandboxed environment,
produces a checkpoint, and respects the same operator policy (denylist,
allowlist, timeout) as `session_execute_python`.

---

## Tools reference

| Category   | Tools                                                                                                    |
| ---------- | -------------------------------------------------------------------------------------------------------- |
| Lifecycle  | `create_session`, `session_list`, `session_close`, `session_tree`                                        |
| Execution  | `session_execute_python`, `session_bash`, `session_install_package`, `session_get_env`, `session_cancel` |
| Navigation | `session_rollback`, `session_fork`, `session_merge`, `session_history`, `get_session_state`              |
| Files      | `session_read_file`, `session_write_file`, `session_delete_file`, `session_mount`, `session_diff`        |
| Host I/O   | `session_export`, `session_commit`, `session_fetch`, `get_fetch_allowlist`, `get_allowed_packages`       |
| Snapshots  | `session_snapshot`, `session_restore`, `list_snapshots`                                                  |
| Labels     | `session_label`, `session_checkpoint_label`, `session_checkpoint_squash`, `session_checkpoint_drop`      |

---

## Further reading

- [Security model](docs/security.md) — isolation layers, threat model, known
  limitations
- [Troubleshooting](docs/troubleshooting.md) — common errors and how to resolve
  them
- [Changelog](CHANGELOG.md)

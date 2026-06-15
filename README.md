# drun

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

**Requires [Deno](https://deno.land).** The one-liner installs it automatically;
all other paths assume it is already on your `PATH`.

**One-liner (recommended)** — detects your platform, installs Deno if needed,
downloads the binary to `/usr/local/bin`, and registers drun with Claude Code:

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

---

## Updating

```bash
# Update to the latest release
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash

# Update to a specific version
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash -s -- v0.1.1
```

No re-registration needed — Claude Code keeps pointing to the same binary path.

## Uninstalling

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/uninstall.sh | bash
```

Removes the binary and deregisters drun from Claude Code.

---

## Typical flows

**Data analysis with rollback:**

```
create_session
  → session_install_package(pandas)
  → session_mount(/data/sales.csv)          # read from host
  → session_execute(load + clean data)      # checkpoint 1
  → session_execute(compute summary)        # checkpoint 2 — something looks off
  → session_rollback(1)                     # back to clean data
  → session_execute(corrected approach)     # checkpoint 3
  → session_export                          # write output to host
```

**Parallel hypothesis testing:**

```
create_session → session_execute(load data) → checkpoint 1

session_fork(checkpoint_1) → session B

session_execute(session A, strategy 1)
session_execute(session B, strategy 2)      # both run from the same starting point

session_close(loser)
```

**Safe host file editing:**

```
session_mount(/path/to/script.py)
session_execute(refactor the code)
session_diff                                # review before touching the host
session_commit                              # writes only changed mounted files back
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
- `session_execute` — run Python code
- `session_bash` — run shell commands (git, npm, make, etc.)
- `session_fork` — explore alternative approaches without losing the original
- `session_rollback` — recover from mistakes
- `session_read_file` / `session_export` — retrieve outputs
- `session_commit` — write changes back to host files after review

## Network access

Never use curl, wget, or Python HTTP libraries to fetch external data directly.
Both session_execute and session_bash have restricted or no network access by
design. Always use session_fetch to retrieve external data — it saves the
response as a workspace file that subsequent session_execute and session_bash
calls can read immediately.
```

### Enforcing drun usage

CLAUDE.md instructions guide but do not hard-constrain — Claude can still choose
to use the built-in `Bash` tool. Add explicit prohibitions to steer it toward
drun:

```markdown
Never use Bash to run Python or shell commands that modify files — no
`python3 script.py`, no `pip install`, no `npm install`, no build tool
invocations. Use drun MCP tools for all code execution.
```

For shell commands that need to run in the workspace, use `session_bash` instead
of the built-in Bash tool. It executes in the same sandboxed environment,
produces a checkpoint, and respects the same operator policy (denylist,
allowlist, timeout) as all other drun tools.

---

## Tools reference

| Category   | Tools                                                                                              |
| ---------- | -------------------------------------------------------------------------------------------------- |
| Lifecycle  | `create_session`, `session_list`, `session_close`, `session_tree`                                  |
| Execution  | `session_execute`, `session_bash`, `session_install_package`, `session_get_env`                    |
| Navigation | `session_rollback`, `session_fork`, `session_history`, `get_session_state`                         |
| Files      | `session_read_file`, `session_write_file`, `session_delete_file`, `session_mount`, `session_diff`  |
| Host I/O   | `session_export`, `session_commit`, `session_fetch`, `get_fetch_allowlist`, `get_allowed_packages` |
| Snapshots  | `session_snapshot`, `session_restore`                                                              |
| Labels     | `session_label`, `session_checkpoint_label`                                                        |

---

## Configuration

Set `DRUN_CONFIG` to a TOML file path. Without it, drun runs with no network
access and no restrictions on workspace size or session count.

```toml
# Domains reachable via session_fetch and Python outbound HTTP.
# Default includes PyPI and jsDelivr so packages can be installed out of the
# box.
domain_allowlist = ["api.example.com", "data.sec.gov"]

max_workspace_mb = 512
max_sessions = 20
max_checkpoints = 100
session_idle_timeout_secs = 3600

# mount_allowlist = ["/tmp/drun-inputs"]       # restrict session_mount source paths
# export_root = "/tmp/drun-outputs"            # restrict session_export destination
# env_allowlist = ["OPENAI_API_KEY"]           # env vars readable via session_get_env
# package_allowlist = ["pandas", "numpy"]      # restrict installable packages
# auto_snapshot = true
# snapshots_dir = "/tmp/drun-snapshots"

# bash_timeout_ms = 30000
# bash_command_denylist = ["rm -rf /"]
# bash_command_allowlist = ["git", "npm"]
```

See [`examples/drun.toml`](examples/drun.toml) for a fully annotated example.

---

## Further reading

- [Security model](docs/security.md) — isolation layers, threat model, known
  limitations
- [Troubleshooting](docs/troubleshooting.md) — common errors and how to resolve
  them
- [Changelog](CHANGELOG.md)

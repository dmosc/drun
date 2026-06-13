# drun

**A sandboxed Python runtime for AI agents — ephemeral, checkpointed, and safe
by design.**

Agents can write code, install packages, generate files, roll back mistakes, and
explore alternative approaches freely without ability to interact with the host
unless explicitly allowed.

---

## The core model

**Every execution is a checkpoint.** Each `session_execute` call snapshots the
full workspace state. Agents can roll back to any prior checkpoint, fork from
one, or diff between two — like `git` for runtime. Nothing is lost until a
session is explicitly closed.

**Nothing escapes the sandbox by default.** Execution runs inside
[Pyodide](https://pyodide.org), a WebAssembly port of CPython, hosted inside
[Deno](https://deno.land). The isolation is architectural, not policy-based —
the WASM boundary prevents arbitrary system calls, host filesystem access, and
process spawning regardless of what the code tries to do. Files only reach the
host when you call `session_export` or `session_commit`.

**Operator controls what agents can reach.** Network access, mount paths, export
destinations, installable packages, and readable env vars are all opt-in
allowlists set in the server config. An agent cannot grant itself permissions —
it can only operate within what the operator pre-approved.

---

## Installation

```bash
# One-liner: installs Deno if needed, downloads the binary, registers with Claude Code
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

Manual install:

```bash
# macOS Apple Silicon
curl -L https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-arm64 -o drun-mcp
chmod +x drun-mcp
claude mcp add drun -- /path/to/drun-mcp
```

Or via Cargo:

```bash
cargo install drun-mcp
claude mcp add drun -- $(which drun-mcp)
```

**Dependency:** [Deno](https://deno.land) — installed automatically by the
one-liner.

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

session_execute(session_A, strategy 1)
session_execute(session_B, strategy 2)      # both run from same starting point

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

Always use drun MCP tools for code execution. Never run code directly on the
host.

- `create_session` — start every coding task
- `session_install_package` — before importing third-party packages
- `session_execute` — run code
- `session_fork` — explore alternative approaches without losing the original
- `session_rollback` — recover from mistakes
- `session_read_file` / `session_export` — retrieve outputs
- `session_commit` — write changes back to host files after review
```

---

## Tools reference

| Category   | Tools                                                                                             |
| ---------- | ------------------------------------------------------------------------------------------------- |
| Lifecycle  | `create_session`, `session_list`, `session_close`, `session_tree`                                 |
| Execution  | `session_execute`, `session_install_package`, `session_get_env`                                   |
| Navigation | `session_rollback`, `session_fork`, `session_history`, `get_session_state`                        |
| Files      | `session_read_file`, `session_write_file`, `session_delete_file`, `session_mount`, `session_diff` |
| Host I/O   | `session_export`, `session_commit`, `session_fetch`, `get_fetch_allowlist`                        |
| Snapshots  | `session_snapshot`, `session_restore`                                                             |
| Labels     | `session_label`, `session_checkpoint_label`                                                       |

---

## Configuration

Set `DRUN_CONFIG` to a TOML file path. Without it, drun runs with no network
access and no restrictions on workspace size or session count.

```toml
[fetch]
# Domains reachable via session_fetch and Python outbound HTTP.
# Python package CDNs (PyPI, jsDelivr) are always included regardless of this list.
allowlist = ["api.example.com", "data.sec.gov"]

[session]
max_workspace_mb = 512       # per-session workspace cap
max_sessions = 20            # concurrent session limit
max_checkpoints = 100        # checkpoints per session
session_idle_timeout_secs = 3600   # reap abandoned sessions after 1h

# mount_allowlist = ["/tmp/drun-inputs"]     # restrict session_mount paths
# export_root = "/tmp/drun-outputs"          # restrict session_export destination
# env_allowlist = ["OPENAI_API_KEY"]         # env vars readable via session_get_env
# allowed_packages = ["pandas", "numpy"]     # restrict installable packages
# auto_snapshot = true                       # snapshot on session_close
# snapshots_dir = "/tmp/drun-snapshots"
```

See [`examples/drun.toml`](examples/drun.toml) for a fully annotated example.

# Changelog

All notable changes to drun are documented here.

---

## v0.1.1 — 2026-06-15

### Python SDK (`drun-sandbox`)

- Added `drun chat` CLI command: runs a local or cloud LLM in an agentic
  tool-use loop with access to four sandbox tools — `execute_python`,
  `execute_bash`, `install_package`, and `write_file`
- Added `drun.chat.run()` for programmatic access to the same loop from Python
- Added `[chat]` optional dependency group (`litellm>=1.0`) for multi-provider
  LLM routing
- Added `drun` console script entry point via `pyproject.toml`
- System prompt explicitly declares the CPython (non-WebAssembly) execution
  environment and tool-use rules to prevent model hallucination
- Assistant messages are serialized as plain dicts in exact OpenAI wire format,
  ensuring tool call ID association is preserved across all litellm backends
  including Ollama's OpenAI-compatible `/v1` endpoint

---

## v0.1.0 — 2026-06-14

Initial public release.

### MCP server

25 tools across 7 categories exposed over stdio MCP:

- **Lifecycle** — `create_session`, `session_list`, `session_close`,
  `session_tree`
- **Execution** — `session_execute`, `session_install_package`,
  `session_get_env`
- **Navigation** — `session_rollback`, `session_fork`, `session_history`,
  `get_session_state`
- **Files** — `session_read_file`, `session_write_file`, `session_delete_file`,
  `session_mount`, `session_diff`
- **Host I/O** — `session_export`, `session_commit`, `session_fetch`,
  `get_fetch_allowlist`, `get_allowed_packages`
- **Snapshots** — `session_snapshot`, `session_restore`
- **Labels** — `session_label`, `session_checkpoint_label`

### Execution sandbox

- Python runs inside [Pyodide](https://pyodide.org) (WebAssembly CPython) hosted
  in [Deno](https://deno.land) — no syscalls, no host filesystem access, no
  process spawning from Python code
- Working directory set to `/workspace` before each execution
- stdout and stderr capped at 1 MB per execution
- Per-session execution timeout with `KeyboardInterrupt` delivery and automatic
  runner recovery after crash or timeout

### Checkpoint model

- Full workspace snapshot on every `session_execute`
- Rollback, fork, diff, and label at any checkpoint
- Per-session checkpoint limit (default 200)
- Snapshot serialization to `.drun` files with magic-byte integrity check

### Operator controls

- Network allowlist: restricts outbound HTTP from both `session_fetch` and
  Python code; PyPI CDNs always included for package installs
- Mount allowlist: canonicalized prefix check on all `session_mount` source
  paths
- Export root: `session_export` and `session_snapshot` confined to a configured
  directory
- Package allowlist: `session_install_package` restricted to an explicit list
- Env allowlist: `session_get_env` restricted to named variables
- Conservative compile-time defaults when no config file is provided (512 MB
  workspace, 50 sessions, 200 checkpoints, 1 h idle timeout)

### Security fixes

- Agents cannot override the server's network allowlist at session creation time
- `session_restore` revalidates the snapshot's network policy against the server
  config before creating a live session
- Path traversal via `..` components blocked at write, export, commit, and
  snapshot
- `session_fetch` streams the response body and aborts before fully buffering
  responses that exceed the workspace limit
- Per-process temp file for the Deno runner (prevents TOCTOU races between
  concurrent server instances)
- `session_snapshot` output path validated against `snapshots_dir` when
  configured
- `session_busy` error returned immediately on concurrent `session_execute`
  calls instead of deadlocking
- Default fetch timeout of 60 seconds

### Installation

- `install.sh` — one-liner for macOS (arm64, x86_64) and Linux (x86_64);
  installs Deno if missing; registers with Claude Code
- `update.sh` — upgrade to latest or a specific tagged release in place
- `uninstall.sh` — removes the binary and deregisters from Claude Code across
  all MCP scopes

### Known limitations

- Deno `--allow-read` and `--allow-write` are global (not path-scoped); see
  [security model](docs/security.md#known-limitation-deno-filesystem-access)
- No Windows support
- No tool-call cancellation (a cancelled MCP request does not interrupt the
  running Deno subprocess)
- Package installs have no progress streaming; large packages appear to hang for
  1–5 minutes

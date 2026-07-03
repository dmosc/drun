# Changelog

All notable changes to drun are documented here.

---

## v0.3.2 — 2026-07-02

### `drun init` subcommand

Per-project setup is now a first-class binary subcommand. Run `drun-mcp init`
from any project root to scaffold the two project-local files:

- `.claude/settings.json` — restricts Claude to drun MCP tools for this
  workspace; blocks native file, shell, network, and agent delegation tools.
- `CLAUDE.md` — tells Claude how to bootstrap and use drun in the project.

The project path is recorded in `~/.drun/projects` so `uninstall.sh` can clean
up `.claude/settings.json` files across all initialized projects.

`install.sh` is now strictly global (binary, config, daemon, MCP registration).
It no longer touches the current directory or creates project-local files.

---

## v0.3.1 — 2026-07-01

### Web UI

- Added per-checkpoint fork chips (↑ child-session) on the timeline so you can
  navigate to sessions that branched off from any checkpoint.
- Fixed fork badge (⑂ parent-id) on session cards: the badge was missing for
  forked sessions whose parent was still alive because the tree serializer was
  omitting `parent_session_id` for nested sessions (only orphan roots carried
  it).
- Fixed diff pane: `+++ b/file` header lines were rendered green (as additions)
  due to a typo in the `startsWith` check; they now render muted as metadata.
- Checkpoint dots are green for the current head, purple for all others.
- Removed file-modification pills (+N ~N -N) from timeline nodes; file change
  counts are now in the checkpoint detail panel header.
- Added `Cache-Control: no-store` to the HTML response so browsers never serve a
  stale page after a daemon restart.

### MCP server

- Fixed `session_merge` with `session_id == source_session_id`: now returns a
  clear "cannot merge a session with itself" error instead of `session_busy`.
- Fixed `host_from_url` IPv6 parsing: URLs like `http://[::1]/path` previously
  included the port suffix in the extracted host string, breaking allowlist
  matching.
- `web_port = 0` in config now disables the web UI as documented.

---

## v0.3.0 — 2026-06-24

### MCP transport

- Switched from stdio to a persistent HTTP/SSE daemon on `127.0.0.1:7273`. A
  single `drun-mcp` process now serves all Claude Code windows and terminal
  sessions on the host simultaneously.
- Added Streamable HTTP endpoint (`/mcp`) alongside the SSE endpoint (`/sse`).
- `install.sh` registers drun as a `launchd` agent (macOS) or `systemd` user
  service (Linux) so the daemon starts on login and restarts automatically.

### Web UI

- Added embedded trajectory viewer at `http://127.0.0.1:7274` — live-polling
  session and checkpoint state, per-checkpoint diff and stdout/stderr viewer.

---

## v0.2.3 — 2026-06-21

- Added `session_label` and `session_checkpoint_label`: attach human-readable
  names to sessions and checkpoints; labels appear in history and tree output
  and can be used in place of IDs for rollback, diff, and fork.
- Added `session_checkpoint_squash`: collapse a range of checkpoints into one,
  keeping the terminal file state and combining stdout/stderr.
- Added `session_checkpoint_drop`: remove a range of checkpoints to free memory
  or stay under the checkpoint limit.
- Locked drun-initialized workspaces to MCP-only tool access via a generated
  `.claude/settings.json` that blocks native file, shell, and network tools.

---

## v0.2.2 — 2026-06-19

- Added `session_merge`: overlay files from one session's checkpoint onto
  another, creating a new checkpoint. Useful for combining parallel
  explorations.
- Replaced inline stdout/stderr content in MCP responses with byte-count
  metadata (`stdout_bytes`, `stderr_bytes`). Use `checkpoint_read_stdstreams` to
  page through the actual output without flooding the context window.
- Added `checkpoint_read_stdstreams`: paginated stdout/stderr reader with the
  same `offset`/`limit`/`total_bytes`/`has_more` envelope as
  `session_read_file`.

---

## v0.2.0 — 2026-06-17

### Execution sandbox

- **Breaking**: replaced the Pyodide/Deno WebAssembly Python executor with a
  native bash sandbox. `session_execute` and `session_install_package` are gone;
  `session_bash` is the single execution primitive.
- macOS: `sandbox-exec` with an SBPL profile that denies everything except
  reads, workspace writes, and process management. Network is denied by default.
- Linux: `bubblewrap` (`bwrap`) with a read-only host root, writable workspace,
  isolated `/tmp`, and `--unshare-net`.
- Timeout enforced by a dedicated kill thread (`bash_timeout_ms`; default 30 s).
- stdout is streamed line-by-line via MCP progress notifications while the
  command runs.

### Files

- Added `mount_overlay_paths` config: large directories (`node_modules`,
  `.venv`, `venv`, `target`, `__pycache__`, `.git`) are symlinked into the
  workspace at execution time instead of being copied into the checkpoint graph.
- Added `session_snapshot` / `session_restore`: serialize and reload a session's
  full checkpoint history to/from a zstd-compressed `.drun` file.
- Added `session_get_env`: read named host environment variables inside a
  session, gated by `env_allowlist` in server config.
- Added `get_fetch_allowlist`: return the server's fetch domain allowlist so
  agents can check what hosts are available before constructing requests.

---

## v0.1.1 — 2026-06-15

_(Pyodide/Deno architecture — superseded by v0.2.0)_

- Added `drun chat` CLI command and `drun.chat.run()` Python API: runs a local
  or cloud LLM in an agentic loop with four sandbox tools.
- Added `[chat]` optional dependency group (`litellm>=1.0`).

---

## v0.1.0 — 2026-06-14

_(Pyodide/Deno architecture — superseded by v0.2.0)_

Initial public release. Python execution via Pyodide (WebAssembly CPython)
hosted in Deno; no syscalls, no host filesystem access from Python code. 25 MCP
tools across lifecycle, execution, navigation, files, host I/O, snapshots, and
labels categories.

# Changelog

All notable changes to drun are documented here.

---

## Unreleased

### Web UI

- Checkpoint detail now shows a real file tree (click any file to preview its
  content — text, or the image itself for `png`/`jpg`/`gif`/`webp`/`svg`)
  instead of just the added/modified/removed file list; entries still carry +/~
  sigils for what changed since the previous checkpoint. Checkpoints created by
  `session_bash` also show the executed command, with a copy button. Backed by
  new `command` fields on `Checkpoint`/`CheckpointRecord`/`CheckpointSummary`
  (persisted through snapshots) and two new endpoints:
  `/api/sessions/{id}/checkpoints/{cp}/files` (path + size listing) and
  `/api/sessions/{id}/checkpoints/{cp}/files/{*path}` (raw content, with an
  `x-drun-binary` header for non-UTF-8 files the browser shouldn't try to render
  as text).
- Session cards get two new buttons: copy the session id to the clipboard, and
  destroy the session outright (with a confirmation prompt — this discards its
  workspace and checkpoint history, honoring `snapshot_on_close` exactly like
  `session_close` does, since it's the same underlying operation via a new
  `DELETE /api/sessions/{id}` endpoint and a `close_session` helper shared with
  the MCP tool).
- The web UI now surfaces daemon and session health that was previously only in
  memory. A new status strip shows version, PID, uptime, memory (RSS), and
  session count against `max_sessions`; an expandable "daemon" panel adds ports,
  idle timeout/workspace/checkpoint limits, and the domain/mount allowlists.
  Session cards now show an idle badge (`idle 12m`) that turns amber past 50% of
  the idle timeout and red past 90%, with a "reaps in Xm" countdown, so it's
  clear which sessions are about to be evicted before it happens. Backed by a
  new `/api/status` endpoint and `age_secs`/`idle_secs` on each node in
  `/api/sessions/tree`; no new session-side state was needed since
  `created_at`/`last_activity` were already tracked, just never serialized.

### `drun chat`

- Added `--session-id` to `drun chat`, to attach to an already-running session
  (e.g. one created from Claude Code, the web UI, or a prior `drun chat` call)
  instead of always creating a new one. `DrunMcpBridge` now owns session
  bootstrapping end to end: given a `session_id` it validates the session exists
  (via `get_session_state`, failing fast with the daemon's error if not) before
  mounting any `--mount` paths into it; given none, it falls back to
  `create_session` as before.
- Unified the CLI's `ChatAgent` loop and the Python SDK examples' standalone
  `drun.chat.run()` function, which had drifted into two near-identical
  tool-calling loops (one against the MCP daemon, one against an embedded
  `DrunSession` with a hardcoded 2-tool subset). `chat.run()` is gone; the
  examples (`financial_analysis.py`, `data_science.py`,
  `fibonacci_benchmark.py`) now build a `ChatAgent` with a new
  `LocalSessionBridge`, which adapts a `Session` to the same `Bridge` interface
  `DrunMcpBridge` implements. `ChatAgent.run()` no longer takes a `mounts`
  argument — mounting is bridge bootstrap logic, not loop logic.
- The `drun chat` CLI now drives a running `drun-mcp` daemon over MCP instead of
  an embedded, 2-tool subset — it gets the full tool suite (`session_fetch`,
  `session_fork`, checkpoints, etc.), the same as Claude Code, and shares
  sessions with the web UI. New `DrunMcpBridge` (MCP client) and `ChatAgent`
  (tool-calling loop) classes back the CLI. New `--mcp-url` flag (default
  `http://127.0.0.1:7273/mcp`); a running daemon is now required for
  `drun chat`.
- Default `--model` changed from `ollama/qwen2.5:14b` to
  `ollama_chat/qwen2.5:14b`. litellm's `ollama/` prefix routes through Ollama's
  legacy `/api/generate` endpoint, which emulates tool calling via a JSON-mode
  hack that silently produces empty responses on models like `gpt-oss`/`qwen3`;
  `ollama_chat/` routes through Ollama's native `/api/chat` endpoint, which
  forwards `tools` as real function-calling.
- Fixed `DrunMcpBridge.call()` swallowing tool-level errors: it now raises with
  the daemon's actual error text instead of silently returning it as if it were
  successful output, which previously surfaced as a confusing
  `json.loads`/`Expecting value` error one layer up.
- Fixed `DrunMcpBridge.call()` sending no `arguments` field at all for
  zero-argument tools (e.g. `create_session`) — the `mcp` client omits `None`
  arguments from the wire request, but the daemon requires the key present. Now
  sends `{}`.
- Documented a macOS-specific `session_mount` failure
  (`Permission denied (os error 13)`) in `docs/troubleshooting.md`: TCC blocks
  the `drun-mcp` `launchd` agent from reading `~/Desktop`/`~/Documents`/
  `~/Downloads`/iCloud Drive until the binary is granted Full Disk Access.

### Reliability fixes

- Config is now re-read from disk on every tool call instead of once at daemon
  startup. `drun-mcp config add-domain/add-path/remove-domain/
  remove-path`
  (and hand-edits to `config.toml`) take effect immediately, in every open
  session — no restart, no dropped sessions. `web_port` and
  `session_idle_timeout_secs` still require a restart (applied at startup only).
  `drun-mcp init` now also allowlists the project directory for `session_mount`
  automatically. `config_cmd`'s TOML writes are atomic (temp file + rename).
- Fixed `install.sh`/`update.sh` writing a freshly downloaded binary directly
  onto `/usr/local/bin/drun-mcp` while the daemon was still running from that
  exact path. Truncating a binary in place while it's actively executing can
  corrupt macOS's code-signing validation for that file, wedging the daemon into
  a `OS_REASON_CODESIGNING` crash loop that not even `kill -9` recovers from.
  Both scripts now download to a temp file and swap it into place with an atomic
  rename. Also anchored the `pkill -f drun-mcp` fallbacks in
  `update.sh`/`uninstall.sh` to `pkill -f "drun-mcp$"` so they can't match
  unrelated processes. Added a "Health check" section to
  `docs/troubleshooting.md` with commands to confirm the daemon is running
  exactly once and not crash-looping, since a dead daemon is otherwise
  indistinguishable from an idle one, plus fixed two stale commands in that doc
  (a nonexistent `claude mcp restart drun`, and re-registration examples that
  used stdio-transport syntax instead of the SSE transport drun actually
  registers with). `DEVELOPMENT.md` now documents how to safely test a local
  build against the installed binary/service manager instead of only a throwaway
  debug build.
- Fixed `drun-mcp init` silently no-oping when `.claude/settings.json` already
  existed — the project was left with native tools (`Bash`, `Edit`, `Write`,
  etc.) fully enabled with no warning, since the sandbox's deny-list was never
  written. `init` now merges drun's required `permissions.deny`/`allow` entries
  into an existing file (preserving everything else in it — other permissions,
  hooks, env, etc.) instead of skipping. If the existing file can't be safely
  parsed/merged, it's left untouched and a loud warning with the exact JSON to
  add by hand is printed, instead of silently doing nothing.

### PyPI packaging

- Fixed the `drun-sandbox` PyPI package being stuck at `0.1.1` since its first
  release, despite every tagged release since then reporting the `publish-pypi`
  workflow job as green. Root cause: a second, stale `pyproject.toml` at the
  repo root duplicated (and diverged from) the real one at
  `crates/drun-py/pyproject.toml`. `maturin build -m crates/drun-py/Cargo.toml`
  (what CI actually runs) resolves metadata from the `pyproject.toml` next to
  the given manifest path, not the repo root — so every release silently rebuilt
  the exact same `drun_sandbox-0.1.1` wheel regardless of the version bump in
  `Cargo.toml`, and PyPI's `skip-existing: true` swallowed the resulting
  "already exists" as a quiet no-op instead of a failure. Removed the dead root
  `pyproject.toml` and changed `crates/drun-py/pyproject.toml`'s hardcoded
  `version = "0.1.1"` to `dynamic = ["version"]`, so it now tracks
  `Cargo.toml`'s version like the Rust crates already do. Verified locally with
  `maturin build` that this now produces `drun_sandbox-0.3.5-*.whl` with the
  correct version and its `chat`/`test` extras and `drun` console-script entry
  point intact.

---

## v0.3.3 — 2026-07-05

### `drun-mcp config` CLI

- Added `drun-mcp config add-domain <domain>` / `remove-domain <domain>` and
  `add-path <path>` / `remove-path <path>` to edit `~/.drun/config.toml`'s
  `domain_allowlist`/`mount_allowlist` without hand-editing the file. Edits
  preserve every existing comment and the file's formatting (via `toml_edit`),
  and the daemon is restarted automatically afterward.
- Added `drun-mcp config list` to print the effective allowlists.
- `CLAUDE.md` (generated by `drun-mcp init`) now tells the agent to point the
  user at these commands when `session_fetch`/`session_mount` denies something.
- `Config::load_from` is now public, so the CLI (and any other caller) can parse
  a config file at an explicit path instead of going through `Config::load`,
  which only consults `$DRUN_CONFIG`.

### Reliability fixes

- Fixed `session_checkpoint_squash` and `session_checkpoint_drop` allowing
  checkpoint 0 (the mounted baseline) into their range. Since `session_commit`
  and `session_diff`'s defaults both read checkpoint 0 as "the state before the
  sandbox touched anything," squashing or dropping it silently moved that
  baseline forward — `session_commit` could then skip writing back files that
  had genuinely changed since the real mount. Both tools now reject ranges
  starting at checkpoint 0 with a clear error.
- Fixed idle sessions being unrecoverable: once a session crossed
  `session_idle_timeout_secs`, every call — including `session_snapshot`,
  `session_export`, `session_commit`, and `session_read_file` — was rejected
  with `session_idle`, even though the whole point of those calls is to rescue a
  session's state before the reaper evicts it. Read and recovery calls are no
  longer gated by the idle check; only calls that would do new work
  (`session_bash`, `session_write_file`, etc.) are.
- Fixed `session_bash` discarding checkpoints ahead of a rollback point _before_
  running the command, so a command that failed after rollback (timeout,
  oversized output, spawn error) permanently lost those checkpoints even though
  nothing new was ever committed. Forward history is now only discarded once a
  run actually succeeds.
- Fixed `session_merge` not discarding checkpoints ahead of a rollback point,
  unlike `session_bash`/`session_write_file`/`session_delete_file` — merging
  after a rollback left orphaned checkpoints reachable by ID, contradicting
  `session_rollback`'s documented destructive semantics. `session_merge` now
  truncates forward history like the other mutating tools.
- Read-only MCP calls (`with_session`) and the web UI's session lookup no longer
  block indefinitely waiting on a session's lock — they now return
  `session_busy` / `503` immediately on contention, matching the behavior
  mutating calls already had.
- Added a first unit test module for `drun-core`'s `Session`, covering the four
  fixes above.

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

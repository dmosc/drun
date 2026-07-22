---
name: integrate-drun
description: >-
  Integrate drun (sandboxed code execution for agentic loops) into a software
  project so its AI agents run file, shell, and network operations inside an
  isolated sandbox instead of directly on the host. Works for a brand-new
  project or an existing AI agent application. Use when the user asks to "add
  drun", "sandbox my agent", "make agent execution safe", or similar.
---

# Skill: Integrate drun into a project

You are integrating **drun** into a project. drun gives AI agents sandboxed code
execution: file writes, shell commands, and network calls run in an isolated
workspace with rollback, forking, and an outbound-network allowlist — instead of
touching the host directly.

Your job is to (1) understand the project you are integrating into, (2) get
`drun-mcp` running and reachable, (3) wire the agent's MCP client to it, and
(4) verify the agent can call drun tools. Follow the steps below. Do not skip
verification.

---

## Background: what drun is

`drun-mcp` is a single self-contained binary that speaks the **MCP Streamable
HTTP** transport. It listens on `http://127.0.0.1:7273/mcp` and exposes sandbox
tools to any MCP-compatible agent (Claude Code, or your own agent framework).

- One long-lived `drun-mcp` process serves many concurrent **sessions**.
- Each **session** is an isolated workspace with its own checkpoint history.
- Agents work through drun tools (prefixed `mcp__drun__` in Claude Code) instead
  of native file/shell tools.

### Release binaries

Pre-built binaries ship with every release at
`https://github.com/dmosc/drun/releases`:

| Platform              | Asset name                |
|-----------------------|---------------------------|
| macOS (Apple Silicon) | `drun-mcp-macos-arm64`    |
| Linux x86-64          | `drun-mcp-linux-x86_64`   |

There is no Windows binary yet. `DRUN_VERSION` (or resolving the latest tag via
the GitHub API) selects the version at build time.

### The full tool catalog

**Session lifecycle**
- `create_session` — create a sandbox session, returns `session_id`
- `session_close` — terminate a session and free its subprocess
- `session_list` / `session_tree` — enumerate sessions (tree shows fork nesting)
- `get_session_state` / `session_history` — inspect current state / checkpoints
- `session_label` / `session_checkpoint_label` — attach human-readable labels

**Files & shell (the core loop)**
- `session_bash` — run a shell command in the workspace. Uses the host PATH, so
  python3/node/go/etc. are available. **No network access** by design.
- `session_read_file` — read a session-relative file (supports offset/limit paging)
- `session_write_file` — create/overwrite a file (`is_base64` for binary)
- `session_delete_file` — delete a file
- `session_mount` — copy a host file/dir into the session (this is how project
  files get in; directories like `node_modules`/venvs are symlinked as read-only
  overlays, never loaded into memory)
- `session_export` — write sandbox-created files back out to the host
- `session_commit` — write changed *mounted* files back to their original host paths

**History navigation (the drun superpower)**
- `session_diff` — unified diff between two checkpoints (default: mounted state → now)
- `session_rollback` — move head to a prior checkpoint. **Destructive**: the next
  successful write discards checkpoints after the rollback point. `session_fork`
  first if you want to keep them.
- `session_fork` — branch a new independent session from any checkpoint
- `session_merge` — overlay files from another session's checkpoint onto this one
- `session_checkpoint_squash` — collapse a checkpoint range into one

**Snapshots (persist to disk)**
- `session_snapshot` — serialize a session's full history to a `.drun` file
- `session_restore` — load a `.drun` file back into a new session
- `list_snapshots` — list `.drun` files in the server's snapshots dir

**Network & secrets**
- `session_fetch` — the *only* outbound-HTTP gateway. Saves the response body as
  a workspace file (never returned inline). Target domain must be in the server's
  allowlist.
- `get_fetch_allowlist` — list domains allowed for `session_fetch`
- `session_get_env` — read a host env var, but only ones in the server's
  `env_allowlist` (this is how you pass secrets in without hardcoding)

### Server configuration

drun reads `~/.drun/config.toml` once at startup (restart to apply changes).
Key fields and their defaults:

- `domain_allowlist` — domains `session_fetch` may reach
- `mount_allowlist` — host paths `session_mount` may load (empty = allow any)
- `mount_overlay_paths` — dir names symlinked read-only (`node_modules`, venvs…)
- `env_allowlist` — env vars `session_get_env` may read (empty = none)
- `bash_command_denylist` / `bash_command_allowlist` — command policy
- `bash_timeout_ms` (30s), `fetch_timeout_ms` (60s), `connect_timeout_ms` (30s)
- `max_workspace_mb` (512), `max_sessions` (50), `max_checkpoints` (200),
  `session_idle_timeout_secs` (3600)

The CLI edits these:
```
drun-mcp config add-domain <name>    # allow a domain for session_fetch
drun-mcp config add-path <path>      # allow a path for session_mount
drun-mcp config remove-domain <name>
drun-mcp config remove-path <path>
drun-mcp config list
```

---

## Critical transport facts (get these wrong and nothing works)

drun-mcp implements **MCP Streamable HTTP only**. The single most common failure
mode is misconfiguring the transport.

| Mistake | Symptom | Fix |
|---------|---------|-----|
| Using SSE transport (`GET /sse`) | connection fails / 404 | Use `POST /mcp` |
| `Accept: application/json` alone | **406 Not Acceptable** | `Accept: application/json, text/event-stream` |
| Wrong config scope (see below) | agent shows no drun tools | register at the right scope |

The correct Claude Code MCP entry is **always** this shape:

```json
{
  "mcpServers": {
    "drun": {
      "type": "http",
      "url": "http://127.0.0.1:7273/mcp",
      "headers": { "Accept": "application/json, text/event-stream" }
    }
  }
}
```

---

## Integration procedure

### Step 0 — Understand the project

Before changing anything, determine:

1. **Is this a greenfield project or an existing AI agent app?**
   - Greenfield: you control the build, the process lifecycle, and agent config.
   - Existing app: you must *find* the existing build step, process manager, and
     agent-config-writing code and extend them minimally. Read those first.
2. **What agent runtime does it use?** Claude Code is the primary target. If it's
   a custom framework, find where it configures MCP servers.
3. **How is the app built and distributed?** (npm/Electron, Go, Python, Docker…)
   This decides how you ship the `drun-mcp` binary.
4. **Does the app already manage a long-running daemon/server process?** If so,
   that is where drun-mcp should be started and stopped.

State your findings before proceeding.

### Step 1 — Get the binary onto the machine

Pick the approach matching the build system. Prefer **embedding at build time**
so end users need zero setup.

- **Node/Electron**: add a `prebuild`/`premake` script that downloads the
  platform-appropriate asset from GitHub releases into your bundled resources.
- **Go**: download into an embed directory and use `//go:embed` behind a build
  tag (e.g. `bundled_drun`); extract to a writable path at runtime.
- **Python**: download in a post-install hook or ship as a platform wheel;
  locate at runtime with `importlib.resources`.
- **Docker**: `curl` the release asset into the image in the Dockerfile.
- **Quick local dev / no packaging**: run drun's `install.sh`, or just place the
  binary on `PATH`.

Always: `chmod +x` the binary, detect the platform, and **fall back gracefully**
(build/run without drun) on unsupported platforms rather than hard-failing.

### Step 2 — Start drun-mcp and confirm it is ready

Start one long-lived `drun-mcp` process when the app's runtime/daemon starts.

- **Idempotency**: before starting, probe port 7273. If something answers, reuse
  it — do not start a second instance.
- **Readiness**: there is no `/health` endpoint. Probe by performing the MCP
  `initialize` handshake (POST `/mcp` with the Accept header above). Retry every
  ~200ms for up to ~10s.
- **Data location**: set `DRUN_SNAPSHOTS_DIR` to a directory inside the app's own
  data dir so drun state lives with the rest of the app's data.

Verify manually before wiring the agent:

```bash
curl -s -X POST http://127.0.0.1:7273/mcp \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}'
```

A JSON (or `data:`-prefixed SSE) response with `serverInfo.name = "drun"` means
it is up.

### Step 3 — Register drun with the agent

You must do **two** things for Claude Code, and they are different:

**(a) Block native tools + allow drun tools.** This is what `drun-mcp claude
init` does: run it in the project directory and it writes `.claude/settings.json`
with a `permissions` block that *denies* `Bash`, `Edit`, `Write`, `Read`, `Glob`,
`Grep`, `WebFetch`, `WebSearch`, `Task`, etc. and *allows* `mcp__drun__*`. It
also creates a `CLAUDE.md` telling the agent to work through drun tools, and
registers the project in `~/.drun/projects`.

```bash
cd /path/to/project && drun-mcp claude init
```

**(b) Register the MCP server connection.** `drun-mcp claude init` *also* runs
`claude mcp add --scope user --transport sse drun http://127.0.0.1:7273/sse` —
but ⚠️ **that scope is usually wrong for an app you're embedding drun into.**
User scope registers drun in *every* Claude Code session on the machine, not
just sessions your app manages. (It uses the SSE transport, not `http`+`/mcp`;
`drun-mcp` does serve `/sse` — confirmed with a raw `curl`, it responds with a
real SSE stream — but this codebase has not verified end-to-end that Claude
Code's own SSE client accepts it, so don't assume it's equivalent to the
`http`+`/mcp` path documented below.) For a per-project/per-session setup you
control, write the correct `mcpServers.drun` entry (exact shape in the
transport section, with `type: "http"` against `/mcp`) into the agent
workspace's `.claude/settings.local.json` yourself, independent of whether
`drun-mcp claude init` ran.

- **Scope gotcha (critical for existing apps):** Claude Code also reads
  `~/.claude.json` project entries keyed by the git repo path. If the user has
  opened this repo in Claude Code before, that entry may contain
  `"mcpServers": {}`, which **silently overrides** `settings.local.json`. If the
  drun tools don't appear, this is almost always why. Fix by writing the drun
  entry into that project's record in `~/.claude.json` (project scope), ideally
  at project-creation time.

For a **non–Claude-Code agent**, translate the same HTTP MCP config into whatever
format that framework expects (env var, config file, CLI flag). Transport is
always `http` → `http://127.0.0.1:7273/mcp` with the Accept header.

**Hermes** (a harness for local models) is simpler: `drun-mcp hermes init`
handles both registration and tool restriction in one step, but — unlike
Claude Code — everything it touches (`~/.hermes/config.yaml`) is machine-wide,
not per-project or per-session, since Hermes has no scoping mechanism for
either. That makes it a poor fit for the "embed drun into an existing app"
pattern this guide covers (which wants per-session isolation); it's really
only appropriate for a single-user, single-machine Hermes setup. See the
[Hermes section of the README](../README.md#hermes) if that's your case.

### Step 4 — Per-session setup

For each agent session that should be sandboxed:

1. `create_session` → get a `session_id`.
2. `session_mount` the project path (and any other host paths the agent needs).
3. Hand the `session_id` to the agent (e.g. via system prompt / CLAUDE.md) so it
   uses that session for `session_bash`, `session_write_file`, etc.
4. On session end, `session_close`.

If the app manages this from host code (not from inside an agent turn), it needs
a small MCP HTTP client. drun does not ship official client libraries yet, so you
may have to write a minimal one (initialize handshake → `tools/call`).

### Step 5 — Verify end to end

Do not declare success until you have confirmed the agent actually sees drun:

- In Claude Code, run `/mcp` and confirm `drun` shows as connected (✔), not
  failed (✘).
- Have the agent make one real `session_bash` call (e.g. `echo hello` or `ls`)
  and confirm it returns.
- Confirm native tools are blocked (the agent should be unable to call `Bash`
  directly if you ran `drun-mcp claude init`).

If `drun` shows ✘ failed, check in order: (1) is drun-mcp actually running on
7273, (2) is the transport `http` + `/mcp` (not `sse` + `/sse`), (3) is the
Accept header present, (4) is a stale `~/.claude.json` project entry overriding
your config.

---

## Common failure modes (check these first when debugging)

- **`drun · ✘ failed`, 406 error** → missing `text/event-stream` in Accept header.
- **`drun · ✘ failed`, 404** → using `sse`/`/sse` instead of `http`/`/mcp`.
- **Config looks right but no tools** → `~/.claude.json` project entry with empty
  `mcpServers` overriding `settings.local.json`. Write drun at project scope there.
- **Tools appear but agent still edits host directly** → you registered the MCP
  server but never ran `drun-mcp claude init`, so native tools were never blocked.
- **Agent can't reach the internet** → expected; `session_bash` has no network.
  Use `session_fetch` (and add the domain to the allowlist with
  `drun-mcp config add-domain`).
- **`session_mount` denied** → the path isn't in `mount_allowlist`; add it with
  `drun-mcp config add-path`.

- `drun-mcp claude init` does register `mcpServers` now (via `claude mcp add`),
  but at **user scope with `--transport sse`** — the wrong scope for an
  embedded app regardless of whether the SSE transport itself works (`/sse`
  does respond to a raw `curl` with a real SSE stream, so the "always use
  `http`+`/mcp`" guidance elsewhere in this doc may be stronger than strictly
  necessary — this hasn't been re-verified against Claude Code's own client).
  Don't rely on the user-scope registration for an embedded app either way —
  write the correct config into `settings.local.json` / `~/.claude.json`
  yourself, at project scope, as described in Step 3.
- No `/health` endpoint (must use the MCP handshake to probe readiness).
- Port 7273 is hardcoded (collisions if two apps each start their own drun-mcp).
- No official Go/Python/Node client library for host-side calls.
- No Windows binary.

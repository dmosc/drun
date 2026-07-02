# drun (deterministic run)

<div align="center">

![drun architecture](assets/architecture.png)

</div>

## Git for agents with ephemeral runtime

Drun is a platform that allows you to virtualize components of your host into an
ephemeral runtime to serve as the agent's workspace with git-like primitives
which allow the agent to explore trajectories in parallel and discard dead-ends
without disrupting the host state.

Drun surfaces a runtime abstration layer with reliability harnesses to guardrail
the agent's behavior across a range of OS-level aspects:

- Network domains (e.g. allowlisted domains)
- Command exeuction (e.g. forbidden commands)
- Access to filesystem paths (e.g. restrict filesystem access)
- Resource limits (e.g. memory and duration caps)

Rather than granting your agent raw CRUD access to your host, Drun exposes and
enforces a highly-customizable policy layer with deterministic knobs for you to
place absolute limits that can't be breached by design.

## Usage

### Installing

The following describes installation steps to integrate with Claude Code. There
are plans in the future to support other user journeys such as OpenAI, Ollama
and even a Python SDK as well as more programming languages. Consider this
document as the current source of truth of what's production-ready.

#### Requirements

- [Claude Code](https://code.claude.com/docs/en/quickstart#step-1-install-claude-code).

Open a terminal and go to your project folder.

```bash
cd ~/path/to/project
```

Run the installation script which does a few things:

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

1. Installs the drun MCP binary to `/usr/local/bin/drun-mcp` (skips if already
   installed).
1. Creates a global config at `~/.drun/config.toml` with common defaults (skips
   if one already exists).
1. Starts `drun-mcp` as a persistent background daemon — via `launchd` on macOS
   or a `systemd` user service on Linux — so a single process serves all MCP
   clients across every terminal and editor window on the host.
1. Creates `$PWD/.claude/settings.json` that restricts Claude to drun tools only
   for this workspace — native file (`Read`, `Edit`, `Write`, `NotebookEdit`,
   `Glob`, `Grep`), shell (`Bash`, `BashOutput`, `KillBash`), network
   (`WebFetch`, `WebSearch`), and subagent delegation (`Task`) tools are all
   blocked, and drun's MCP tools are pre-allowed so Claude isn't prompted on
   every call.
1. Creates `$PWD/CLAUDE.md` with instructions that tell Claude to use drun tools
   instead of native ones, including how to bootstrap a session
   (`create_session` then `session_mount`).
1. Registers the MCP in Claude Code pointing at the running daemon over SSE
   (`http://127.0.0.1:7273/sse`) — one registration shared across all projects.

Once installed, two endpoints are available:

| Endpoint                    | Purpose                                   |
| --------------------------- | ----------------------------------------- |
| `http://127.0.0.1:7273/sse` | MCP transport (SSE) — used by Claude Code |
| `http://127.0.0.1:7273/mcp` | MCP transport (streamable HTTP)           |
| `http://127.0.0.1:7274`     | Trajectory viewer web UI                  |

Validate that the MCP is live:

```bash
claude mcp list
```

#### Upgrading

Run the following commands to upgrade drun's MCP to the latest release:

```bash
# MCP binary
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash

# Update to a specific version
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash -s -- v0.1.1
```

#### Uninstalling

Run the following command to uninstall drun from your host:

```
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/uninstall.sh | bash
```

1. Stops the background daemon and removes the `launchd` agent (macOS) or
   `systemd` user service (Linux).
1. Removes the drun MCP binary from `/usr/local/bin/drun-mcp`.
1. Unlinks the MCP reference from Claude Code.
1. Removes `.claude/settings.json` from each project so native Claude tools are
   restored automatically.
1. Leaves `~/.drun/config.toml` and any `CLAUDE.md` files untouched; delete
   these manually if not needed.

### Configuration

The behavior of the drun MCP is orchestrated via `~/.drun/config.toml`, a single
global file shared by the background daemon. It is read once at daemon startup;
without it, built-in defaults apply.

The following is a reference of all the controls available for tuning. All
fields are optional.

| Field                       | Default                                                              | Description                                                                                                                                                                                                   |
| --------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `domain_allowlist`          | `["pypi.org", "files.pythonhosted.org", "cdn.jsdelivr.net"]`         | Additional domains reachable via `session_fetch`. Use `["*"]` to allow all, or `"*.example.com"` for subdomains.                                                                                              |
| `fetch_timeout_ms`          | `60000`                                                              | Timeout for the full `session_fetch` response in milliseconds.                                                                                                                                                |
| `connect_timeout_ms`        | `30000`                                                              | TCP connection timeout for `session_fetch` in milliseconds.                                                                                                                                                   |
| `bash_timeout_ms`           | `30000`                                                              | Maximum wall time for a single `session_bash` call.                                                                                                                                                           |
| `max_workspace_mb`          | `512`                                                                | Maximum workspace size per session in megabytes. Checked before each new checkpoint is appended.                                                                                                              |
| `max_sessions`              | `50`                                                                 | Maximum number of concurrent sessions.                                                                                                                                                                        |
| `max_checkpoints`           | `200`                                                                | Maximum checkpoints stored per session. When the limit is reached, squash or drop old checkpoints.                                                                                                            |
| `session_idle_timeout_secs` | `3600`                                                               | Seconds of inactivity before a session is considered abandoned and rejected.                                                                                                                                  |
| `mount_allowlist`           | `[]`                                                                 | Host path prefixes that `session_mount` may read from. Empty means all paths are permitted. Non-empty restricts mounts to the listed prefixes.                                                                |
| `mount_overlay_paths`       | `["node_modules", ".venv", "venv", "target", "__pycache__", ".git"]` | Directory names that `session_mount` registers as read-only host overlays instead of loading into the workspace. Overlay dirs are symlinked at execution time and never checkpointed. Set to `[]` to disable. |
| `export_root`               | `"drun-export"`                                                      | Directory that `session_export` must write into. Relative paths are resolved from the current working directory.                                                                                              |
| `snapshots_dir`             | `"drun-snapshots"`                                                   | Directory where `session_snapshot` writes `.drun` files.                                                                                                                                                      |
| `snapshot_on_close`         | `false`                                                              | When `true`, automatically write a snapshot when `session_close` is called.                                                                                                                                   |
| `env_allowlist`             | `[]`                                                                 | Host environment variable names exposed to agents via `session_get_env`. Empty means no variables are exposed.                                                                                                |
| `bash_command_denylist`     | `[]`                                                                 | Command substrings always rejected by `session_bash` before execution.                                                                                                                                        |
| `bash_command_allowlist`    | `[]`                                                                 | Command substrings permitted by `session_bash`. Empty means all commands are allowed (subject to the denylist).                                                                                               |
| `web_port`                  | `7274`                                                               | TCP port for the trajectory viewer web UI. Set to `0` or remove the field to disable it.                                                                                                                      |

#### Reloading the MCP

`~/.drun/config.toml` is read once when the daemon starts. To apply changes,
restart the daemon:

**macOS**

```bash
launchctl unload ~/Library/LaunchAgents/com.drun.mcp-server.plist
launchctl load -w ~/Library/LaunchAgents/com.drun.mcp-server.plist
```

**Linux**

```bash
systemctl --user restart drun-mcp.service
```

# drun (deterministic run)

<div align="center">

![drun architecture](assets/architecture.png)

</div>

## Git for agents with ephemeral runtime

Drun is a platform that allows you to virtualize components of your host into an
ephemeral runtime to serve as the agent's workspace with git-like primitives
which allow the agent to explore trajectories in parallel and discard dead-ends
without disrupting the host state.

Drun surfaces a runtime abstraction layer with reliability harnesses to
guardrail the agent's behavior across a range of OS-level aspects:

- Network domains (e.g. allowlisted domains)
- Command execution (e.g. forbidden commands)
- Access to filesystem paths (e.g. restrict filesystem access)
- Resource limits (e.g. memory and duration caps)

Rather than granting your agent raw CRUD access to your host, Drun exposes and
enforces a highly-customizable policy layer with deterministic knobs for you to
place absolute limits that can't be breached by design.

## Usage

drun supports three independent journeys — pick the one that fits:

- **[Using Claude Code](#using-claude-code)** — drun's MCP tools replace
  Claude's native file/shell/network tools inside a sandboxed workspace.
- **[Using drun chat](#using-drun-chat)** — a CLI agent loop against Ollama or
  any LiteLLM-supported model.
- **[Using the Python SDK](#using-the-python-sdk)** — script sandboxed sessions
  directly, no LLM or daemon required.

### Installing

Claude Code and drun chat both talk to the same background `drun-mcp` daemon —
install it once per machine with the steps below, then jump to whichever journey
you need. The Python SDK is standalone and skips this section entirely; go
straight to [Using the Python SDK](#using-the-python-sdk).

#### Global install (once per machine)

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

Installs globally (each step is skipped if already done):

1. The drun MCP binary to `/usr/local/bin/drun-mcp`.
2. A global config at `~/.drun/config.toml` with sensible defaults.
3. `drun-mcp` as a persistent background daemon — via `launchd` on macOS or
   `systemd` on Linux — so a single process serves every Claude Code window,
   terminal, and `drun chat` invocation on the host simultaneously.
4. The MCP registration in Claude Code pointing at the running daemon over SSE
   (`http://127.0.0.1:7273/sse`) — one registration shared across all projects.

Once installed, the following endpoints are available:

| Endpoint                    | Purpose                                               |
| --------------------------- | ----------------------------------------------------- |
| `http://127.0.0.1:7273/sse` | MCP transport (SSE) — used by Claude Code             |
| `http://127.0.0.1:7273/mcp` | MCP transport (streamable HTTP) — used by `drun chat` |
| `http://127.0.0.1:7274`     | Trajectory viewer web UI                              |

#### Upgrading

Run the following commands to upgrade drun's MCP to the latest release:

```bash
# MCP binary
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash

# Update to a specific version
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/update.sh | bash -s -- v0.3.1
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

### Using Claude Code

#### Requirements

- [Claude Code](https://code.claude.com/docs/en/quickstart#step-1-install-claude-code).
- The `drun-mcp` daemon [installed](#global-install-once-per-machine) above.

#### Per-project setup

From the root of any project you want drun to manage:

```bash
drun-mcp init
```

Creates two files in the current directory (appends if they already exist):

1. `.claude/settings.json` — restricts Claude to drun tools only for this
   workspace. Native file (`Read`, `Edit`, `Write`, `NotebookEdit`, `Glob`,
   `Grep`), shell (`Bash`, `BashOutput`, `KillBash`), network (`WebFetch`,
   `WebSearch`), and subagent delegation (`Task`) tools are all blocked, and
   drun's MCP tools are pre-allowed so Claude isn't prompted on every call.
2. `CLAUDE.md` — tells Claude to use drun tools instead of native ones and how
   to bootstrap a session (`create_session` then `session_mount`).

This restriction is intentionally per-project; you wouldn't want native tools
blocked globally across every workspace. Run `drun-mcp init` from any project
root to opt that project into the drun sandbox.

Validate that the MCP is live:

```bash
claude mcp list
```

### Using drun chat

`drun chat` drives an LLM — local via [Ollama](https://ollama.com) or any cloud
model supported by [LiteLLM](https://docs.litellm.ai/docs/providers) — against a
sandboxed session.

#### Requirements

- Python 3.9+.
- The `drun-mcp` daemon [installed](#global-install-once-per-machine) above.
- [Ollama](https://ollama.com) for local models, or an API key for a cloud
  model.

```bash
pip install 'drun-sandbox[chat]'
```

For a local model, install Ollama and pull a tool-calling-capable model:

```bash
ollama pull qwen2.5:14b
```

Then run:

```bash
drun chat "your prompt" --mount ./my-project
```

`--model` defaults to `ollama_chat/qwen2.5:14b`. To use a cloud model instead,
pass `--model` and set the provider's API key:

```bash
ANTHROPIC_API_KEY=... drun chat "your prompt" --model claude-sonnet-4-6
```

Run `drun chat --help` for all flags (mounts, system prompt override, max
iterations).

### Using the Python SDK

For scripting drun sessions directly — no LLM, no daemon required:

#### Requirements

- Python 3.9+.

```bash
pip install drun-sandbox
```

```python
from drun import Session

session = Session()
session.write_file("hello.py", b"print('hi')")
checkpoint = session.execute_bash("python3 hello.py")
print(checkpoint.stdout)
```

See [`examples/quickstart.py`](examples/quickstart.py) for a fuller walkthrough
(bash execution, write, diff, rollback, export).

### Configuration

The behavior of the drun MCP is orchestrated via `~/.drun/config.toml`, a single
global file shared by the background daemon. It's re-read on every tool call;
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
| `web_port`                  | `7274`                                                               | TCP port for the trajectory viewer web UI. Set to `0`, or remove the field from the config file, to disable it.                                                                                               |

#### Updating configuration via the CLI

A couple of utility commands to update the configuration via the `drun-mcp` CLI
are available:

```bash
drun-mcp config add-domain example.com
drun-mcp config add-path /path/to/allow
drun-mcp config remove-domain example.com
drun-mcp config remove-path /path/to/allow
# To validate latest changes to config.
drun-mcp config list
```

Run `drun-mcp config --help` to print a list of available commands.

`~/.drun/config.toml` is re-read on every tool call, so edits — via the CLI
above or by hand — take effect on the very next call, no restart, no dropped
sessions. `drun-mcp init` also allowlists the current project directory for
`session_mount` automatically.

The two exceptions are `web_port` and `session_idle_timeout_secs`: both are only
applied at daemon startup, so changing either still requires a restart:

**macOS**

```bash
launchctl unload ~/Library/LaunchAgents/com.drun.mcp-server.plist
launchctl load -w ~/Library/LaunchAgents/com.drun.mcp-server.plist
```

**Linux**

```bash
systemctl --user restart drun-mcp.service
```

#### Verifying the daemon is healthy

A dead or crash-looping daemon can look identical to an idle one from the
outside. See
[docs/troubleshooting.md's Health check section](docs/troubleshooting.md#health-check--is-drun-actually-running)
for commands to confirm it's running exactly once, actually listening, and not
stuck being killed and retried by launchd/systemd.

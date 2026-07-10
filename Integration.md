# drun Integration Guide

This document covers how to embed drun into an existing AI agent application so that your
agents get safe, sandboxed code execution out of the box — no user setup required.

---

## Integrating drun with an AI Agent Application

The core integration is straightforward: your app bundles the `drun-mcp` binary, starts it
as a subprocess when the agent runtime starts, and configures the agent's MCP client to
connect to it. Everything else (session isolation, filesystem sandboxing, snapshots) is
handled by drun.

### What you are integrating

`drun-mcp` is a standalone binary that speaks the
[MCP Streamable HTTP](https://spec.modelcontextprotocol.io/specification/2025-03-26/basic/transports/#streamable-http)
protocol. It listens on `http://127.0.0.1:7273` and exposes sandbox tools (`session_bash`,
`session_read_file`, `session_write_file`, etc.) to any MCP-compatible agent.

Your app needs to:
1. Ship the binary alongside your own
2. Start it on daemon/server boot
3. Tell the agent's MCP client where to find it

### Getting the binary

Pre-built binaries are published with every drun release on GitHub:

| Platform         | Asset name                |
|------------------|---------------------------|
| macOS (Apple Silicon) | `drun-mcp-macos-arm64` |
| Linux x86-64     | `drun-mcp-linux-x86_64`   |

Latest release: `https://github.com/dmosc/drun/releases/latest`

You can pin a specific version with the `DRUN_VERSION` environment variable or fetch the
latest tag at build time via the GitHub API.

---

### Build-time embedding

#### Node.js / Electron (recommended for desktop apps)

Fetch the binary in a `prebuild` / `premake` script and embed it into your app bundle.
This keeps the build reproducible without requiring users to install anything.

```js
// scripts/embed-drun.mjs
import { createWriteStream, mkdirSync, chmodSync } from "node:fs";
import { pipeline } from "node:stream/promises";

const REPO   = "dmosc/drun";
const VERSION = process.env.DRUN_VERSION ?? "latest";

function assetName() {
  const { platform, arch } = process;
  if (platform === "darwin" && arch === "arm64") return "drun-mcp-macos-arm64";
  if (platform === "linux"  && arch === "x64")   return "drun-mcp-linux-x86_64";
  return null; // unsupported — build without drun
}

async function download(dest) {
  const asset = assetName();
  if (!asset) { console.warn("drun: unsupported platform, skipping"); return false; }

  let tag = VERSION;
  if (tag === "latest") {
    const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`,
                          { headers: { "User-Agent": "my-app-build" } });
    tag = (await r.json()).tag_name;
  }

  const url = `https://github.com/${REPO}/releases/download/${tag}/${asset}`;
  console.log(`Downloading drun-mcp ${tag}…`);
  const res = await fetch(url, { headers: { "User-Agent": "my-app-build" } });
  if (!res.ok) { console.warn(`Download failed (${res.status})`); return false; }

  mkdirSync(new URL("../resources/", import.meta.url).pathname, { recursive: true });
  await pipeline(res.body, createWriteStream(dest));
  chmodSync(dest, 0o755);
  return true;
}

await download("resources/drun-mcp");
```

Wire it into your `package.json`:

```json
{
  "scripts": {
    "prebuild": "node scripts/embed-drun.mjs",
    "premake":  "node scripts/embed-drun.mjs"
  }
}
```

#### Go (binary embedding with build tags)

Use `//go:embed` behind a build tag so the binary is baked into your server binary and
extracted at runtime:

```go
// internal/drun/embed_bundled.go
//go:build bundled_drun

package drun

import _ "embed"

//go:embed binaries/drun-mcp
var embeddedDrunMCP []byte
```

```go
// internal/drun/embed_stub.go
//go:build !bundled_drun

package drun

var embeddedDrunMCP []byte // empty — resolved from PATH at runtime
```

Your build script downloads the binary to `internal/drun/binaries/drun-mcp` before calling
`go build -tags bundled_drun`.

#### Python / pip

Download the binary in a `setup.py` or `pyproject.toml` post-install hook, or ship it as
a platform wheel via a separate `myapp-drun` package that contains only the binary for the
target platform. At runtime, locate it with `importlib.resources`.

#### Docker

```dockerfile
ARG DRUN_VERSION=v0.3.4
RUN ASSET=drun-mcp-linux-x86_64 && \
    curl -fsSL "https://github.com/dmosc/drun/releases/download/${DRUN_VERSION}/${ASSET}" \
         -o /usr/local/bin/drun-mcp && \
    chmod +x /usr/local/bin/drun-mcp
```

#### Homebrew / system package managers

If your app is distributed through Homebrew or a system package manager, declare
`drun-mcp` as a runtime dependency (once drun ships a formula/package) or vendor the
binary inside your formula's `resource` block.

---

### Runtime: starting and managing drun-mcp

#### Lifecycle

Start drun-mcp once when your agent runtime/daemon starts. It is a long-lived process —
one instance serves all concurrent agent sessions via isolated drun sessions.

```bash
# Minimal start
drun-mcp &

# With custom snapshot directory
DRUN_SNAPSHOTS_DIR=/var/myapp/snapshots drun-mcp &
```

**Readiness probe**: drun-mcp does not expose a `/healthz` endpoint. Probe readiness by
performing the MCP `initialize` handshake (see transport section below) with a short
timeout. Retry every 200 ms up to ~10 s.

**Idempotency**: Before starting, probe whether drun-mcp is already running on port 7273.
If it responds, reuse it. This prevents double-start on daemon restart without a clean
shutdown.

**Extraction (embedded binary)**: If you embedded the binary, extract it to a writable
location on first run (e.g. `~/.myapp/bin/drun-mcp`). Use a size or hash check so you
only re-extract when the embedded version differs.

---

### MCP transport: critical details

drun-mcp implements **MCP Streamable HTTP** — not the older SSE transport. Common
integration mistakes:

| Mistake | Symptom | Fix |
|---------|---------|-----|
| `GET /sse` (SSE transport) | 404 | Use `POST /mcp` |
| `Accept: application/json` only | **406 Not Acceptable** | Set `Accept: application/json, text/event-stream` |
| Wrong scope for MCP registration | Agent doesn't see drun tools | See registration section below |

**Correct request headers:**
```
Content-Type: application/json
Accept: application/json, text/event-stream
```

**Endpoint:** `http://127.0.0.1:7273/mcp` (HTTP POST for all requests)

**MCP client config for Claude Code:**
```json
{
  "mcpServers": {
    "drun": {
      "type": "http",
      "url": "http://127.0.0.1:7273/mcp",
      "headers": {
        "Accept": "application/json, text/event-stream"
      }
    }
  }
}
```

---

### Registering drun with the agent's MCP client

How you register drun depends on whether you control the agent's config or inject it at
session start.

#### Claude Code (settings.local.json)

Write the MCP config to the agent worktree's `.claude/settings.local.json` before
starting the agent. This is the right place for per-session, per-worktree configuration:

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

**Important gotcha:** Claude Code also reads `~/.claude.json` project-scoped entries, keyed
by the main git repository path. If the user has previously opened the project in Claude
Code, a `"mcpServers": {}` entry may already exist for that project path, which overrides
`settings.local.json`. To prevent this, also write the drun entry into the project-level
record in `~/.claude.json` for the user's repo — ideally at project creation time, not
per-worktree, so it is scoped correctly to that repo and not applied globally.

#### Other agent frameworks

For agents that accept MCP config via environment variable, config file, or CLI flag,
pass the drun HTTP server config in whatever format they expect. The transport type
is always `http` (Streamable HTTP) pointing to `http://127.0.0.1:7273/mcp`.

---

### Per-session setup

For each agent session that should use drun:

1. **Create a drun session** via `create_session` — returns a `session_id`
2. **Mount the project** via `session_mount` — gives the sandbox read/write access to
   the host path the agent is working on
3. Pass the `session_id` to the agent (e.g. via system prompt) so it can use
   `session_bash`, `session_write_file`, etc. on the correct session

On session end, call `session_close` to free resources.

---

### Example: Agent Orchestrator (AO) — migrating an existing product

[Agent Orchestrator](https://github.com/AgentWrapper/agent-orchestrator) (AO) is a
production Electron desktop app that manages multiple Claude Code agent sessions as git
worktrees. The integration described here was done on a fork
([Domene99/agent-orchestrator](https://github.com/Domene99/agent-orchestrator)) to
demonstrate what adding drun to an **already-shipped product** looks like in practice —
as opposed to designing for it from the start.

AO already had its own daemon (`ao`), its own session/worktree lifecycle, and its own
mechanism for writing Claude Code hook config (`settings.local.json`) to each agent
workspace. drun was added on top of all of that without changing the existing
architecture, which makes it a realistic template for any team that wants to bolt on
sandboxed execution to a product that is already in users' hands.

The key constraint: **zero new user steps**. Users should get drun out of the box when
they install or update the app — no separate install, no CLI command, no config file.

#### Build: adding drun to an existing `npm run make` pipeline

AO already had a `build-daemon.mjs` script that compiled the Go `ao` binary as a
`premake` step. drun was added by extending that script — the existing pipeline did not
change, drun was layered in before the `go build` call:

1. Calls the GitHub API to resolve `latest` to a concrete tag
2. Downloads the platform-appropriate binary (`drun-mcp-macos-arm64` on Apple Silicon,
   `drun-mcp-linux-x86_64` on Linux)
3. Writes it to `backend/internal/drun/binaries/drun-mcp`
4. Passes `-tags bundled_drun` to `go build`, which activates the `//go:embed` directive
   that bakes the drun-mcp binary into the `ao` daemon binary

If the download fails or the platform is unsupported, the script falls back to building
without the tag and `ao` resolves `drun-mcp` from PATH at runtime. The rest of the build
— Vite bundles, Electron packaging, zip output — was untouched.

This illustrates the migration pattern: **the existing build system does not need to
change**. You find the step that produces your server/daemon binary and prepend a binary
download + embed step.

#### Runtime: extraction and startup alongside an existing daemon

AO's daemon (`ao`) was already managing its own lifecycle — starting, stopping, and
health-checking its own HTTP server. drun-mcp was added as a second subprocess managed
by the same daemon. A new `drun.Server` type was introduced in
`backend/internal/drun/server.go`; it is started once during `ao`'s boot sequence and
torn down on shutdown. The rest of `ao`'s startup logic was untouched.

`server.go` responsibilities:
- Probes port 7273 first — if drun-mcp is already running (e.g. from a previous daemon
  instance), reuses it rather than starting a second one
- Extracts the embedded binary to `~/.ao/bin/drun-mcp` on first run; skips extraction if
  the file size matches the embedded bytes (fast idempotency check)
- Starts `drun-mcp` as a subprocess with `DRUN_SNAPSHOTS_DIR` pointed inside `ao`'s data
  directory so drun state lives alongside the rest of AO's data
- Waits up to 10 s for readiness via the MCP `initialize` handshake
- After confirming readiness, writes the drun MCP entry to `~/.claude.json` at the
  project scope for the user's current project

The migration effort for this layer was roughly an afternoon: write the `Server` type,
add two lines to the daemon boot sequence, done.

#### Per-session config: extending an existing hooks mechanism

AO already wrote a `.claude/settings.local.json` to each agent worktree before starting
Claude Code — it used this to inject lifecycle hooks (session-start, stop, notification,
etc.). Adding drun MCP was a one-function addition to that existing write path in
`backend/internal/adapters/agent/claudecode/hooks.go`:

```go
const drunMCPURL = "http://127.0.0.1:7273/mcp"

mcpServers["drun"] = map[string]any{
    "type": "http",
    "url":  drunMCPURL,
    "headers": map[string]string{
        "Accept": "application/json, text/event-stream",
    },
}
```

The function is idempotent: it compares `type` and `url` before writing, and upgrades
stale entries left by older app versions (e.g. `"type": "sse"` from an earlier prototype
that used the wrong transport — see the transport gotchas section above). This matters
for a migration scenario: users who already had the app installed have existing
`settings.local.json` files that need to be corrected on the next run, not just on first
install.

#### Daemon-level drun client: calling drun outside of agent turns

`backend/internal/drun/client.go` is a minimal Go MCP client the daemon itself uses to
call drun tools directly (create sessions, mount paths, take snapshots) without going
through Claude Code. This is useful for automation that happens outside of an agent turn —
for example, AO creates and mounts the drun session for a new worktree before the agent
even starts, so the agent's first tool call lands in an already-prepared sandbox.

This client had to be written from scratch because drun does not ship an official Go
library (see proposal #5 below).

---

## Proposals for drun

The following are gaps identified during the AO integration that, if addressed in drun,
would make "plug and play" integration significantly easier.

### 1. Fix `install.sh` transport: SSE → HTTP

`install.sh` registers drun with:
```bash
claude mcp add --scope user --transport sse drun "$MCP_URL"
```
But drun-mcp does not serve the SSE endpoint (`GET /sse`). It only serves
`POST /mcp` (Streamable HTTP). Anyone who runs the installer and then opens Claude Code
sees `drun · ✘ failed` immediately. The correct registration is:
```bash
claude mcp add --scope user --transport http \
  --header "Accept: application/json, text/event-stream" \
  drun http://127.0.0.1:7273/mcp
```

### 2. Per-project `drun init` command

Right now drun only offers `--scope user` (global) registration. For orchestrators that
manage per-project agent sessions, global registration creates noise in every Claude Code
session, not just the ones drun is configured for.

A `drun init` (or `drun mcp register --scope project`) command that registers drun in
the current project's Claude Code settings would let orchestrators call it once at project
creation time:
```bash
# run from inside the user's project directory
drun init
# writes drun to ~/.claude.json under the project key for this repo
# or writes to .claude/settings.json in the project root
```

This maps cleanly to the orchestrator workflow: when a user creates a new project and
points it at their repo, the orchestrator calls `drun init` in that directory.

### 3. `/health` or `/readyz` endpoint

Probing readiness currently requires a full MCP `initialize` handshake, which creates a
session, requires `notifications/initialized`, and allocates state. A simple HTTP health
endpoint would let orchestrators probe with a cheap HEAD or GET request:
```
GET /health → 200 OK
```

### 4. Configurable port

Port 7273 is hardcoded. If two apps on the same machine both bundle drun-mcp (or if a
user runs a standalone drun alongside an orchestrator), they collide. An env var
(`DRUN_PORT`) or a flag would let each app run its own drun-mcp instance without
conflict.

### 5. Official client libraries

Integrators currently have to write their own MCP HTTP client to call drun tools from
host code. An official thin client for Go, Python, and Node.js would eliminate this
boilerplate and ensure compatibility across drun versions:
```go
// example: official Go client
c := drun.NewClient("http://127.0.0.1:7273/mcp")
sid, _ := c.CreateSession(ctx)
c.Mount(ctx, sid, "/path/to/project")
```

### 6. Windows support

No pre-built Windows binary is published. For Electron apps targeting Windows this means
drun is silently disabled on that platform. A `drun-mcp-windows-x86_64.exe` release
asset would close this gap.

### 7. Checksum / signature verification for release binaries

Integrators downloading drun-mcp at build time have no way to verify the binary's
integrity beyond trusting the TLS connection to GitHub. Publishing SHA-256 checksums (or
a cosign signature) alongside each release asset would let build scripts verify before
embedding.

### 8. Version compatibility signal

There is no mechanism for the daemon to know whether a bundled drun-mcp binary is
compatible with the current drun session format. A version field in the `/health`
response, or a minimum-protocol-version check during `initialize`, would let
orchestrators warn users when they need to rebuild.

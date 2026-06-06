# drun

**An ephemeral, fully-sealed execution runtime for AI agents.**

drun gives agents a safe place to think with code. Every execution runs in a
fully isolated, stateful sandbox — no host filesystem access, no side effects,
no blast radius. Agents can explore, mutate state, generate files, roll back
mistakes, and iterate freely without touching the machine they run on.

---

## Why drun

Agentic systems that execute code face a hard tradeoff: either sandbox
everything and lose developer ergonomics, or give agents full access and accept
the risk. drun rejects the tradeoff.

- **Ephemeral by default.** Nothing persists to the host machine unless
  explicitly exported. Agents can write files, install packages, and mutate
  state — all contained inside the session.
- **Checkpointing and rollback.** Every execution step is a checkpoint. Agents
  can explore a branch of execution, decide it was wrong, and roll back to any
  prior state — like `git` for runtime.
- **Frozen dependency surface.** The runtime bundles its own execution
  environment. Agents can't accidentally pull in a compromised or outdated
  package at the system level — the core sandbox is immutable and auditable.
- **Persistent sessions.** State accumulates across steps within a session.
  Packages installed in step one are available in step ten. No re-importing, no
  cold starts between tool calls.
- **Native artifact output.** Files generated inside the sandbox — images,
  reports, datasets — are returned in the right format. Images render inline.
  Text and binary are handled automatically.
- **Two consumption models.** Use it as an MCP server (Claude Code, any
  MCP-compatible client) or embed it as a library (`pip install drun-sandbox`).

---

## Language support

drun currently supports **Python**, executed via [Pyodide](https://pyodide.org)
— a WebAssembly port of CPython. Support for additional languages is on the
roadmap.

---

## Installation

### As an MCP server (recommended for Claude Code)

The one-liner installs Deno if needed, downloads the right binary for your
platform, and registers drun with Claude Code:

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

**Supported platforms:** macOS (Apple Silicon, Intel), Linux (x86_64, arm64)

**Dependencies:** [Deno](https://deno.land) — installed automatically if not
present.

To install manually:

```bash
# macOS Apple Silicon
curl -L https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-arm64 -o drun-mcp
chmod +x drun-mcp
claude mcp add drun -- /path/to/drun-mcp

# macOS Intel
curl -L https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-x86_64 -o drun-mcp

# Linux x86_64
curl -L https://github.com/dmosc/drun/releases/latest/download/drun-mcp-linux-x86_64 -o drun-mcp
```

Or via Cargo if you have Rust installed:

```bash
cargo install drun-mcp
claude mcp add drun -- $(which drun-mcp)
```

### As a library

```bash
pip install drun-sandbox
```

Requires Python ≥ 3.9. Deno must be installed separately:

```bash
curl -fsSL https://deno.land/install.sh | sh
```

---

## Usage

### MCP tools

Once registered, drun exposes five tools to any MCP-compatible client:

| Tool                      | Description                                                              |
| ------------------------- | ------------------------------------------------------------------------ |
| `create_session`          | Start a new sandbox session. Returns a `session_id`.                     |
| `session_execute`         | Run code in the session. Returns stdout and a `checkpoint_id`.           |
| `session_install_package` | Install a package into the session. Available in all subsequent steps.   |
| `session_read_file`       | Read a file from the session. Text, JSON, images — all handled natively. |
| `session_rollback`        | Roll back to a prior checkpoint, discarding everything after it.         |

A typical agent flow:

```
create_session
session_install_package(numpy)
session_execute(data analysis)
session_execute(generate chart)
session_read_file(chart.png)
session_rollback (if something went wrong)
```

### Library

```python
from drun import Session

s = Session()

# Execute code — nothing writes to your disk
result = s.execute("x = 42\nprint(x)")
print(result.stdout)  # "42"

# Packages survive across steps
s.execute("import micropip\nawait micropip.install('faker')")
result = s.execute("from faker import Faker\nprint(Faker().name())")

# Checkpoint and rollback
checkpoint_a = s.execute("data = [1, 2, 3]")
s.execute("data.append(4)")
s.rollback(checkpoint_a.id)  # data is [1, 2, 3] again

# Explicitly export a file to disk when you want it
s.execute("open('/workspace/output.csv', 'w').write('a,b\\n1,2')")
s.export("output.csv", dest="~/Downloads/output.csv")
```

### File isolation

All files written during execution live in `/workspace` inside the sandbox. They
are snapshotted at each checkpoint and never written to your host filesystem
automatically.

```python
# Writes to the sandbox, not your machine
s.execute("open('/workspace/report.txt', 'w').write('done')")

# Export explicitly when you want it locally
s.export("report.txt")               # writes to ./report.txt
s.export_all(dest_dir="./outputs")   # exports everything in /workspace
```

---

## Claude Code integration

After running the installer, drun is immediately available in Claude Code. To
make Claude always route code execution through drun — rather than running code
directly on your machine — add this to your `~/.claude/CLAUDE.md`:

```markdown
## Code execution

Always use drun MCP tools for code execution:

- Use `create_session` at the start of any coding task
- Use `session_install_package` before importing third-party packages
- Use `session_execute` to run code
- Use `session_read_file` to inspect output files and images
- Never run code directly on the host machine
```

This instruction is picked up by Claude Code at the start of every conversation.

---

## How it works

drun runs code via [Pyodide](https://pyodide.org) — a WebAssembly port of
CPython — inside a [Deno](https://deno.land) subprocess. The Deno process stays
alive for the lifetime of a session, communicating with the Rust host. Pyodide's
in-memory filesystem provides an ephemeral directory; drun snapshots it at each
step to power checkpointing and rollback.

The isolation is structural, not policy-based. Because execution happens inside
WebAssembly, the sandbox cannot make arbitrary system calls, access the host
filesystem, or spawn processes. There is no escape hatch to configure wrong —
the boundary is the architecture.

This also means the dependency surface is frozen at the Deno + Pyodide layer. A
compromised or malicious package installed inside a session cannot affect the
host runtime, and the core execution environment itself is version-pinned and
auditable.

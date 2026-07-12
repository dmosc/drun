# drun Examples

Runnable examples and configuration recipes for drun. Each recipe is a
self-contained `.toml` file you can point `DRUN_CONFIG` at; each Python script
is a working example you can run immediately.

`session_bash` (the only execution tool) runs in a sandbox with **no network
access and no package-install mechanism**, on either platform. Examples that
need external data fetch it on the host (via `urllib`, before the session is
created, or via the MCP-only `session_fetch` tool) and push it into the session
with `write_file`/`session_write_file` or `--mount`.

---

## What's here

| File                                                   | Kind             | What it shows                                                                               |
| ------------------------------------------------------ | ---------------- | ------------------------------------------------------------------------------------------- |
| [`quickstart.py`](quickstart.py)                       | Python SDK       | Core API without an LLM: bash, write, rollback, diff, export                                |
| [`financial_analysis.py`](financial_analysis.py)       | Python SDK + LLM | Host fetches SEC EDGAR revenue data for Apple; agent formats a Markdown report              |
| [`data_science.py`](data_science.py)                   | Python SDK + LLM | Host fetches the Iris dataset; agent implements and compares three classifiers from scratch |
| [`fibonacci_slow.py`](fibonacci_slow.py)               | Baseline         | Naive O(2ⁿ) Fibonacci — the baseline for the benchmark example                              |
| [`fibonacci_benchmark.py`](fibonacci_benchmark.py)     | Python SDK + LLM | Stress-tests every sandbox constraint, then benchmarks four Fibonacci implementations       |
| [`financial_analysis.toml`](financial_analysis.toml)   | Config recipe    | SEC/Yahoo Finance fetch domains, generous timeout, snapshots enabled                        |
| [`data_science.toml`](data_science.toml)               | Config recipe    | Dataset fetch domains, larger workspace and timeout                                         |
| [`heavy_workloads.toml`](heavy_workloads.toml)         | Config recipe    | 4 GB workspace, 10-minute timeouts, auto-snapshot on close                                  |
| [`fibonacci_benchmark.toml`](fibonacci_benchmark.toml) | Config recipe    | Tight bash denylist, short timeout — used by the stress test                                |

---

## Prerequisites

### Python SDK path

```bash
pip install 'drun-sandbox[chat]'
```

The `[chat]` extra is only required for examples that drive an LLM
(`financial_analysis.py`, `data_science.py`, `fibonacci_benchmark.py`).
`quickstart.py` works with the base package alone.

### MCP path

```bash
# macOS (Apple Silicon)
curl -fsSL https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-arm64 \
  -o /usr/local/bin/drun-mcp && chmod +x /usr/local/bin/drun-mcp

# Linux x86-64
curl -fsSL https://github.com/dmosc/drun/releases/latest/download/drun-mcp-linux-x86_64 \
  -o /usr/local/bin/drun-mcp && chmod +x /usr/local/bin/drun-mcp

# Or use the one-liner (detects platform automatically):
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

---

## Path 1 — Python SDK

### Step 1: install

```bash
pip install 'drun-sandbox[chat]'
```

### Step 2: run the quickstart (no config, no API key)

```bash
python examples/quickstart.py
```

This exercises the raw SDK: create a session, run a bash command, write a file,
diff two checkpoints, roll back to a prior state, run another bash command, and
export to the host. No LLM or API key required.

Expected output (checkpoint IDs will vary):

```
[1] bash       — hello from the sandbox
[2] write      — checkpoint 1 -> version A
[3] change     — checkpoint 3 -> version B
[4] diff       — unified diff of the two file states
[5] rollback   — version A
[6] bash       — directory listing
[7] export     — list of exported paths
```

### Step 3: run a config-backed LLM example

Choose a recipe TOML and an API key:

```bash
# Anthropic
export ANTHROPIC_API_KEY=sk-ant-...

DRUN_CONFIG=examples/financial_analysis.toml \
    python examples/financial_analysis.py
```

The script fetches SEC data on the host, pushes it into a drun session, drives
the LLM to parse it and produce a Markdown report, then exports the result to
`/tmp/drun-financial/`.

```bash
# OpenAI
DRUN_CONFIG=examples/financial_analysis.toml \
    MODEL=gpt-4o python examples/financial_analysis.py

# Google Gemini
DRUN_CONFIG=examples/financial_analysis.toml \
    MODEL=gemini/gemini-2.0-flash python examples/financial_analysis.py
```

### Step 4: run with a local model (no API key)

Install Ollama from [ollama.com](https://ollama.com), pull a model, then pass
`MODEL` and `BASE_URL`:

```bash
ollama pull qwen2.5:14b

DRUN_CONFIG=examples/financial_analysis.toml \
    MODEL=openai/qwen2.5:14b \
    BASE_URL=http://localhost:11434/v1 \
    python examples/financial_analysis.py
```

Use the `openai/<model>` prefix with Ollama's `/v1` endpoint — it threads tool
call IDs more reliably than the `ollama/<model>` prefix.

Recommended local models: `qwen2.5:14b`, `qwen2.5:7b`. Avoid reasoning variants
(`deepseek-r1`, `qwen3.*`) — they emit tool calls as plain text rather than
structured JSON.

### Available Python examples

| Script                   | Config needed              | API key         | What to expect                                                         |
| ------------------------ | -------------------------- | --------------- | ---------------------------------------------------------------------- |
| `quickstart.py`          | No                         | No              | Prints 7 labeled steps demonstrating the core SDK API                  |
| `financial_analysis.py`  | `financial_analysis.toml`  | Yes (or Ollama) | Exports `/tmp/drun-financial/results.md` with Apple revenue table      |
| `data_science.py`        | `data_science.toml`        | Yes (or Ollama) | Exports `/tmp/drun-data-science/results.md` with classifier comparison |
| `fibonacci_benchmark.py` | `fibonacci_benchmark.toml` | Yes (or Ollama) | Runs 4 constraint probes then exports a benchmark table                |

---

## Path 2 — MCP + Claude Code

The MCP binary exposes the full drun tool suite to Claude Code:
`create_session`, `session_bash`, `session_fork`, `session_rollback`,
`session_merge`, `session_fetch`, and more. Claude drives the sandbox directly
from within your editor.

### Step 1: install and register

```bash
# One-liner (recommended)
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash

# The install script runs: claude mcp add drun -- /usr/local/bin/drun-mcp
# You can verify registration with:
claude mcp list
```

### Step 2: point DRUN_CONFIG at a recipe

**VSCode extension** — VSCode does not source `.zshrc`, so shell exports do not
reach the MCP server. Set the env var inside Claude Code's settings:

```json
// ~/.claude/settings.json  (global — applies to every project)
{
  "env": {
    "DRUN_CONFIG": "/absolute/path/to/examples/financial_analysis.toml"
  }
}
```

```json
// .claude/settings.json  (project-level — overrides global for this repo)
{
  "env": {
    "DRUN_CONFIG": "/absolute/path/to/examples/financial_analysis.toml"
  }
}
```

**Reload:** `Cmd+Shift+P` → **Developer: Reload Window**.

**Terminal** — when Claude Code is launched from a terminal, it inherits the
shell environment:

```bash
export DRUN_CONFIG=/absolute/path/to/examples/financial_analysis.toml
claude
```

**Reload:** exit and relaunch `claude`. Opening a new chat window within the
same Claude Code process does not restart the MCP server.

### Step 3: verify the connection

Open a Claude chat and run:

```
Use drun to list active sessions.
```

Claude should call `session_list` and return an empty list. If the tool is
missing, check the Output panel (VSCode) under **Claude Code MCP** for errors.

### Step 4: run an example via a prompt

Give Claude an equivalent natural-language instruction to any `.py` example's
`PROMPT` constant. Unlike the Python SDK's chat agent, Claude over MCP has
`session_fetch` available, so it can fetch allowlisted URLs itself —
`session_bash` still has no network access either way.

**Financial analysis:**

```
Use drun to analyze Apple's revenue trends. Use session_fetch to pull
https://data.sec.gov/api/xbrl/companyconcept/CIK0000320193/us-gaap/Revenues.json,
then use session_bash (stdlib only — no pandas) to filter 10-K filings, build
a year-over-year revenue table, and write results.md.
```

**Data science:**

```
Use drun to benchmark classifiers on the Iris dataset. Use session_fetch to
pull https://raw.githubusercontent.com/mwaskom/seaborn-data/master/iris.csv,
then implement a decision tree, a small random forest, and a KNN classifier
from scratch in session_bash (stdlib only — no scikit-learn), and export a
Markdown report comparing accuracy.
```

**Sandbox stress test:**

```
Use drun to run the Fibonacci constraint probes: trigger a timeout with
time.sleep(300) in session_bash, confirm session_bash has no network access
at all (even to allowlisted domains) via urllib, try a bash denylist
rejection with curl, run a correctness gate, then implement and benchmark
four Fibonacci algorithms.
```

### What Claude sees (MCP tool suite)

| Category   | Tools                                                                                                                             |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Lifecycle  | `create_session`, `session_list`, `session_close`, `session_tree`                                                                 |
| Execution  | `session_bash`, `session_get_env`                                                                                                 |
| Navigation | `session_rollback`, `session_fork`, `session_merge`, `session_history`, `get_session_state`                                       |
| Files      | `session_read_file`, `session_write_file`, `session_delete_file`, `session_mount`, `session_diff`                                 |
| Host I/O   | `session_export`, `session_commit`, `session_fetch`, `get_fetch_allowlist`                                                        |
| Snapshots  | `session_snapshot`, `session_restore`, `list_snapshots`                                                                           |
| Labels     | `session_label`, `session_checkpoint_label`, `session_checkpoint_squash`, `session_checkpoint_drop`, `checkpoint_read_stdstreams` |

There is no `session_execute_python`, no package-install tool, and no
`session_cancel` — only `session_bash` for execution. `session_rollback` is
destructive past the rollback point once you continue the session; use
`session_fork` first if you need to keep the abandoned branch.

---

## Path 3 — MCP + Ollama (no API key)

`drun chat` drives a local Ollama model against a **running `drun-mcp` daemon**
over MCP — the same daemon and full tool suite Claude Code uses (`session_bash`,
`session_mount`, `session_fetch`, `session_fork`, and more), just with a local
model instead of a cloud provider.

### Step 1: install and start the daemon

```bash
curl -fsSL https://raw.githubusercontent.com/dmosc/drun/main/install.sh | bash
```

### Step 2: install Ollama and pull a model

```bash
# Install from https://ollama.com, then:
ollama pull qwen2.5:14b
```

### Step 3: install the drun Python package and chat

```bash
pip install 'drun-sandbox[chat]'

drun chat \
    --mount ./data \
    "Load the CSV files in data/, summarize each column using only the
     standard library, and write a Markdown report"
```

Use the `ollama_chat/` model prefix (the default), not `ollama/` — only
`ollama_chat/` forwards tool calls to Ollama's native `/api/chat` endpoint;
`ollama/` silently fails to call any tools.

`--mount` paths are mounted into a fresh session before the model sees the
prompt. The model can also call `session_fetch` directly for network data,
subject to the daemon's `domain_allowlist`.

**`drun chat` flags:**

| Flag                 | Default                     | Description                                     |
| -------------------- | --------------------------- | ----------------------------------------------- |
| `--mcp-url URL`      | `http://127.0.0.1:7273/mcp` | drun-mcp daemon endpoint                        |
| `--model MODEL`      | `ollama_chat/qwen2.5:14b`   | litellm model identifier                        |
| `--base-url URL`     | —                           | LLM API base URL override                       |
| `--mount PATH`       | —                           | Mount a host path into the session (repeatable) |
| `--system PROMPT`    | built-in                    | Override the system prompt                      |
| `--max-iterations N` | `30`                        | Maximum agent loop iterations                   |

---

## Configuration recipes

Each TOML file is designed around a specific class of workload. The fields shown
are only the ones that differ meaningfully from the defaults — unset fields fall
back to the built-in values documented in the main README.

> **`session_bash` has no network access, ever.** `domain_allowlist` only
> governs `session_fetch` (an MCP-only, host-side tool). There is no package
> install mechanism in either path — sandbox code must use the standard library,
> or rely on data/packages mounted in by the host.

### `financial_analysis.toml` — live market and filing data

**What it allows:**

- `session_fetch` to `data.sec.gov`, `efts.sec.gov`, `www.sec.gov` (SEC EDGAR
  filings and XBRL data), and `query1/2.finance.yahoo.com` (Yahoo Finance)

**What it prevents:**

- `session_fetch` to any other domain
- `curl`, `wget`, `nc` via `session_bash` (denylisted; bash has no network
  regardless)
- `bash_timeout_ms`: 3 minutes (for large XBRL payloads)

**Notable settings:**

- `snapshot_on_close = true` — financial analysis sessions are worth keeping
- `session_idle_timeout_secs = 7200` — long sessions for multi-step analysis

### `data_science.toml` — dataset fetching and analysis

**What it allows:**

- `session_fetch` to `raw.githubusercontent.com`, `archive.ics.uci.edu`

**What it prevents:**

- `session_fetch` to any other domain
- `rm -rf`, `curl`, `wget` via `session_bash`

**Notable settings:**

- `max_workspace_mb = 1024` — datasets and generated reports can be large
- `bash_timeout_ms = 60_000` — 1 minute per command for stdlib model training

### `heavy_workloads.toml` — long-running compute

**What it allows:**

- Long `session_bash` calls (no domain access either way — this recipe sets no
  `domain_allowlist`, which only matters if you also use `session_fetch`)

**What it prevents:**

- More than 2 concurrent sessions (resource contention)
- More than 25 checkpoints per session (memory usage)

**Notable settings:**

- `max_workspace_mb = 4096` — 4 GB workspace
- `bash_timeout_ms = 600_000` — 10 minutes per `session_bash` call
- `snapshot_on_close = true` — long sessions can be resumed from a snapshot
- `snapshots_dir = "/tmp/drun-snapshots"`

### `fibonacci_benchmark.toml` — tight sandbox for constraint testing

**What it allows:**

- Nothing beyond the workspace and mounted `examples/` directory — this recipe
  sets `domain_allowlist = []` since the benchmark needs no `session_fetch`
  access at all

**What it prevents:**

- `curl`, `wget`, `nc` via `session_bash` (denylist fires before the sandbox
  runs; bash has no network regardless)
- Executions longer than 30 seconds (tight timeout for the stress test)

**Notable settings:**

- `bash_timeout_ms = 30_000` — exposes the timeout probe in Phase 1
- `mount_allowlist` — set to the local `examples/` directory so the agent can
  read `fibonacci_slow.py`
- `snapshot_on_close = false` — all session state is fully ephemeral

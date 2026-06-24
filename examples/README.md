# drun Examples

Runnable examples and configuration recipes for drun. Each recipe is a
self-contained `.toml` file you can point `DRUN_CONFIG` at; each Python script
is a working example you can run immediately.

---

## What's here

| File                                                   | Kind             | What it shows                                                                         |
| ------------------------------------------------------ | ---------------- | ------------------------------------------------------------------------------------- |
| [`quickstart.py`](quickstart.py)                       | Python SDK       | Core API without an LLM: execute, rollback, install, bash, diff, export               |
| [`financial_analysis.py`](financial_analysis.py)       | Python SDK + LLM | Agent pulls SEC EDGAR revenue data for Apple, formats a Markdown report               |
| [`data_science.py`](data_science.py)                   | Python SDK + LLM | Agent trains three classifiers on the Iris dataset, compares accuracy                 |
| [`fibonacci_slow.py`](fibonacci_slow.py)               | Baseline         | Naive O(2ⁿ) Fibonacci — the baseline for the benchmark example                        |
| [`fibonacci_benchmark.py`](fibonacci_benchmark.py)     | Python SDK + LLM | Stress-tests every sandbox constraint, then benchmarks four Fibonacci implementations |
| [`financial_analysis.toml`](financial_analysis.toml)   | Config recipe    | SEC/Yahoo Finance domains, financial packages, snapshots enabled                      |
| [`data_science.toml`](data_science.toml)               | Config recipe    | Dataset domains, ML packages, larger workspace and timeouts                           |
| [`heavy_workloads.toml`](heavy_workloads.toml)         | Config recipe    | 4 GB workspace, 10-minute timeouts, persistent pip cache, auto-snapshot               |
| [`fibonacci_benchmark.toml`](fibonacci_benchmark.toml) | Config recipe    | Tight allowlists, short timeout — used by the stress test                             |

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
# macOS Apple Silicon
curl -fsSL https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-arm64 \
  -o /usr/local/bin/drun-mcp && chmod +x /usr/local/bin/drun-mcp

# macOS Intel
curl -fsSL https://github.com/dmosc/drun/releases/latest/download/drun-mcp-macos-x86_64 \
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

This exercises the raw SDK: create a session, run Python, write a file, roll
back to a prior state, install a package, run a bash command, diff two
checkpoints, and export to the host. No LLM or API key required.

Expected output:

```
[1] execute   — hello from the sandbox
[2] write     — checkpoint 1 → version A
[3] change    — checkpoint 3 → version B
[4] rollback  — version A
[5] install   —
| operation      | description                             |
|----------------|-----------------------------------------|
| execute_python | sandbox-isolated Python execution       |
| execute_bash   | sandbox-isolated shell commands         |
| rollback       | rewind to any prior checkpoint          |
| install        | pip-install into the session            |
| export         | copy workspace files to the host        |
[6] bash      — notes.txt
[7] diff      —
--- a/notes.txt
+++ b/notes.txt
@@ -1 +1 @@
-version A
\ No newline at end of file
+version B
\ No newline at end of file
[8] export    — ['/tmp/drun-quickstart/notes.txt']
```

### Step 3: run a config-backed LLM example

Choose a recipe TOML and an API key:

```bash
# Anthropic
export ANTHROPIC_API_KEY=sk-ant-...

DRUN_CONFIG=examples/financial_analysis.toml \
    python examples/financial_analysis.py
```

The script creates a drun session, drives the LLM to fetch SEC data and produce
a Markdown report, then exports the result to `/tmp/drun-financial/`.

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
| `quickstart.py`          | No                         | No              | Prints 8 labeled steps demonstrating the core SDK API                  |
| `financial_analysis.py`  | `financial_analysis.toml`  | Yes (or Ollama) | Exports `/tmp/drun-financial/results.md` with Apple revenue table      |
| `data_science.py`        | `data_science.toml`        | Yes (or Ollama) | Exports `/tmp/drun-data-science/results.md` with classifier comparison |
| `fibonacci_benchmark.py` | `fibonacci_benchmark.toml` | Yes (or Ollama) | Runs 5 constraint probes then exports a benchmark table                |

---

## Path 2 — MCP + Claude Code

The MCP binary exposes the full drun tool suite to Claude Code:
`create_session`, `session_execute_python`, `session_bash`, `session_fork`,
`session_rollback`, `session_merge`, `session_fetch`, and more. Claude drives
the sandbox directly from within your editor.

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

Paste the contents of the `PROMPT` constant from any `.py` example directly into
Claude Code, or give an equivalent natural-language instruction:

**Financial analysis:**

```
Use drun to analyze Apple's revenue trends. Fetch the data from the SEC EDGAR
XBRL API at data.sec.gov, install pandas and tabulate, format a year-over-year
table, and export results.md.
```

**Data science:**

```
Use drun to benchmark three scikit-learn classifiers on the Iris dataset.
Fetch the CSV from raw.githubusercontent.com, install the required packages,
train the models, and export a Markdown report comparing accuracy.
```

**Sandbox stress test:**

```
Use drun to run the Fibonacci constraint probes: trigger a timeout with
time.sleep(300), test domain allowlist enforcement with urllib.request, try a
bash denylist rejection with curl, attempt a bad package install, run a
correctness gate, then implement and benchmark four Fibonacci algorithms.
```

### What Claude sees (MCP tool suite)

| Category   | Tools                                                                                                    |
| ---------- | -------------------------------------------------------------------------------------------------------- |
| Lifecycle  | `create_session`, `session_list`, `session_close`, `session_tree`                                        |
| Execution  | `session_execute_python`, `session_bash`, `session_install_package`, `session_get_env`, `session_cancel` |
| Navigation | `session_rollback`, `session_fork`, `session_merge`, `session_history`, `get_session_state`              |
| Files      | `session_read_file`, `session_write_file`, `session_delete_file`, `session_mount`, `session_diff`        |
| Host I/O   | `session_export`, `session_commit`, `session_fetch`, `get_fetch_allowlist`, `get_allowed_packages`       |
| Snapshots  | `session_snapshot`, `session_restore`, `list_snapshots`                                                  |
| Labels     | `session_label`, `session_checkpoint_label`, `session_checkpoint_squash`, `session_checkpoint_drop`      |

---

## Path 3 — MCP + Ollama (no API key)

Use the `drun chat` CLI to drive a local Ollama model against the sandbox. This
is the same tool-use loop as the Python SDK path, running a local model instead
of a cloud provider.

### Step 1: install Ollama and pull a model

```bash
# Install from https://ollama.com, then:
ollama pull qwen2.5:14b
```

### Step 2: install the drun Python package

```bash
pip install 'drun-sandbox[chat]'
```

This provides the `drun` CLI in addition to the Python API.

### Step 3: run

```bash
DRUN_CONFIG=examples/financial_analysis.toml \
    drun chat \
        --model openai/qwen2.5:14b \
        --base-url http://localhost:11434/v1 \
        "Fetch Apple revenue from data.sec.gov, install pandas and tabulate,
         format a year-over-year table, and write results.md"
```

**Mount host files:**

```bash
DRUN_CONFIG=examples/data_science.toml \
    drun chat \
        --model openai/qwen2.5:14b \
        --base-url http://localhost:11434/v1 \
        --mount ./data \
        "Load the CSV files in /workspace/data, clean them, and summarize each column"
```

**`drun chat` flags:**

| Flag                 | Default              | Description                                     |
| -------------------- | -------------------- | ----------------------------------------------- |
| `--model MODEL`      | `ollama/qwen2.5:14b` | litellm model identifier                        |
| `--base-url URL`     | —                    | API base URL override                           |
| `--mount PATH`       | —                    | Mount a host path into the session (repeatable) |
| `--system PROMPT`    | built-in             | Override the system prompt                      |
| `--max-iterations N` | `30`                 | Maximum agent loop iterations                   |

---

## Configuration recipes

Each TOML file is designed around a specific class of workload. The fields shown
are only the ones that differ meaningfully from the defaults — unset fields fall
back to the built-in values documented in the main README.

> **Package installation always works.** PyPI, PyPI file storage, and jsDelivr
> are injected as defaults regardless of what `domain_allowlist` says. You never
> need to list them in your own config.

### `financial_analysis.toml` — live market and filing data

**What it allows:**

- `data.sec.gov`, `efts.sec.gov`, `www.sec.gov` — SEC EDGAR filings and XBRL
  data
- `query1.finance.yahoo.com`, `query2.finance.yahoo.com` — Yahoo Finance price
  history
- Packages: pandas, requests, tabulate, yfinance, matplotlib

**What it prevents:**

- Any domain not listed above (e.g., raw.githubusercontent.com, google.com)
- Any package not in the allowlist
- `curl`, `wget`, `nc` via bash
- Exec timeout: 3 minutes (for large XBRL payloads)

**Notable settings:**

- `snapshot_on_close = true` — financial analysis sessions are worth keeping
- `session_idle_timeout_secs = 7200` — long sessions for multi-step analysis

### `data_science.toml` — dataset fetching and model training

**What it allows:**

- `raw.githubusercontent.com`, `archive.ics.uci.edu`, `datasets.huggingface.co`
- Packages: pandas, numpy, matplotlib, scikit-learn, seaborn, tabulate, requests

**What it prevents:**

- Financial/market data domains, general web access
- `curl`, `wget`, `rm -rf` via bash

**Notable settings:**

- `max_workspace_mb = 1024` — datasets and model artifacts can be large
- `exec_timeout_ms = 180_000` — 3 minutes per execution for training loops
- `install_timeout_ms = 300_000` — 5 minutes for large packages (numpy, scipy)

### `heavy_workloads.toml` — long-running compute

**What it allows:**

- All PyPI packages (no `package_allowlist` set)

**What it prevents:**

- More than 2 concurrent sessions (resource contention)
- More than 25 checkpoints per session (disk usage)

**Notable settings:**

- `max_workspace_mb = 4096` — 4 GB workspace
- `exec_timeout_ms = 600_000` — 10 minutes per execution
- `bash_timeout_ms = 600_000` — 10 minutes per bash command
- `install_timeout_ms = 600_000` — 10 minutes for installs (PyTorch, TensorFlow)
- `snapshot_on_close = true` — long sessions can be resumed from a snapshot
- `packages_dir = "/tmp/drun-packages"` — persistent pip cache across restarts

### `fibonacci_benchmark.toml` — tight sandbox for constraint testing

**What it allows:**

- Only pyperf and tabulate packages
- No external domains beyond PyPI

**What it prevents:**

- Any other package install (rejected by MCP layer)
- `curl`, `wget`, `nc` via bash (denylist fires before the sandbox runs)
- Executions longer than 30 seconds (tight timeout for the stress test)

**Notable settings:**

- `exec_timeout_ms = 30_000` — exposes the timeout probe in Phase 1
- `snapshot_on_close = false` — all session state is fully ephemeral

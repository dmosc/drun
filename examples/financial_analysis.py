"""
Financial analysis agent — SEC EDGAR revenue trends.

The host fetches Apple's annual revenue data from the SEC EDGAR XBRL API and
pushes it into the sandbox as a file. An LLM agent then parses that JSON
using only the Python standard library, builds a year-over-year comparison
table, and writes a Markdown report.

session_bash has no network access and no package-install mechanism, so the
fetch happens here on the host (unsandboxed) before the session is created —
not from inside the agent's sandboxed tool calls.

Prerequisites:
    pip install 'drun-sandbox[chat]'
    # One of: ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY

Usage:
    # Anthropic (recommended)
    DRUN_CONFIG=examples/financial_analysis.toml \\
        python examples/financial_analysis.py

    # OpenAI
    DRUN_CONFIG=examples/financial_analysis.toml \\
        MODEL=gpt-4o python examples/financial_analysis.py

    # Local Ollama (no API key)
    DRUN_CONFIG=examples/financial_analysis.toml \\
        MODEL=openai/qwen2.5:14b BASE_URL=http://localhost:11434/v1 \\
        python examples/financial_analysis.py

Expected behavior:
    1. Host fetches Apple's revenue concept from the EDGAR XBRL API and
       writes it into the session as revenues.json.
    2. Agent parses the JSON with the standard library and builds a
       year-over-year table.
    3. Agent writes results.md, and the script exports it to
       /tmp/drun-financial/results.md.
    4. Session is preserved as a .drun snapshot (snapshot_on_close = true).
"""

import asyncio
import os
import textwrap
import urllib.request

from drun import Session
from drun.chat import ChatAgent, LocalSessionBridge

EDGAR_URL = (
    "https://data.sec.gov/api/xbrl/companyconcept/CIK0000320193/us-gaap/Revenues.json"
)

PROMPT = textwrap.dedent("""\
    You are a financial analysis agent operating inside a drun sandbox.
    revenues.json is already present in your workspace — it was fetched by
    the host before this session started (the sandbox itself has no network
    access). Your task: parse it, build a year-over-year revenue comparison
    table, and write a Markdown report.

    STEP 1 — Inspect the data
    --------------------------
    Read revenues.json. The response has this structure:

        {
          "entityName": "Apple Inc.",
          "units": {
            "USD": [
              {"accn": "...", "end": "YYYY-MM-DD", "form": "10-K", "val": 123456789, ...},
              ...
            ]
          }
        }

    Filter for annual filings only (form == "10-K"). Extract the most recent
    filing per fiscal year (deduplicate by year from "end" date). Sort by
    year ascending.

    STEP 2 — Build the table
    -------------------------
    Build a table with columns:
        Year | Revenue (USD billions) | YoY Change

    Use only the standard library (json, no pandas/tabulate — they are not
    installable inside the sandbox).

    STEP 3 — Write the report
    --------------------------
    Write the table to results.md as GitHub Markdown:

        # Apple Inc. — Annual Revenue (10-K filings)

        | Year | Revenue (USD B) | YoY Change |
        |------|-----------------|------------|
        | 2020 | 274.5           | +5.5%      |
        ...

    Add a one-sentence summary below the table naming the highest-growth
    year.
""")


def main():
    model = os.environ.get("MODEL", "claude-sonnet-4-6")
    base_url = os.environ.get("BASE_URL")

    request = urllib.request.Request(
        EDGAR_URL, headers={"User-Agent": "drun-example research@example.com"}
    )
    with urllib.request.urlopen(request) as response:
        revenues_json = response.read()

    session = Session()
    session.write_file("revenues.json", revenues_json)

    agent = ChatAgent(
        LocalSessionBridge(session),
        model=model,
        base_url=base_url,
        max_iterations=30,
    )
    asyncio.run(agent.run(PROMPT))

    export_dir = "/tmp/drun-financial"
    exported = session.export(export_dir)
    if exported:
        print(f"\nExported: {exported}")
        print(f"Report: {export_dir}/results.md")


if __name__ == "__main__":
    main()

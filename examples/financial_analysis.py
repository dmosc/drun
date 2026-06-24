"""
Financial analysis agent — SEC EDGAR revenue trends.

An LLM agent fetches Apple's annual revenue data from the SEC EDGAR XBRL API,
installs pandas and tabulate inside the sandbox, formats a year-over-year
comparison table, and exports a Markdown report.

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
    1. Agent fetches Apple's revenue concept from the EDGAR XBRL API.
    2. Installs pandas and tabulate inside the sandbox.
    3. Parses the JSON response and builds a year-over-year table.
    4. Writes results.md to /workspace and the script exports it to
       /tmp/drun-financial/results.md.
    5. Session is preserved as a .drun snapshot (snapshot_on_close = true).
"""

import os
import textwrap

from drun import Session
from drun.chat import run

PROMPT = textwrap.dedent("""\
    You are a financial analysis agent operating inside a drun sandbox.
    Your task: pull Apple Inc.'s annual revenue from the SEC EDGAR XBRL API,
    build a year-over-year comparison table, and write a Markdown report.

    STEP 1 — Fetch revenue data
    ---------------------------
    Use urllib.request (stdlib — no install needed) to fetch:

        https://data.sec.gov/api/xbrl/companyconcept/CIK0000320193/us-gaap/Revenues.json

    Store the raw JSON response in /workspace/revenues.json.

    STEP 2 — Install packages
    -------------------------
    install_package("pandas")
    install_package("tabulate")

    STEP 3 — Parse and format
    -------------------------
    Load revenues.json. The response has this structure:

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
    filing per fiscal year (deduplicate by year from "end" date). Sort by year
    ascending.

    Build a table with columns:
        Year | Revenue (USD billions) | YoY Change

    STEP 4 — Write the report
    -------------------------
    Write the table to /workspace/results.md as GitHub Markdown:

        # Apple Inc. — Annual Revenue (10-K filings)

        | Year | Revenue (USD B) | YoY Change |
        |------|-----------------|------------|
        | 2020 | 274.5           | +5.5%      |
        ...

    Add a one-sentence summary below the table naming the highest-growth year.

    Do not create snapshots — the config handles that automatically.
""")


def main():
    model = os.environ.get("MODEL", "claude-sonnet-4-6")
    base_url = os.environ.get("BASE_URL")

    session = Session()

    run(
        session,
        PROMPT,
        model=model,
        base_url=base_url,
        max_iterations=30,
    )

    export_dir = "/tmp/drun-financial"
    exported = session.export(export_dir)
    if exported:
        print(f"\nExported: {exported}")
        print(f"Report: {export_dir}/results.md")


if __name__ == "__main__":
    main()

"""
Fibonacci optimization benchmark driven by an LLM agent inside a drun sandbox.

The agent first stress-tests every configured sandbox constraint, then
implements four faster Fibonacci algorithms, benchmarks them with a stdlib
timing harness, and writes a comparison table.  The session is fully
ephemeral: workspace state is discarded when it closes, only the exported
results file survives.

Prerequisites:
    pip install 'drun-sandbox[chat]'

Usage:
    # Anthropic model (recommended — requires ANTHROPIC_API_KEY):
    DRUN_CONFIG=examples/fibonacci_benchmark.toml \\
        python examples/fibonacci_benchmark.py

    # Override model:
    DRUN_CONFIG=examples/fibonacci_benchmark.toml \\
        MODEL=openai/gpt-4o python examples/fibonacci_benchmark.py

    # Local model via Ollama (no API key needed):
    DRUN_CONFIG=examples/fibonacci_benchmark.toml \\
        MODEL=openai/qwen2.5:14b BASE_URL=http://localhost:11434/v1 \\
        python examples/fibonacci_benchmark.py

Note on networking:
    session_bash has zero network access on either platform, regardless of
    domain_allowlist — that config only governs session_fetch (a separate,
    host-side tool not used by this example, and not available via the
    Python SDK's chat agent at all). Phase 1b demonstrates this directly.

Note on packages:
    There is no install mechanism inside the sandbox, so this benchmark uses
    only the standard library — no pyperf, no tabulate.
"""

import asyncio
import os
import textwrap

from drun import Session
from drun.chat import ChatAgent, LocalSessionBridge

PROMPT = textwrap.dedent("""\
    You are operating inside a drun sandbox. Your job has two parts: first
    verify that every configured sandbox constraint fires correctly, then
    implement and benchmark faster Fibonacci algorithms.

    Read each probe carefully. Expected errors in Phase 1 are correct
    behavior — report what you observe, then move to the next probe.

    ═══════════════════════════════════════════════════════════════════
    PHASE 1 — CONSTRAINT PROBES  (run every probe before moving on)
    ═══════════════════════════════════════════════════════════════════

    After each probe print one line: "probe <name>: <observed outcome>".

    ── 1a. Execution timeout ──────────────────────────────────────────

    Run this via execute_bash (python3 -c "..."):

        import time
        print("sleeping — should never finish")
        time.sleep(300)
        print("done")    # must not appear

    Expected: the sandbox kills the command after bash_timeout_ms (30 s).
    You will see a timeout error — "done" must never print. Report the
    exact error message you received.

    ── 1b. No network access from session_bash ────────────────────────

    Run this via execute_bash (python3 -c "..."), targeting pypi.org —
    a domain that IS allowlisted for session_fetch:

        import urllib.request
        try:
            urllib.request.urlopen("https://pypi.org", timeout=5)
            print("reached network — unexpected")
        except Exception as e:
            print(f"no network: {type(e).__name__}: {e}")

    Expected: a connection error, even though pypi.org is allowlisted.
    domain_allowlist only governs session_fetch — session_bash has no
    network access at all, for any domain. Report the exact error.

    ── 1c. Bash command denylist ──────────────────────────────────────

    Run these two commands via execute_bash:

      ALLOWED — ls
        Expected: directory listing printed to stdout.

      DENIED — curl https://example.com
        Expected: a denylist rejection error before the command runs.
        The server matches "curl" against bash_command_denylist and
        rejects it immediately — no execution is ever attempted (the
        sandbox would have no network for it anyway, but the denylist
        fires first). Print the exact error message.

    ── 1d. Correctness gate ───────────────────────────────────────────

    Write and run a deliberately broken Fibonacci via execute_bash:

        def fib_broken(n):
            return n + 1   # wrong on purpose

        result = fib_broken(35)
        assert result == 9_227_465, (
            f"correctness check failed: got {result}, expected 9_227_465"
        )

    Expected: AssertionError showing the wrong value. This confirms the
    correctness assertion you will use in Phase 3 actually catches bugs.
    Do not keep fib_broken — it must not affect Phase 3.

    ═══════════════════════════════════════════════════════════════════
    PHASE 2 — SETUP  (only after all four probes complete)
    ═══════════════════════════════════════════════════════════════════

    5. Read fibonacci_slow.py to understand the baseline algorithm.

    6. Load and verify the baseline via execute_bash:
         from fibonacci_slow import fibonacci
         assert fibonacci(35) == 9_227_465

    ═══════════════════════════════════════════════════════════════════
    PHASE 3 — IMPLEMENT AND BENCHMARK
    ═══════════════════════════════════════════════════════════════════

    7. Write a benchmark harness using time.perf_counter() (stdlib — no
       pyperf): for each implementation, run 5 trials, each trial timing
       1 call, and report mean and standard deviation in microseconds
       (use the statistics module).

       For the SLOW baseline use n=30 (not 35) to stay within the 30 s
       bash_timeout_ms.  For every optimized implementation use n=35.

       Correct values for the assertion:
         fibonacci(30) == 832_040
         fibonacci(35) == 9_227_465

       Assert correctness for every function before timing it.

    8. Implement, verify, and benchmark each candidate:
          a. Memoized recursion     — functools.lru_cache on the baseline logic
          b. Iterative (bottom-up)  — O(n) time, O(1) space, no recursion
          c. Matrix exponentiation  — O(log n) via 2x2 matrix multiply
          d. Fast doubling          — O(log n) via:
                                        F(2k)   = F(k) * (2*F(k+1) - F(k))
                                        F(2k+1) = F(k)^2 + F(k+1)^2

    9. Collect results and print a comparison table in GitHub Markdown:

          | Algorithm        | n  | Mean (us) | Std Dev (us) | Speedup vs. baseline |
          |------------------|----|-----------|--------------|----------------------|
          | naive_recursive  | 30 | ...       | ...          | 1x                   |
          | memoized         | 35 | ...       | ...          | Nx                   |
          | iterative        | 35 | ...       | ...          | Nx                   |
          | matrix_exp       | 35 | ...       | ...          | Nx                   |
          | fast_doubling    | 35 | ...       | ...          | Nx                   |

        State the overall winner and the speedup factor over the baseline.

    10. Write the table to results.md (the script exports it after you finish).

    The session is ephemeral — do not create snapshots.
""")


def main():
    model = os.environ.get("MODEL", "claude-sonnet-4-6")
    base_url = os.environ.get("BASE_URL")

    session = Session()
    session.mount("examples/")

    agent = ChatAgent(
        LocalSessionBridge(session),
        model=model,
        base_url=base_url,
        max_iterations=60,
    )
    asyncio.run(agent.run(PROMPT))

    export_dir = os.environ.get("EXPORT_DIR", "/tmp/drun-fibonacci-results")
    exported = session.export(export_dir)
    if exported:
        print(f"\nExported: {exported}")
        print(f"Report: {export_dir}/results.md")


if __name__ == "__main__":
    main()

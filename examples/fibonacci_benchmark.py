"""
Fibonacci optimization benchmark driven by an LLM agent inside a drun sandbox.

The agent first stress-tests every configured sandbox constraint, then
implements four faster Fibonacci algorithms, benchmarks them with pyperf, and
exports a comparison table.  The session is fully ephemeral: workspace state
is discarded when it closes, only the exported results file survives.

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

    # Equivalent one-liner via the drun CLI:
    DRUN_CONFIG=examples/fibonacci_benchmark.toml \\
        drun chat --model claude-sonnet-4-6 --mount examples/ "$(python -c \\
            'import examples.fibonacci_benchmark as m; print(m.PROMPT)')"

Note on package_allowlist:
    package_allowlist is enforced at the MCP layer (Claude Code / drun-mcp).
    The Python SDK path (this script) routes install_package directly to pip,
    so the allowlist is not checked here.  Phase 1d demonstrates install
    failure using a nonexistent package name instead.
"""

import os
import textwrap

from drun import Session
from drun.chat import run

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

    Run this via execute_python:

        import time
        print("sleeping — should never finish")
        time.sleep(300)
        print("done")    # must not appear

    Expected: the sandbox kills the runner after exec_timeout_ms (30 s).
    You will see a timeout or crash error — "done" must never print.
    After a timeout the runner is automatically rebuilt, so you can
    continue immediately. Report the exact error message you received.

    ── 1b. Egress domain allowlist (Python outbound HTTP) ─────────────

    Run these two fetches in a single execute_python call using only
    urllib.request (stdlib, no install needed):

      ALLOWED — https://pypi.org/pypi/pyperf/json
        Expected: HTTP 200. Print the first 100 bytes of the response body.

      BLOCKED — https://www.google.com/
        Expected: the egress proxy returns 403 Forbidden because
        www.google.com is not in domain_allowlist. Print the exact
        exception type and message.

    Use this template:

        import urllib.request, urllib.error
        for label, url in [
            ("pypi.org (allowed)", "https://pypi.org/pypi/pyperf/json"),
            ("google.com (blocked)", "https://www.google.com/"),
        ]:
            try:
                with urllib.request.urlopen(url, timeout=10) as r:
                    print(f"{label} -> HTTP {r.status}: {r.read(100)}")
            except Exception as e:
                print(f"{label} -> {type(e).__name__}: {e}")

    ── 1c. Bash command denylist ──────────────────────────────────────

    Run these two commands via execute_bash:

      ALLOWED — ls /workspace
        Expected: directory listing printed to stdout.

      DENIED — curl https://example.com
        Expected: a denylist rejection error before the command runs.
        The server matches "curl" against bash_command_denylist and
        rejects it immediately — no network attempt is ever made.
        Print the exact error message.

    ── 1d. Failed package install ─────────────────────────────────────

    Attempt to install a package that does not exist on PyPI:

        install_package("this-package-does-not-exist-xyz-drun-test")

    Expected: pip reports "No matching distribution found" (or similar).
    This shows how install failures surface to the agent.
    Report the error and continue — the session remains usable.

    ── 1e. Correctness gate ───────────────────────────────────────────

    Write and run a deliberately broken Fibonacci via execute_python:

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
    PHASE 2 — SETUP  (only after all five probes complete)
    ═══════════════════════════════════════════════════════════════════

    6. Read fibonacci_slow.py to understand the baseline algorithm.

    7. Install the required packages:
         install_package("pyperf")
         install_package("tabulate")

    8. Load and verify the baseline:
         from fibonacci_slow import fibonacci
         assert fibonacci(35) == 9_227_465

    ═══════════════════════════════════════════════════════════════════
    PHASE 3 — IMPLEMENT AND BENCHMARK
    ═══════════════════════════════════════════════════════════════════

    9. Write a benchmark harness using pyperf.Runner configured with:
         loops=1     (implementations span many orders of magnitude)
         values=5
         warmups=2

       For the SLOW baseline use n=30 (not 35) to stay within the 30 s
       exec_timeout_ms.  For every optimized implementation use n=35.

       Correct values for the assertion:
         fibonacci(30) == 832_040
         fibonacci(35) == 9_227_465

       Assert correctness for every function before timing it.

    10. Implement, verify, and benchmark each candidate:
          a. Memoized recursion     — functools.lru_cache on the baseline logic
          b. Iterative (bottom-up)  — O(n) time, O(1) space, no recursion
          c. Matrix exponentiation  — O(log n) via 2x2 matrix multiply
          d. Fast doubling          — O(log n) via:
                                        F(2k)   = F(k) * (2*F(k+1) - F(k))
                                        F(2k+1) = F(k)^2 + F(k+1)^2

    11. Collect results and print a comparison table in GitHub Markdown:

          | Algorithm        | n  | Mean (us) | Std Dev (us) | Speedup vs. baseline |
          |------------------|----|-----------|--------------|----------------------|
          | naive_recursive  | 30 | ...       | ...          | 1x                   |
          | memoized         | 35 | ...       | ...          | Nx                   |
          | iterative        | 35 | ...       | ...          | Nx                   |
          | matrix_exp       | 35 | ...       | ...          | Nx                   |
          | fast_doubling    | 35 | ...       | ...          | Nx                   |

        State the overall winner and the speedup factor over the baseline.

    12. Write the table to /workspace/results.md and export it.

    The session is ephemeral — do not create snapshots.
""")


def main():
    model = os.environ.get("MODEL", "claude-sonnet-4-6")
    base_url = os.environ.get("BASE_URL")

    session = Session()
    session.mount("examples/")

    run(
        session,
        PROMPT,
        model=model,
        base_url=base_url,
        max_iterations=60,
    )


if __name__ == "__main__":
    main()

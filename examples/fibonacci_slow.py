"""
Deliberately naive recursive Fibonacci — O(2^n) time, no memoization.

This file exists as the baseline for the fibonacci_benchmark.py example,
which mounts it into a drun session and drives an LLM agent to produce
faster alternatives and benchmark them.
"""

import time


def fibonacci(n: int) -> int:
    if n <= 1:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)


if __name__ == "__main__":
    n = 35
    start = time.perf_counter()
    result = fibonacci(n)
    elapsed = time.perf_counter() - start
    print(f"fibonacci({n}) = {result}  ({elapsed:.3f}s)")

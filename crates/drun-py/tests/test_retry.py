"""Tests for RetryPolicy's attempt/backoff bookkeeping."""

from __future__ import annotations

import pytest

from drun.retry import RetryPolicy


async def test_run_returns_the_first_successful_result():
    policy = RetryPolicy(attempts=3, base_delay=0)

    async def action() -> str:
        return "ok"

    assert await policy.run(action) == "ok"


async def test_run_retries_after_a_failure_then_succeeds():
    calls = 0

    async def action() -> str:
        nonlocal calls
        calls += 1
        if calls < 2:
            raise RuntimeError("transient")
        return "ok"

    result = await RetryPolicy(attempts=3, base_delay=0).run(action)

    assert result == "ok"
    assert calls == 2


async def test_run_reraises_the_last_exception_once_attempts_are_exhausted():
    calls = 0

    async def action() -> str:
        nonlocal calls
        calls += 1
        raise ValueError(f"failure {calls}")

    with pytest.raises(ValueError, match="failure 3"):
        await RetryPolicy(attempts=3, base_delay=0).run(action)

    assert calls == 3


async def test_attempts_is_clamped_to_at_least_one():
    policy = RetryPolicy(attempts=0, base_delay=0)

    calls = 0

    async def action() -> str:
        nonlocal calls
        calls += 1
        raise RuntimeError("boom")

    with pytest.raises(RuntimeError):
        await policy.run(action)

    assert calls == 1

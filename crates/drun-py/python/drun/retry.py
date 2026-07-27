"""Retry policy shared by `ChatAgent`'s LLM calls and tool calls."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import TypeVar

T = TypeVar("T")


class RetryPolicy:
    """Retries an async action up to `attempts` times with exponential
    backoff, re-raising the last exception once attempts are exhausted."""

    def __init__(self, attempts: int = 3, base_delay: float = 0.5, max_delay: float = 8.0) -> None:
        self.attempts = max(1, attempts)
        self.base_delay = base_delay
        self.max_delay = max_delay

    async def run(self, action: Callable[[], Awaitable[T]]) -> T:
        last_exc: BaseException | None = None
        for attempt in range(self.attempts):
            try:
                return await action()
            except Exception as exc:
                last_exc = exc
                if attempt + 1 < self.attempts:
                    await asyncio.sleep(min(self.base_delay * 2 ** attempt, self.max_delay))
        assert last_exc is not None
        raise last_exc

"""Formats a session's `session_history` checkpoints into agent-facing context."""

from __future__ import annotations

import json
from typing import Any


class SessionHistoryContext:
    """Turns `session_history`'s checkpoint summaries into a prior-actions
    block for an agent's system prompt, so it can pick up where it left off
    instead of starting blind."""

    def __init__(self, checkpoints: list[dict[str, Any]]) -> None:
        self._checkpoints = checkpoints

    @classmethod
    def from_json(cls, raw: str) -> SessionHistoryContext:
        return cls(json.loads(raw))

    def as_prompt_block(self) -> str:
        described = [self._describe(c)
                     for c in self._checkpoints if c.get("steps")]
        if not described:
            return ""
        return "Prior actions already taken in this session:\n" + "\n".join(described)

    @staticmethod
    def _describe(checkpoint: dict[str, Any]) -> str:
        label = f" ({checkpoint['label']})" if checkpoint.get("label") else ""
        header = f"- checkpoint {checkpoint['checkpoint_id']}{label}"
        steps = (
            f"    {step['tool']}: {step['description']}"
            for step in checkpoint["steps"]
        )
        return "\n".join([header, *steps])

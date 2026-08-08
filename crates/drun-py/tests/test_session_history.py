"""Tests for SessionHistoryContext's checkpoint-history-to-prompt formatting."""

from __future__ import annotations

import json

from drun.session_history import SessionHistoryContext


def test_as_prompt_block_is_empty_for_no_checkpoints():
    assert SessionHistoryContext([]).as_prompt_block() == ""


def test_as_prompt_block_skips_checkpoints_with_no_steps():
    checkpoints = [{"checkpoint_id": 0, "steps": []}]
    assert SessionHistoryContext(checkpoints).as_prompt_block() == ""


def test_as_prompt_block_describes_each_step_of_a_checkpoint():
    checkpoints = [{
        "checkpoint_id": 1,
        "steps": [
            {"tool": "session_bash", "description": "installed dependencies"},
            {"tool": "session_diff", "description": "checked what changed"},
        ],
    }]

    block = SessionHistoryContext(checkpoints).as_prompt_block()

    assert "- checkpoint 1" in block
    assert "session_bash: installed dependencies" in block
    assert "session_diff: checked what changed" in block


def test_as_prompt_block_includes_the_checkpoint_label_when_present():
    checkpoints = [{
        "checkpoint_id": 2,
        "label": "deps-installed",
        "steps": [{"tool": "session_bash", "description": "ran install"}],
    }]

    block = SessionHistoryContext(checkpoints).as_prompt_block()

    assert "- checkpoint 2 (deps-installed)" in block


def test_from_json_parses_the_raw_session_history_tool_output():
    raw = json.dumps([{
        "checkpoint_id": 1,
        "steps": [{"tool": "session_bash", "description": "ran tests"}],
    }])

    block = SessionHistoryContext.from_json(raw).as_prompt_block()

    assert "session_bash: ran tests" in block

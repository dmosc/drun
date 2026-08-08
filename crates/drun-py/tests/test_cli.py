"""Tests for ChatCommand._run_chat's error-handling split: a failure to
connect to drun-mcp gets the "is drun-mcp running?" hint, but a failure inside
an already-connected session (e.g. the LLM call) does not — that hint would
misdirect troubleshooting for an unrelated error."""

from __future__ import annotations

import argparse

import pytest

from drun import cli


class FakeBridge:
    def __init__(
        self, enter_error: Exception | None = None, call_error: Exception | None = None
    ) -> None:
        self._enter_error = enter_error
        self._call_error = call_error
        self.calls: list[tuple[str, dict[str, object]]] = []

    async def __aenter__(self) -> "FakeBridge":
        if self._enter_error is not None:
            raise self._enter_error
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        return None

    async def call(self, name: str, arguments: dict[str, object] | None = None) -> str:
        self.calls.append((name, arguments or {}))
        if self._call_error is not None:
            raise self._call_error
        return "{}"


class FailingAgent:
    def __init__(self, *args: object, **kwargs: object) -> None:
        pass

    async def run(self, prompt: str) -> str:
        raise RuntimeError("litellm exploded")


class SucceedingAgent:
    def __init__(self, *args: object, **kwargs: object) -> None:
        pass

    async def run(self, prompt: str) -> str:
        return f"answer to: {prompt}"


def make_args(**overrides: object) -> argparse.Namespace:
    defaults: dict[str, object] = dict(
        prompt="hello",
        mcp_url="http://127.0.0.1:7273/mcp",
        model="m",
        base_url=None,
        session_id=None,
        mount=[],
        system=None,
        max_iterations=5,
        llm_retries=3,
        reasoning_effort=None,
    )
    defaults.update(overrides)
    return argparse.Namespace(**defaults)


async def test_connection_failure_gets_the_is_drun_mcp_running_hint(monkeypatch, capsys):
    monkeypatch.setattr(
        cli, "DrunMcpBridge",
        lambda *a, **k: FakeBridge(enter_error=ConnectionRefusedError("boom")),
    )

    with pytest.raises(SystemExit):
        await cli.ChatCommand()._run_chat(make_args())

    err = capsys.readouterr().err
    assert "boom" in err
    assert "Is drun-mcp running?" in err


async def test_agent_failure_after_connecting_has_no_mcp_hint(monkeypatch, capsys):
    monkeypatch.setattr(cli, "DrunMcpBridge", lambda *a, **k: FakeBridge())
    monkeypatch.setattr(cli, "ChatAgent", FailingAgent)

    with pytest.raises(SystemExit):
        await cli.ChatCommand()._run_chat(make_args())

    err = capsys.readouterr().err
    assert "litellm exploded" in err
    assert "Is drun-mcp running?" not in err


async def test_successful_chat_records_the_turn_via_session_chat_record(monkeypatch):
    bridge = FakeBridge()
    monkeypatch.setattr(cli, "DrunMcpBridge", lambda *a, **k: bridge)
    monkeypatch.setattr(cli, "ChatAgent", SucceedingAgent)

    await cli.ChatCommand()._run_chat(make_args(prompt="hello"))

    assert bridge.calls == [
        ("session_chat_record", {"prompt": "hello", "response": "answer to: hello"})
    ]


async def test_a_failure_recording_the_chat_turn_warns_but_does_not_exit(monkeypatch, capsys):
    bridge = FakeBridge(call_error=RuntimeError("daemon unreachable"))
    monkeypatch.setattr(cli, "DrunMcpBridge", lambda *a, **k: bridge)
    monkeypatch.setattr(cli, "ChatAgent", SucceedingAgent)

    await cli.ChatCommand()._run_chat(make_args())

    err = capsys.readouterr().err
    assert "warning: failed to record chat turn" in err
    assert "daemon unreachable" in err

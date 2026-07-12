"""Tests for ChatAgent's tool-calling loop, against a fake DrunMcpBridge."""

from __future__ import annotations

import itertools
import json
from collections.abc import Iterable
from typing import Any

import litellm

from drun.chat import ChatAgent


class FakeBridge:
    def __init__(self, tool_results: dict[str, str]) -> None:
        self._tool_results = tool_results
        self.calls: list[tuple[str, dict[str, Any] | None]] = []

    async def tools(self) -> list[dict[str, Any]]:
        return []

    async def call(self, name: str, arguments: dict[str, Any] | None = None) -> str:
        self.calls.append((name, arguments))
        return self._tool_results.get(name, "")


class FakeFunctionCall:
    def __init__(self, name: str, arguments: str) -> None:
        self.name = name
        self.arguments = arguments


class FakeToolCall:
    def __init__(self, call_id: str, name: str, arguments: str) -> None:
        self.id = call_id
        self.function = FakeFunctionCall(name, arguments)


class FakeMessage:
    def __init__(self, content: str | None, tool_calls: list[FakeToolCall] | None = None) -> None:
        self.content = content
        self.tool_calls = tool_calls


class FakeChoice:
    def __init__(self, message: FakeMessage, finish_reason: str = "stop") -> None:
        self.message = message
        self.finish_reason = finish_reason


class FakeResponse:
    def __init__(self, choice: FakeChoice) -> None:
        self.choices = [choice]


def stub_acompletion(responses: Iterable[FakeResponse]):
    remaining = iter(responses)

    async def _acompletion(**_: object) -> FakeResponse:
        return next(remaining)

    return _acompletion


async def test_run_bootstraps_a_session_and_mounts_paths_before_the_first_turn(monkeypatch):
    bridge = FakeBridge({"create_session": json.dumps({"session_id": "s1"})})
    monkeypatch.setattr(
        litellm, "acompletion", stub_acompletion(
            [FakeResponse(FakeChoice(FakeMessage("done")))])
    )

    agent = ChatAgent(bridge)
    result = await agent.run("do the thing", mounts=["/tmp/data"])

    assert result == "done"
    assert bridge.calls[0] == ("create_session", None)
    assert bridge.calls[1] == (
        "session_mount", {"session_id": "s1", "path": "/tmp/data"})


async def test_run_executes_a_tool_call_then_returns_the_final_answer(monkeypatch):
    bridge = FakeBridge(
        {"create_session": json.dumps(
            {"session_id": "s1"}), "session_bash": "hello world"}
    )
    tool_call = FakeToolCall(
        "call-1", "session_bash", json.dumps(
            {"session_id": "s1", "command": "echo hi"})
    )
    monkeypatch.setattr(
        litellm,
        "acompletion",
        stub_acompletion(
            [
                FakeResponse(FakeChoice(FakeMessage(
                    None, tool_calls=[tool_call]))),
                FakeResponse(FakeChoice(FakeMessage(
                    "the output was hello world"))),
            ]
        ),
    )

    agent = ChatAgent(bridge)
    result = await agent.run("run echo", mounts=[])

    assert result == "the output was hello world"
    assert ("session_bash", {"session_id": "s1",
            "command": "echo hi"}) in bridge.calls


async def test_run_stops_after_max_iterations_without_a_final_answer(monkeypatch):
    bridge = FakeBridge({"create_session": json.dumps(
        {"session_id": "s1"}), "session_bash": "ok"})
    tool_call = FakeToolCall(
        "call-1", "session_bash", json.dumps(
            {"session_id": "s1", "command": "true"})
    )
    always_tool_call = FakeResponse(FakeChoice(
        FakeMessage(None, tool_calls=[tool_call])))
    monkeypatch.setattr(
        litellm, "acompletion", stub_acompletion(
            itertools.repeat(always_tool_call))
    )

    agent = ChatAgent(bridge, max_iterations=2)
    result = await agent.run("loop forever", mounts=[])

    assert result == "(max iterations reached)"

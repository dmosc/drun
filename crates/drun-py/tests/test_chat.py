"""Tests for ChatAgent's tool-calling loop and LocalSessionBridge, against
fakes standing in for a Bridge and a DrunSession."""

from __future__ import annotations

import itertools
import json
from collections.abc import Iterable
from typing import Any

import litellm

from drun.chat import ChatAgent, LocalSessionBridge


class FakeBridge:
    default_system_prompt = "fake system prompt"

    def __init__(self, tool_results: dict[str, str] | None = None) -> None:
        self._tool_results = tool_results or {}
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


async def test_run_uses_the_bridges_default_system_prompt(monkeypatch):
    bridge = FakeBridge()
    captured_messages: list[dict[str, Any]] = []

    async def _acompletion(**kwargs: object) -> FakeResponse:
        captured_messages.extend(kwargs["messages"])
        return FakeResponse(FakeChoice(FakeMessage("done")))

    monkeypatch.setattr(litellm, "acompletion", _acompletion)

    agent = ChatAgent(bridge)
    result = await agent.run("do the thing")

    assert result == "done"
    assert captured_messages[0] == {
        "role": "system", "content": "fake system prompt"}


async def test_run_prefers_an_explicit_system_prompt_override(monkeypatch):
    bridge = FakeBridge()
    captured_messages: list[dict[str, Any]] = []

    async def _acompletion(**kwargs: object) -> FakeResponse:
        captured_messages.extend(kwargs["messages"])
        return FakeResponse(FakeChoice(FakeMessage("done")))

    monkeypatch.setattr(litellm, "acompletion", _acompletion)

    agent = ChatAgent(bridge, system="custom prompt")
    await agent.run("do the thing")

    assert captured_messages[0] == {
        "role": "system", "content": "custom prompt"}


async def test_run_executes_a_tool_call_then_returns_the_final_answer(monkeypatch):
    bridge = FakeBridge({"session_bash": "hello world"})
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
    result = await agent.run("run echo")

    assert result == "the output was hello world"
    assert ("session_bash", {"session_id": "s1",
            "command": "echo hi"}) in bridge.calls


async def test_run_stops_after_max_iterations_without_a_final_answer(monkeypatch):
    bridge = FakeBridge({"session_bash": "ok"})
    tool_call = FakeToolCall(
        "call-1", "session_bash", json.dumps({"command": "true"})
    )
    always_tool_call = FakeResponse(FakeChoice(
        FakeMessage(None, tool_calls=[tool_call])))
    monkeypatch.setattr(
        litellm, "acompletion", stub_acompletion(
            itertools.repeat(always_tool_call))
    )

    agent = ChatAgent(bridge, max_iterations=2)
    result = await agent.run("loop forever")

    assert result == "(max iterations reached)"


class FakeCheckpoint:
    def __init__(self, stdout: str = "", stderr: str = "") -> None:
        self.stdout = stdout
        self.stderr = stderr


class FakeDrunSession:
    def __init__(self) -> None:
        self.written: dict[str, bytes] = {}

    def execute_bash(self, command: str) -> FakeCheckpoint:
        if command == "boom":
            raise RuntimeError("command failed")
        return FakeCheckpoint(stdout="hello world")

    def write_file(self, path: str, content: bytes) -> None:
        self.written[path] = content


async def test_local_session_bridge_runs_bash_and_formats_stdout():
    bridge = LocalSessionBridge(FakeDrunSession())

    result = await bridge.call("execute_bash", {"command": "echo hi"})

    assert result == "stdout:\nhello world"


async def test_local_session_bridge_writes_files_through_the_session():
    session = FakeDrunSession()
    bridge = LocalSessionBridge(session)

    result = await bridge.call("write_file", {"path": "a.txt", "content": "hi"})

    assert result == "wrote a.txt"
    assert session.written == {"a.txt": b"hi"}


async def test_local_session_bridge_reports_tool_errors_without_raising():
    bridge = LocalSessionBridge(FakeDrunSession())

    result = await bridge.call("execute_bash", {"command": "boom"})

    assert result == "error: command failed"


async def test_local_session_bridge_reports_unknown_tools():
    bridge = LocalSessionBridge(FakeDrunSession())

    result = await bridge.call("mystery_tool", {})

    assert result == "unknown tool: mystery_tool"

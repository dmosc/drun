"""Tests for ChatAgent's tool-calling loop and LocalSessionBridge, against
fakes standing in for a Bridge and a DrunSession."""

from __future__ import annotations

import itertools
import json
from collections.abc import Iterable
from typing import Any

import litellm
import pytest

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


def finish_message(content: str) -> FakeMessage:
    """A message that ends the run, standing in for a real model calling
    ChatAgent's synthetic `finish_trajectory` tool."""
    tool_call = FakeToolCall(
        "finish-1", "finish_trajectory", json.dumps({"content": content}))
    return FakeMessage(None, tool_calls=[tool_call])


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
        return FakeResponse(FakeChoice(finish_message("done")))

    monkeypatch.setattr(litellm, "acompletion", _acompletion)

    agent = ChatAgent(bridge)
    result = await agent.run("do the thing")

    assert result == "done"
    assert captured_messages[0] == {
        "role": "system", "content": "fake system prompt"}


async def test_run_omits_reasoning_effort_by_default(monkeypatch):
    bridge = FakeBridge()
    captured_kwargs: dict[str, Any] = {}

    async def _acompletion(**kwargs: object) -> FakeResponse:
        captured_kwargs.update(kwargs)
        return FakeResponse(FakeChoice(finish_message("done")))

    monkeypatch.setattr(litellm, "acompletion", _acompletion)

    agent = ChatAgent(bridge)
    await agent.run("do the thing")

    assert "reasoning_effort" not in captured_kwargs


async def test_run_passes_reasoning_effort_when_given(monkeypatch):
    bridge = FakeBridge()
    captured_kwargs: dict[str, Any] = {}

    async def _acompletion(**kwargs: object) -> FakeResponse:
        captured_kwargs.update(kwargs)
        return FakeResponse(FakeChoice(finish_message("done")))

    monkeypatch.setattr(litellm, "acompletion", _acompletion)

    agent = ChatAgent(bridge, reasoning_effort="high")
    await agent.run("do the thing")

    assert captured_kwargs["reasoning_effort"] == "high"


async def test_run_prefers_an_explicit_system_prompt_override(monkeypatch):
    bridge = FakeBridge()
    captured_messages: list[dict[str, Any]] = []

    async def _acompletion(**kwargs: object) -> FakeResponse:
        captured_messages.extend(kwargs["messages"])
        return FakeResponse(FakeChoice(finish_message("done")))

    monkeypatch.setattr(litellm, "acompletion", _acompletion)

    agent = ChatAgent(bridge, system="custom prompt")
    await agent.run("do the thing")

    assert captured_messages[0] == {
        "role": "system", "content": "custom prompt"}


async def test_run_executes_a_tool_call_then_returns_the_final_answer(monkeypatch):
    bridge = FakeBridge({"session_bash": "hello world"})
    tool_call = FakeToolCall(
        "call-1", "session_bash", json.dumps({"command": "echo hi"})
    )
    monkeypatch.setattr(
        litellm,
        "acompletion",
        stub_acompletion(
            [
                FakeResponse(FakeChoice(FakeMessage(
                    None, tool_calls=[tool_call]))),
                FakeResponse(FakeChoice(finish_message(
                    "the output was hello world"))),
            ]
        ),
    )

    agent = ChatAgent(bridge)
    result = await agent.run("run echo")

    assert result == "the output was hello world"
    assert ("session_bash", {"command": "echo hi"}) in bridge.calls


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


async def test_run_recovers_from_a_failing_tool_call_and_continues(monkeypatch):
    """A tool call that raises must not abort the run: its error is reported
    back to the model as the call's result, and the loop continues."""
    bridge = FakeBridge()

    async def _failing_call(name: str, arguments: dict[str, Any] | None = None) -> str:
        raise RuntimeError("boom")

    bridge.call = _failing_call  # type: ignore[method-assign]
    tool_call = FakeToolCall(
        "call-1", "session_bash", json.dumps({"command": "echo hi"})
    )
    captured_messages: list[dict[str, Any]] = []

    async def _acompletion(**kwargs: object) -> FakeResponse:
        captured_messages[:] = kwargs["messages"]  # type: ignore[arg-type]
        if not captured_messages or captured_messages[-1]["role"] != "tool":
            return FakeResponse(FakeChoice(FakeMessage(None, tool_calls=[tool_call])))
        return FakeResponse(FakeChoice(finish_message("recovered")))

    monkeypatch.setattr(litellm, "acompletion", _acompletion)

    agent = ChatAgent(bridge)
    result = await agent.run("try something risky")

    assert result == "recovered"
    assert captured_messages[-1] == {
        "role": "tool", "tool_call_id": "call-1", "content": "error: boom"}


async def test_run_survives_invalid_tool_call_arguments(monkeypatch):
    bridge = FakeBridge()
    tool_call = FakeToolCall("call-1", "session_bash", "{not json")
    monkeypatch.setattr(
        litellm,
        "acompletion",
        stub_acompletion(
            [
                FakeResponse(FakeChoice(FakeMessage(
                    None, tool_calls=[tool_call]))),
                FakeResponse(FakeChoice(
                    finish_message("handled the bad call"))),
            ]
        ),
    )

    agent = ChatAgent(bridge)
    result = await agent.run("send garbage arguments")

    assert result == "handled the bad call"
    assert bridge.calls == []


async def test_failing_tool_calls_are_reported_once_without_retrying(monkeypatch):
    """Tool calls aren't retried: repeating an unchanged call can't fix a
    semantic failure and could re-run a non-idempotent tool's side effect.
    The model sees the error and decides what to do next, so one attempt is
    all the loop itself should make."""
    bridge = FakeBridge()
    attempts = 0

    async def _flaky_call(name: str, arguments: dict[str, Any] | None = None) -> str:
        nonlocal attempts
        attempts += 1
        raise RuntimeError("boom")

    bridge.call = _flaky_call  # type: ignore[method-assign]
    tool_call = FakeToolCall(
        "call-1", "session_bash", json.dumps({"command": "echo hi"})
    )
    monkeypatch.setattr(
        litellm,
        "acompletion",
        stub_acompletion(
            [
                FakeResponse(FakeChoice(FakeMessage(
                    None, tool_calls=[tool_call]))),
                FakeResponse(FakeChoice(finish_message("saw the error"))),
            ]
        ),
    )

    agent = ChatAgent(bridge)
    result = await agent.run("try something risky")

    assert result == "saw the error"
    assert attempts == 1


async def test_run_retries_a_flaky_llm_completion_before_succeeding(monkeypatch):
    """Unlike tool calls, LLM requests are safe to retry transparently: same
    idempotent request, and transient infra blips (timeouts, rate limits) are
    common enough to be worth a bounded retry."""
    bridge = FakeBridge()
    attempts = 0

    async def _flaky_acompletion(**kwargs: object) -> FakeResponse:
        nonlocal attempts
        attempts += 1
        if attempts < 2:
            raise RuntimeError("connection reset")
        return FakeResponse(FakeChoice(finish_message("done")))

    monkeypatch.setattr(litellm, "acompletion", _flaky_acompletion)

    agent = ChatAgent(bridge, llm_retry_base_delay=0)
    result = await agent.run("do the thing")

    assert result == "done"
    assert attempts == 2


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


async def test_local_session_bridge_raises_on_tool_errors():
    bridge = LocalSessionBridge(FakeDrunSession())

    with pytest.raises(RuntimeError, match="command failed"):
        await bridge.call("execute_bash", {"command": "boom"})


async def test_local_session_bridge_reports_unknown_tools():
    bridge = LocalSessionBridge(FakeDrunSession())

    result = await bridge.call("mystery_tool", {})

    assert result == "unknown tool: mystery_tool"

"""Tests for DrunMcpBridge's MCP <-> OpenAI function-calling translation and
its session bootstrap/attach logic."""

from __future__ import annotations

import json

import pytest
from mcp.types import CallToolResult, ListToolsResult, TextContent, Tool

from drun.mcp_bridge import DrunMcpBridge


class FakeSession:
    """Stands in for mcp's ClientSession. `results` maps a tool name to the
    CallToolResult it should return; a single CallToolResult applies to every
    call. Every call is recorded, in order, in `calls`."""

    def __init__(
        self,
        tools: list[Tool] | None = None,
        results: CallToolResult | dict[str, CallToolResult] | None = None,
    ) -> None:
        self._tools = tools or []
        self._results = results or CallToolResult(content=[])
        self.calls: list[tuple[str, dict | None]] = []

    async def list_tools(self) -> ListToolsResult:
        return ListToolsResult(tools=self._tools)

    async def call_tool(self, name: str, arguments: dict | None = None) -> CallToolResult:
        self.calls.append((name, arguments))
        if isinstance(self._results, dict):
            return self._results[name]
        return self._results

    @property
    def called_with(self) -> tuple[str, dict | None] | None:
        return self.calls[-1] if self.calls else None


def bridge_with(session: FakeSession, **kwargs: object) -> DrunMcpBridge:
    bridge = DrunMcpBridge("http://unused", **kwargs)
    bridge._session = session  # bypasses __aenter__'s real network connection
    return bridge


def ok_result(text: str = "") -> CallToolResult:
    content = [TextContent(type="text", text=text)] if text else []
    return CallToolResult(content=content)


async def test_tools_translates_mcp_tools_to_openai_function_format():
    tool = Tool(
        name="session_bash",
        description="Run a shell command",
        inputSchema={"type": "object", "properties": {
            "command": {"type": "string"}}},
    )
    bridge = bridge_with(FakeSession([tool]))

    tools = await bridge.tools()

    assert tools == [
        {
            "type": "function",
            "function": {
                "name": "session_bash",
                "description": "Run a shell command",
                "parameters": {
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                },
            },
        }
    ]


async def test_tools_defaults_a_missing_description_to_empty_string():
    tool = Tool(name="create_session", inputSchema={
                "type": "object", "properties": {}})
    bridge = bridge_with(FakeSession([tool]))

    tools = await bridge.tools()

    assert tools[0]["function"]["description"] == ""


async def test_call_joins_text_content_blocks():
    result = CallToolResult(
        content=[
            TextContent(type="text", text="line one"),
            TextContent(type="text", text="line two"),
        ]
    )
    session = FakeSession(results=result)
    bridge = bridge_with(session)

    output = await bridge.call("session_bash", {"session_id": "s1", "command": "echo hi"})

    assert output == "line one\nline two"
    assert session.called_with == (
        "session_bash", {"session_id": "s1", "command": "echo hi"})


async def test_call_returns_a_placeholder_for_empty_content():
    bridge = bridge_with(FakeSession())

    output = await bridge.call("session_close", {"session_id": "s1"})

    assert output == "(no output)"


async def test_call_sends_an_empty_dict_instead_of_none_for_no_arguments():
    session = FakeSession()
    bridge = bridge_with(session)

    await bridge.call("create_session")

    assert session.called_with == ("create_session", {})


async def test_call_raises_with_the_daemon_error_text_when_the_tool_call_fails():
    result = CallToolResult(
        isError=True,
        content=[TextContent(
            type="text", text="session limit reached (max 50)")],
    )
    bridge = bridge_with(FakeSession(results=result))

    with pytest.raises(RuntimeError, match="session limit reached"):
        await bridge.call("create_session")


async def test_call_before_entering_the_bridge_raises():
    bridge = DrunMcpBridge("http://unused")

    with pytest.raises(RuntimeError):
        await bridge.call("create_session")


async def test_bootstrap_creates_a_session_and_mounts_paths_when_no_session_id_is_given():
    session = FakeSession(results={
        "create_session": ok_result(json.dumps({"session_id": "s1"})),
        "session_mount": ok_result(),
    })
    bridge = bridge_with(session, mounts=["/tmp/data"])

    await bridge._bootstrap()

    assert bridge.session_id == "s1"
    assert session.calls == [
        ("create_session", {}),
        ("session_mount", {"session_id": "s1", "path": "/tmp/data"}),
    ]


async def test_bootstrap_attaches_to_an_existing_session_id_without_creating_one():
    session = FakeSession(results={
        "get_session_state": ok_result("{}"),
        "session_mount": ok_result(),
    })
    bridge = bridge_with(session, session_id="existing", mounts=["/tmp/data"])

    await bridge._bootstrap()

    assert bridge.session_id == "existing"
    assert session.calls == [
        ("get_session_state", {"session_id": "existing"}),
        ("session_mount", {"session_id": "existing", "path": "/tmp/data"}),
    ]


async def test_bootstrap_raises_when_the_given_session_id_does_not_exist():
    result = CallToolResult(
        isError=True,
        content=[TextContent(type="text", text="session 'missing' not found")],
    )
    session = FakeSession(results={"get_session_state": result})
    bridge = bridge_with(session, session_id="missing")

    with pytest.raises(RuntimeError, match="session 'missing' not found"):
        await bridge._bootstrap()


async def test_default_system_prompt_embeds_the_resolved_session_id():
    session = FakeSession(
        results={"create_session": ok_result(json.dumps({"session_id": "s1"}))})
    bridge = bridge_with(session)
    await bridge._bootstrap()

    assert 'session_id="s1"' in bridge.default_system_prompt


async def test_session_id_before_bootstrap_raises():
    bridge = DrunMcpBridge("http://unused")

    with pytest.raises(RuntimeError):
        bridge.session_id

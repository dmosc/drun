"""Tests for DrunMcpBridge's MCP <-> OpenAI function-calling translation."""

from __future__ import annotations

import pytest
from mcp.types import CallToolResult, ListToolsResult, TextContent, Tool

from drun.mcp_bridge import DrunMcpBridge


class FakeSession:
    def __init__(self, tools: list[Tool], call_result: CallToolResult) -> None:
        self._tools = tools
        self._call_result = call_result
        self.called_with: tuple[str, dict | None] | None = None

    async def list_tools(self) -> ListToolsResult:
        return ListToolsResult(tools=self._tools)

    async def call_tool(self, name: str, arguments: dict | None = None) -> CallToolResult:
        self.called_with = (name, arguments)
        return self._call_result


def bridge_with(session: FakeSession) -> DrunMcpBridge:
    bridge = DrunMcpBridge("http://unused")
    bridge._session = session  # bypasses __aenter__'s real network connection
    return bridge


async def test_tools_translates_mcp_tools_to_openai_function_format():
    tool = Tool(
        name="session_bash",
        description="Run a shell command",
        inputSchema={"type": "object", "properties": {
            "command": {"type": "string"}}},
    )
    bridge = bridge_with(FakeSession([tool], CallToolResult(content=[])))

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
    bridge = bridge_with(FakeSession([tool], CallToolResult(content=[])))

    tools = await bridge.tools()

    assert tools[0]["function"]["description"] == ""


async def test_call_joins_text_content_blocks():
    result = CallToolResult(
        content=[
            TextContent(type="text", text="line one"),
            TextContent(type="text", text="line two"),
        ]
    )
    session = FakeSession([], result)
    bridge = bridge_with(session)

    output = await bridge.call("session_bash", {"session_id": "s1", "command": "echo hi"})

    assert output == "line one\nline two"
    assert session.called_with == (
        "session_bash", {"session_id": "s1", "command": "echo hi"})


async def test_call_returns_a_placeholder_for_empty_content():
    bridge = bridge_with(FakeSession([], CallToolResult(content=[])))

    output = await bridge.call("session_close", {"session_id": "s1"})

    assert output == "(no output)"


async def test_call_sends_an_empty_dict_instead_of_none_for_no_arguments():
    session = FakeSession([], CallToolResult(content=[]))
    bridge = bridge_with(session)

    await bridge.call("create_session")

    assert session.called_with == ("create_session", {})


async def test_call_raises_with_the_daemon_error_text_when_the_tool_call_fails():
    result = CallToolResult(
        isError=True,
        content=[TextContent(
            type="text", text="session limit reached (max 50)")],
    )
    bridge = bridge_with(FakeSession([], result))

    with pytest.raises(RuntimeError, match="session limit reached"):
        await bridge.call("create_session")


async def test_call_before_entering_the_bridge_raises():
    bridge = DrunMcpBridge("http://unused")

    with pytest.raises(RuntimeError):
        await bridge.call("create_session")

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

    output = await bridge.call("session_bash", {"command": "echo hi"})

    assert output == "line one\nline two"
    assert session.called_with == ("session_bash", {"command": "echo hi"})


async def test_call_returns_a_placeholder_for_empty_content():
    bridge = bridge_with(FakeSession())

    output = await bridge.call("session_close")

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
        "get_system_instructions": ok_result("tool guide"),
        "session_history": ok_result("[]"),
        "session_mount": ok_result(),
    })
    bridge = bridge_with(session, mounts=["/tmp/data"])

    await bridge._bootstrap()

    assert bridge.session_id == "s1"
    assert session.calls == [
        ("create_session", {}),
        ("get_system_instructions", {}),
        ("session_history", {
            "description": "Loading session context."}),
        ("session_mount", {"path": "/tmp/data"}),
    ]


async def test_bootstrap_attaches_to_an_existing_session_id_without_creating_one():
    session = FakeSession(results={
        "session_switch": ok_result("{}"),
        "get_system_instructions": ok_result("tool guide"),
        "session_history": ok_result("[]"),
        "session_mount": ok_result(),
    })
    bridge = bridge_with(session, session_id="existing", mounts=["/tmp/data"])

    await bridge._bootstrap()

    assert bridge.session_id == "existing"
    assert session.calls == [
        ("session_switch", {"session_id": "existing"}),
        ("get_system_instructions", {}),
        ("session_history", {
            "description": "Loading session context."}),
        ("session_mount", {"path": "/tmp/data"}),
    ]


async def test_bootstrap_raises_when_the_given_session_id_matches_neither_a_session_nor_a_snapshot():
    switch_failure = CallToolResult(
        isError=True,
        content=[TextContent(type="text", text="session 'missing' not found")],
    )
    restore_failure = CallToolResult(
        isError=True,
        content=[TextContent(type="text", text="No such file or directory")],
    )
    session = FakeSession(results={
        "session_switch": switch_failure,
        "get_config": ok_result(json.dumps({"snapshots_dir": "/snaps"})),
        "session_restore": restore_failure,
    })
    bridge = bridge_with(session, session_id="missing")

    with pytest.raises(RuntimeError, match="No such file or directory"):
        await bridge._bootstrap()


async def test_bootstrap_falls_back_to_a_snapshot_when_the_session_id_is_not_active():
    session = FakeSession(results={
        "session_switch": CallToolResult(
            isError=True,
            content=[TextContent(
                type="text", text="session 'archived' not found")],
        ),
        "get_config": ok_result(json.dumps({"snapshots_dir": "/snaps"})),
        "session_restore": ok_result(json.dumps({"session_id": "restored-1"})),
        "get_system_instructions": ok_result("tool guide"),
        "session_history": ok_result("[]"),
    })
    bridge = bridge_with(session, session_id="archived")

    await bridge._bootstrap()

    assert bridge.session_id == "restored-1"
    assert session.calls == [
        ("session_switch", {"session_id": "archived"}),
        ("get_config", {}),
        ("session_restore", {"path": "/snaps/archived.drun"}),
        ("get_system_instructions", {}),
        ("session_history", {
            "description": "Loading session context."}),
    ]


async def test_default_system_prompt_embeds_the_resolved_session_id():
    session = FakeSession(results={
        "create_session": ok_result(json.dumps({"session_id": "s1"})),
        "get_system_instructions": ok_result("tool guide"),
        "session_history": ok_result("[]"),
    })
    bridge = bridge_with(session)
    await bridge._bootstrap()

    assert 'Session "s1"' in bridge.default_system_prompt


async def test_default_system_prompt_embeds_the_fetched_tool_instructions():
    session = FakeSession(results={
        "create_session": ok_result(json.dumps({"session_id": "s1"})),
        "get_system_instructions": ok_result("the always-current tool guide"),
        "session_history": ok_result("[]"),
    })
    bridge = bridge_with(session)
    await bridge._bootstrap()

    assert "the always-current tool guide" in bridge.default_system_prompt


async def test_default_system_prompt_omits_history_context_for_a_fresh_session():
    session = FakeSession(results={
        "create_session": ok_result(json.dumps({"session_id": "s1"})),
        "get_system_instructions": ok_result("tool guide"),
        "session_history": ok_result("[]"),
    })
    bridge = bridge_with(session)
    await bridge._bootstrap()

    assert "Prior actions" not in bridge.default_system_prompt


async def test_default_system_prompt_includes_prior_checkpoint_steps():
    history = [
        {"checkpoint_id": 1, "steps": [
            {"tool": "session_bash", "description": "ran the test suite"}]},
    ]
    session = FakeSession(results={
        "session_switch": ok_result("{}"),
        "get_system_instructions": ok_result("tool guide"),
        "session_history": ok_result(json.dumps(history)),
    })
    bridge = bridge_with(session, session_id="existing")
    await bridge._bootstrap()

    prompt = bridge.default_system_prompt
    assert "Prior actions already taken in this session:" in prompt
    assert "session_bash: ran the test suite" in prompt


async def test_session_id_before_bootstrap_raises():
    bridge = DrunMcpBridge("http://unused")

    with pytest.raises(RuntimeError):
        bridge.session_id

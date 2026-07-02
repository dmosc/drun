"""
conftest.py — shared pytest infrastructure for drun e2e tests.

How an e2e test works
---------------------
Each test receives a `make_drun` fixture and uses it as an async context manager:

    async def test_something(make_drun):
        async with make_drun({"domain_allowlist": ["example.com"]}) as drun:
            response = await drun.run("Fetch https://example.com and show the title.")
        assert "Example Domain" in response

Under the hood this does four things:

1. Writes a config.toml to a pytest-managed temp directory with the supplied
   overrides (any key from drun's config schema is accepted).

2. Spawns drun-mcp as a subprocess using that config (DRUN_CONFIG env var),
   communicating over stdin/stdout via the MCP protocol.

3. Runs the MCP initialize handshake and fetches the live tool list so Claude
   receives exactly the tools the running server advertises.

4. On drun.run(prompt): sends the prompt to Claude (Haiku), then loops —
   forwarding every tool_use block Claude emits to the live drun-mcp process
   and feeding the results back — until Claude returns end_turn. The final
   text response is returned and drun.tools_called holds the ordered list of
   MCP tool names that were invoked.

Prerequisites
-------------
- drun-mcp binary on PATH or built at target/debug/drun-mcp
- ANTHROPIC_API_KEY environment variable set
"""

import os
import shutil
from pathlib import Path
from typing import Any, Optional

import anthropic
import pytest
from mcp import ClientSession
from mcp.client.stdio import StdioServerParameters, stdio_client

MODEL = "claude-haiku-4-5"
MAX_TOKENS = 2048
MAX_TOOL_ROUNDS = 20


def find_drun_mcp() -> str:
    """Return the path to the drun-mcp binary, checking PATH then workspace target/."""
    if binary := shutil.which("drun-mcp"):
        return binary
    workspace_root = Path(__file__).parent.parent.parent
    for profile in ("debug", "release"):
        candidate = workspace_root / "target" / profile / "drun-mcp"
        if candidate.exists():
            return str(candidate)
    raise RuntimeError(
        "drun-mcp binary not found — run `cargo build -p drun-mcp` or install it."
    )


def write_config(tmp_path: Path, overrides: dict) -> Path:
    """Serialize a dict of drun config keys to a TOML file and return its path."""
    lines: list[str] = []
    for key, value in overrides.items():
        if isinstance(value, list):
            items = ", ".join(f'"{v}"' if isinstance(v, str) else str(v) for v in value)
            lines.append(f"{key} = [{items}]")
        elif isinstance(value, bool):
            lines.append(f"{key} = {'true' if value else 'false'}")
        elif isinstance(value, (int, float)):
            lines.append(f"{key} = {value}")
        elif isinstance(value, str):
            lines.append(f'{key} = "{value}"')
    config_path = tmp_path / "config.toml"
    config_path.write_text("\n".join(lines) + "\n")
    return config_path


class DrunHarness:
    """
    Async context manager that owns a single drun-mcp process for the duration
    of a test. Acquire it via the make_drun fixture.

    Attributes
    ----------
    tools_called : list[str]
        Ordered list of MCP tool names Claude invoked during the last run() call.
        Useful for asserting that Claude chose the right tool (e.g. session_fetch
        rather than session_bash) regardless of its final text response.
    """

    def __init__(self, config: dict, tmp_path: Path) -> None:
        self._config = config
        self._tmp_path = tmp_path
        self._stdio_cm: Any = None
        self._session_cm: Any = None
        self.session: Optional[ClientSession] = None
        self.tools: list[dict] = []
        self.tools_called: list[str] = []
        self._claude = anthropic.AsyncAnthropic()

    async def __aenter__(self) -> "DrunHarness":
        config_path = write_config(self._tmp_path, self._config)
        env = os.environ.copy()
        env["DRUN_CONFIG"] = str(config_path)

        # Spawn drun-mcp and open an MCP stdio transport to it.
        server_params = StdioServerParameters(command=find_drun_mcp(), env=env)
        self._stdio_cm = stdio_client(server_params)
        read, write = await self._stdio_cm.__aenter__()

        # Complete the MCP handshake and fetch the live tool list.
        self._session_cm = ClientSession(read, write)
        self.session = await self._session_cm.__aenter__()
        await self.session.initialize()

        result = await self.session.list_tools()
        self.tools = [
            {
                "name": t.name,
                "description": t.description or "",
                "input_schema": t.inputSchema,
            }
            for t in result.tools
        ]
        return self

    async def __aexit__(self, *args: Any) -> None:
        if self._session_cm:
            await self._session_cm.__aexit__(*args)
        if self._stdio_cm:
            await self._stdio_cm.__aexit__(*args)

    async def run(self, prompt: str) -> str:
        """
        Send prompt to Claude and drive the tool-use loop until end_turn.

        Each tool_use block Claude emits is forwarded to the live drun-mcp
        process; the result is fed back as a tool_result. The loop repeats up
        to MAX_TOOL_ROUNDS times. Returns Claude's final text response.
        """
        self.tools_called = []
        messages: list[dict] = [{"role": "user", "content": prompt}]

        for _ in range(MAX_TOOL_ROUNDS):
            response = await self._claude.messages.create(
                model=MODEL,
                max_tokens=MAX_TOKENS,
                tools=self.tools,
                messages=messages,
            )

            if response.stop_reason == "end_turn":
                return "".join(
                    block.text
                    for block in response.content
                    if hasattr(block, "text")
                )

            if response.stop_reason != "tool_use":
                break

            messages.append({"role": "assistant", "content": response.content})

            tool_results = []
            for block in response.content:
                if block.type != "tool_use":
                    continue
                self.tools_called.append(block.name)
                result = await self.session.call_tool(block.name, block.input)
                content_text = "\n".join(
                    c.text for c in result.content if hasattr(c, "text")
                )
                tool_results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": content_text,
                    }
                )
            messages.append({"role": "user", "content": tool_results})

        return ""


@pytest.fixture
def make_drun(tmp_path: Path):
    """
    Pytest fixture that returns a factory for DrunHarness instances.

    Usage:
        async with make_drun({"domain_allowlist": ["example.com"]}) as drun:
            response = await drun.run("...")

    Each call to make_drun() produces an independent harness with its own
    drun-mcp process and config, isolated in a pytest-managed temp directory.
    """
    def _make(config: Optional[dict] = None) -> DrunHarness:
        return DrunHarness(config or {}, tmp_path)
    return _make

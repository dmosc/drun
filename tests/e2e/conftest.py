"""
conftest.py — shared pytest infrastructure for drun e2e tests.

How an e2e test works
---------------------
Each test receives a `make_drun` fixture and uses it as an async context manager:

    async def test_something(make_drun):
        async with make_drun({"domain_allowlist": ["example.com"]}) as drun:
            response = await drun.run("Fetch https://example.com and show the title.")
        assert "Example Domain" in response

Under the hood this does five things:

1. Writes a config.toml to a pytest-managed temp directory with the supplied
   overrides (any key from drun's Config schema is accepted).

2. Runs `drun-mcp init` in that temp directory, which writes
   .claude/settings.json with the project's deny/allow lists (the same file
   a real user gets when they initialise drun in a project).

3. Starts drun-mcp as a subprocess using the test config (DRUN_CONFIG env
   var). The server is a singleton that listens on http://127.0.0.1:7273 and
   exposes both SSE (/sse) and streamable-HTTP (/mcp) transports. It is
   killed when the context manager exits.

4. Waits until the server is ready (polls /mcp until any HTTP response
   arrives), then opens an MCP SSE session and completes the initialize
   handshake, fetching the live tool list.

5. On drun.run(prompt): sends the prompt to Claude (Haiku), then loops —
   forwarding every tool_use block Claude emits to the live drun-mcp process
   and feeding the results back — until Claude returns end_turn. Before each
   forward, the deny list from .claude/settings.json is enforced: exact tool
   name matches are blocked outright; for session_bash, any command
   containing a non-tool-name deny entry (e.g. "curl", "wget") is also
   blocked. The final text response is returned and drun.tools_called holds
   the ordered list of MCP tool names that were invoked.

Prerequisites
-------------
- drun-mcp binary on PATH or built at target/debug/drun-mcp
- ANTHROPIC_API_KEY environment variable set
- Tests must run sequentially: drun-mcp always binds port 7273, so parallel
  test execution would cause port conflicts. The default pytest runner (no
  -n flag) satisfies this.
"""

import asyncio
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any, Optional

import anthropic
import httpx
import pytest
from mcp import ClientSession
from mcp.client.sse import sse_client

MODEL = "claude-haiku-4-5"
MAX_TOKENS = 2048
MAX_TOOL_ROUNDS = 20
MCP_PORT = 7273
_MCP_SSE_URL = f"http://127.0.0.1:{MCP_PORT}/sse"
_MCP_HTTP_URL = f"http://127.0.0.1:{MCP_PORT}/mcp"
_READY_POLL_INTERVAL = 0.1
_READY_TIMEOUT = 10.0
_LAUNCHD_PLIST = Path.home() / "Library/LaunchAgents/com.drun.mcp-server.plist"

# Claude Code native tool names that appear in the deny list. Anything in
# the deny list that is NOT one of these is treated as a bash command pattern
# to block within session_bash calls.
_CLAUDE_CODE_TOOLS = {
    "Bash", "BashOutput", "KillBash", "Edit", "Write", "NotebookEdit",
    "Read", "Glob", "Grep", "WebFetch", "WebSearch", "Task",
}


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


def load_deny_list(settings_path: Path) -> tuple[set[str], list[str]]:
    """Parse .claude/settings.json and return (denied_tool_names, bash_patterns).

    denied_tool_names: MCP tool names that should not be forwarded to the server.
    bash_patterns: strings that, if found in a session_bash command, cause the
                   call to be blocked before it reaches the server.
    """
    if not settings_path.exists():
        return set(), []
    try:
        settings = json.loads(settings_path.read_text())
        deny = settings.get("permissions", {}).get("deny", [])
    except Exception:
        return set(), []
    denied_tools = {d for d in deny if d in _CLAUDE_CODE_TOOLS}
    bash_patterns = [d for d in deny if d not in _CLAUDE_CODE_TOOLS]
    return denied_tools, bash_patterns


async def _wait_for_server(timeout: float = _READY_TIMEOUT) -> None:
    """Poll /mcp with a GET until any HTTP response arrives, then return."""
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            try:
                await client.get(_MCP_HTTP_URL, timeout=1.0)
                return
            except (httpx.ConnectError, httpx.TimeoutException):
                await asyncio.sleep(_READY_POLL_INTERVAL)
    raise RuntimeError(
        f"drun-mcp did not become ready at {_MCP_HTTP_URL} within {timeout}s"
    )


class DrunHarness:
    """
    Async context manager that owns a single drun-mcp process for the duration
    of a test. Acquire it via the make_drun fixture.

    The server starts with the test's config, binds port 7273, and is killed
    when the context manager exits. Because the port is fixed, tests must run
    sequentially.

    Attributes
    ----------
    tools_called : list[str]
        Ordered list of MCP tool names Claude invoked during the last run() call.
    """

    def __init__(self, config: dict, tmp_path: Path) -> None:
        self._config = config
        self._tmp_path = tmp_path
        self._process: Optional[subprocess.Popen] = None
        self._sse_cm: Any = None
        self._session_cm: Any = None
        self.session: Optional[ClientSession] = None
        self.tools: list[dict] = []
        self.tools_called: list[str] = []
        self._denied_tools: set[str] = set()
        self._bash_patterns: list[str] = []
        self._claude = anthropic.AsyncAnthropic()

    async def __aenter__(self) -> "DrunHarness":
        config_path = write_config(self._tmp_path, self._config)
        env = os.environ.copy()
        env["DRUN_CONFIG"] = str(config_path)

        # Run init so the test environment matches a real initialised project:
        # this writes .claude/settings.json with the deny/allow lists.
        subprocess.run(
            [find_drun_mcp(), "init"],
            cwd=self._tmp_path,
            env=env,
            capture_output=True,
        )
        self._denied_tools, self._bash_patterns = load_deny_list(
            self._tmp_path / ".claude" / "settings.json"
        )

        # Start the singleton drun-mcp server.
        self._process = subprocess.Popen(
            [find_drun_mcp()],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        await _wait_for_server()

        # Open an MCP session over SSE and complete the initialize handshake.
        self._sse_cm = sse_client(_MCP_SSE_URL)
        read, write = await self._sse_cm.__aenter__()
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
        if self._sse_cm:
            await self._sse_cm.__aexit__(*args)
        if self._process:
            self._process.terminate()
            try:
                self._process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._process.kill()

    def _check_deny(self, tool_name: str, tool_input: dict) -> Optional[str]:
        """Return an error string if this call should be blocked, else None."""
        if tool_name in self._denied_tools:
            return f"tool '{tool_name}' is denied by project settings"
        if tool_name == "session_bash" and self._bash_patterns:
            cmd = tool_input.get("command", "")
            for pattern in self._bash_patterns:
                if pattern.lower() in cmd.lower():
                    return f"command contains denied pattern '{pattern}'"
        return None

    async def run(self, prompt: str) -> str:
        """
        Send prompt to Claude and drive the tool-use loop until end_turn.

        Tool calls are checked against the deny list loaded from
        .claude/settings.json before being forwarded to the server.
        Returns Claude's final text response.
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

                deny_reason = self._check_deny(block.name, block.input or {})
                if deny_reason:
                    content_text = f"Error: {deny_reason}"
                else:
                    result = await self.session.call_tool(block.name, block.input)
                    content_text = "\n".join(
                        c.text for c in result.content if hasattr(c, "text")
                    )

                tool_results.append({
                    "type": "tool_result",
                    "tool_use_id": block.id,
                    "content": content_text,
                })
            messages.append({"role": "user", "content": tool_results})

        return ""


@pytest.fixture(scope="session", autouse=True)
def _manage_daemon():
    """Stop the launchd-managed drun-mcp daemon for the test session.

    drun-mcp always binds port 7273. If the system daemon holds that port,
    per-test servers can't start. This fixture unloads the plist before any
    test runs and reloads it afterward so the daemon is restored when done.
    """
    plist = _LAUNCHD_PLIST
    daemon_present = plist.exists()
    if daemon_present:
        subprocess.run(["launchctl", "unload", str(plist)], capture_output=True)
        time.sleep(0.5)
    try:
        yield
    finally:
        if daemon_present:
            subprocess.run(["launchctl", "load", str(plist)], capture_output=True)


@pytest.fixture
def make_drun(tmp_path: Path):
    """
    Pytest fixture that returns a factory for DrunHarness instances.

    Usage:
        async with make_drun({"domain_allowlist": ["example.com"]}) as drun:
            response = await drun.run("...")

    Each call runs `drun-mcp init`, starts a fresh server on port 7273 with
    the given config, and tears both down when the context manager exits.
    Run tests sequentially (the default runner, no -n flag) to avoid port
    conflicts between tests.
    """
    def _make(config: Optional[dict] = None) -> DrunHarness:
        return DrunHarness(config or {}, tmp_path)
    return _make

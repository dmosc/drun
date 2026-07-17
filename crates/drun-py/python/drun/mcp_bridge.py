"""MCP client for the drun-mcp daemon, exposing its tools to an LLM."""

from __future__ import annotations

import json
from contextlib import AsyncExitStack
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from mcp import ClientSession


class DrunMcpBridge:
    """Connects to a running drun-mcp daemon over streamable HTTP, bootstraps
    a sandbox session, and proxies an LLM's tool calls to the daemon's full tool
    suite.
    """

    _SYSTEM_PROMPT_TEMPLATE = """\
You are a coding assistant with access to a sandboxed execution environment through \
drun's tools. Session "{session_id}" is already created, with any requested paths \
mounted — pass session_id="{session_id}" to every session_* tool call.

Use session_bash for shell commands, session_read_file/session_write_file/
session_delete_file for file access, session_mount to load more host paths, and
session_fetch for network requests (subject to the server's domain allowlist). Call
create_session yourself only if you need a second, independent sandbox.
"""

    def __init__(
        self,
        url: str,
        *,
        session_id: str | None = None,
        mounts: list[str] | None = None,
    ) -> None:
        self._url = url
        self._requested_session_id = session_id
        self._mounts = mounts or []
        self._exit_stack = AsyncExitStack()
        self._session: ClientSession | None = None
        self._session_id: str | None = None

    async def __aenter__(self) -> DrunMcpBridge:
        try:
            from mcp import ClientSession
            from mcp.client.streamable_http import streamable_http_client
        except ImportError as exc:
            raise ImportError(
                "mcp is required for drun chat. "
                "Install it with: pip install 'drun-sandbox[chat]'"
            ) from exc

        try:
            read_stream, write_stream, _ = await self._exit_stack.enter_async_context(
                streamable_http_client(self._url)
            )
            self._session = await self._exit_stack.enter_async_context(
                ClientSession(read_stream, write_stream)
            )
            await self._session.initialize()
            await self._bootstrap()
        except BaseException:
            # __aenter__ raising means Python will never call our __aexit__, so
            # any nested context already pushed onto the exit stack (the HTTP
            # connection, the ClientSession) would otherwise leak.
            await self._exit_stack.aclose()
            raise
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        await self._exit_stack.aclose()

    @property
    def session_id(self) -> str:
        if self._session_id is None:
            raise RuntimeError(
                "DrunMcpBridge must be entered with 'async with' before use")
        return self._session_id

    @property
    def default_system_prompt(self) -> str:
        return self._SYSTEM_PROMPT_TEMPLATE.format(session_id=self.session_id)

    async def tools(self) -> list[dict[str, Any]]:
        """The daemon's tools, translated to OpenAI function-calling format."""
        result = await self._require_session().list_tools()
        return [
            {
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description or "",
                    "parameters": tool.inputSchema,
                },
            }
            for tool in result.tools
        ]

    async def call(self, name: str, arguments: dict[str, Any] | None = None) -> str:
        result = await self._require_session().call_tool(name, arguments or {})
        text = "\n".join(
            block.text for block in result.content if block.type == "text")
        if result.isError:
            raise RuntimeError(
                f"drun tool '{name}' failed: {text or '(no error message)'}")
        return text or "(no output)"

    async def _bootstrap(self) -> None:
        """Attach to `session_id` if one was requested, else create a fresh
        session, then mount every requested host path into it."""
        self._session_id = await self._resolve_session_id()
        for path in self._mounts:
            await self.call("session_mount", {"session_id": self._session_id, "path": path})

    async def _resolve_session_id(self) -> str:
        if self._requested_session_id is not None:
            # Fails fast with a clear daemon error if the session doesn't exist.
            await self.call("get_session_state", {"session_id": self._requested_session_id})
            return self._requested_session_id
        created = await self.call("create_session")
        return json.loads(created)["session_id"]

    def _require_session(self) -> ClientSession:
        if self._session is None:
            raise RuntimeError(
                "DrunMcpBridge must be entered with 'async with' before use")
        return self._session

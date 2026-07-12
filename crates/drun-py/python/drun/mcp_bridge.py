"""MCP client for the drun-mcp daemon, exposing its tools to an LLM."""

from __future__ import annotations

from contextlib import AsyncExitStack
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from mcp import ClientSession


class DrunMcpBridge:
    """Connects to a running drun-mcp daemon over streamable HTTP.

    Discovers the daemon's full tool suite and proxies calls to it, so an LLM
    driven through this bridge has the same capabilities as Claude Code.
    """

    def __init__(self, url: str) -> None:
        self._url = url
        self._exit_stack = AsyncExitStack()
        self._session: ClientSession | None = None

    async def __aenter__(self) -> DrunMcpBridge:
        try:
            from mcp import ClientSession
            from mcp.client.streamable_http import streamable_http_client
        except ImportError as exc:
            raise ImportError(
                "mcp is required for drun chat. "
                "Install it with: pip install 'drun-sandbox[chat]'"
            ) from exc

        read_stream, write_stream, _ = await self._exit_stack.enter_async_context(
            streamable_http_client(self._url)
        )
        self._session = await self._exit_stack.enter_async_context(
            ClientSession(read_stream, write_stream)
        )
        await self._session.initialize()
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        await self._exit_stack.aclose()

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

    def _require_session(self) -> ClientSession:
        if self._session is None:
            raise RuntimeError(
                "DrunMcpBridge must be entered with 'async with' before use")
        return self._session

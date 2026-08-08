"""MCP client for the drun-mcp daemon, exposing its tools to an LLM."""

from __future__ import annotations

import json
import os
from contextlib import AsyncExitStack
from typing import TYPE_CHECKING, Any

from .session_history import SessionHistoryContext

if TYPE_CHECKING:
    from mcp import ClientSession


class DrunMcpBridge:
    """Connects to a running drun-mcp daemon over streamable HTTP, bootstraps
    a sandbox session, and proxies an LLM's tool calls to the daemon's full tool
    suite.
    """

    _SESSION_PREAMBLE = """\
You are a coding assistant with access to a sandboxed execution environment through \
drun's tools. Session "{session_id}" is already created and active, with any requested \
paths mounted; session_* tool calls apply to it automatically, no need to remember or \
re-pass session_id yourself.
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
        self._tool_instructions = ""
        self._history_context = ""

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
            read_stream, write_stream = await self._exit_stack.enter_async_context(
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
        prompt = self._SESSION_PREAMBLE.format(session_id=self.session_id)
        prompt += f"\n{self._tool_instructions}\n"
        if self._history_context:
            prompt += f"\n{self._history_context}\n"
        return prompt

    async def tools(self) -> list[dict[str, Any]]:
        """The daemon's tools, translated to OpenAI function-calling format."""
        result = await self._require_session().list_tools()
        return [
            {
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description or "",
                    "parameters": tool.input_schema,
                },
            }
            for tool in result.tools
        ]

    async def call(self, name: str, arguments: dict[str, Any] | None = None) -> str:
        result = await self._require_session().call_tool(name, arguments or {})
        text = "\n".join(
            block.text for block in result.content if block.type == "text")
        if result.is_error:
            raise RuntimeError(
                f"drun tool '{name}' failed: {text or '(no error message)'}")
        return text or "(no output)"

    async def _bootstrap(self) -> None:
        """Attach to `session_id` if one was requested, else create a fresh
        session, fetch the always-current tool guide and prior checkpoint
        history for the system prompt, then mount every requested host path
        into it."""
        self._session_id = await self._resolve_session_id()
        self._tool_instructions = await self.call("get_system_instructions")
        self._history_context = await self._load_history_context()
        for path in self._mounts:
            await self.call("session_mount", {"path": path})

    async def _load_history_context(self) -> str:
        raw = await self.call(
            "session_history",
            {"description": "Loading session context."},
        )
        return SessionHistoryContext.from_json(raw).as_prompt_block()

    async def _resolve_session_id(self) -> str:
        if self._requested_session_id is not None:
            try:
                await self.call("session_switch", {"session_id": self._requested_session_id})
                return self._requested_session_id
            except RuntimeError:
                print(
                    f'Session {self._requested_session_id} not found in active session; checking in snapshots_dir')
                config = json.loads(await self.call("get_config"))
                snapshot_path = os.path.join(
                    config["snapshots_dir"], f"{self._requested_session_id}.drun")
                return await self._restore_from_snapshot(snapshot_path)
        created = await self.call("create_session")
        return json.loads(created)["session_id"]

    async def _restore_from_snapshot(self, path: str) -> str:
        restored = await self.call("session_restore", {"path": path})
        return json.loads(restored)["session_id"]

    def _require_session(self) -> ClientSession:
        if self._session is None:
            raise RuntimeError(
                "DrunMcpBridge must be entered with 'async with' before use")
        return self._session

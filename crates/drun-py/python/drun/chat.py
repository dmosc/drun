"""Tool-calling agent loop shared by the `drun chat` CLI and the Python SDK.

`ChatAgent` drives the LLM <-> tool loop against anything satisfying `Bridge`:
`DrunMcpBridge` (crates/drun-py/python/drun/mcp_bridge.py) proxies a running
drun-mcp daemon's full tool suite for the CLI; `LocalSessionBridge` below
wraps an in-process `Session` for scripting a chat loop with no daemon
required, at the cost of a smaller, fixed tool set (execute_bash, write_file).
"""

from __future__ import annotations

import json
import sys
from typing import TYPE_CHECKING, Any, Protocol

from .retry import RetryPolicy

if TYPE_CHECKING:
    from .drun_internal import DrunSession

_FINISH_TOOL_NAME = "finish_trajectory"
_FINISH_TOOL: dict[str, Any] = {
    "type": "function",
    "function": {
        "name": _FINISH_TOOL_NAME,
        "description": (
            "Call this once, when and only when the task is fully complete, "
            "with your final response. A plain-text reply does not end the "
            "run — you'll be prompted again until you call this."
        ),
        "parameters": {
            "type": "object",
            "properties": {"content": {"type": "string", "description": "The final response."}},
            "required": ["content"],
        },
    },
}


class Bridge(Protocol):
    """Lists tool schemas, executes tool calls, and supplies a default system
    prompt. Implemented by `DrunMcpBridge` and `LocalSessionBridge`. `call`
    raises on failure; `ChatAgent` reports that failure to the model rather
    than retrying, since retrying an unchanged call can't fix a semantic
    error and could repeat a non-idempotent tool's side effect."""

    @property
    def default_system_prompt(self) -> str: ...

    async def tools(self) -> list[dict[str, Any]]: ...

    async def call(self, name: str,
                   arguments: dict[str, Any] | None = None) -> str: ...


class ChatAgent:
    """Runs a tool-calling loop between an LLM (via litellm) and a `Bridge`."""

    def __init__(
        self,
        bridge: Bridge,
        *,
        model: str = "ollama_chat/qwen3.6:latest",
        base_url: str | None = None,
        system: str | None = None,
        max_iterations: int = 30,
        llm_retries: int = 3,
        llm_retry_base_delay: float = 0.5,
        reasoning_effort: str | None = None,
    ) -> None:
        self._bridge = bridge
        self._model = model
        self._base_url = base_url
        self._system = system
        self._max_iterations = max_iterations
        self._completion_retry = RetryPolicy(
            attempts=llm_retries, base_delay=llm_retry_base_delay)
        self._reasoning_effort = reasoning_effort

    async def run(self, prompt: str) -> str:
        litellm = self._import_litellm()
        tools = [*await self._bridge.tools(), _FINISH_TOOL]
        messages: list[dict[str, Any]] = [
            {"role": "system", "content": self._system or self._bridge.default_system_prompt},
            {"role": "user", "content": prompt},
        ]

        for _ in range(self._max_iterations):
            message = await self._completion_retry.run(
                lambda: self._complete(litellm, messages, tools)
            )
            messages.append(self._message_to_dict(message))

            finish_call = next(
                (tc for tc in message.tool_calls or []
                 if tc.function.name == _FINISH_TOOL_NAME),
                None,
            )
            if finish_call is not None:
                return self._finish_content(finish_call)

            for tool_call in message.tool_calls or []:
                messages.append(await self._execute_tool_call(tool_call))

        return "(max iterations reached)"

    async def _execute_tool_call(self, tool_call: Any) -> dict[str, Any]:
        """Runs one tool call and returns its result as a tool-role message.
        Bad arguments and bridge failures are caught and reported back to the
        model as the call's content instead of propagating, so a single
        failure can't end the run."""
        name = tool_call.function.name
        try:
            arguments = json.loads(tool_call.function.arguments)
        except json.JSONDecodeError as exc:
            return self._tool_result(tool_call.id, f"error: invalid arguments ({exc})")

        print(f"[{name}] {arguments}", file=sys.stderr)
        try:
            content = await self._bridge.call(name, arguments)
        except Exception as exc:
            print(f"[{name}] failed: {exc}", file=sys.stderr)
            content = f"error: {exc}"
        return self._tool_result(tool_call.id, content)

    @staticmethod
    def _tool_result(tool_call_id: str, content: str) -> dict[str, Any]:
        return {"role": "tool", "tool_call_id": tool_call_id, "content": content}

    @staticmethod
    def _import_litellm() -> Any:
        try:
            import litellm
        except ImportError as exc:
            raise ImportError(
                "litellm is required for drun chat. "
                "Install it with: pip install 'drun-sandbox[chat]'"
            ) from exc
        return litellm

    async def _complete(
        self, litellm: Any, messages: list[dict[str, Any]], tools: list[dict[str, Any]]
    ) -> Any:
        reasoning_kwargs = (
            {"reasoning_effort": self._reasoning_effort}
            if self._reasoning_effort is not None
            else {}
        )
        response = await litellm.acompletion(
            model=self._model,
            messages=messages,
            tools=tools,
            base_url=self._base_url,
            _skip_mcp_handler=True,
            **reasoning_kwargs,
        )
        return response.choices[0].message

    @staticmethod
    def _message_to_dict(message: Any) -> dict[str, Any]:
        message_dict: dict[str, Any] = {
            "role": "assistant", "content": message.content}
        if message.tool_calls:
            message_dict["tool_calls"] = [
                {
                    "id": tool_call.id,
                    "type": "function",
                    "function": {
                        "name": tool_call.function.name,
                        "arguments": tool_call.function.arguments,
                    },
                }
                for tool_call in message.tool_calls
            ]
        return message_dict

    @staticmethod
    def _finish_content(tool_call: Any) -> str:
        print(f"[{_FINISH_TOOL_NAME}] {tool_call.function.arguments}",
              file=sys.stderr)
        try:
            content = json.loads(
                tool_call.function.arguments).get("content", "")
        except json.JSONDecodeError:
            content = tool_call.function.arguments
        print(content)
        return content


class LocalSessionBridge:
    """Adapts an in-process `Session` to the `Bridge` protocol, so a script can
    drive a `ChatAgent` directly against an embedded sandbox with no drun-mcp
    daemon running. Exposes a fixed, minimal tool set — execute_bash and
    write_file — rather than the daemon's full suite."""

    _TOOLS: list[dict[str, Any]] = [
        {
            "type": "function",
            "function": {
                "name": "execute_bash",
                "description": (
                    "Run a shell command in the sandboxed session workspace. "
                    "The host PATH is available (python3, node, etc). No network access."
                ),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "Shell command to run"},
                    },
                    "required": ["command"],
                },
            },
        },
        {
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write a file into the session workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path relative to the session root",
                        },
                        "content": {"type": "string", "description": "Text content to write"},
                    },
                    "required": ["path", "content"],
                },
            },
        },
    ]

    default_system_prompt = """\
You are a helpful coding assistant with access to a sandboxed server-side execution environment.

Environment facts:
- Linux/macOS sandbox with a shell; any binary on the host PATH is available (python3, node, etc)
- Files persist across tool calls inside the session workspace
- No network access from within the sandbox
- No package-install mechanism; only packages/virtualenvs the host explicitly mounted are usable

Rules:
- Use execute_bash for everything: shell commands, scripts, and one-off code via e.g. `python3 -c "..."`
- Use write_file to create files; read them back via execute_bash (cat, python3, etc.)
- Do NOT call write_file or any drun tool as a function inside code run by execute_bash
- Work step-by-step: run code, check output, then continue
"""

    def __init__(self, session: "DrunSession") -> None:
        self._session = session

    async def tools(self) -> list[dict[str, Any]]:
        return self._TOOLS

    async def call(self, name: str, arguments: dict[str, Any] | None = None) -> str:
        arguments = arguments or {}
        if name == "execute_bash":
            checkpoint = self._session.execute_bash(arguments["command"])
            return self._format_checkpoint(checkpoint.stdout, checkpoint.stderr)
        if name == "write_file":
            self._session.write_file(
                arguments["path"], arguments["content"].encode())
            return f"wrote {arguments['path']}"
        return f"unknown tool: {name}"

    @staticmethod
    def _format_checkpoint(stdout: str, stderr: str) -> str:
        parts = []
        if stdout:
            parts.append(f"stdout:\n{stdout.rstrip()}")
        if stderr:
            parts.append(f"stderr:\n{stderr.rstrip()}")
        return "\n".join(parts) if parts else "(no output)"

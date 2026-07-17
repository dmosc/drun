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

if TYPE_CHECKING:
    from .drun_internal import DrunSession


class Bridge(Protocol):
    """Lists tool schemas, executes tool calls, and supplies a default system
    prompt. Implemented by `DrunMcpBridge` and `LocalSessionBridge`."""

    default_system_prompt: str

    async def tools(self) -> list[dict[str, Any]]: ...

    async def call(self, name: str,
                   arguments: dict[str, Any] | None = None) -> str: ...


class ChatAgent:
    """Runs a tool-calling loop between an LLM (via litellm) and a `Bridge`."""

    def __init__(
        self,
        bridge: Bridge,
        *,
        model: str = "ollama_chat/qwen2.5:14b",
        base_url: str | None = None,
        system: str | None = None,
        max_iterations: int = 30,
    ) -> None:
        self._bridge = bridge
        self._model = model
        self._base_url = base_url
        self._system = system
        self._max_iterations = max_iterations

    async def run(self, prompt: str) -> str:
        tools = await self._bridge.tools()
        messages: list[dict[str, Any]] = [
            {"role": "system", "content": self._system or self._bridge.default_system_prompt},
            {"role": "user", "content": prompt},
        ]

        for _ in range(self._max_iterations):
            message, finish_reason = await self._complete(messages, tools)
            messages.append(self._message_to_dict(message))

            if not message.tool_calls:
                return self._final_answer(message, finish_reason)

            for tool_call in message.tool_calls:
                arguments = json.loads(tool_call.function.arguments)
                print(f"[{tool_call.function.name}] {arguments}",
                      file=sys.stderr)
                result = await self._bridge.call(tool_call.function.name, arguments)
                messages.append(
                    {"role": "tool", "tool_call_id": tool_call.id, "content": result}
                )

        return "(max iterations reached)"

    async def _complete(
        self, messages: list[dict[str, Any]], tools: list[dict[str, Any]]
    ) -> tuple[Any, str]:
        try:
            import litellm
        except ImportError as exc:
            raise ImportError(
                "litellm is required for drun chat. "
                "Install it with: pip install 'drun-sandbox[chat]'"
            ) from exc

        response = await litellm.acompletion(
            model=self._model, messages=messages, tools=tools, base_url=self._base_url
        )
        choice = response.choices[0]
        return choice.message, choice.finish_reason

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
    def _final_answer(message: Any, finish_reason: str) -> str:
        # Thinking models (Qwen3, DeepSeek-R1) may put reasoning in
        # reasoning_content and leave content empty.
        answer = message.content or getattr(
            message, "reasoning_content", None) or ""
        if not answer:
            print(
                f"[drun] model returned empty content (finish_reason={finish_reason!r}). "
                "Try a non-thinking model such as ollama_chat/qwen2.5:14b.",
                file=sys.stderr,
            )
        print(answer)
        return answer


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
        try:
            if name == "execute_bash":
                checkpoint = self._session.execute_bash(arguments["command"])
                return self._format_checkpoint(checkpoint.stdout, checkpoint.stderr)
            if name == "write_file":
                self._session.write_file(
                    arguments["path"], arguments["content"].encode())
                return f"wrote {arguments['path']}"
        except Exception as exc:
            return f"error: {exc}"

        return f"unknown tool: {name}"

    @staticmethod
    def _format_checkpoint(stdout: str, stderr: str) -> str:
        parts = []
        if stdout:
            parts.append(f"stdout:\n{stdout.rstrip()}")
        if stderr:
            parts.append(f"stderr:\n{stderr.rstrip()}")
        return "\n".join(parts) if parts else "(no output)"

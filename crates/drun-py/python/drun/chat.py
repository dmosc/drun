"""Core agent loop for drun chat."""

from __future__ import annotations

import json
import sys
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from .drun_internal import DrunSession

SYSTEM_PROMPT = """\
You are a helpful coding assistant with access to a sandboxed server-side execution environment.

Environment facts:
- Native CPython (NOT Pyodide, NOT a browser runtime, NOT WebAssembly)
- Standard library and pip-installed packages available
- Files persist across tool calls inside the session workspace
- No network access from within the sandbox

Rules:
- Use execute_python to run Python code; variables and imports carry over between calls
- Use execute_bash for shell commands (ls, cat, grep, etc.)
- Use install_package before importing third-party packages
- Use write_file to create files; read them back via execute_bash or execute_python open()
- Do NOT call write_file or any drun tool as a Python function inside execute_python code
- Work step-by-step: run code, check output, then continue
"""

TOOLS: list[dict[str, Any]] = [
    {
        "type": "function",
        "function": {
            "name": "execute_python",
            "description": (
                "Execute Python code in the sandboxed session. "
                "Variables and imports persist between calls."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "code": {"type": "string", "description": "Python code to execute"},
                },
                "required": ["code"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "execute_bash",
            "description": "Run a shell command in the sandboxed session workspace. No network access.",
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
            "name": "install_package",
            "description": "Install a Python package in the session environment via pip.",
            "parameters": {
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Package name, e.g. 'pandas' or 'requests==2.31.0'",
                    },
                },
                "required": ["package"],
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


def _format_checkpoint(stdout: str, stderr: str) -> str:
    parts = []
    if stdout:
        parts.append(f"stdout:\n{stdout.rstrip()}")
    if stderr:
        parts.append(f"stderr:\n{stderr.rstrip()}")
    return "\n".join(parts) if parts else "(no output)"


def _dispatch(tool_name: str, args: dict[str, Any], session: "DrunSession") -> str:
    if tool_name == "execute_python":
        try:
            cp = session.execute_python(args["code"])
            return _format_checkpoint(cp.stdout, cp.stderr)
        except Exception as e:
            return f"error: {e}"

    if tool_name == "execute_bash":
        try:
            cp = session.execute_bash(args["command"])
            return _format_checkpoint(cp.stdout, cp.stderr)
        except Exception as e:
            return f"error: {e}"

    if tool_name == "install_package":
        try:
            session.install(args["package"])
            return f"installed {args['package']}"
        except Exception as e:
            return f"error: {e}"

    if tool_name == "write_file":
        try:
            session.write_file(args["path"], args["content"].encode())
            return f"wrote {args['path']}"
        except Exception as e:
            return f"error: {e}"

    return f"unknown tool: {tool_name}"


def run(
    session: "DrunSession",
    prompt: str,
    *,
    model: str = "ollama/qwen2.5:14b",
    base_url: str | None = None,
    system: str | None = None,
    max_iterations: int = 30,
) -> str:
    try:
        import litellm
    except ImportError as exc:
        raise ImportError(
            "litellm is required for drun chat. "
            "Install it with: pip install 'drun-sandbox[chat]'"
        ) from exc

    messages: list[Any] = [
        {"role": "system", "content": system or SYSTEM_PROMPT},
        {"role": "user", "content": prompt},
    ]

    kwargs: dict[str, Any] = {"model": model,
                              "messages": messages, "tools": TOOLS}
    if base_url:
        kwargs["base_url"] = base_url

    for _ in range(max_iterations):
        response = litellm.completion(**kwargs)
        choice = response.choices[0]
        msg = choice.message

        # Must be a plain dict in exact OpenAI wire format; litellm's Message object
        # serializes inconsistently across backends and breaks tool_call_id association.
        msg_dict: dict[str, Any] = {
            "role": "assistant", "content": msg.content}
        if msg.tool_calls:
            msg_dict["tool_calls"] = [
                {
                    "id": tc.id,
                    "type": "function",
                    "function": {"name": tc.function.name, "arguments": tc.function.arguments},
                }
                for tc in msg.tool_calls
            ]
        messages.append(msg_dict)

        if msg.tool_calls:
            for tc in msg.tool_calls:
                args = json.loads(tc.function.arguments)
                label = ", ".join(
                    f"{k}={repr(v)[:80]}" for k, v in args.items())
                print(f"[{tc.function.name}] {label}", file=sys.stderr)
                result = _dispatch(tc.function.name, args, session)
                messages.append(
                    {"role": "tool", "tool_call_id": tc.id, "content": result})
        else:
            # Thinking models (Qwen3, DeepSeek-R1) may put reasoning in reasoning_content
            # and leave content empty.
            final = msg.content or getattr(
                msg, "reasoning_content", None) or ""
            if not final:
                print(
                    f"[drun] model returned empty content "
                    f"(finish_reason={choice.finish_reason!r}). "
                    "Try a non-thinking model such as ollama/qwen2.5:14b.",
                    file=sys.stderr,
                )
            print(final)
            return final

    return "(max iterations reached)"

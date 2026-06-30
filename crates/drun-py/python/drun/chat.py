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
- Linux/macOS sandbox with a shell and python3 on PATH
- Files persist across tool calls inside the session workspace
- No network access from within the sandbox
- pip-installed packages are NOT available unless the host mounted a virtualenv;
  there is no way to install new packages from inside the sandbox

Rules:
- Use execute_bash for everything: shell commands, and Python via `python3 -c "..."`
  or by writing a script with write_file and running `python3 script.py`
- Use write_file to create files; read them back via execute_bash (cat, python3, etc.)
- Do NOT call write_file or any drun tool as a Python function inside python3 code
- Work step-by-step: run code, check output, then continue
"""

TOOLS: list[dict[str, Any]] = [
    {
        "type": "function",
        "function": {
            "name": "execute_bash",
            "description": (
                "Run a shell command in the sandboxed session workspace. "
                "Use this for shell commands and to run Python via python3. "
                "No network access."
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


def _format_checkpoint(stdout: str, stderr: str) -> str:
    parts = []
    if stdout:
        parts.append(f"stdout:\n{stdout.rstrip()}")
    if stderr:
        parts.append(f"stderr:\n{stderr.rstrip()}")
    return "\n".join(parts) if parts else "(no output)"


def _dispatch(tool_name: str, args: dict[str, Any], session: "DrunSession") -> str:
    if tool_name == "execute_bash":
        try:
            cp = session.execute_bash(args["command"])
            return _format_checkpoint(cp.stdout, cp.stderr)
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

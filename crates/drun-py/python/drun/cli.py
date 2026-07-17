"""Command-line entry point: `drun chat <prompt>`."""
from __future__ import annotations

import argparse
import asyncio
import sys

from .chat import ChatAgent
from .mcp_bridge import DrunMcpBridge

DEFAULT_MCP_URL = "http://127.0.0.1:7273/mcp"


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="drun",
        description="drun — sandboxed code execution for agentic loops",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    chat_parser = subparsers.add_parser(
        "chat", help="Run an LLM agent against a running drun-mcp daemon")
    chat_parser.add_argument("prompt", help="Task prompt for the agent")
    chat_parser.add_argument(
        "--mcp-url",
        default=DEFAULT_MCP_URL,
        metavar="URL",
        help="drun-mcp daemon endpoint. Default: %(default)s",
    )
    chat_parser.add_argument(
        "--model",
        default="ollama_chat/qwen2.5:14b",
        metavar="MODEL",
        help=(
            "litellm model identifier. Use the ollama_chat/ prefix (not ollama/) "
            "for local Ollama models — it's the one that forwards tool calls to "
            "Ollama's native /api/chat endpoint. "
            "Examples: ollama_chat/qwen2.5:14b, claude-sonnet-4-6, gpt-4o, "
            "gemini/gemini-2.0-flash. Default: %(default)s"
        ),
    )
    chat_parser.add_argument(
        "--base-url",
        default=None,
        metavar="URL",
        help=(
            "LLM API base URL override (e.g. http://localhost:11434/v1 for Ollama "
            "or any OpenAI-compatible endpoint)."
        ),
    )
    chat_parser.add_argument(
        "--session-id",
        default=None,
        metavar="ID",
        help="Attach to an existing session instead of creating a new one",
    )
    chat_parser.add_argument(
        "--mount",
        action="append",
        metavar="PATH",
        default=[],
        help="Mount a local file or directory into the session (repeatable)",
    )
    chat_parser.add_argument(
        "--system",
        default=None,
        metavar="PROMPT",
        help="Override the default system prompt",
    )
    chat_parser.add_argument(
        "--max-iterations",
        type=int,
        default=30,
        metavar="N",
        help="Maximum agent iterations before stopping. Default: %(default)s",
    )

    args = parser.parse_args()

    if args.command == "chat":
        asyncio.run(_run_chat(args))


async def _run_chat(args: argparse.Namespace) -> None:
    try:
        async with DrunMcpBridge(
            args.mcp_url, session_id=args.session_id, mounts=args.mount
        ) as bridge:
            try:
                agent = ChatAgent(
                    bridge,
                    model=args.model,
                    base_url=args.base_url,
                    system=args.system,
                    max_iterations=args.max_iterations,
                )
                await agent.run(args.prompt)
            except Exception as exc:
                print(f"error: {exc}", file=sys.stderr)
                sys.exit(1)
    except KeyboardInterrupt:
        print("\ninterrupted", file=sys.stderr)
        sys.exit(1)
    except Exception as exc:
        print(
            f"error: {exc}\nIs drun-mcp running? Check with: curl {args.mcp_url}",
            file=sys.stderr,
        )
        sys.exit(1)


if __name__ == "__main__":
    main()

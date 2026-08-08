"""Command-line entry point: `drun chat <prompt>`."""
from __future__ import annotations

import argparse
import asyncio
import sys

from .chat import ChatAgent
from .mcp_bridge import DrunMcpBridge


class ChatCommand:
    """Parses `drun chat` arguments and drives a `ChatAgent` against a running
    drun-mcp daemon.

    Errors are split by where they occur: a failure to connect to drun-mcp
    gets a "is it running?" hint; a failure afterwards (e.g. the LLM call)
    does not, since that hint would misdirect troubleshooting.
    """

    DEFAULT_MCP_URL = "http://127.0.0.1:7273/mcp"
    DEFAULT_MODEL = "ollama_chat/qwen3.6:latest"

    @classmethod
    def main(cls) -> None:
        args = cls._build_parser().parse_args()
        if args.command == "chat":
            asyncio.run(cls()._run_chat(args))

    @classmethod
    def _build_parser(cls) -> argparse.ArgumentParser:
        parser = argparse.ArgumentParser(
            prog="drun",
            description="drun — sandboxed code execution for agentic loops",
        )
        subparsers = parser.add_subparsers(dest="command", required=True)
        chat = subparsers.add_parser(
            "chat", help="Run an LLM agent against a running drun-mcp daemon")
        chat.add_argument("prompt", help="Task prompt for the agent")
        chat.add_argument(
            "--mcp-url", default=cls.DEFAULT_MCP_URL, metavar="URL",
            help="drun-mcp daemon endpoint. Default: %(default)s",
        )
        chat.add_argument(
            "--model", default=cls.DEFAULT_MODEL, metavar="MODEL",
            help=(
                "litellm model id. Use the ollama_chat/ prefix (not ollama/) for "
                "local Ollama models — it forwards tool calls to Ollama's native "
                "/api/chat endpoint. Examples: ollama_chat/qwen3.6:latest, "
                "claude-sonnet-4-6, gpt-4o, gemini/gemini-2.0-flash. "
                "Default: %(default)s"
            ),
        )
        chat.add_argument(
            "--base-url", default=None, metavar="URL",
            help="LLM API base URL override (e.g. http://localhost:11434/v1)",
        )
        chat.add_argument(
            "--session-id", default=None, metavar="ID",
            help=(
                "Attach to an existing session instead of creating a new one. "
                "If no such session is currently active in the daemon, falls "
                "back to loading a matching <id>.drun file from the server's "
                "configured snapshots directory."
            ),
        )
        chat.add_argument(
            "--mount", action="append", default=[], metavar="PATH",
            help="Mount a local file or directory into the session (repeatable)",
        )
        chat.add_argument(
            "--system", default=None, metavar="PROMPT",
            help="Override the default system prompt",
        )
        chat.add_argument(
            "--max-iterations", type=int, default=30, metavar="N",
            help="Maximum agent iterations before stopping. Default: %(default)s",
        )
        chat.add_argument(
            "--llm-retries", type=int, default=3, metavar="N",
            help=(
                "Retries for a failing LLM request, with backoff, before giving "
                "up. Tool-call failures are always reported to the model instead "
                "of retried. Default: %(default)s"
            ),
        )
        chat.add_argument(
            "--reasoning-effort", default=None, choices=["low", "medium", "high"],
            help=(
                "Enable thinking mode on models that support it. For "
                "ollama_chat/ models this maps to Ollama's native `think` field "
                "(qwen3, deepseek-r1, gpt-oss, ...); ignored by models that "
                "don't support reasoning. Omit to leave thinking off/at the "
                "model's own default."
            ),
        )
        return parser

    async def _run_chat(self, args: argparse.Namespace) -> None:
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
                        llm_retries=args.llm_retries,
                        reasoning_effort=args.reasoning_effort,
                    )
                    response = await agent.run(args.prompt)
                    await self._record_chat_turn(bridge, args.prompt, response)
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

    @staticmethod
    async def _record_chat_turn(bridge: DrunMcpBridge, prompt: str, response: str) -> None:
        """Persists this exchange into the session's chat log via session_chat_record.
        Best-effort: the chat response already succeeded and printed, so a
        bookkeeping failure here is a warning, not a reason to exit non-zero.
        """
        try:
            await bridge.call("session_chat_record", {"prompt": prompt, "response": response})
        except Exception as exc:
            print(
                f"warning: failed to record chat turn: {exc}", file=sys.stderr)


def main() -> None:
    ChatCommand.main()


if __name__ == "__main__":
    main()

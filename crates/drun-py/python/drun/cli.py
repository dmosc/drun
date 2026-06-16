"""Command-line entry point: `drun chat <prompt>`."""
from __future__ import annotations

import argparse
import sys

from .chat import run


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="drun",
        description="drun — sandboxed code execution for agentic loops",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    chat_parser = subparsers.add_parser(
        "chat", help="Run an LLM agent with drun sandbox tools")
    chat_parser.add_argument("prompt", help="Task prompt for the agent")
    chat_parser.add_argument(
        "--model",
        default="ollama/qwen2.5:14b",
        metavar="MODEL",
        help=(
            "litellm model identifier. "
            "Examples: openai/qwen2.5:14b, claude-sonnet-4-6, gpt-4o, "
            "gemini/gemini-2.0-flash. Default: %(default)s"
        ),
    )
    chat_parser.add_argument(
        "--base-url",
        default=None,
        metavar="URL",
        help=(
            "API base URL override (e.g. http://localhost:11434/v1 for Ollama "
            "or any OpenAI-compatible endpoint)."
        ),
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
        _run_chat(args)


def _run_chat(args: argparse.Namespace) -> None:
    try:
        from .drun_internal import DrunSession
    except ImportError as exc:
        print(
            f"error: drun native extension not found ({exc}). "
            "Make sure drun-sandbox is installed correctly.",
            file=sys.stderr,
        )
        sys.exit(1)

    session = DrunSession()
    for path in args.mount:
        try:
            session.mount(path)
        except Exception as exc:
            print(f"error: could not mount {path!r}: {exc}", file=sys.stderr)
            sys.exit(1)

    try:
        run(
            session,
            args.prompt,
            model=args.model,
            base_url=args.base_url,
            system=args.system,
            max_iterations=args.max_iterations,
        )
    except KeyboardInterrupt:
        print("\ninterrupted", file=sys.stderr)
        sys.exit(1)
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()

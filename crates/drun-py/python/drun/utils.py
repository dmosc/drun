import os
import difflib

from rich.console import Console
from rich.syntax import Syntax
from rich.panel import Panel
from rich.prompt import Confirm

from .drun_internal import DrunOutput


def commit(result: DrunOutput, interactive=True) -> DrunOutput:
    if result.files:
        for filename, content_bytes in result.files.items():
            new_content = content_bytes.decode('utf-8', errors='replace')
            host_path = os.path.abspath(filename)
            previous_content = ''
            if os.path.exists(host_path):
                with open(host_path, 'r') as file:
                    previous_content = file.read()

            diff = list(difflib.unified_diff(
                previous_content.splitlines(),
                new_content.splitlines(),
                fromfile=f'a/{filename}',
                tofile=f'b/{filename}',
                lineterm=''
            ))

            if not diff:
                continue

            diff_text = "\n".join(diff)
            console = Console()
            console.print(
                Panel(
                    f'[bold yellow]Changes in:[bold cyan] {filename}',
                    expand=False
                )
            )
            console.print(
                Syntax(
                    diff_text,
                    'diff',
                    theme='monokai',
                    background_color='default'
                )
            )

            if not interactive:
                os.makedirs(os.path.dirname(host_path), exist_ok=True)
                with open(host_path, 'w') as file:
                    file.write(new_content)
            elif Confirm.ask(
                f'Apply changes to [bold cyan]{filename}?',
                default=False
            ):
                os.makedirs(os.path.dirname(host_path), exist_ok=True)
                with open(host_path, 'w') as file:
                    file.write(new_content)
                console.print(f'[bold green]Applied {filename}[/]\n')
            else:
                console.print(f'[bold red]Skipped {filename}[/]\n')

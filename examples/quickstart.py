"""
Python SDK quickstart — no LLM required.

Walks through the core SDK operations: execute, write, rollback, install,
bash, diff, and export. Run it directly against the default config (no
DRUN_CONFIG needed):

    pip install drun-sandbox
    python examples/quickstart.py

To use a custom config:

    DRUN_CONFIG=examples/financial_analysis.toml python examples/quickstart.py

Expected output (checkpoint IDs will vary):
    [1] execute   — hello from the sandbox
    [2] write     — checkpoint 1 → version A
    [3] change    — checkpoint 3 → version B
    [4] rollback  — version A
    [5] install   — table printed via tabulate
    [6] bash      — directory listing of /workspace
    [7] diff      — unified diff of the two file states
    [8] export    — list of exported paths
"""

import os
from drun import Session


def main():
    session = Session()

    # 1. Execute Python and read stdout
    cp = session.execute_python("print('hello from the sandbox')")
    print(f"[1] execute   — {cp.stdout.strip()}")

    # 2. Write a file (write_file takes bytes) then read it back
    session.write_file("/workspace/notes.txt", b"version A")
    cp_a = session.execute_python("print(open('/workspace/notes.txt').read())")
    print(f"[2] write     — checkpoint {cp_a.id} → {cp_a.stdout.strip()}")

    # 3. Overwrite it via Python code
    session.execute_python(
        "open('/workspace/notes.txt', 'w').write('version B')"
    )
    cp_b = session.execute_python("print(open('/workspace/notes.txt').read())")
    print(f"[3] change    — checkpoint {cp_b.id} → {cp_b.stdout.strip()}")

    # 4. Roll back to checkpoint A — file returns to "version A"
    session.rollback(cp_a.id)
    cp_back = session.execute_python(
        "print(open('/workspace/notes.txt').read())")
    print(f"[4] rollback  — {cp_back.stdout.strip()}")

    # 5. Install a package and use it inside the sandbox
    session.install("tabulate")
    cp_tab = session.execute_python("""\
from tabulate import tabulate
rows = [
    ["execute_python", "sandbox-isolated Python execution"],
    ["execute_bash",   "sandbox-isolated shell commands"],
    ["rollback",       "rewind to any prior checkpoint"],
    ["install",        "pip-install into the session"],
    ["export",         "copy workspace files to the host"],
]
print(tabulate(rows, headers=["operation", "description"], tablefmt="github"))
""")
    print(f"[5] install   —\n{cp_tab.stdout.strip()}")

    # 6. Run a bash command
    cp_bash = session.execute_bash("ls -1 /workspace")
    print(f"[6] bash      — {cp_bash.stdout.strip()}")

    # 7. Diff between two checkpoints
    patch = session.diff(cp_a.id, cp_b.id)
    print(f"[7] diff      —\n{patch.strip()}")

    # 8. Export workspace to the host (writes to export_root from config,
    #    or the current directory if export_root is not set)
    export_dir = os.environ.get("EXPORT_DIR", "/tmp/drun-quickstart")
    exported = session.export(export_dir)
    print(f"[8] export    — {exported}")


if __name__ == "__main__":
    main()

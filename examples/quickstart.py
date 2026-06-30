"""
Python SDK quickstart — no LLM required.

Walks through the core SDK operations: bash execution, write, rollback,
diff, and export. Run it directly against the default config (no
DRUN_CONFIG needed):

    pip install drun-sandbox
    python examples/quickstart.py

To use a custom config:

    DRUN_CONFIG=examples/financial_analysis.toml python examples/quickstart.py

Expected output (checkpoint IDs will vary):
    [1] bash       — hello from the sandbox
    [2] write      — checkpoint 1 -> version A
    [3] change     — checkpoint 3 -> version B
    [4] diff       — unified diff of the two file states
    [5] rollback   — version A
    [6] bash       — directory listing
    [7] export     — list of exported paths

Note: session_bash runs in a sandbox with no network access and no way to
install packages — there is no Python-level "execute_python" or "install"
call, only execute_bash (which can invoke python3 directly) and write_file.

Note: rollback() is destructive once you act on it. diff() is called here
*before* rollback — rolling back to checkpoint A and then running any more
execute_bash/write_file/delete_file calls permanently discards every
checkpoint after the rollback point (including checkpoint B), so there
would be nothing left to diff against afterward. Use session.mount /
create a fork first if you need to keep the abandoned branch around.
"""

import os
from drun import Session


def main():
    session = Session()

    # 1. Run a shell command and read stdout
    cp = session.execute_bash("python3 -c \"print('hello from the sandbox')\"")
    print(f"[1] bash       — {cp.stdout.strip()}")

    # 2. Write a file (write_file takes bytes) then read it back
    session.write_file("notes.txt", b"version A")
    cp_a = session.execute_bash("cat notes.txt")
    print(f"[2] write      — checkpoint {cp_a.id} -> {cp_a.stdout.strip()}")

    # 3. Overwrite it via a shell command
    session.execute_bash("printf '%s' 'version B' > notes.txt")
    cp_b = session.execute_bash("cat notes.txt")
    print(f"[3] change     — checkpoint {cp_b.id} -> {cp_b.stdout.strip()}")

    # 4. Diff between the two checkpoints — must happen before rollback,
    #    since continuing past a rollback discards the checkpoints being
    #    rolled back past (see the module docstring).
    patch = session.diff(cp_a.id, cp_b.id)
    print(f"[4] diff       —\n{patch.strip()}")

    # 5. Roll back to checkpoint A — file returns to "version A". From here
    #    on, checkpoint B (and this diff) are no longer reachable.
    session.rollback(cp_a.id)
    cp_back = session.execute_bash("cat notes.txt")
    print(f"[5] rollback   — {cp_back.stdout.strip()}")

    # 6. Run a bash command
    cp_bash = session.execute_bash("ls -1")
    print(f"[6] bash       — {cp_bash.stdout.strip()}")

    # 7. Export workspace to the host (writes to export_root from config,
    #    or the current directory if export_root is not set)
    export_dir = os.environ.get("EXPORT_DIR", "/tmp/drun-quickstart")
    exported = session.export(export_dir)
    print(f"[7] export     — {exported}")


if __name__ == "__main__":
    main()

//! The full tool-usage guide returned by the `get_system_instructions` tool.
//!
//! Project-level instructions files are written once when calling
//! `drun-mcp <bridge> init`.
pub(crate) const SYSTEM_INSTRUCTIONS: &str = r#"## Getting started

1. Call `create_session` — sessions start with an empty workspace and become
   the active session for this connection.
2. Call `get_config` to see what domains, host paths, and env vars are already
   allowed, and the resource limits in effect. Check this before your first
   `session_fetch`, `session_mount`, or `session_package_install` instead of
   discovering the allowlist through denied calls.
3. Call `session_mount` with the absolute path to your project's root
   directory to load its files into the session (already allowlisted by
   drun's setup for this project). Re-mount any other host paths you need the
   same way.
4. From there, work entirely through drun tools — there is no host file or
   shell access outside of them.

Every `session_*` tool below acts on the session you last created, forked,
restored, or switched to — none of them take a session_id, except
`session_switch` (which takes the id to switch to) and `session_merge` (which
takes a source_session_id to merge from).

## Picking up a session you didn't start

If you're resuming work after a break, or joining a session another agent
created, reconstruct what happened before acting on assumptions:

- **`session_tree`** — the full session/fork graph in one call, across every
  session on the server, not just the active one. Each checkpoint carries the
  tool, command, description, and file-delta counts recorded when it was
  made, plus `is_current` for the active head of every session. Call this
  first when the session predates you.
- **`session_history`** — the same per-checkpoint detail (tool, command,
  description, file deltas), scoped to just the active session. Use this once
  you already know which session you're in.
- **`get_session_state`** — a cheaper single-checkpoint snapshot: current file
  list and deltas since the previous checkpoint, no history walk.
- **`checkpoint_read_stdstreams`** — once you've found the checkpoint you care
  about via one of the above, read its actual stdout/stderr text (see below —
  none of the tools above return output inline).

## Reading command output

`session_bash` does not return stdout/stderr text inline — it returns state:
checkpoint_id, exit_code, stdout_bytes/stderr_bytes (byte counts, not content),
and which files changed. To read what a command actually printed, call
`checkpoint_read_stdstreams` against that checkpoint:

```
session_bash({"command": "pytest -q"})
→ {"checkpoint_id": 3, "exit_code": 0, "stdout_bytes": 842, "stderr_bytes": 0, ...}

checkpoint_read_stdstreams({})
→ {"stream": "stdout", "exit_code": 0, "content": "...12 passed in 0.4s", ...}
```

`checkpoint_read_stdstreams` defaults to the current checkpoint's stdout; pass
`{"stream": "stderr"}` for stderr, or `{"checkpoint_id": N}` for an older one.
Don't treat the JSON from `session_bash` itself as the command's output, and
don't treat non-empty stderr as failure — check `exit_code` instead; plenty of
commands write warnings or progress to stderr on success.

## Core tools

Grouped by purpose — each tool's schema documents its exact parameters.

**Sandbox I/O**
- `session_bash` — run shell commands in the sandboxed workspace (also covers
  listing/searching files — e.g. `ls`, `grep`, `find`). Returns state, not
  output — see "Reading command output" above.
- `session_read_file` / `session_write_file` / `session_delete_file` — read,
  write, and delete files in the session by session-relative path. For large
  files, don't page through blind with offset/limit to find something — set
  `pattern` to a case-sensitive regex (use `(?i)` for case-insensitive) and
  `session_read_file` returns only the matching lines, each with its
  `line_number` and `byte_offset`, searched within the offset/limit byte range
  (the whole file if omitted). Use a match's `byte_offset` in a follow-up
  offset/limit read to pull the surrounding context.
- `session_mount` — load a host file or directory into the session. The
  session doesn't remember where a file came from beyond that call — it's a
  one-time copy into the workspace, not a live link back to host.
- `session_export` — write current workspace files to a host directory.
  Only ever creates/overwrites, never deletes — including a file deleted from
  the session after being mounted. Doesn't need to be the directory
  session_mount was called with, or a mount at all. The sandbox is a
  scratchpad seeded from the host, not something drun keeps in sync with it;
  if you need the host to reflect a deletion or rename you did in the
  sandbox, do that on the host yourself.
- `session_fetch` — the designated gateway for outbound HTTP; `session_bash`
  has no network access by design. Saves the response (and, for HTML, its
  linked assets) into the session instead of returning it inline. Subject to
  the server's domain_allowlist — check `get_config` first.
- `session_extract_text` — pull plain text out of a binary document already in
  the session (e.g. a PDF) and save it as a new file.
- `session_package_install` — install `pip`/`npm` packages so later
  `session_bash` calls can import them. Disabled by default; if it returns
  `package_install_disabled`, tell the user to set
  `package_install_enabled = true` in `~/.drun/config.toml` (hand-edit — no
  restart needed, takes effect on your next call).
- `session_get_env` — read a host environment variable by name (only names in
  the server's env_allowlist). Use this to pass secrets into the session
  instead of hardcoding them.

**History & branching**
- `session_diff` — unified diff between two checkpoints (defaults to
  checkpoint 0 vs. the current one).
- `session_rollback` — move the active session's head back to a prior
  checkpoint. Destructive past that point once you continue the session — call
  `session_fork` first if you need to keep what you're rolling back past.
- `session_fork` — branch a new, independent session off any checkpoint.
- `session_merge` — overlay files from another session's checkpoint onto the
  active one, creating a new checkpoint.
- `session_checkpoint_squash` — collapse a checkpoint range into one, keeping
  the end file state and merged stdout/stderr. Useful for cleaning up
  exploration history before committing to a direction.
- `session_checkpoint_drop` — remove a checkpoint range outright to stay under
  the checkpoint limit.
- `session_checkpoint_label` — attach a label to a checkpoint (e.g.
  "baseline") so it shows up in `session_history`/`session_tree` and can be
  addressed by label instead of id elsewhere.

**Sessions**
- `session_list` / `session_switch` — see every session and change which one
  is active.
- `session_label` — attach a human-readable label to the active session, shown
  in `session_list`, `get_session_state`, and `session_tree`.
- `session_close` — terminate the active session and free its resources
  (including the sandbox subprocess). Switch to a different session first if
  you meant to close one other than the active one.

**Persisting across restarts**
- `session_snapshot` — serialize the active session's full checkpoint history
  and files to a `.drun` file on the host.
- `list_snapshots` — see what's already been saved.
- `session_restore` — reload a `.drun` file as a new active session.

## If a fetch or mount is denied

`session_fetch` and `session_mount` are restricted to an allowlist — check
`get_config` first to see what's already permitted. If either is denied for a
domain or path you need, tell the user to run:

- `drun-mcp config add-domain <domain>` to allow a new domain for
  `session_fetch`
- `drun-mcp config add-path <path>` to allow a new host path for
  `session_mount`

Both commands edit `~/.drun/config.toml` directly — no restart needed, and
the change is visible on your very next tool call in this same session.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_the_core_tools() {
        assert!(SYSTEM_INSTRUCTIONS.contains("session_bash"));
        assert!(SYSTEM_INSTRUCTIONS.contains("session_mount"));
    }

    #[test]
    fn explains_reading_command_output() {
        assert!(SYSTEM_INSTRUCTIONS.contains("checkpoint_read_stdstreams"));
        assert!(SYSTEM_INSTRUCTIONS.contains("does not return stdout/stderr text inline"));
    }

    #[test]
    fn documents_get_config() {
        assert!(SYSTEM_INSTRUCTIONS.contains("get_config"));
    }

    #[test]
    fn documents_session_read_file_pattern_search() {
        assert!(SYSTEM_INSTRUCTIONS.contains("pattern"));
        assert!(SYSTEM_INSTRUCTIONS.contains("byte_offset"));
    }

    #[test]
    fn documents_resuming_a_session() {
        assert!(SYSTEM_INSTRUCTIONS.contains("session_tree"));
        assert!(SYSTEM_INSTRUCTIONS.contains("session_history"));
        assert!(SYSTEM_INSTRUCTIONS.contains("get_session_state"));
    }

    #[test]
    fn documents_snapshots() {
        assert!(SYSTEM_INSTRUCTIONS.contains("session_snapshot"));
        assert!(SYSTEM_INSTRUCTIONS.contains("session_restore"));
        assert!(SYSTEM_INSTRUCTIONS.contains("list_snapshots"));
    }

    #[test]
    fn does_not_hardcode_a_project_path() {
        assert!(!SYSTEM_INSTRUCTIONS.contains("/home"));
        assert!(!SYSTEM_INSTRUCTIONS.contains("/tmp"));
    }
}

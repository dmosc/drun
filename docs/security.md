# Security model

drun is designed to protect the host from untrusted AI-generated code. It is not
designed to protect against a vulnerability in the OS sandbox primitives
themselves (`sandbox-exec` on macOS, `bwrap` on Linux), or a misconfigured
operator allowlist. Understanding what each layer does — and where it stops — is
important before deploying drun in any sensitive environment.

---

## Isolation layers

### 1. OS-level sandbox (`session_bash`)

Every `session_bash` call runs inside a sandboxed child process:

- **macOS** — `sandbox-exec` with a profile that denies everything by default
  and allows file writes only under the session's tempdir, `/private/tmp`, and
  `/dev/null`.
- **Linux** — `bwrap` with `--ro-bind / /` (host filesystem mounted read-only)
  and a read-write bind only for the session workspace, plus `--unshare-net` to
  remove network access entirely.

There is no network access from inside `session_bash` on either platform — not
even to allowlisted domains. The only outbound network path is `session_fetch`,
which runs on the host (not in the sandbox) and is gated by `domain_allowlist`.

### 2. Operator allowlists (runtime policy)

The server enforces a set of policy restrictions on all sessions:

| Config key               | What it restricts                                                                                                             |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------- |
| `domain_allowlist`       | Domains reachable via `session_fetch`. Supports exact hostnames and `*.example.com` wildcards, or `["*"]` for all.            |
| `mount_allowlist`        | Host path prefixes that `session_mount` may read from. Checked against the canonicalized path; empty means all paths allowed. |
| `export_root`            | Directory that `session_export` and `session_snapshot` may write into.                                                        |
| `env_allowlist`          | Host environment variable names readable via `session_get_env`.                                                               |
| `bash_command_denylist`  | Command substrings always rejected by `session_bash` before execution.                                                        |
| `bash_command_allowlist` | Command substrings permitted by `session_bash`. Empty means all commands allowed (subject to the denylist).                   |

Agents operate within whatever the operator configured. They cannot expand their
own permissions at runtime.

---

## Default posture

When no `DRUN_CONFIG` is set, drun applies the following defaults:

| Property                           | Default                                                  |
| ---------------------------------- | -------------------------------------------------------- |
| Outbound network (`session_bash`)  | None — fully unshared from the host network              |
| Outbound network (`session_fetch`) | `pypi.org`, `files.pythonhosted.org`, `cdn.jsdelivr.net` |
| Mount path restrictions            | None                                                     |
| Export path restrictions           | `./drun-export`                                          |
| Env var exposure                   | None                                                     |
| Command restrictions               | None                                                     |
| Max workspace                      | 512 MB per session                                       |
| Max sessions                       | 50 concurrent                                            |
| Max checkpoints                    | 200 per session                                          |
| Idle session timeout               | 1 hour                                                   |
| `session_bash` timeout             | 30 seconds                                               |

The default posture is conservative on network access, permissive on filesystem
scope. If you are deploying drun in a shared environment, set `mount_allowlist`
and `export_root` explicitly.

---

## Path traversal prevention

Workspace file keys containing `..` components are rejected at write time in
`session_write_file` and `session_fetch`'s `save_to` parameter. Export and
commit paths are re-validated after joining to confirm they remain within the
configured output directory. An agent cannot write a workspace key that escapes
to an arbitrary host path.

---

## Session isolation

Each session keeps its own in-memory `FileMap` and checkpoint history. Sessions
do not share memory or filesystem state. A session's files exist only in the MCP
server's in-memory session map; no data is written to the host until
`session_export`, `session_commit`, or `session_snapshot` is explicitly called.

---

## Known limitations

`session_mount` overlays (`node_modules`, `.venv`, `target`, etc.) are symlinked
into the sandbox read-only at execution time rather than copied — they rely on
the same sandbox profile as the rest of the workspace, so a write attempt
through the symlink is rejected by the OS sandbox, not by drun's own logic.
Mounting an untrusted directory whose name doesn't match `mount_overlay_paths`
loads its full contents into the session's in-memory workspace.

---

## Threat model summary

**drun protects against:**

- AI-generated code reading host environment variables not in the allowlist
- AI-generated code making outbound network connections from `session_bash` (no
  network access in the sandbox at all)
- Workspace state exceeding configured resource limits
- Sessions lingering indefinitely (idle reaper)
- Path traversal via crafted workspace keys
- Unauthorized outbound HTTP via `session_fetch` (domain allowlist enforced on
  the host before any request is made)

**drun does not protect against:**

- A misconfigured operator allowlist (e.g., `domain_allowlist = ["*"]`)
- Side-channel attacks between sessions (timing, cache) — all sessions share the
  same OS process
- Multi-tenant workloads where sessions from different users must be mutually
  isolated at the OS level — all sessions share the same OS process and user

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

- **macOS** — `sandbox-exec` with a profile that denies everything by default.
  Reads are limited to the session workspace, any mounted overlays
  (`node_modules`, `.venv`, `target`, etc.), the operator's `mount_allowlist`
  entries, a fixed set of system directories (`/usr`, `/bin`, `/sbin`, `/opt`,
  `/System`, `/Library`, `/etc`, `/dev`, `/private/tmp`), and every directory on
  the daemon's own `$PATH` — not the whole host filesystem. Writes are limited
  to the session's tempdir, `/private/tmp`, and `/dev/null`.
- **Linux** — `bwrap` with individual `--ro-bind` mounts for the same read set
  (workspace, overlays, `mount_allowlist`, fixed system directories, `$PATH`
  entries) instead of binding the whole host root, plus a read-write bind for
  the session workspace and `--unshare-net` to remove network access entirely.

`mount_allowlist` is read fresh from `config.toml` on every `session_bash` call,
the same as every other policy field below — an operator edit (e.g. via
`drun-mcp config add-path`) takes effect on the next call with no daemon
restart. Unlike its effect on `session_mount`, an _empty_ `mount_allowlist`
grants no extra sandbox reads — it only relaxes the `session_mount` check below;
it never falls back to "the whole host is readable."

There is no network access from inside `session_bash` on either platform — not
even to allowlisted domains. `session_fetch` and `session_package_install` are
the only outbound network paths — see below.

### 2. Package installs (`session_package_install`)

Disabled by default (`package_install_enabled = false`). When enabled, it runs
`pip`/`npm` with a different sandbox trade-off than `session_bash`: network
access is allowed (registries need it), but the process is confined to a
disposable staging directory, never the session workspace — so even unrestricted
egress can't be used to exfiltrate session files. Installed packages are merged
into the session's checkpoint (under `.packages/<manager>/`) only after the
install succeeds.

- **macOS** — the SBPL profile adds `(allow network*)` but also
  `(deny network-outbound (remote ip "localhost:*"))`, so the sandboxed install
  process cannot connect to services bound to loopback on the same machine (e.g.
  an unauthenticated local dev database), while registry traffic to the real
  internet is untouched.
- **Linux** — `bwrap` has no equivalent per-destination filter (that would need
  a real network namespace plus `nftables` or a filtering proxy), so
  `session_package_install` shares the host's network namespace outright,
  loopback included.

**This is not domain-scoped.** Neither SBPL nor bwrap can filter network access
by hostname or specific IP — SBPL's `(remote ip ...)` only recognizes the
literal tokens `*` or `localhost`. So while a malicious package's install-time
code can't reach the session's files, it can still reach anything else the host
can reach over the network except (on macOS) loopback — including a cloud
metadata endpoint or other internal/RFC1918 addresses if the daemon runs
somewhere those are reachable. Treat enabling this the same as running an
arbitrary `pip`/`npm install` locally: fine on a personal machine or trusted CI
runner, and something to reconsider before enabling on a host with
network-reachable internal services.

Package specifiers are validated before anything runs (rejecting flag injection
like a leading `-` and shell metacharacters), and the install binary
(`python3`/`npm`) must already exist on the daemon's `$PATH` — there is no
bundled interpreter.

### 3. Operator allowlists (runtime policy)

The server enforces a set of policy restrictions on all sessions:

| Config key                | What it restricts                                                                                                                                                                                                                                                                                                               |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `domain_allowlist`        | Domains reachable via `session_fetch`. Supports exact hostnames and `*.example.com` wildcards, or `["*"]` for all.                                                                                                                                                                                                              |
| `mount_allowlist`         | Host path prefixes that `session_mount` may read from and `session_export` may write to (empty means all paths allowed there) — and, separately, host directories `session_bash`'s sandbox may read from directly, in addition to the workspace, overlays, and fixed system/PATH dirs (empty means no extra directories there). |
| `env_allowlist`           | Host environment variable names readable via `session_get_env`.                                                                                                                                                                                                                                                                 |
| `bash_command_denylist`   | Command substrings always rejected by `session_bash` before execution.                                                                                                                                                                                                                                                          |
| `bash_command_allowlist`  | Command substrings permitted by `session_bash`. Empty means all commands allowed (subject to the denylist).                                                                                                                                                                                                                     |
| `package_install_enabled` | Whether `session_package_install` is available at all. Disabled by default since, unlike every other tool, its sandbox has network access.                                                                                                                                                                                      |

Agents operate within whatever the operator configured. They cannot expand their
own permissions at runtime.

---

## Default posture

When no `DRUN_CONFIG` is set, drun applies the following defaults:

| Property                           | Default                                                  |
| ---------------------------------- | -------------------------------------------------------- |
| Outbound network (`session_bash`)  | None — fully unshared from the host network              |
| Outbound network (`session_fetch`) | `pypi.org`, `files.pythonhosted.org`, `cdn.jsdelivr.net` |
| `session_package_install`          | Disabled                                                 |
| Mount/export path restrictions     | None                                                     |
| Env var exposure                   | None                                                     |
| Command restrictions               | None                                                     |
| Max workspace                      | 512 MB per session                                       |
| Max sessions                       | 50 concurrent                                            |
| Max checkpoints                    | 200 per session                                          |
| Idle session timeout               | 1 hour                                                   |
| `session_bash` timeout             | 30 seconds                                               |
| `session_package_install` timeout  | 3 minutes                                                |

The default posture is conservative on network access, permissive on filesystem
scope. If you are deploying drun in a shared environment, set `mount_allowlist`
explicitly.

---

## Path traversal prevention

Workspace file keys containing `..` components are rejected at write time in
`session_write_files` and `session_fetch`'s `save_to` parameter. Export paths are
re-validated after joining to confirm they remain within the configured output
directory. An agent cannot write a workspace key that escapes to an arbitrary
host path.

---

## Session isolation

Each session keeps its own in-memory `FileMap` and checkpoint history. Sessions
do not share memory or filesystem state. A session's files exist only in the MCP
server's in-memory session map; no data is written to the host until
`session_export` or `session_snapshot` is explicitly called. The sandbox is a
scratchpad seeded from the host via `session_mount`, not something drun keeps in
sync with it afterward — `session_mount` doesn't remember where a file came from
beyond the initial copy, and `session_export` only ever creates or overwrites at
its target path, never deletes. A rename or delete performed inside the sandbox
has no host-side effect unless you explicitly reproduce it on the host yourself.

---

## Drunmon

Monitoring service where drun clients stream telemetry to for monitoring and
continuous improvement purposes.

---

## Known limitations

`session_mount` overlays (`node_modules`, `.venv`, `target`, etc.) are symlinked
into the sandbox read-only at execution time rather than copied — they rely on
the same sandbox profile as the rest of the workspace, so a write attempt
through the symlink is rejected by the OS sandbox, not by drun's own logic.
Mounting an untrusted directory whose name doesn't match `mount_overlay_paths`
loads its full contents into the session's in-memory workspace.

`session_package_install`'s network access is unrestricted except for loopback
on macOS (see above) — there is no domain allowlist equivalent to
`session_fetch`'s for it, and no equivalent loopback carve-out on Linux at all.

---

## Threat model summary

**drun protects against:**

- AI-generated code reading arbitrary host files — `session_bash` can only read
  the session workspace, mounted overlays, `mount_allowlist` entries, and a
  fixed set of system/PATH directories needed to run installed toolchains, not
  the rest of the host filesystem
- AI-generated code reading host environment variables not in the allowlist
- AI-generated code making outbound network connections from `session_bash` (no
  network access in the sandbox at all)
- Workspace state exceeding configured resource limits
- Sessions lingering indefinitely (idle reaper)
- Path traversal via crafted workspace keys
- Unauthorized outbound HTTP via `session_fetch` (domain allowlist enforced on
  the host before any request is made)
- A malicious `session_package_install` package exfiltrating session files (its
  network access is confined to a disposable staging directory that is never the
  session workspace)
- `session_package_install` attacking other local services on the same machine
  via loopback (denied on macOS; see Known limitations for the Linux gap)

**drun does not protect against:**

- The daemon's own `$PATH` pointing at a directory an untrusted party controls —
  every directory on `$PATH` is readable inside the sandbox so installed
  toolchains keep working, so a compromised or attacker-writable `$PATH` entry
  is readable (and, since `process-exec*` is unrestricted, executable) from
  inside `session_bash`
- A misconfigured operator allowlist (e.g., `domain_allowlist = ["*"]`)
- Side-channel attacks between sessions (timing, cache) — all sessions share the
  same OS process
- Multi-tenant workloads where sessions from different users must be mutually
  isolated at the OS level — all sessions share the same OS process and user
- `session_package_install` reaching a cloud metadata endpoint or other
  internal/RFC1918 addresses — neither SBPL nor bwrap can filter network access
  by hostname or specific IP, only "all" vs. (on macOS) "all except loopback"

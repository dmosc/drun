# Security model

drun is designed to protect the host from untrusted AI-generated code. It is not
designed to protect against a compromised Deno binary, a vulnerability in the
Pyodide WASM runtime itself, or a misconfigured operator allowlist.
Understanding what each layer does — and where it stops — is important before
deploying drun in any sensitive environment.

---

## Isolation layers

### 1. Pyodide WASM boundary (primary)

Python code executes inside [Pyodide](https://pyodide.org), a WebAssembly port
of CPython. The WASM boundary means:

- `os.system()`, `subprocess.run()`, `ctypes` — fail at the WASM level; there
  are no host syscalls available
- `open('/etc/passwd')` — Pyodide has its own virtual filesystem that does not
  map to the host; host paths are inaccessible from Python
- Python HTTP libraries (`urllib`, `httpx`, `requests`) — route through Deno's
  `fetch()` API, which is subject to Deno's network permission flags

This boundary is architectural: it holds regardless of what the Python code
tries to do, and it is not affected by operator configuration mistakes.

### 2. Deno permission flags (secondary)

The Deno subprocess is spawned with explicit permission flags:

| Flag            | Value                               | What it controls                                       |
| --------------- | ----------------------------------- | ------------------------------------------------------ |
| `--allow-net`   | Server-configured hosts + PyPI CDNs | Outbound HTTP from the Deno process                    |
| `--allow-read`  | Global                              | Filesystem reads by Deno (see known limitation below)  |
| `--allow-write` | Global                              | Filesystem writes by Deno (see known limitation below) |
| `--allow-run`   | Not granted                         | Cannot spawn subprocesses                              |
| `--allow-env`   | Not granted                         | Cannot read host environment variables                 |

### 3. Operator allowlists (runtime policy)

The server enforces a second set of restrictions on top of Deno's flags:

| Config key          | What it restricts                                                                                                                   |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `domain_allowlist`  | Domains reachable via `session_fetch` and Python outbound HTTP. PyPI CDNs are always included.                                      |
| `mount_allowlist`   | Host paths that may be mounted. Checked against the canonicalized path; symlinks that point outside an allowed prefix are rejected. |
| `export_root`       | Directory that `session_export` and `session_snapshot` may write into.                                                              |
| `env_allowlist`     | Host environment variable names readable via `session_get_env`.                                                                     |
| `package_allowlist` | If set, `session_install_package` rejects any package not in the list.                                                              |

Agents operate within whatever the operator configured. They cannot expand their
own permissions at runtime.

---

## Default posture

When no `DRUN_CONFIG` is set, drun applies the following defaults:

| Property                  | Default                  |
| ------------------------- | ------------------------ |
| Outbound network (Python) | Blocked except PyPI CDNs |
| Mount path restrictions   | None                     |
| Export path restrictions  | None                     |
| Env var exposure          | None                     |
| Package restrictions      | None                     |
| Max workspace             | 512 MB per session       |
| Max sessions              | 50 concurrent            |
| Max checkpoints           | 200 per session          |
| Idle session timeout      | 1 hour                   |
| stdout/stderr capture     | 1 MB per execution       |

The default posture is conservative on network and env access, permissive on
filesystem scope. If you are deploying drun in a shared environment, set
`mount_allowlist` and `export_root` explicitly.

---

## Path traversal prevention

Workspace file keys containing `..` components are rejected at write time in
`session_write_file` and `session_fetch`'s `save_to` parameter. Export and
commit paths are re-validated after joining to confirm they remain within the
configured output directory. An agent cannot write a workspace key that escapes
to an arbitrary host path.

---

## Session isolation

Each session runs in its own Deno subprocess with its own Pyodide instance and
virtual filesystem. Sessions do not share memory or filesystem state. A
session's files exist only in the MCP server's in-memory `SessionMap`; no data
is written to the host until `session_export` or `session_commit` is explicitly
called.

---

## Known limitation: Deno filesystem access

`--allow-read` and `--allow-write` are currently global flags, not scoped to
specific paths. The Deno subprocess itself (`runner.ts`) can read and write any
file the OS user can access.

In practice this is mitigated because:

- Python code cannot directly invoke Deno filesystem APIs from within the
  Pyodide WASM context — there is no bridge between Pyodide's virtual FS and
  Deno's native FS APIs
- `runner.ts` is a small, auditable TypeScript file compiled into the binary at
  build time; it does not perform any host filesystem I/O of its own

The residual risk is that a future Deno vulnerability or an unexpected WASM
bridge could allow escalation. Scoping these flags to the minimal required paths
is on the roadmap for a future release.

---

## Threat model summary

**drun protects against:**

- AI-generated code reading arbitrary host files from Python
- AI-generated code making unauthorized outbound network requests
- AI-generated code spawning host processes
- AI-generated code reading host environment variables not in the allowlist
- Workspace state exceeding configured resource limits
- Sessions lingering indefinitely (idle reaper)
- Path traversal via crafted workspace keys

**drun does not protect against:**

- A compromised or backdoored Deno binary
- A vulnerability in Pyodide's WASM runtime (WASM escape)
- Deno-level filesystem access if `runner.ts` were modified or replaced
- Side-channel attacks between sessions (timing, cache)
- An operator who misconfigures the allowlists (e.g., `allowed_hosts = ["*"]`)
- Multi-tenant workloads where sessions from different users must be mutually
  isolated at the OS level — all sessions share the same OS process and user

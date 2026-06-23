# Security model

drun is designed to protect the host from untrusted AI-generated code. It is not
designed to protect against a vulnerability in the Python interpreter itself, or
a misconfigured operator allowlist. Understanding what each layer does — and
where it stops — is important before deploying drun in any sensitive
environment.

---

## Isolation layers

### 1. Operator allowlists (runtime policy)

The server enforces a set of policy restrictions on all sessions:

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

| Property                  | Default            |
| ------------------------- | ------------------ |
| Outbound network (Python) | Unrestricted       |
| Mount path restrictions   | None               |
| Export path restrictions  | None               |
| Env var exposure          | None               |
| Package restrictions      | None               |
| Max workspace             | 512 MB per session |
| Max sessions              | 50 concurrent      |
| Max checkpoints           | 200 per session    |
| Idle session timeout      | 1 hour             |
| stdout/stderr capture     | 1 MB per execution |

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

Each session runs in its own Python subprocess with its own interpreter state
and workspace. Sessions do not share memory or filesystem state. A session's
files exist only in the MCP server's in-memory `SessionMap`; no data is written
to the host until `session_export` or `session_commit` is explicitly called.

---

## Known limitations

The Python subprocess can access the host filesystem and network without
restriction. `session_fetch` is the designated network gateway and enforces
`domain_allowlist`; Python's own HTTP libraries are not yet constrained.
Filesystem and network sandboxing for the Python runner is planned for a future
release.

---

## Threat model summary

**drun protects against:**

- AI-generated code reading host environment variables not in the allowlist
- Workspace state exceeding configured resource limits
- Sessions lingering indefinitely (idle reaper)
- Path traversal via crafted workspace keys
- Unauthorized outbound HTTP via `session_fetch` (domain allowlist enforced)

**drun does not protect against:**

- AI-generated Python code making direct outbound network requests
- AI-generated Python code reading arbitrary host files
- Side-channel attacks between sessions (timing, cache)
- An operator who misconfigures the allowlists (e.g., `allowed_hosts = ["*"]`)
- Multi-tenant workloads where sessions from different users must be mutually
  isolated at the OS level — all sessions share the same OS process and user

# Troubleshooting

Common issues and how to resolve them.

---

## `deno: command not found`

The MCP server spawns a Deno subprocess on first use. If Deno is not on your
`PATH`, every `session_execute` and `create_session` will fail immediately.

**Fix:** Install Deno:

```bash
curl -fsSL https://deno.land/install.sh | sh
```

Then add it to your shell profile (`~/.zshrc`, `~/.bashrc`, etc.):

```bash
export DENO_INSTALL="$HOME/.deno"
export PATH="$DENO_INSTALL/bin:$PATH"
```

Restart your shell and verify: `deno --version`

---

## First execution is very slow (30–60 seconds)

Pyodide downloads approximately 50 MB of WebAssembly assets on its first run.
This is a one-time cost — the assets are cached in Deno's npm cache
(`~/.cache/deno` on Linux, `~/Library/Caches/deno` on macOS) and reused on every
subsequent execution.

If the first run times out before Pyodide finishes loading, increase the session
timeout when creating the session:

```
create_session(timeout_ms=120000)   # 2 minutes
```

---

## Package install times out (`execution_timeout`)

Large scientific packages under Pyodide — `scipy`, `Pillow`, `scikit-learn` —
can take 2–5 minutes to download and compile on first install. The default
per-session execution timeout may be too short.

**Fix:** Create the session with a longer timeout specifically for installs:

```
create_session(timeout_ms=300000)   # 5 minutes
```

You can fork the session after packages are installed if you want a shorter
timeout for subsequent executions.

Note: not all PyPI packages are available in Pyodide. See the
[Pyodide package list](https://pyodide.org/en/stable/usage/packages-in-pyodide.html)
for what is supported. Packages with C extensions that are not pre-compiled for
WASM will fail to install.

---

## `mount_denied`: file or directory rejected by mount allowlist

If the server is configured with `mount_allowlist`, `session_mount` will reject
any host path that does not start with one of the listed prefixes.

**Error:** `mount_denied` with a message like
`path is not under any allowed mount
prefix`

**Fix:** Either use a path within an allowed directory, or update `DRUN_CONFIG`
to add the path:

```toml
[session]
mount_allowlist = ["/tmp/drun-inputs", "/Users/you/projects/data"]
```

---

## `package_denied`: package not in allowlist

If the server is configured with `package_allowlist`, `session_install_package`
will reject any package not in the list.

**Error:** `package_denied` with a message naming the rejected package

**Fix:** Either use a package on the list (call `get_allowed_packages` to see
it), or update `DRUN_CONFIG`:

```toml
[session]
package_allowlist = ["pandas", "numpy", "matplotlib", "your-package"]
```

---

## `fetch_denied`: domain not in allowlist

`session_fetch` and Python outbound HTTP are restricted to the server's fetch
allowlist. By default, only PyPI CDNs are reachable (for package installs).

**Error:** `fetch_denied` with a message naming the blocked domain

**Fix:** Add the domain to `DRUN_CONFIG`:

```toml
domain_allowlist = ["api.example.com", "data.sec.gov"]
```

Call `get_fetch_allowlist` to see the current effective list.

---

## `session_busy`: concurrent execution on the same session

Two simultaneous calls to `session_execute` on the same session return
`session_busy` immediately. Sessions execute one code block at a time.

**Fix:** Wait for the current execution to complete, or create a separate
session (or fork) for parallel work.

---

## Session disappeared / `session_not_found`

Sessions are evicted after the configured idle timeout
(`session_idle_timeout_secs`, default 1 hour). A session that was idle for
longer than the timeout will have been cleaned up by the reaper.

**Fix:** Use `session_snapshot` to persist long-lived sessions before they go
idle, and `session_restore` to reload them. Alternatively, increase the idle
timeout in config.

---

## MCP server not appearing in Claude Code

If `claude mcp list` does not show `drun`, re-register it:

```bash
claude mcp add drun -- /usr/local/bin/drun-mcp
```

If it was registered in multiple scopes and you see a scope conflict error when
removing it:

```bash
for scope in local user project; do
  claude mcp remove drun -s "$scope" 2>/dev/null
done
claude mcp add drun -- /usr/local/bin/drun-mcp
```

---

## Still stuck?

[Open an issue on GitHub](https://github.com/dmosc/drun/issues/new) with:

- Your OS and architecture (`uname -sm`)
- Deno version (`deno --version`)
- The exact error message or structured error code from the tool response
- The `DRUN_CONFIG` you are using (redact any secrets)
- Steps to reproduce

# Troubleshooting

Common issues and how to resolve them.

---

## Configuration lifecycle

drun reads `DRUN_CONFIG` once at startup and holds the parsed config in memory
for the lifetime of the server process. Changing `drun.toml` while the server is
running has no effect — not on open sessions, and not on new sessions created
after the edit.

To apply a config change:

1. Edit `drun.toml`
2. Restart the MCP server (e.g. `claude mcp restart drun`, or stop and re-add
   it)
3. Claude Code reconnects automatically on the next tool call

Open sessions that were created before the restart are gone — sessions live only
in server memory and are not persisted across restarts unless you called
`session_snapshot` first.

---

## `python3: command not found`

The MCP server spawns a Python 3 subprocess on first use. If `python3` is not on
your `PATH`, every `session_execute_python` and `create_session` will fail
immediately.

**Fix:** Install Python 3 via your system package manager or from
[python.org](https://www.python.org/downloads/), then verify:
`python3 --version`

On macOS with Homebrew:

```bash
brew install python
```

---

## Package install times out (`execution_timeout`)

Large packages — `scipy`, `Pillow`, `scikit-learn`, `torch` — can take several
minutes to download on first install. The default `install_timeout_ms` (2
minutes) may be too short on a slow connection.

**Fix:** Increase `install_timeout_ms` in your `drun.toml`:

```toml
install_timeout_ms = 300000   # 5 minutes
```

Packages are cached in `packages_dir` (defaults to a `drun-packages` folder in
the OS temp directory) and reused across sessions, so the slow download only
happens once.

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
- Python version (`python3 --version`)
- The exact error message or structured error code from the tool response
- The `DRUN_CONFIG` you are using (redact any secrets)
- Steps to reproduce

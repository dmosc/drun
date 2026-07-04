# Troubleshooting

Common issues and how to resolve them.

---

## Configuration lifecycle

drun reads `DRUN_CONFIG` once at startup and holds the parsed config in memory
for the lifetime of the server process. Changing `config.toml` while the server
is running has no effect — not on open sessions, and not on new sessions created
after the edit.

To apply a config change:

1. Edit `config.toml`
2. Restart the MCP server (e.g. `claude mcp restart drun`, or stop and re-add
   it)
3. Claude Code reconnects automatically on the next tool call

Open sessions that were created before the restart are gone — sessions live only
in server memory and are not persisted across restarts unless you called
`session_snapshot` first.

---

## `session_bash` command times out (`execution_timeout`)

The default `bash_timeout_ms` (30 seconds) is tight for slow or long-running
commands (large downloads via a mounted overlay, big builds, training loops).

**Fix:** Increase `bash_timeout_ms` in your `config.toml`:

```toml
bash_timeout_ms = 300000   # 5 minutes
```

---

## `mount_denied`: file or directory rejected by mount allowlist

If the server is configured with `mount_allowlist`, `session_mount` will reject
any host path that does not start with one of the listed prefixes.

**Error:** `mount_denied` with a message like
`path is not under any allowed mount prefix`

**Fix:** Either use a path within an allowed directory, or update `config.toml`
to add the path:

```toml
mount_allowlist = ["/tmp/drun-inputs", "/Users/you/projects/data"]
```

---

## `command_denied`: command rejected by bash denylist

If the server is configured with `bash_command_denylist`, `session_bash` will
reject any command containing one of the listed substrings before it ever
reaches the sandbox.

**Error:** `command_denied` with a message naming the rejected command

**Fix:** Either avoid the denied substring, or update `config.toml`:

```toml
bash_command_denylist = ["curl", "wget", "nc"]
```

---

## `fetch_denied`: domain not in allowlist

`session_fetch` is the only network-capable tool — `session_bash` has no network
access at all, on either platform. `session_fetch` is restricted to the server's
domain allowlist, which by default only permits PyPI's CDNs.

**Error:** `fetch_denied` with a message naming the blocked domain

**Fix:** Add the domain to `config.toml`:

```toml
domain_allowlist = ["api.example.com", "data.sec.gov"]
```

Call `get_fetch_allowlist` to see the current effective list.

---

## `session_busy`: concurrent execution on the same session

Two simultaneous tool calls that mutate the same session (e.g. two
`session_bash` calls, or `session_bash` and `session_write_file` at once) return
`session_busy` immediately — a session executes one mutating call at a time.

**Fix:** Wait for the current call to complete, or create a separate session (or
fork) for parallel work.

---

## Session disappeared / `session_not_found`

Sessions are evicted after the configured idle timeout
(`session_idle_timeout_secs`, default 1 hour). A session that was idle for
longer than the timeout will have been cleaned up by the reaper.

Crossing the idle timeout does not immediately destroy the session; it is a
two-stage process:

1. Once a session has been idle longer than `session_idle_timeout_secs`, calls
   that would do new work (`session_bash`, `session_write_file`,
   `session_mount`, `session_rollback`, `session_merge`, label/squash/drop,
   etc.) start returning `session_idle` instead of running.
2. Read and recovery calls like `get_session_state`, `session_history`,
   `session_read_file`, `session_diff`, `session_commit`, `session_export`,
   `session_snapshot` and/ or `checkpoint_read_stdstreams`, keep working on an
   idle session. Use one of these to pull the session's state out before it is
   physically evicted.
3. The idle reaper sweeps roughly every
   `max(session_idle_timeout_secs / 2,
   30)` seconds and removes sessions
   still over the limit at that point, after which every call returns
   `session_not_found`.

**Fix:** As soon as you see `session_idle`, call `session_snapshot` (or
`session_export` / `session_commit`) to persist the session before the next
reaper sweep evicts it, then `session_restore` to reload it into a fresh
session. Alternatively, increase the idle timeout in config so long analyses
don't cross it in the first place.

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
- The exact error message or structured error code from the tool response
- The `DRUN_CONFIG` / `config.toml` you are using (redact any secrets)
- Steps to reproduce

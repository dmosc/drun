use std::{
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
};

use serde_json::{Map, Value};

/// A provider CLI exposing `<bin> mcp add|list|remove --scope user
/// --transport sse <name> <url>` — the shape Claude Code and Gemini CLI both
/// happen to share.
pub(crate) struct CliMcp {
    pub(crate) bin: &'static str,
    pub(crate) display_name: &'static str,
}

impl CliMcp {
    fn available(&self) -> bool {
        match Command::new(self.bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(_) => true,
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/sse", crate::mcp_port())
    }

    fn is_registered(list_output: &str) -> bool {
        list_output.lines().any(|l| l.starts_with("drun"))
    }

    fn add_command_hint(&self, url: &str) -> String {
        format!(
            "  {} mcp add --scope user --transport sse drun {url}",
            self.bin
        )
    }

    /// Registers drun as an SSE MCP server, user-scoped. Idempotent — checks
    /// `<bin> mcp list` first. Prints the equivalent command instead of
    /// running it if `bin` isn't on `PATH`, or if the add itself fails.
    pub(crate) fn register(&self) {
        let url = self.url();

        if !self.available() {
            eprintln!("drun: {} not found. Add drun manually:", self.display_name);
            eprintln!("{}", self.add_command_hint(&url));
            return;
        }

        let list_output = Command::new(self.bin)
            .args(["mcp", "list"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();

        if Self::is_registered(&list_output) {
            eprintln!(
                "drun: already registered in {}, skipping.",
                self.display_name
            );
            return;
        }

        let status = Command::new(self.bin)
            .args([
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "sse",
                "drun",
                &url,
            ])
            .status();

        if matches!(status, Ok(s) if s.success()) {
            eprintln!(
                "drun: added to {} (SSE → {url}, user scope).",
                self.display_name
            );
        } else {
            eprintln!(
                "drun: failed to register with {}. Add it manually:",
                self.display_name
            );
            eprintln!("{}", self.add_command_hint(&url));
        }
    }

    /// Undoes [`Self::register`]. No-op if `bin` isn't on `PATH`.
    pub(crate) fn deregister(&self) {
        if !self.available() {
            return;
        }

        let status = Command::new(self.bin)
            .args(["mcp", "remove", "--scope", "user", "drun"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if matches!(status, Ok(s) if s.success()) {
            eprintln!("drun: removed from {} (user scope).", self.display_name);
        }
    }
}

/// Writes a per-project JSON settings file: creates it from `render_default()`
/// if missing, otherwise merges drun's required keys into the existing file
/// via `merge()`. Shared by bridges (Claude Code, Gemini CLI) whose
/// native-tool-blocking config lives in such a file.
pub(crate) fn write_json_settings(
    settings_file: &Path,
    label: &str,
    render_default: impl Fn() -> String,
    merge: impl Fn(&str) -> Result<Option<String>, String>,
) {
    if let Some(dir) = settings_file.parent() {
        std::fs::create_dir_all(dir).expect("cannot create settings directory");
    }

    if !settings_file.exists() {
        std::fs::write(settings_file, render_default()).expect("cannot write settings file");
        eprintln!("drun: created {label}");
        return;
    }

    let existing =
        std::fs::read_to_string(settings_file).expect("cannot read existing settings file");
    match merge(&existing) {
        Ok(Some(merged)) => {
            std::fs::write(settings_file, merged).expect("cannot write settings file");
            eprintln!(
                "drun: updated {label} — merged in drun's required settings (native tools are \
                 now blocked for this project)"
            );
        }
        Ok(None) => eprintln!("drun: {label} already configured for drun, skipping"),
        Err(e) => eprintln!(
            "drun: could not merge into existing {label} ({e}) — leaving it untouched. Native \
             tools are NOT blocked until you add this yourself:\n{}",
            render_default()
        ),
    }
}

pub(crate) fn drun_instructions_body(project_path: &str) -> String {
    format!(
        r#"## Getting started

1. Call `create_session` — sessions start with an empty workspace.
2. Call `get_config` to see what domains, host paths, and env vars are already
   allowed. Check this before your first `session_fetch` or `session_mount`
   instead of discovering the allowlist through denied calls.
3. Call `session_mount` with path `{project_path}` to load this project's files
   into the session (already allowlisted by drun's setup for this project).
   Re-mount any other host paths you need the same way.
4. From there, work entirely through drun tools — there is no host file or shell
   access outside of them.

Every session_* tool below applies to the session you last created, forked, restored,
or switched to — none of them take a session_id.

## Reading command output

`session_bash` does not return stdout/stderr text inline — it returns state:
checkpoint_id, exit_code, stdout_bytes/stderr_bytes (byte counts, not content),
and which files changed. To read what a command actually printed, call
`checkpoint_read_stdstreams` against that checkpoint:

```
session_bash({{"command": "pytest -q"}})
→ {{"checkpoint_id": 3, "exit_code": 0, "stdout_bytes": 842, "stderr_bytes": 0, ...}}

checkpoint_read_stdstreams({{}})
→ {{"stream": "stdout", "exit_code": 0, "content": "...12 passed in 0.4s", ...}}
```

`checkpoint_read_stdstreams` defaults to the current checkpoint's stdout; pass
`{{"stream": "stderr"}}` for stderr, or `{{"checkpoint_id": N}}` for an older one.
Don't treat the JSON from `session_bash` itself as the command's output, and
don't treat non-empty stderr as failure — check `exit_code` instead; plenty of
commands write warnings or progress to stderr on success.

## Core tools

- **`get_config`** — see the domain/path/env allowlists and resource limits
  before you hit a denial
- **`session_bash`** — run shell commands in the sandboxed workspace (also
  covers listing/searching files — e.g. `ls`, `grep`, `find`). Returns state,
  not output — see "Reading command output" above.
- **`checkpoint_read_stdstreams`** — read the actual stdout/stderr text from
  any checkpoint
- **`session_read_file`** / **`session_write_file`** / **`session_delete_file`**
  — read, write, and delete files in the session
- **`session_mount`** — load a host file or directory into the session
- **`session_fetch`** — make HTTP requests from the sandbox (subject to the
  server's domain_allowlist — check with `get_config` first)
- **`session_export`** — write session files back out to the host
- **`session_diff`** / **`session_rollback`** / **`session_fork`** — inspect and
  navigate checkpoint history (session_rollback is destructive past the rollback
  point once you continue the session — use session_fork first if you need to
  keep that history)
- **`session_list`** / **`session_switch`** — see every session and change which
  one is active

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
"#
    )
}

pub(crate) fn write_project_instructions(project_dir: &Path, filename: &str, content: &str) {
    let path = project_dir.join(filename);

    if path.exists() {
        eprintln!("drun: {filename} already exists, skipping");
        return;
    }

    match std::fs::write(&path, content) {
        Ok(()) => eprintln!("drun: created {filename}"),
        Err(e) => eprintln!("drun: could not write {filename} ({e})"),
    }
}

pub(crate) fn register_project(drun_home: &Path, project_dir: &Path) {
    std::fs::create_dir_all(drun_home).expect("cannot create ~/.drun");
    let registry = drun_home.join("projects");
    let project_path = project_dir.to_str().expect("non-UTF-8 project path");

    let existing = std::fs::read_to_string(&registry).unwrap_or_default();
    if existing.lines().any(|l| l == project_path) {
        return;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&registry)
        .expect("cannot open project registry");
    writeln!(file, "{project_path}").expect("cannot write to project registry");
}

/// Adds `project_dir` to `mount_allowlist` in `drun_home`'s config.toml, if
/// that config already exists (i.e. drun-mcp itself has been installed) —
/// a no-op otherwise, since there's nothing to update yet.
pub(crate) fn allow_mount_path(drun_home: &Path, project_dir: &Path) {
    let config_path = drun_home.join("config.toml");
    if !config_path.exists() {
        return;
    }

    match crate::config_cmd::add_path_to(&config_path, project_dir) {
        Ok(true) => eprintln!("drun: added '{}' to mount_allowlist", project_dir.display()),
        Ok(false) => {}
        Err(e) => eprintln!(
            "drun: could not update mount_allowlist ({e}) — add it manually with \
             `drun-mcp config add-path {}`",
            project_dir.display()
        ),
    }
}

/// Adds any of `required` not already present in the JSON array at `key`
/// (creating it if missing). Returns whether the array changed. Used by any
/// bridge whose native-tool-blocking config is a JSON array under some key
/// (Claude's `permissions.deny`/`.allow`, Gemini's `excludeTools`).
pub(crate) fn merge_string_array(
    obj: &mut Map<String, Value>,
    key: &str,
    required: &[&str],
) -> Result<bool, String> {
    let array = obj.entry(key).or_insert_with(|| Value::Array(Vec::new()));
    let array = array
        .as_array_mut()
        .ok_or_else(|| format!("'{key}' is not an array"))?;

    let mut changed = false;
    for &item in required {
        if !array.iter().any(|v| v.as_str() == Some(item)) {
            array.push(Value::String(item.to_string()));
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_mcp_is_registered_matches_a_leading_drun_line() {
        assert!(CliMcp::is_registered(
            "drun: http://127.0.0.1:7273/sse (SSE) - Ready\n"
        ));
        assert!(!CliMcp::is_registered("other-server: some-url - Ready\n"));
        assert!(!CliMcp::is_registered(""));
    }

    #[test]
    fn merge_string_array_creates_the_key_when_missing() {
        let mut obj = Map::new();
        let changed = merge_string_array(&mut obj, "excludeTools", &["ShellTool"]).unwrap();
        assert!(changed);
        assert_eq!(obj["excludeTools"][0], "ShellTool");
    }

    #[test]
    fn merge_string_array_dedupes_against_existing_entries() {
        let mut obj = Map::new();
        obj.insert(
            "excludeTools".into(),
            Value::Array(vec!["ShellTool".into()]),
        );
        let changed = merge_string_array(&mut obj, "excludeTools", &["ShellTool"]).unwrap();
        assert!(!changed);
        assert_eq!(obj["excludeTools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn merge_string_array_preserves_user_entries() {
        let mut obj = Map::new();
        obj.insert("excludeTools".into(), Value::Array(vec!["Custom".into()]));
        merge_string_array(&mut obj, "excludeTools", &["ShellTool"]).unwrap();
        let array = obj["excludeTools"].as_array().unwrap();
        assert!(array.contains(&Value::String("Custom".into())));
        assert!(array.contains(&Value::String("ShellTool".into())));
    }

    #[test]
    fn merge_string_array_errors_when_key_is_not_an_array() {
        let mut obj = Map::new();
        obj.insert("excludeTools".into(), Value::String("oops".into()));
        let err = merge_string_array(&mut obj, "excludeTools", &["ShellTool"]).unwrap_err();
        assert!(err.contains("not an array"));
    }

    #[test]
    fn drun_instructions_body_includes_the_project_path() {
        let body = drun_instructions_body("/home/user/myproject");
        assert!(body.contains("/home/user/myproject"));
    }

    #[test]
    fn drun_instructions_body_documents_the_core_tools() {
        let body = drun_instructions_body("/tmp/project");
        assert!(body.contains("session_bash"));
        assert!(body.contains("session_mount"));
    }

    #[test]
    fn drun_instructions_body_explains_reading_command_output() {
        let body = drun_instructions_body("/tmp/project");
        assert!(body.contains("checkpoint_read_stdstreams"));
        assert!(body.contains("does not return stdout/stderr text inline"));
    }

    #[test]
    fn drun_instructions_body_documents_get_config() {
        let body = drun_instructions_body("/tmp/project");
        assert!(body.contains("get_config"));
    }

    #[test]
    fn write_project_instructions_creates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_project_instructions(dir.path(), "HERMES.md", "content");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("HERMES.md")).unwrap(),
            "content"
        );
    }

    #[test]
    fn write_project_instructions_does_not_overwrite_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("HERMES.md"), "custom").unwrap();

        write_project_instructions(dir.path(), "HERMES.md", "generated");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("HERMES.md")).unwrap(),
            "custom"
        );
    }

    #[test]
    fn register_project_appends_the_project_path() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        register_project(drun_home.path(), project_dir.path());

        let registry = std::fs::read_to_string(drun_home.path().join("projects")).unwrap();
        assert!(
            registry
                .lines()
                .any(|l| l == project_dir.path().to_str().unwrap())
        );
    }

    #[test]
    fn register_project_does_not_duplicate_an_already_registered_path() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        register_project(drun_home.path(), project_dir.path());
        register_project(drun_home.path(), project_dir.path());

        let registry = std::fs::read_to_string(drun_home.path().join("projects")).unwrap();
        let occurrences = registry
            .lines()
            .filter(|l| *l == project_dir.path().to_str().unwrap())
            .count();
        assert_eq!(occurrences, 1);
    }

    #[test]
    fn allow_mount_path_adds_the_project_dir_to_an_existing_config() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            drun_home.path().join("config.toml"),
            "mount_allowlist = []\n",
        )
        .unwrap();

        allow_mount_path(drun_home.path(), project_dir.path());

        let config = drun_core::Config::load_from(Some(&drun_home.path().join("config.toml")));
        assert!(
            config
                .mount_allowlist
                .contains(&project_dir.path().to_path_buf())
        );
    }

    #[test]
    fn allow_mount_path_is_a_no_op_without_a_daemon_config() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        allow_mount_path(drun_home.path(), project_dir.path());

        assert!(!drun_home.path().join("config.toml").exists());
    }
}

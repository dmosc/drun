pub mod claude;
pub mod codex;
pub mod gemini;
pub mod hermes;

use std::{
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
};

use serde_json::{Map, Value};

/// Where a bridge's registration takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Scoped to the current working directory. Safe to run once per
    /// project without affecting drun's behavior in any other project.
    Project,
    /// Affects every session on this machine, regardless of project.
    Machine,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Machine => "machine",
        }
    }
}

/// A single agentic coding tool drun can wire itself into.
///
/// Implementations live in their own module (see `claude`, `gemini`, `codex`,
/// `hermes`) and are exposed on the CLI by name — `drun-mcp <name> init` /
/// `drun-mcp <name> deregister` — via `REGISTRY` below, which both `main`'s
/// dispatch and `drun-mcp bridges list`/`deregister-all` read generically.
/// Adding a new bridge (Cline, Amazon Q Developer CLI, ...) means adding a new
/// module that implements this trait and one line in `REGISTRY` — every CLI
/// entry point picks it up automatically. The one exception: if it writes a
/// per-project settings file (like Claude's/Gemini's), add that filename to
/// `BRIDGE_SETTINGS_FILES` in `uninstall.sh` too, so uninstalling removes it.
///
/// The default methods below are the plumbing every bridge shares — writing
/// the instructions file, registering the project, merging a JSON settings
/// file, registering with a provider CLI's own `mcp add`. `init_common` is
/// the template method that sequences the first three; each impl calls it,
/// then does whatever is left that's actually provider-specific.
pub trait Bridge {
    /// Stable identifier used on the CLI (`drun-mcp <name> init`).
    fn name(&self) -> &'static str;

    /// One-line summary shown in `--help` and `drun-mcp bridges list`.
    fn description(&self) -> &'static str;

    fn scope(&self) -> Scope;

    /// Wire drun into this bridge. Must be idempotent: safe to call
    /// repeatedly, including once per project for a `Scope::Project` bridge.
    fn init(&self);

    /// Undo `init`. Must be safe to call even if never registered.
    fn deregister(&self);

    /// `(bin, display_name)` for bridges that register via a provider CLI's
    /// own `<bin> mcp add/list/remove` (Claude Code, Gemini CLI). Bridges
    /// that hand-edit their config file directly (Codex, Hermes) leave this
    /// as the default `None` — `register_cli_mcp`/`deregister_cli_mcp` then
    /// no-op.
    fn cli_bin(&self) -> Option<(&'static str, &'static str)> {
        None
    }

    /// The part of `init` every bridge shares: write the agent-instructions
    /// file, then register `project_dir` in drun's mount allowlist and
    /// project registry. Call this first, before any provider-specific
    /// config mutation.
    fn init_common(
        &self,
        project_dir: &Path,
        drun_home: &Path,
        instructions_filename: &str,
        instructions_content: &str,
    ) {
        self.write_project_instructions(project_dir, instructions_filename, instructions_content);
        self.allow_mount_path(drun_home, project_dir);
        self.register_project(drun_home, project_dir);
    }

    fn write_project_instructions(&self, project_dir: &Path, filename: &str, content: &str) {
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

    fn register_project(&self, drun_home: &Path, project_dir: &Path) {
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
    fn allow_mount_path(&self, drun_home: &Path, project_dir: &Path) {
        let config_path = drun_home.join("config.toml");
        if !config_path.exists() {
            return;
        }

        match crate::ConfigCmd::add_path_to(&config_path, project_dir) {
            Ok(true) => eprintln!("drun: added '{}' to mount_allowlist", project_dir.display()),
            Ok(false) => {}
            Err(e) => eprintln!(
                "drun: could not update mount_allowlist ({e}) — add it manually with \
                 `drun-mcp config add-path {}`",
                project_dir.display()
            ),
        }
    }

    /// Writes a per-project JSON settings file: creates it from `render_default()`
    /// if missing, otherwise merges drun's required keys into the existing file
    /// via `merge()`. Used by bridges (Claude Code, Gemini CLI) whose
    /// native-tool-blocking config lives in such a file.
    fn write_json_settings(
        &self,
        settings_file: &Path,
        label: &str,
        render_default: impl Fn() -> String,
        merge: impl Fn(&str) -> Result<Option<String>, String>,
    ) where
        Self: Sized,
    {
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

    /// Adds any of `required` not already present in the JSON array at `key`
    /// (creating it if missing). Returns whether the array changed. Used by any
    /// bridge whose native-tool-blocking config is a JSON array under some key
    /// (Claude's `permissions.deny`/`.allow`, Gemini's `excludeTools`).
    fn merge_string_array(
        &self,
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

    fn drun_instructions_body(&self, project_path: &str) -> String {
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
- **`session_extract_text`** — pull plain text out of a binary document already
  in the session (e.g. a PDF) and save it as a new file
- **`session_package_install`** — install `pip`/`npm` packages so later
  `session_bash` calls can import them. Disabled by default; if it returns
  `package_install_disabled`, tell the user to set
  `package_install_enabled = true` in `~/.drun/config.toml` (no CLI helper for
  this one — hand-edit, then it applies on your next call, no restart)
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

    fn cli_mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/sse", crate::Env.mcp_port())
    }

    fn cli_mcp_available(&self, bin: &str) -> bool {
        match Command::new(bin)
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

    fn cli_mcp_is_registered(&self, list_output: &str) -> bool {
        list_output.lines().any(|l| l.starts_with("drun"))
    }

    fn cli_mcp_add_hint(&self, bin: &str, url: &str) -> String {
        format!("  {bin} mcp add --scope user --transport sse drun {url}")
    }

    /// Registers drun as an SSE MCP server, user-scoped, with the provider
    /// CLI named by `cli_bin`. No-ops if `cli_bin` is `None`. Idempotent —
    /// checks `<bin> mcp list` first. Prints the equivalent command instead
    /// of running it if `bin` isn't on `PATH`, or if the add itself fails.
    fn register_cli_mcp(&self) {
        let Some((bin, display_name)) = self.cli_bin() else {
            return;
        };
        let url = self.cli_mcp_url();

        if !self.cli_mcp_available(bin) {
            eprintln!("drun: {display_name} not found. Add drun manually:");
            eprintln!("{}", self.cli_mcp_add_hint(bin, &url));
            return;
        }

        let list_output = Command::new(bin)
            .args(["mcp", "list"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();

        if self.cli_mcp_is_registered(&list_output) {
            eprintln!("drun: already registered in {display_name}, skipping.");
            return;
        }

        let status = Command::new(bin)
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
            eprintln!("drun: added to {display_name} (SSE → {url}, user scope).");
        } else {
            eprintln!("drun: failed to register with {display_name}. Add it manually:");
            eprintln!("{}", self.cli_mcp_add_hint(bin, &url));
        }
    }

    /// Undoes [`Self::register_cli_mcp`]. No-op if `cli_bin` is `None` or
    /// `bin` isn't on `PATH`.
    fn deregister_cli_mcp(&self) {
        let Some((bin, display_name)) = self.cli_bin() else {
            return;
        };
        if !self.cli_mcp_available(bin) {
            return;
        }

        let status = Command::new(bin)
            .args(["mcp", "remove", "--scope", "user", "drun"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if matches!(status, Ok(s) if s.success()) {
            eprintln!("drun: removed from {display_name} (user scope).");
        }
    }
}

/// Every bridge drun knows about, plus the operations that act on all of them
/// at once — lookup by name, listing, mass deregistration. Distinct from
/// `Bridge` itself: these operate on the collection, not on one instance.
pub struct BridgeRegistry(&'static [&'static dyn Bridge]);

impl BridgeRegistry {
    /// Looks up a bridge by its CLI name (`drun-mcp <name> ...`).
    pub fn find(&self, name: &str) -> Option<&'static dyn Bridge> {
        self.0.iter().copied().find(|b| b.name() == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static dyn Bridge> + '_ {
        self.0.iter().copied()
    }

    /// Best-effort `deregister()` across every known bridge — used by
    /// `uninstall.sh` so it doesn't need to name providers individually.
    pub fn deregister_all(&self) {
        for bridge in self.0 {
            bridge.deregister();
        }
    }

    /// Human-readable listing for `drun-mcp bridges list`. Printed to stdout
    /// (unlike `init`/`deregister`'s progress messages on stderr) since this
    /// is query output meant to be scriptable — e.g. `install.sh` pipes it
    /// directly into its closing summary.
    pub fn print_list(&self) {
        let width = self.0.iter().map(|b| b.name().len()).max().unwrap_or(0);
        for bridge in self.0 {
            println!(
                "{:<width$}  [{:<7}]  {}",
                bridge.name(),
                bridge.scope().label(),
                bridge.description()
            );
        }
    }
}

/// Display and dispatch order.
pub const REGISTRY: BridgeRegistry = BridgeRegistry(&[
    &claude::Claude,
    &gemini::Gemini,
    &codex::Codex,
    &hermes::Hermes,
]);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_names_are_unique() {
        let mut names: Vec<&str> = REGISTRY.iter().map(|b| b.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate bridge name in REGISTRY");
    }

    #[test]
    fn find_locates_every_registered_bridge_by_name() {
        for bridge in REGISTRY.iter() {
            assert_eq!(
                REGISTRY.find(bridge.name()).map(|b| b.name()),
                Some(bridge.name())
            );
        }
    }

    #[test]
    fn find_returns_none_for_an_unknown_name() {
        assert!(REGISTRY.find("does-not-exist").is_none());
    }

    struct Dummy;
    impl Bridge for Dummy {
        fn name(&self) -> &'static str {
            "dummy"
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn scope(&self) -> Scope {
            Scope::Project
        }
        fn init(&self) {}
        fn deregister(&self) {}
    }

    #[test]
    fn init_common_writes_the_instructions_file_and_registers_the_project() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        Dummy.init_common(project_dir.path(), drun_home.path(), "AGENTS.md", "hello");

        assert_eq!(
            std::fs::read_to_string(project_dir.path().join("AGENTS.md")).unwrap(),
            "hello"
        );
        let registry = std::fs::read_to_string(drun_home.path().join("projects")).unwrap();
        assert!(
            registry
                .lines()
                .any(|l| l == project_dir.path().to_str().unwrap())
        );
    }

    #[test]
    fn init_common_adds_the_project_dir_to_an_existing_mount_allowlist() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            drun_home.path().join("config.toml"),
            "mount_allowlist = []\n",
        )
        .unwrap();

        Dummy.init_common(project_dir.path(), drun_home.path(), "AGENTS.md", "hello");

        let config = drun_core::Config::load_from(Some(&drun_home.path().join("config.toml")));
        assert!(
            config
                .mount_allowlist
                .contains(&project_dir.path().to_path_buf())
        );
    }

    #[test]
    fn write_project_instructions_creates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        Dummy.write_project_instructions(dir.path(), "HERMES.md", "content");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("HERMES.md")).unwrap(),
            "content"
        );
    }

    #[test]
    fn write_project_instructions_does_not_overwrite_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("HERMES.md"), "custom").unwrap();

        Dummy.write_project_instructions(dir.path(), "HERMES.md", "generated");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("HERMES.md")).unwrap(),
            "custom"
        );
    }

    #[test]
    fn register_project_appends_the_project_path() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        Dummy.register_project(drun_home.path(), project_dir.path());

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

        Dummy.register_project(drun_home.path(), project_dir.path());
        Dummy.register_project(drun_home.path(), project_dir.path());

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

        Dummy.allow_mount_path(drun_home.path(), project_dir.path());

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

        Dummy.allow_mount_path(drun_home.path(), project_dir.path());

        assert!(!drun_home.path().join("config.toml").exists());
    }

    #[test]
    fn merge_string_array_creates_the_key_when_missing() {
        let mut obj = Map::new();
        let changed = Dummy
            .merge_string_array(&mut obj, "excludeTools", &["ShellTool"])
            .unwrap();
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
        let changed = Dummy
            .merge_string_array(&mut obj, "excludeTools", &["ShellTool"])
            .unwrap();
        assert!(!changed);
        assert_eq!(obj["excludeTools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn merge_string_array_preserves_user_entries() {
        let mut obj = Map::new();
        obj.insert("excludeTools".into(), Value::Array(vec!["Custom".into()]));
        Dummy
            .merge_string_array(&mut obj, "excludeTools", &["ShellTool"])
            .unwrap();
        let array = obj["excludeTools"].as_array().unwrap();
        assert!(array.contains(&Value::String("Custom".into())));
        assert!(array.contains(&Value::String("ShellTool".into())));
    }

    #[test]
    fn merge_string_array_errors_when_key_is_not_an_array() {
        let mut obj = Map::new();
        obj.insert("excludeTools".into(), Value::String("oops".into()));
        let err = Dummy
            .merge_string_array(&mut obj, "excludeTools", &["ShellTool"])
            .unwrap_err();
        assert!(err.contains("not an array"));
    }

    #[test]
    fn drun_instructions_body_includes_the_project_path() {
        let body = Dummy.drun_instructions_body("/home/user/myproject");
        assert!(body.contains("/home/user/myproject"));
    }

    #[test]
    fn drun_instructions_body_documents_the_core_tools() {
        let body = Dummy.drun_instructions_body("/tmp/project");
        assert!(body.contains("session_bash"));
        assert!(body.contains("session_mount"));
    }

    #[test]
    fn drun_instructions_body_explains_reading_command_output() {
        let body = Dummy.drun_instructions_body("/tmp/project");
        assert!(body.contains("checkpoint_read_stdstreams"));
        assert!(body.contains("does not return stdout/stderr text inline"));
    }

    #[test]
    fn drun_instructions_body_documents_get_config() {
        let body = Dummy.drun_instructions_body("/tmp/project");
        assert!(body.contains("get_config"));
    }

    #[test]
    fn cli_mcp_is_registered_matches_a_leading_drun_line() {
        assert!(Dummy.cli_mcp_is_registered("drun: http://127.0.0.1:7273/sse (SSE) - Ready\n"));
        assert!(!Dummy.cli_mcp_is_registered("other-server: some-url - Ready\n"));
        assert!(!Dummy.cli_mcp_is_registered(""));
    }
}

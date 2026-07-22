#[cfg(test)]
use std::path::Path;
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use serde_json::{Map, Value, json};

const REQUIRED_DENY: &[&str] = &[
    "Bash",
    "BashOutput",
    "KillBash",
    "Edit",
    "Write",
    "NotebookEdit",
    "Read",
    "Glob",
    "Grep",
    "WebFetch",
    "WebSearch",
    "Task",
    "Curl",
    "Wget",
];
const REQUIRED_ALLOW: &[&str] = &["mcp__drun__*"];

/// [`super::Bridge`] impl — see that trait for the extensibility contract.
pub struct Claude;

impl super::Bridge for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn description(&self) -> &'static str {
        "Claude Code — blocks native tools per-project, registers the MCP server"
    }

    fn scope(&self) -> super::Scope {
        super::Scope::Project
    }

    fn init(&self) {
        register_mcp();

        let project = ProjectInit {
            project_dir: std::env::current_dir().expect("cannot read current directory"),
            drun_home: crate::drun_home(),
        };

        project.write_settings();
        project.write_claude_md();
        project.register_project();
        project.allow_mount_path();

        eprintln!("drun: initialized for {}", project.project_dir.display());
    }

    fn deregister(&self) {
        if !claude_available() {
            return;
        }

        let status = Command::new("claude")
            .args(["mcp", "remove", "--scope", "user", "drun"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if matches!(status, Ok(s) if s.success()) {
            eprintln!("drun: removed from Claude Code (user scope).");
        }
    }
}

fn rendered_default_settings() -> String {
    let value = json!({
        "permissions": {
            "deny": REQUIRED_DENY,
            "allow": REQUIRED_ALLOW,
        }
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("static json value")
    )
}

fn mcp_url() -> String {
    format!("http://127.0.0.1:{}/sse", crate::mcp_port())
}

fn claude_available() -> bool {
    match Command::new("claude")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

fn already_registered(mcp_list_output: &str) -> bool {
    mcp_list_output.lines().any(|l| l.starts_with("drun"))
}

fn register_mcp() {
    let url = mcp_url();

    if !claude_available() {
        eprintln!("drun: Claude Code CLI not found. Add drun manually:");
        eprintln!("  claude mcp add --scope user --transport sse drun {url}");
        return;
    }

    let list_output = Command::new("claude")
        .args(["mcp", "list"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    if already_registered(&list_output) {
        eprintln!("drun: already registered in Claude Code, skipping.");
        return;
    }

    let status = Command::new("claude")
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
        eprintln!("drun: added to Claude Code (SSE → {url}, user scope).");
    } else {
        eprintln!("drun: failed to register with Claude Code. Add it manually:");
        eprintln!("  claude mcp add --scope user --transport sse drun {url}");
    }
}

struct ProjectInit {
    project_dir: PathBuf,
    drun_home: PathBuf,
}

impl ProjectInit {
    fn allow_mount_path(&self) {
        super::shared::allow_mount_path(&self.drun_home, &self.project_dir);
    }

    fn write_settings(&self) {
        let settings_dir = self.project_dir.join(".claude");
        let settings_file = settings_dir.join("settings.json");

        std::fs::create_dir_all(&settings_dir).expect("cannot create .claude/");

        if !settings_file.exists() {
            std::fs::write(&settings_file, rendered_default_settings())
                .expect("cannot write settings.json");
            eprintln!("drun: created .claude/settings.json");
            return;
        }

        let existing =
            std::fs::read_to_string(&settings_file).expect("cannot read existing settings.json");
        match merge_settings(&existing) {
            Ok(Some(merged)) => {
                std::fs::write(&settings_file, merged).expect("cannot write settings.json");
                eprintln!(
                    "drun: updated .claude/settings.json — merged in drun's required permissions \
                     (native tools are now blocked for this project)"
                );
            }
            Ok(None) => {
                eprintln!("drun: .claude/settings.json already configured for drun, skipping");
            }
            Err(e) => {
                eprintln!(
                    "drun: could not merge into existing .claude/settings.json ({e}) — leaving \
                     it untouched. Native tools are NOT blocked until you add this yourself:\n{}",
                    rendered_default_settings()
                );
            }
        }
    }

    fn write_claude_md(&self) {
        let project_path = self.project_dir.to_str().expect("non-UTF-8 project path");
        super::shared::write_project_instructions(
            &self.project_dir,
            "CLAUDE.md",
            &claude_md_content(project_path),
        );
    }

    fn register_project(&self) {
        std::fs::create_dir_all(&self.drun_home).expect("cannot create ~/.drun");
        let registry = self.drun_home.join("projects");
        let project_path = self.project_dir.to_str().expect("non-UTF-8 project path");

        let existing = std::fs::read_to_string(&registry).unwrap_or_default();
        if existing.lines().any(|l| l == project_path) {
            return;
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&registry)
            .expect("cannot open ~/.drun/projects");
        writeln!(file, "{project_path}").expect("cannot write to project registry");
    }
}

fn merge_settings(existing: &str) -> Result<Option<String>, String> {
    let mut value: Value = serde_json::from_str(existing).map_err(|e| e.to_string())?;
    let Some(root) = value.as_object_mut() else {
        return Err("root is not a JSON object".to_string());
    };

    let permissions = root.entry("permissions").or_insert_with(|| json!({}));
    let Some(permissions) = permissions.as_object_mut() else {
        return Err("'permissions' is not an object".to_string());
    };

    let deny_changed = merge_string_array(permissions, "deny", REQUIRED_DENY)?;
    let allow_changed = merge_string_array(permissions, "allow", REQUIRED_ALLOW)?;

    if !deny_changed && !allow_changed {
        return Ok(None);
    }

    let rendered = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Ok(Some(format!("{rendered}\n")))
}

fn merge_string_array(
    obj: &mut Map<String, Value>,
    key: &str,
    required: &[&str],
) -> Result<bool, String> {
    let array = obj.entry(key).or_insert_with(|| json!([]));
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

fn claude_md_content(project_path: &str) -> String {
    format!(
        "# Agent instructions\n\n\
         This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.\n\
         Native file, shell, and network tools (`Bash`, `Edit`, `Write`, `NotebookEdit`,\n\
         `Read`, `Glob`, `Grep`, `WebFetch`, `WebSearch`, `Task`) are disabled for this\n\
         workspace — they would otherwise read or write the host directly, bypassing the\n\
         sandbox. Use the drun MCP tools (prefixed `mcp__drun__`) for everything.\n\n{}",
        super::shared::drun_instructions_body(project_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_init(drun_home: &Path, project_dir: &Path) -> ProjectInit {
        ProjectInit {
            drun_home: drun_home.to_path_buf(),
            project_dir: project_dir.to_path_buf(),
        }
    }

    #[test]
    fn already_registered_matches_a_leading_drun_line() {
        assert!(already_registered(
            "drun: http://127.0.0.1:7273/sse (SSE) - ✓ Connected\n"
        ));
        assert!(!already_registered(
            "other-server: some-url - ✓ Connected\n"
        ));
        assert!(!already_registered(""));
    }

    #[test]
    fn claude_md_content_includes_the_project_path() {
        let content = claude_md_content("/home/user/myproject");
        assert!(content.contains("/home/user/myproject"));
    }

    #[test]
    fn claude_md_content_documents_the_core_tools() {
        let content = claude_md_content("/tmp/project");
        assert!(content.contains("session_bash"));
        assert!(content.contains("session_mount"));
    }

    #[test]
    fn write_settings_creates_claude_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        project_init(dir.path(), dir.path()).write_settings();
        let settings_path = dir.path().join(".claude/settings.json");
        assert!(settings_path.exists());
        let content = std::fs::read_to_string(&settings_path).unwrap();
        assert!(content.contains("mcp__drun__*"));
    }

    #[test]
    fn write_settings_leaves_unparseable_existing_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_path = settings_dir.join("settings.json");
        std::fs::write(&settings_path, "custom content").unwrap();

        project_init(dir.path(), dir.path()).write_settings();

        assert_eq!(
            std::fs::read_to_string(&settings_path).unwrap(),
            "custom content"
        );
    }

    #[test]
    fn write_settings_merges_into_an_existing_file_with_unrelated_content() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_path = settings_dir.join("settings.json");
        std::fs::write(&settings_path, r#"{"env": {"FOO": "bar"}}"#).unwrap();

        project_init(dir.path(), dir.path()).write_settings();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["env"]["FOO"], "bar");
        assert_eq!(value["permissions"]["allow"][0], "mcp__drun__*");
        assert!(
            value["permissions"]["deny"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "Bash")
        );
    }

    #[test]
    fn write_settings_merges_missing_entries_into_partial_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_path = settings_dir.join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{"permissions": {"deny": ["Bash"], "allow": ["SomeOtherTool"]}}"#,
        )
        .unwrap();

        project_init(dir.path(), dir.path()).write_settings();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        let deny = value["permissions"]["deny"].as_array().unwrap();
        let allow = value["permissions"]["allow"].as_array().unwrap();
        for required in REQUIRED_DENY {
            assert!(deny.iter().any(|v| v == required), "missing {required}");
        }
        assert!(allow.iter().any(|v| v == "SomeOtherTool"));
        assert!(allow.iter().any(|v| v == "mcp__drun__*"));
    }

    #[test]
    fn write_settings_is_idempotent_once_fully_merged() {
        let dir = tempfile::tempdir().unwrap();
        project_init(dir.path(), dir.path()).write_settings();
        let settings_path = dir.path().join(".claude/settings.json");
        let first = std::fs::read_to_string(&settings_path).unwrap();

        project_init(dir.path(), dir.path()).write_settings();

        let second = std::fs::read_to_string(&settings_path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn merge_settings_errors_when_permissions_is_not_an_object() {
        let err = merge_settings(r#"{"permissions": "oops"}"#).unwrap_err();
        assert!(err.contains("'permissions'"));
    }

    #[test]
    fn merge_settings_errors_when_root_is_not_an_object() {
        let err = merge_settings(r#"["not", "an", "object"]"#).unwrap_err();
        assert!(err.contains("root"));
    }

    #[test]
    fn write_claude_md_creates_the_file_with_project_path() {
        let dir = tempfile::tempdir().unwrap();
        project_init(dir.path(), dir.path()).write_claude_md();
        let path = dir.path().join("CLAUDE.md");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(dir.path().to_str().unwrap()));
    }

    #[test]
    fn write_claude_md_does_not_overwrite_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "custom").unwrap();

        project_init(dir.path(), dir.path()).write_claude_md();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap(),
            "custom"
        );
    }

    #[test]
    fn allow_mount_path_delegates_to_the_shared_implementation() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            drun_home.path().join("config.toml"),
            "mount_allowlist = []\n",
        )
        .unwrap();

        project_init(drun_home.path(), project_dir.path()).allow_mount_path();

        let config = drun_core::Config::load_from(Some(&drun_home.path().join("config.toml")));
        assert!(
            config
                .mount_allowlist
                .contains(&project_dir.path().to_path_buf())
        );
    }

    #[test]
    fn register_project_appends_the_project_path() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        project_init(drun_home.path(), project_dir.path()).register_project();

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

        project_init(drun_home.path(), project_dir.path()).register_project();
        project_init(drun_home.path(), project_dir.path()).register_project();

        let registry = std::fs::read_to_string(drun_home.path().join("projects")).unwrap();
        let occurrences = registry
            .lines()
            .filter(|l| *l == project_dir.path().to_str().unwrap())
            .count();
        assert_eq!(occurrences, 1);
    }
}

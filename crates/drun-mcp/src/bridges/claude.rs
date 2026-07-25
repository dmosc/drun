#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use serde_json::{Value, json};

const CLI: super::shared::CliMcp = super::shared::CliMcp {
    bin: "claude",
    display_name: "Claude Code",
};

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
        let project = ProjectInit {
            project_dir: std::env::current_dir().expect("cannot read current directory"),
            drun_home: crate::drun_home(),
        };
        project.write_settings();
        project.write_claude_md();
        project.allow_mount_path();
        project.register_project();
        CLI.register();
        eprintln!("drun: initialized for {}", project.project_dir.display());
    }

    fn deregister(&self) {
        CLI.deregister();
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

struct ProjectInit {
    project_dir: PathBuf,
    drun_home: PathBuf,
}

impl ProjectInit {
    fn allow_mount_path(&self) {
        super::shared::allow_mount_path(&self.drun_home, &self.project_dir);
    }

    fn write_settings(&self) {
        let settings_file = self.project_dir.join(".claude").join("settings.json");
        super::shared::write_json_settings(
            &settings_file,
            ".claude/settings.json",
            rendered_default_settings,
            merge_settings,
        );
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
        super::shared::register_project(&self.drun_home, &self.project_dir);
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

    let deny_changed = super::shared::merge_string_array(permissions, "deny", REQUIRED_DENY)?;
    let allow_changed = super::shared::merge_string_array(permissions, "allow", REQUIRED_ALLOW)?;

    if !deny_changed && !allow_changed {
        return Ok(None);
    }

    let rendered = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Ok(Some(format!("{rendered}\n")))
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
    fn register_project_delegates_to_the_shared_implementation() {
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
}

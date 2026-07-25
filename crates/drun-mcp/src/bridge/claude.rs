use std::path::Path;

use serde_json::{Value, json};

use super::Bridge;

/// [`Bridge`] impl — see that trait for the extensibility contract.
pub struct Claude;

impl Bridge for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn description(&self) -> &'static str {
        "Claude Code — blocks native tools per-project, registers the MCP server"
    }

    fn scope(&self) -> super::Scope {
        super::Scope::Project
    }

    fn cli_bin(&self) -> Option<(&'static str, &'static str)> {
        Some(("claude", "Claude Code"))
    }

    fn init(&self) {
        let project_dir = std::env::current_dir().expect("cannot read current directory");
        let project_path = project_dir.to_str().expect("non-UTF-8 project path");
        self.init_common(
            &project_dir,
            &crate::Env.drun_home(),
            "CLAUDE.md",
            &self.claude_md_content(project_path),
        );
        self.write_settings(&project_dir);
        self.register_cli_mcp();
        eprintln!("drun: initialized for {}", project_dir.display());
    }

    fn deregister(&self) {
        self.deregister_cli_mcp();
    }
}

impl Claude {
    const REQUIRED_DENY: &'static [&'static str] = &[
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
    const REQUIRED_ALLOW: &'static [&'static str] = &["mcp__drun__*"];

    fn write_settings(&self, project_dir: &Path) {
        let settings_file = project_dir.join(".claude").join("settings.json");
        self.write_json_settings(
            &settings_file,
            ".claude/settings.json",
            || self.rendered_default_settings(),
            |existing| self.merge_settings(existing),
        );
    }

    fn rendered_default_settings(&self) -> String {
        let value = json!({
            "permissions": {
                "deny": Self::REQUIRED_DENY,
                "allow": Self::REQUIRED_ALLOW,
            }
        });
        format!(
            "{}\n",
            serde_json::to_string_pretty(&value).expect("static json value")
        )
    }

    fn merge_settings(&self, existing: &str) -> Result<Option<String>, String> {
        let mut value: Value = serde_json::from_str(existing).map_err(|e| e.to_string())?;
        let Some(root) = value.as_object_mut() else {
            return Err("root is not a JSON object".to_string());
        };

        let permissions = root.entry("permissions").or_insert_with(|| json!({}));
        let Some(permissions) = permissions.as_object_mut() else {
            return Err("'permissions' is not an object".to_string());
        };

        let deny_changed = self.merge_string_array(permissions, "deny", Self::REQUIRED_DENY)?;
        let allow_changed = self.merge_string_array(permissions, "allow", Self::REQUIRED_ALLOW)?;

        if !deny_changed && !allow_changed {
            return Ok(None);
        }

        let rendered = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        Ok(Some(format!("{rendered}\n")))
    }

    fn claude_md_content(&self, project_path: &str) -> String {
        format!(
            "# Agent instructions\n\n\
             This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.\n\
             Native file, shell, and network tools (`Bash`, `Edit`, `Write`, `NotebookEdit`,\n\
             `Read`, `Glob`, `Grep`, `WebFetch`, `WebSearch`, `Task`) are disabled for this\n\
             workspace — they would otherwise read or write the host directly, bypassing the\n\
             sandbox. Use the drun MCP tools (prefixed `mcp__drun__`) for everything.\n\n{}",
            self.drun_instructions_body(project_path)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_md_content_includes_the_project_path() {
        let content = Claude.claude_md_content("/home/user/myproject");
        assert!(content.contains("/home/user/myproject"));
    }

    #[test]
    fn claude_md_content_documents_the_core_tools() {
        let content = Claude.claude_md_content("/tmp/project");
        assert!(content.contains("session_bash"));
        assert!(content.contains("session_mount"));
    }

    #[test]
    fn write_settings_creates_claude_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        Claude.write_settings(dir.path());
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

        Claude.write_settings(dir.path());

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

        Claude.write_settings(dir.path());

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

        Claude.write_settings(dir.path());

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        let deny = value["permissions"]["deny"].as_array().unwrap();
        let allow = value["permissions"]["allow"].as_array().unwrap();
        for required in Claude::REQUIRED_DENY {
            assert!(deny.iter().any(|v| v == required), "missing {required}");
        }
        assert!(allow.iter().any(|v| v == "SomeOtherTool"));
        assert!(allow.iter().any(|v| v == "mcp__drun__*"));
    }

    #[test]
    fn write_settings_is_idempotent_once_fully_merged() {
        let dir = tempfile::tempdir().unwrap();
        Claude.write_settings(dir.path());
        let settings_path = dir.path().join(".claude/settings.json");
        let first = std::fs::read_to_string(&settings_path).unwrap();

        Claude.write_settings(dir.path());

        let second = std::fs::read_to_string(&settings_path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn merge_settings_errors_when_permissions_is_not_an_object() {
        let err = Claude
            .merge_settings(r#"{"permissions": "oops"}"#)
            .unwrap_err();
        assert!(err.contains("'permissions'"));
    }

    #[test]
    fn merge_settings_errors_when_root_is_not_an_object() {
        let err = Claude
            .merge_settings(r#"["not", "an", "object"]"#)
            .unwrap_err();
        assert!(err.contains("root"));
    }
}

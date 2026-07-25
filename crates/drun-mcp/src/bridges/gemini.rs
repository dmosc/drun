use std::path::Path;

use serde_json::{Value, json};

// https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html

const CLI: super::shared::CliMcp = super::shared::CliMcp {
    bin: "gemini",
    display_name: "Gemini CLI",
};

const EXCLUDED_TOOLS: &[&str] = &[
    "ShellTool",
    "EditTool",
    "WriteFileTool",
    "ReadFileTool",
    "GlobTool",
    "GrepTool",
    "ReadManyFilesTool",
    "LSTool",
    "WebFetchTool",
    "WebSearchTool",
];

/// [`super::Bridge`] impl — see that trait for the extensibility contract.
pub struct Gemini;

impl super::Bridge for Gemini {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn description(&self) -> &'static str {
        "Gemini CLI — blocks native tools per-project, registers the MCP server"
    }

    fn scope(&self) -> super::Scope {
        super::Scope::Project
    }

    fn init(&self) {
        let project_dir = std::env::current_dir().expect("cannot read current directory");
        let project_path = project_dir.to_str().expect("non-UTF-8 project path");
        write_settings(&project_dir);
        super::shared::write_project_instructions(
            &project_dir,
            "GEMINI.md",
            &gemini_md_content(project_path),
        );
        super::shared::allow_mount_path(&crate::drun_home(), &project_dir);
        super::shared::register_project(&crate::drun_home(), &project_dir);
        CLI.register();
        eprintln!("drun: initialized for {project_path}");
    }

    fn deregister(&self) {
        CLI.deregister();
    }
}

fn rendered_default_settings() -> String {
    let value = json!({ "excludeTools": EXCLUDED_TOOLS });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("static json value")
    )
}

fn write_settings(project_dir: &Path) {
    let settings_file = project_dir.join(".gemini").join("settings.json");
    super::shared::write_json_settings(
        &settings_file,
        ".gemini/settings.json",
        rendered_default_settings,
        merge_settings,
    );
}

fn merge_settings(existing: &str) -> Result<Option<String>, String> {
    let mut value: Value = serde_json::from_str(existing).map_err(|e| e.to_string())?;
    let Some(root) = value.as_object_mut() else {
        return Err("root is not a JSON object".to_string());
    };

    let changed = super::shared::merge_string_array(root, "excludeTools", EXCLUDED_TOOLS)?;
    if !changed {
        return Ok(None);
    }

    let rendered = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Ok(Some(format!("{rendered}\n")))
}

fn gemini_md_content(project_path: &str) -> String {
    format!(
        "# Agent instructions\n\n\
         This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.\n\
         Native file, shell, and network tools ({}) are disabled for this workspace — they \
         would otherwise read or write the host directly, bypassing the sandbox. Use the drun \
         MCP tools for everything.\n\n{}",
        EXCLUDED_TOOLS.join(", "),
        super::shared::drun_instructions_body(project_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_md_content_includes_the_project_path() {
        let content = gemini_md_content("/home/user/myproject");
        assert!(content.contains("/home/user/myproject"));
    }

    #[test]
    fn gemini_md_content_documents_the_core_tools() {
        let content = gemini_md_content("/tmp/project");
        assert!(content.contains("session_bash"));
        assert!(content.contains("session_mount"));
    }

    #[test]
    fn gemini_md_content_lists_the_excluded_tools() {
        let content = gemini_md_content("/tmp/project");
        assert!(content.contains("ShellTool"));
        assert!(content.contains("WebSearchTool"));
    }

    #[test]
    fn write_settings_creates_gemini_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path());
        let settings_path = dir.path().join(".gemini/settings.json");
        assert!(settings_path.exists());
        let content = std::fs::read_to_string(&settings_path).unwrap();
        assert!(content.contains("ShellTool"));
    }

    #[test]
    fn write_settings_leaves_unparseable_existing_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".gemini");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_path = settings_dir.join("settings.json");
        std::fs::write(&settings_path, "custom content").unwrap();

        write_settings(dir.path());

        assert_eq!(
            std::fs::read_to_string(&settings_path).unwrap(),
            "custom content"
        );
    }

    #[test]
    fn write_settings_merges_into_an_existing_file_with_unrelated_content() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".gemini");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_path = settings_dir.join("settings.json");
        std::fs::write(&settings_path, r#"{"theme": "dark"}"#).unwrap();

        write_settings(dir.path());

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["theme"], "dark");
        assert!(
            value["excludeTools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "ShellTool")
        );
    }

    #[test]
    fn write_settings_merges_missing_entries_into_partial_exclude_list() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".gemini");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_path = settings_dir.join("settings.json");
        std::fs::write(&settings_path, r#"{"excludeTools": ["SomeOtherTool"]}"#).unwrap();

        write_settings(dir.path());

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        let exclude = value["excludeTools"].as_array().unwrap();
        for required in EXCLUDED_TOOLS {
            assert!(exclude.iter().any(|v| v == required), "missing {required}");
        }
        assert!(exclude.iter().any(|v| v == "SomeOtherTool"));
    }

    #[test]
    fn write_settings_is_idempotent_once_fully_merged() {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path());
        let settings_path = dir.path().join(".gemini/settings.json");
        let first = std::fs::read_to_string(&settings_path).unwrap();

        write_settings(dir.path());

        let second = std::fs::read_to_string(&settings_path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn merge_settings_errors_when_root_is_not_an_object() {
        let err = merge_settings(r#"["not", "an", "object"]"#).unwrap_err();
        assert!(err.contains("root"));
    }
}

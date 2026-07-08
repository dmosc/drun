use std::path::{Path, PathBuf};

use serde_json::Value;

/// Reverses `drun init` for the given project directory (defaults to cwd).
/// Removes drun-specific entries from `.claude/settings.json`, removes a
/// drun-generated `CLAUDE.md`, and drops the path from `~/.drun/projects`.
pub fn run(target_dir: Option<PathBuf>) {
    let project_dir = resolve_dir(target_dir);
    let drun_home = crate::init::drun_home();

    remove_settings(&project_dir);
    remove_claude_md(&project_dir);
    deregister_project(&drun_home, &project_dir);

    eprintln!("drun: de-initialized {}", project_dir.display());
}

fn resolve_dir(target_dir: Option<PathBuf>) -> PathBuf {
    match target_dir {
        Some(d) => d.canonicalize().unwrap_or(d),
        None => std::env::current_dir().expect("cannot read current directory"),
    }
}

/// Strips drun's required `deny` and `allow` entries from `.claude/settings.json`.
/// If the file would become effectively empty after stripping (only empty permission
/// arrays remain), the file and its parent `.claude/` directory are removed instead.
fn remove_settings(project_dir: &Path) {
    let settings_dir = project_dir.join(".claude");
    let settings_file = settings_dir.join("settings.json");
    if !settings_file.exists() {
        return;
    }

    let existing =
        std::fs::read_to_string(&settings_file).expect("cannot read .claude/settings.json");

    match strip_drun_permissions(&existing) {
        Ok(stripped) => {
            if is_effectively_empty(&stripped) {
                std::fs::remove_file(&settings_file)
                    .expect("cannot remove .claude/settings.json");
                // Remove the .claude/ dir too if it's now empty
                std::fs::remove_dir(&settings_dir).ok();
                eprintln!("drun: removed .claude/settings.json");
            } else {
                std::fs::write(&settings_file, stripped)
                    .expect("cannot write .claude/settings.json");
                eprintln!("drun: removed drun permissions from .claude/settings.json");
            }
        }
        Err(e) => {
            eprintln!(
                "drun: could not update .claude/settings.json ({e}) — leaving it untouched"
            );
        }
    }
}

/// Returns true when the only remaining content is empty permission arrays —
/// i.e. the file was created by drun and has no user-added configuration.
fn is_effectively_empty(json: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    // Non-permissions keys mean the user has their own config in here
    if obj.keys().any(|k| k != "permissions") {
        return false;
    }
    let Some(perms) = obj.get("permissions").and_then(|v| v.as_object()) else {
        return true;
    };
    // All permission arrays must be empty
    perms.values().all(|v| {
        v.as_array().map(|a| a.is_empty()).unwrap_or(false)
    })
}

fn strip_drun_permissions(existing: &str) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(existing).map_err(|e| e.to_string())?;
    let Some(root) = value.as_object_mut() else {
        return Err("root is not a JSON object".to_string());
    };

    if let Some(permissions) = root.get_mut("permissions").and_then(Value::as_object_mut) {
        strip_from_array(permissions, "deny", crate::init::REQUIRED_DENY);
        strip_from_array(permissions, "allow", crate::init::REQUIRED_ALLOW);
    }

    let rendered = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Ok(format!("{rendered}\n"))
}

fn strip_from_array(obj: &mut serde_json::Map<String, Value>, key: &str, to_remove: &[&str]) {
    if let Some(array) = obj.get_mut(key).and_then(Value::as_array_mut) {
        array.retain(|v| {
            let s = v.as_str().unwrap_or("");
            !to_remove.contains(&s)
        });
    }
}

/// Removes `CLAUDE.md` only if its content exactly matches what drun generated.
/// Any edit — including appending lines — causes the file to be left intact.
fn remove_claude_md(project_dir: &Path) {
    let claude_md = project_dir.join("CLAUDE.md");
    if !claude_md.exists() {
        return;
    }

    let content = std::fs::read_to_string(&claude_md).unwrap_or_default();
    let project_path = project_dir.to_str().expect("non-UTF-8 project path");
    let expected = crate::init::claude_md_content(project_path);

    if content == expected {
        std::fs::remove_file(&claude_md).expect("cannot remove CLAUDE.md");
        eprintln!("drun: removed CLAUDE.md");
    } else {
        eprintln!("drun: CLAUDE.md has been edited — leaving it untouched");
    }
}

fn deregister_project(drun_home: &Path, project_dir: &Path) {
    let registry = drun_home.join("projects");
    if !registry.exists() {
        return;
    }

    let project_path = project_dir.to_str().expect("non-UTF-8 project path");
    let existing = std::fs::read_to_string(&registry).unwrap_or_default();
    let filtered: String = existing
        .lines()
        .filter(|l| !l.is_empty() && *l != project_path)
        .map(|l| format!("{l}\n"))
        .collect();

    std::fs::write(&registry, filtered).expect("cannot update project registry");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_drun_permissions_removes_required_entries() {
        let input = serde_json::json!({
            "permissions": {
                "deny": ["Bash", "Edit", "MyCustomTool"],
                "allow": ["mcp__drun__*", "SomethingElse"]
            }
        });
        let json = serde_json::to_string(&input).unwrap();
        let result = strip_drun_permissions(&json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        let deny = value["permissions"]["deny"].as_array().unwrap();
        let allow = value["permissions"]["allow"].as_array().unwrap();

        // drun entries removed
        assert!(!deny.iter().any(|v| v == "Bash"));
        assert!(!deny.iter().any(|v| v == "Edit"));
        assert!(!allow.iter().any(|v| v == "mcp__drun__*"));

        // non-drun entries preserved
        assert!(deny.iter().any(|v| v == "MyCustomTool"));
        assert!(allow.iter().any(|v| v == "SomethingElse"));
    }

    #[test]
    fn strip_drun_permissions_is_fine_with_no_permissions_key() {
        let input = r#"{"theme": "dark"}"#;
        let result = strip_drun_permissions(input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["theme"], "dark");
    }

    #[test]
    fn remove_claude_md_deletes_unmodified_drun_generated_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        let expected = crate::init::claude_md_content(dir.path().to_str().unwrap());
        std::fs::write(&path, &expected).unwrap();

        remove_claude_md(dir.path());

        assert!(!path.exists());
    }

    #[test]
    fn remove_claude_md_leaves_appended_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        let mut content = crate::init::claude_md_content(dir.path().to_str().unwrap());
        content.push_str("\n## My extra notes\n");
        std::fs::write(&path, &content).unwrap();

        remove_claude_md(dir.path());

        assert!(path.exists());
    }

    #[test]
    fn remove_claude_md_leaves_completely_custom_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(&path, "# My custom notes\n\nNot drun generated.").unwrap();

        remove_claude_md(dir.path());

        assert!(path.exists());
    }

    #[test]
    fn remove_settings_deletes_file_when_only_drun_content_remains() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_file = settings_dir.join("settings.json");
        // Write a file that contains only drun-added entries
        let drun_only = serde_json::json!({
            "permissions": {
                "deny": crate::init::REQUIRED_DENY,
                "allow": crate::init::REQUIRED_ALLOW,
            }
        });
        std::fs::write(&settings_file, serde_json::to_string_pretty(&drun_only).unwrap()).unwrap();

        remove_settings(dir.path());

        assert!(!settings_file.exists());
    }

    #[test]
    fn remove_settings_strips_entries_and_preserves_file_when_user_config_exists() {
        let dir = tempfile::tempdir().unwrap();
        let settings_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_file = settings_dir.join("settings.json");
        let mixed = serde_json::json!({
            "theme": "dark",
            "permissions": {
                "deny": crate::init::REQUIRED_DENY,
                "allow": crate::init::REQUIRED_ALLOW,
            }
        });
        std::fs::write(&settings_file, serde_json::to_string_pretty(&mixed).unwrap()).unwrap();

        remove_settings(dir.path());

        assert!(settings_file.exists());
        let content = std::fs::read_to_string(&settings_file).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["theme"], "dark");
        assert!(value["permissions"]["deny"].as_array().unwrap().is_empty());
    }

    #[test]
    fn deregister_project_removes_only_the_target_path() {
        let drun_home = tempfile::tempdir().unwrap();
        let registry = drun_home.path().join("projects");
        std::fs::write(&registry, "/keep/this\n/remove/this\n/also/keep\n").unwrap();

        let target = PathBuf::from("/remove/this");
        deregister_project(drun_home.path(), &target);

        let content = std::fs::read_to_string(&registry).unwrap();
        assert!(content.contains("/keep/this"));
        assert!(content.contains("/also/keep"));
        assert!(!content.contains("/remove/this"));
    }
}

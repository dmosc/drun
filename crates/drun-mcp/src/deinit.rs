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
/// Other content in the file is preserved exactly.
fn remove_settings(project_dir: &Path) {
    let settings_file = project_dir.join(".claude/settings.json");
    if !settings_file.exists() {
        return;
    }

    let existing =
        std::fs::read_to_string(&settings_file).expect("cannot read .claude/settings.json");

    match strip_drun_permissions(&existing) {
        Ok(stripped) => {
            std::fs::write(&settings_file, stripped)
                .expect("cannot write .claude/settings.json");
            eprintln!("drun: removed drun permissions from .claude/settings.json");
        }
        Err(e) => {
            eprintln!(
                "drun: could not update .claude/settings.json ({e}) — leaving it untouched"
            );
        }
    }
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

/// Removes `CLAUDE.md` if its content looks like what drun generated. If the
/// user edited it, a warning is printed and the file is left intact.
fn remove_claude_md(project_dir: &Path) {
    let claude_md = project_dir.join("CLAUDE.md");
    if !claude_md.exists() {
        return;
    }

    let content = std::fs::read_to_string(&claude_md).unwrap_or_default();

    if content.starts_with("# Agent instructions\n\nThis project uses [drun]") {
        std::fs::remove_file(&claude_md).expect("cannot remove CLAUDE.md");
        eprintln!("drun: removed CLAUDE.md");
    } else {
        eprintln!("drun: CLAUDE.md appears to have been edited — leaving it untouched");
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
    fn remove_claude_md_deletes_drun_generated_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(
            &path,
            "# Agent instructions\n\nThis project uses [drun](https://github.com/dmosc/drun)",
        )
        .unwrap();

        remove_claude_md(dir.path());

        assert!(!path.exists());
    }

    #[test]
    fn remove_claude_md_leaves_edited_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(&path, "# My custom notes\n\nNot drun generated.").unwrap();

        remove_claude_md(dir.path());

        assert!(path.exists());
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

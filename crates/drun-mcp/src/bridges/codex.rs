use std::path::PathBuf;

use toml_edit::{DocumentMut, Item, Table, value};

// Codex has no CLI for registering arbitrary MCP servers, so this edits
// ~/.codex/config.toml directly. Unlike Hermes's YAML (bridges::hermes),
// TOML editing goes through toml_edit's format-preserving parser (already a
// dependency — see config_cmd.rs) instead of line surgery, so comments and
// formatting survive a real structural merge rather than string matching.
// See: https://developers.openai.com/codex/config-reference
//      https://developers.openai.com/codex/mcp

/// [`super::Bridge`] impl — see that trait for the extensibility contract.
pub struct Codex;

impl super::Bridge for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn description(&self) -> &'static str {
        "Codex CLI — blocks the native shell tool machine-wide, registers the MCP server"
    }

    fn scope(&self) -> super::Scope {
        super::Scope::Machine
    }

    fn init(&self) {
        let project_dir = std::env::current_dir().expect("cannot read current directory");
        let project_path = project_dir.to_str().expect("non-UTF-8 project path");
        super::shared::write_project_instructions(
            &project_dir,
            "AGENTS.md",
            &agents_md_content(project_path),
        );
        super::shared::allow_mount_path(&crate::drun_home(), &project_dir);
        super::shared::register_project(&crate::drun_home(), &project_dir);
        let path = config_path();
        let mut doc = match load_doc() {
            Ok(doc) => doc,
            Err(e) => {
                eprintln!(
                    "drun: could not read {} ({e}) — leaving it untouched.",
                    path.display()
                );
                return;
            }
        };

        let mcp_changed = merge_mcp_entry(&mut doc);
        let shell_changed = disable_shell_tool(&mut doc);

        if !mcp_changed && !shell_changed {
            eprintln!("drun: already registered in Codex CLI, skipping.");
            return;
        }

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match crate::atomic_write(&path, &doc.to_string()) {
            Ok(()) => eprintln!(
                "drun: updated {} — registered the MCP server (HTTP → {}) and disabled the native \
             shell tool. Both apply machine-wide, not just this project.",
                path.display(),
                mcp_url()
            ),
            Err(e) => eprintln!("drun: could not write {} ({e})", path.display()),
        }
        eprintln!("drun: initialized for {project_path}");
    }

    fn deregister(&self) {
        let path = config_path();
        let Ok(mut doc) = load_doc() else {
            return;
        };

        let mcp_removed = remove_mcp_entry(&mut doc);
        let shell_restored = remove_shell_tool_override(&mut doc);

        if !mcp_removed && !shell_restored {
            return;
        }

        if crate::atomic_write(&path, &doc.to_string()).is_ok() {
            eprintln!("drun: removed from Codex CLI ({}).", path.display());
        }
    }
}

fn config_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".codex/config.toml")
}

fn mcp_url() -> String {
    format!("http://127.0.0.1:{}/mcp", crate::mcp_port())
}

fn load_doc() -> Result<DocumentMut, String> {
    let path = config_path();
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    contents
        .parse::<DocumentMut>()
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

/// Adds `[mcp_servers.drun]` with our URL if not already present. Returns
/// whether the document changed.
fn merge_mcp_entry(doc: &mut DocumentMut) -> bool {
    let servers = doc
        .entry("mcp_servers")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(servers) = servers.as_table_mut() else {
        return false;
    };
    if servers.contains_key("drun") {
        return false;
    }

    let mut drun = Table::new();
    drun.insert("url", value(mcp_url()));
    servers.insert("drun", Item::Table(drun));
    true
}

/// Removes `mcp_servers.drun`, leaving any sibling server entries untouched.
fn remove_mcp_entry(doc: &mut DocumentMut) -> bool {
    let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
        return false;
    };
    servers.remove("drun").is_some()
}

/// Sets `features.shell_tool = false` if not already false. Returns whether
/// the document changed.
fn disable_shell_tool(doc: &mut DocumentMut) -> bool {
    let features = doc
        .entry("features")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(features) = features.as_table_mut() else {
        return false;
    };
    if features.get("shell_tool").and_then(Item::as_bool) == Some(false) {
        return false;
    }

    features.insert("shell_tool", value(false));
    true
}

/// Removes `features.shell_tool` if it's exactly the `false` drun set,
/// restoring Codex's default (enabled). Leaves any other value — e.g. a
/// user's own override — untouched, same tradeoff as Hermes's
/// `remove_disabled_toolsets`: we can't distinguish "drun set this" from "the
/// user independently wanted this too."
fn remove_shell_tool_override(doc: &mut DocumentMut) -> bool {
    let Some(features) = doc.get_mut("features").and_then(Item::as_table_mut) else {
        return false;
    };
    if features.get("shell_tool").and_then(Item::as_bool) != Some(false) {
        return false;
    }

    features.remove("shell_tool");
    true
}

fn agents_md_content(project_path: &str) -> String {
    format!(
        "# Agent instructions\n\n\
         This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.\n\
         Codex's native shell tool is disabled (machine-wide — see `features.shell_tool` in \
         `~/.codex/config.toml`) so it can't bypass the sandbox. Use the drun MCP tools for \
         everything.\n\n{}",
        super::shared::drun_instructions_body(project_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> DocumentMut {
        s.parse().unwrap()
    }

    #[test]
    fn merge_mcp_entry_adds_the_drun_server() {
        let mut d = doc("");
        assert!(merge_mcp_entry(&mut d));
        let rendered = d.to_string();
        assert!(rendered.contains("[mcp_servers.drun]"));
        assert!(rendered.contains(&mcp_url()));
    }

    #[test]
    fn merge_mcp_entry_is_idempotent() {
        let mut d = doc("");
        assert!(merge_mcp_entry(&mut d));
        assert!(!merge_mcp_entry(&mut d));
    }

    #[test]
    fn merge_mcp_entry_preserves_sibling_servers() {
        let mut d = doc("[mcp_servers.other]\ncommand = \"npx\"\n");
        assert!(merge_mcp_entry(&mut d));
        let rendered = d.to_string();
        assert!(rendered.contains("[mcp_servers.other]"));
        assert!(rendered.contains("command = \"npx\""));
        assert!(rendered.contains("[mcp_servers.drun]"));
    }

    #[test]
    fn remove_mcp_entry_strips_only_drun() {
        let mut d = doc("[mcp_servers.other]\ncommand = \"npx\"\n");
        merge_mcp_entry(&mut d);
        assert!(remove_mcp_entry(&mut d));
        let rendered = d.to_string();
        assert!(!rendered.contains("drun"));
        assert!(rendered.contains("[mcp_servers.other]"));
    }

    #[test]
    fn remove_mcp_entry_is_a_no_op_when_absent() {
        let mut d = doc("");
        assert!(!remove_mcp_entry(&mut d));
    }

    #[test]
    fn disable_shell_tool_sets_the_flag() {
        let mut d = doc("");
        assert!(disable_shell_tool(&mut d));
        assert!(d.to_string().contains("shell_tool = false"));
    }

    #[test]
    fn disable_shell_tool_is_idempotent() {
        let mut d = doc("");
        assert!(disable_shell_tool(&mut d));
        assert!(!disable_shell_tool(&mut d));
    }

    #[test]
    fn disable_shell_tool_preserves_sibling_features() {
        let mut d = doc("[features]\napps = true\n");
        assert!(disable_shell_tool(&mut d));
        let rendered = d.to_string();
        assert!(rendered.contains("apps = true"));
        assert!(rendered.contains("shell_tool = false"));
    }

    #[test]
    fn remove_shell_tool_override_drops_our_false() {
        let mut d = doc("");
        disable_shell_tool(&mut d);
        assert!(remove_shell_tool_override(&mut d));
        assert!(!d.to_string().contains("shell_tool"));
    }

    #[test]
    fn remove_shell_tool_override_leaves_a_user_true_alone() {
        let mut d = doc("[features]\nshell_tool = true\n");
        assert!(!remove_shell_tool_override(&mut d));
        assert!(d.to_string().contains("shell_tool = true"));
    }

    #[test]
    fn remove_shell_tool_override_is_a_no_op_when_absent() {
        let mut d = doc("");
        assert!(!remove_shell_tool_override(&mut d));
    }

    #[test]
    fn agents_md_content_includes_the_project_path() {
        let content = agents_md_content("/home/user/myproject");
        assert!(content.contains("/home/user/myproject"));
    }

    #[test]
    fn agents_md_content_documents_the_core_tools() {
        let content = agents_md_content("/tmp/project");
        assert!(content.contains("session_bash"));
        assert!(content.contains("session_mount"));
    }
}

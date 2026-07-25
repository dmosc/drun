use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, value};

use super::Bridge;

// Codex has no CLI for registering MCP servers, so this edits
// ~/.codex/config.toml directly via toml_edit (format-preserving).
// See: https://developers.openai.com/codex/config-reference

/// [`Bridge`] impl — see that trait for the extensibility contract.
pub struct Codex;

impl Bridge for Codex {
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
        self.init_common(
            &project_dir,
            &crate::Env.drun_home(),
            "AGENTS.md",
            &self.agents_md_content(project_path),
        );

        let path = self.config_path();
        let mut config = match CodexConfig::load(&path) {
            Ok(config) => config,
            Err(e) => {
                eprintln!(
                    "drun: could not read {} ({e}) — leaving it untouched.",
                    path.display()
                );
                return;
            }
        };

        let mcp_changed = config.merge_mcp_entry();
        let shell_changed = config.disable_shell_tool();

        if !mcp_changed && !shell_changed {
            eprintln!("drun: already registered in Codex CLI, skipping.");
            return;
        }

        match config.save() {
            Ok(()) => eprintln!(
                "drun: updated {} — registered the MCP server (HTTP → {}) and disabled the native \
             shell tool. Both apply machine-wide, not just this project.",
                path.display(),
                config.mcp_url()
            ),
            Err(e) => eprintln!("drun: could not write {} ({e})", path.display()),
        }
        eprintln!("drun: initialized for {project_path}");
    }

    fn deregister(&self) {
        let path = self.config_path();
        let Ok(mut config) = CodexConfig::load(&path) else {
            return;
        };

        let mcp_removed = config.remove_mcp_entry();
        let shell_restored = config.remove_shell_tool_override();

        if !mcp_removed && !shell_restored {
            return;
        }

        if config.save().is_ok() {
            eprintln!("drun: removed from Codex CLI ({}).", path.display());
        }
    }
}

impl Codex {
    fn config_path(&self) -> PathBuf {
        crate::Env.home_dir().join(".codex/config.toml")
    }

    fn agents_md_content(&self, project_path: &str) -> String {
        format!(
            "# Agent instructions\n\n\
             This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.\n\
             Codex's native shell tool is disabled (machine-wide — see `features.shell_tool` in \
             `~/.codex/config.toml`) so it can't bypass the sandbox. Use the drun MCP tools for \
             everything.\n\n{}",
            self.drun_instructions_body(project_path)
        )
    }
}

struct CodexConfig {
    path: PathBuf,
    doc: DocumentMut,
}

impl CodexConfig {
    fn load(path: &Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        let doc = contents
            .parse::<DocumentMut>()
            .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    fn render(&self) -> String {
        self.doc.to_string()
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        crate::FileManager::write(&self.path, &self.render())
    }

    fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", crate::Env.mcp_port())
    }

    fn merge_mcp_entry(&mut self) -> bool {
        let url = self.mcp_url();
        let servers = self
            .doc
            .entry("mcp_servers")
            .or_insert_with(|| Item::Table(Table::new()));
        let Some(servers) = servers.as_table_mut() else {
            return false;
        };
        if servers.contains_key("drun") {
            return false;
        }

        let mut drun = Table::new();
        drun.insert("url", value(url));
        servers.insert("drun", Item::Table(drun));
        true
    }

    fn remove_mcp_entry(&mut self) -> bool {
        let Some(servers) = self.doc.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
            return false;
        };
        servers.remove("drun").is_some()
    }

    fn disable_shell_tool(&mut self) -> bool {
        let features = self
            .doc
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

    // Only clears our own `false` — a user's `true` override is left alone,
    // since we can't tell it apart from one we'd have set ourselves.
    fn remove_shell_tool_override(&mut self) -> bool {
        let Some(features) = self.doc.get_mut("features").and_then(Item::as_table_mut) else {
            return false;
        };
        if features.get("shell_tool").and_then(Item::as_bool) != Some(false) {
            return false;
        }

        features.remove("shell_tool");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(s: &str) -> CodexConfig {
        CodexConfig {
            path: PathBuf::new(),
            doc: s.parse().unwrap(),
        }
    }

    #[test]
    fn merge_mcp_entry_adds_the_drun_server() {
        let mut c = config("");
        assert!(c.merge_mcp_entry());
        let rendered = c.render();
        assert!(rendered.contains("[mcp_servers.drun]"));
        assert!(rendered.contains(&c.mcp_url()));
    }

    #[test]
    fn merge_mcp_entry_is_idempotent() {
        let mut c = config("");
        assert!(c.merge_mcp_entry());
        assert!(!c.merge_mcp_entry());
    }

    #[test]
    fn merge_mcp_entry_preserves_sibling_servers() {
        let mut c = config("[mcp_servers.other]\ncommand = \"npx\"\n");
        assert!(c.merge_mcp_entry());
        let rendered = c.render();
        assert!(rendered.contains("[mcp_servers.other]"));
        assert!(rendered.contains("command = \"npx\""));
        assert!(rendered.contains("[mcp_servers.drun]"));
    }

    #[test]
    fn remove_mcp_entry_strips_only_drun() {
        let mut c = config("[mcp_servers.other]\ncommand = \"npx\"\n");
        c.merge_mcp_entry();
        assert!(c.remove_mcp_entry());
        let rendered = c.render();
        assert!(!rendered.contains("drun"));
        assert!(rendered.contains("[mcp_servers.other]"));
    }

    #[test]
    fn remove_mcp_entry_is_a_no_op_when_absent() {
        let mut c = config("");
        assert!(!c.remove_mcp_entry());
    }

    #[test]
    fn disable_shell_tool_sets_the_flag() {
        let mut c = config("");
        assert!(c.disable_shell_tool());
        assert!(c.render().contains("shell_tool = false"));
    }

    #[test]
    fn disable_shell_tool_is_idempotent() {
        let mut c = config("");
        assert!(c.disable_shell_tool());
        assert!(!c.disable_shell_tool());
    }

    #[test]
    fn disable_shell_tool_preserves_sibling_features() {
        let mut c = config("[features]\napps = true\n");
        assert!(c.disable_shell_tool());
        let rendered = c.render();
        assert!(rendered.contains("apps = true"));
        assert!(rendered.contains("shell_tool = false"));
    }

    #[test]
    fn remove_shell_tool_override_drops_our_false() {
        let mut c = config("");
        c.disable_shell_tool();
        assert!(c.remove_shell_tool_override());
        assert!(!c.render().contains("shell_tool"));
    }

    #[test]
    fn remove_shell_tool_override_leaves_a_user_true_alone() {
        let mut c = config("[features]\nshell_tool = true\n");
        assert!(!c.remove_shell_tool_override());
        assert!(c.render().contains("shell_tool = true"));
    }

    #[test]
    fn remove_shell_tool_override_is_a_no_op_when_absent() {
        let mut c = config("");
        assert!(!c.remove_shell_tool_override());
    }

    #[test]
    fn agents_md_content_includes_the_project_path() {
        let content = Codex.agents_md_content("/home/user/myproject");
        assert!(content.contains("/home/user/myproject"));
    }

    #[test]
    fn agents_md_content_documents_the_core_tools() {
        let content = Codex.agents_md_content("/tmp/project");
        assert!(content.contains("session_bash"));
        assert!(content.contains("session_mount"));
    }
}

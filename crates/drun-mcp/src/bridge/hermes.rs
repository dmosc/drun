use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::Bridge;

// Hermes has no CLI for registering MCP servers, so this edits
// ~/.hermes/config.yaml directly as line surgery (no YAML lib dependency),
// preserving the user's existing formatting and comments.
// See: https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/mcp.md

/// [`Bridge`] impl — see that trait for the extensibility contract.
pub struct Hermes;

impl Bridge for Hermes {
    fn name(&self) -> &'static str {
        "hermes"
    }

    fn description(&self) -> &'static str {
        "Hermes — registers the MCP server, disables native tools machine-wide"
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
            "HERMES.md",
            &self.hermes_md_content(project_path),
        );
        if !self.hermes_available() {
            let path = self.hermes_config_path();
            eprintln!(
                "drun: Hermes CLI not found. Add drun manually to {}:",
                path.display()
            );
            eprint!("{}", self.rendered_mcp_entry(&self.mcp_http_url()));
            return;
        }
        self.register_mcp();
        self.restrict_toolsets();
        eprintln!("drun: initialized for {project_path}");
    }

    fn deregister(&self) {
        let path = self.hermes_config_path();
        let Ok(mut config) = HermesConfig::try_load(&path) else {
            return;
        };

        let mcp_removed = config.remove_mcp_entry();
        let toolsets_restored = config.remove_disabled_toolsets(Self::REQUIRED_TOOLSETS);

        if !mcp_removed && !toolsets_restored {
            return;
        }

        if config.save(&path).is_ok() {
            eprintln!("drun: removed from Hermes ({}).", path.display());
        }
    }
}

impl Hermes {
    const REQUIRED_TOOLSETS: &'static [&'static str] =
        &["terminal", "file", "web", "search", "delegation"];

    fn hermes_config_path(&self) -> PathBuf {
        PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".hermes/config.yaml")
    }

    fn mcp_http_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", crate::Env.mcp_port())
    }

    fn hermes_available(&self) -> bool {
        match Command::new("hermes")
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

    fn rendered_mcp_entry(&self, url: &str) -> String {
        format!(
            "mcp_servers:\n  drun:\n    url: \"{url}\"\n    headers:\n      Accept: \"application/json, text/event-stream\"\n"
        )
    }

    fn hermes_md_content(&self, project_path: &str) -> String {
        format!(
            "# Agent instructions\n\n\
             This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.\n\
             Hermes's native `terminal`, `file`, `web`, `search`, and `delegation` toolsets are\n\
             disabled (machine-wide — see `~/.hermes/config.yaml`) so they don't bypass the\n\
             sandbox. Use the drun MCP tools for everything.\n\n{}",
            self.drun_instructions_body(project_path)
        )
    }

    fn register_mcp(&self) {
        let path = self.hermes_config_path();
        let mut config = HermesConfig::load_or_default(&path);

        if config.has_mcp_entry() {
            eprintln!("drun: already registered in Hermes, skipping.");
            return;
        }

        config.merge_mcp_entry(&self.mcp_http_url());
        match config.save(&path) {
            Ok(()) => eprintln!(
                "drun: added to Hermes (HTTP → {}, {}).",
                self.mcp_http_url(),
                path.display()
            ),
            Err(e) => eprintln!("drun: could not write {} ({e})", path.display()),
        }
    }

    fn restrict_toolsets(&self) {
        let path = self.hermes_config_path();
        let mut config = HermesConfig::load_or_default(&path);

        if !config.merge_disabled_toolsets(Self::REQUIRED_TOOLSETS) {
            eprintln!("drun: Hermes toolset restriction already applied, skipping.");
            return;
        }

        match config.save(&path) {
            Ok(()) => eprintln!(
                "drun: disabled Hermes's native {} toolsets globally (agent.disabled_toolsets in \
                 {}) — this applies to every Hermes session on this machine, not just this project.",
                Self::REQUIRED_TOOLSETS.join("/"),
                path.display()
            ),
            Err(e) => eprintln!("drun: could not write {} ({e})", path.display()),
        }
    }
}

struct HermesConfig {
    lines: Vec<String>,
}

impl HermesConfig {
    fn parse(existing: &str) -> Self {
        Self {
            lines: existing.lines().map(str::to_string).collect(),
        }
    }

    fn load_or_default(path: &Path) -> Self {
        Self::parse(&std::fs::read_to_string(path).unwrap_or_default())
    }

    fn try_load(path: &Path) -> io::Result<Self> {
        std::fs::read_to_string(path).map(|s| Self::parse(&s))
    }

    fn render(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let mut rendered = self.lines.join("\n");
        rendered.push('\n');
        rendered
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render())
    }

    fn has_mcp_entry(&self) -> bool {
        self.lines.iter().any(|l| l.trim_end() == "  drun:")
    }

    fn merge_mcp_entry(&mut self, url: &str) -> bool {
        if self.has_mcp_entry() {
            return false;
        }

        let entry = [
            "  drun:".to_string(),
            format!("    url: \"{url}\""),
            "    headers:".to_string(),
            "      Accept: \"application/json, text/event-stream\"".to_string(),
        ];

        match self
            .lines
            .iter()
            .position(|l| l.trim_end() == "mcp_servers:")
        {
            Some(pos) => {
                for (i, e) in entry.iter().enumerate() {
                    self.lines.insert(pos + 1 + i, e.clone());
                }
            }
            None => {
                if !self.lines.is_empty() {
                    self.lines.push(String::new());
                }
                self.lines.push("mcp_servers:".to_string());
                self.lines.extend(entry);
            }
        }
        true
    }

    fn remove_mcp_entry(&mut self) -> bool {
        if !self.has_mcp_entry() {
            return false;
        }

        let mut out = Vec::with_capacity(self.lines.len());
        let mut skipping = false;
        for line in &self.lines {
            if line.trim_end() == "  drun:" {
                skipping = true;
                continue;
            }
            if skipping {
                let indent = line.len() - line.trim_start().len();
                if line.trim().is_empty() || indent >= 3 {
                    continue;
                }
                skipping = false;
            }
            out.push(line.clone());
        }
        self.lines = out;
        true
    }

    fn agent_block_range(&self) -> Option<(usize, usize)> {
        let start = self.lines.iter().position(|l| l.trim_end() == "agent:")?;
        let end = self.lines[start + 1..]
            .iter()
            .position(|l| !l.trim().is_empty() && !l.starts_with("  "))
            .map(|i| start + 1 + i)
            .unwrap_or(self.lines.len());
        Some((start, end))
    }

    fn disabled_toolsets_pos(&self, agent_start: usize, agent_end: usize) -> Option<usize> {
        self.lines[agent_start + 1..agent_end]
            .iter()
            .position(|l| l.trim_end() == "  disabled_toolsets:")
            .map(|i| agent_start + 1 + i)
    }

    fn disabled_toolsets_list_range(&self, dt_pos: usize) -> (usize, usize) {
        let end = self.lines[dt_pos + 1..]
            .iter()
            .position(|l| !(l.starts_with("    - ") || l.trim().is_empty()))
            .map(|i| dt_pos + 1 + i)
            .unwrap_or(self.lines.len());
        (dt_pos + 1, end)
    }

    #[cfg(test)]
    fn has_any_disabled_toolset(&self, names: &[&str]) -> bool {
        let Some((agent_start, agent_end)) = self.agent_block_range() else {
            return false;
        };
        let Some(dt_pos) = self.disabled_toolsets_pos(agent_start, agent_end) else {
            return false;
        };
        let (list_start, list_end) = self.disabled_toolsets_list_range(dt_pos);
        self.lines[list_start..list_end]
            .iter()
            .any(|l| names.contains(&l.trim_start().trim_start_matches("- ")))
    }

    // Never removes an existing (possibly user-added) entry.
    fn merge_disabled_toolsets(&mut self, required: &[&str]) -> bool {
        let Some((agent_start, agent_end)) = self.agent_block_range() else {
            if !self.lines.is_empty() {
                self.lines.push(String::new());
            }
            self.lines.push("agent:".to_string());
            self.lines.push("  disabled_toolsets:".to_string());
            for t in required {
                self.lines.push(format!("    - {t}"));
            }
            return true;
        };

        match self.disabled_toolsets_pos(agent_start, agent_end) {
            Some(dt_pos) => {
                let (list_start, list_end) = self.disabled_toolsets_list_range(dt_pos);
                let existing_items: Vec<String> = self.lines[list_start..list_end]
                    .iter()
                    .map(|l| l.trim_start().trim_start_matches("- ").to_string())
                    .collect();

                let mut insert_at = list_end;
                let mut changed = false;
                for t in required {
                    if !existing_items.iter().any(|i| i == t) {
                        self.lines.insert(insert_at, format!("    - {t}"));
                        insert_at += 1;
                        changed = true;
                    }
                }
                changed
            }
            None => {
                let mut insert_at = agent_start + 1;
                self.lines
                    .insert(insert_at, "  disabled_toolsets:".to_string());
                insert_at += 1;
                for t in required {
                    self.lines.insert(insert_at, format!("    - {t}"));
                    insert_at += 1;
                }
                true
            }
        }
    }

    // Leaves `agent:` in place even if it ends up empty — it may hold
    // unrelated keys we can't see here.
    fn remove_disabled_toolsets(&mut self, names: &[&str]) -> bool {
        let Some((agent_start, agent_end)) = self.agent_block_range() else {
            return false;
        };
        let Some(dt_pos) = self.disabled_toolsets_pos(agent_start, agent_end) else {
            return false;
        };
        let (list_start, list_end) = self.disabled_toolsets_list_range(dt_pos);

        let remaining: Vec<String> = self.lines[list_start..list_end]
            .iter()
            .filter(|l| !names.contains(&l.trim_start().trim_start_matches("- ")))
            .cloned()
            .collect();

        if remaining.len() == list_end - list_start {
            return false;
        }

        let mut out: Vec<String> = Vec::new();
        out.extend_from_slice(&self.lines[..dt_pos]);
        if !remaining.is_empty() {
            out.push(self.lines[dt_pos].clone());
            out.extend(remaining);
        }
        out.extend_from_slice(&self.lines[list_end..]);
        self.lines = out;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:7273/mcp";

    #[test]
    fn hermes_md_content_includes_the_project_path() {
        let content = Hermes.hermes_md_content("/home/user/myproject");
        assert!(content.contains("/home/user/myproject"));
    }

    #[test]
    fn hermes_md_content_documents_the_core_tools() {
        let content = Hermes.hermes_md_content("/tmp/project");
        assert!(content.contains("session_bash"));
        assert!(content.contains("session_mount"));
    }

    #[test]
    fn merge_mcp_entry_creates_key_in_empty_file() {
        let mut config = HermesConfig::parse("");
        assert!(config.merge_mcp_entry(URL));
        assert_eq!(
            config.render(),
            "mcp_servers:\n  drun:\n    url: \"http://127.0.0.1:7273/mcp\"\n    headers:\n      Accept: \"application/json, text/event-stream\"\n"
        );
    }

    #[test]
    fn merge_mcp_entry_appends_key_when_missing_alongside_other_settings() {
        let existing = "model: \"hermes-4-70b\"\ntemperature: 0.7\n";
        let mut config = HermesConfig::parse(existing);
        assert!(config.merge_mcp_entry(URL));
        let out = config.render();
        assert!(out.starts_with(existing));
        assert!(out.contains("mcp_servers:\n  drun:"));
    }

    #[test]
    fn merge_mcp_entry_inserts_under_existing_key_and_preserves_siblings() {
        let existing = "mcp_servers:\n  filesystem:\n    command: \"npx\"\n  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth\n";
        let mut config = HermesConfig::parse(existing);
        assert!(config.merge_mcp_entry(URL));
        let out = config.render();
        assert!(out.contains("  drun:\n    url: \"http://127.0.0.1:7273/mcp\""));
        assert!(out.contains("  filesystem:\n    command: \"npx\""));
        assert!(
            out.contains("  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth")
        );
    }

    #[test]
    fn merge_mcp_entry_is_idempotent() {
        let mut config = HermesConfig::parse("");
        assert!(config.merge_mcp_entry(URL));
        assert!(!config.merge_mcp_entry(URL));
    }

    #[test]
    fn has_mcp_entry_detects_existing_registration() {
        let mut config = HermesConfig::parse("");
        config.merge_mcp_entry(URL);
        assert!(config.has_mcp_entry());
        assert!(!HermesConfig::parse("mcp_servers:\n  other:\n    url: \"x\"\n").has_mcp_entry());
    }

    #[test]
    fn remove_mcp_entry_strips_only_the_drun_block() {
        let existing = "mcp_servers:\n  drun:\n    url: \"http://127.0.0.1:7273/mcp\"\n    headers:\n      Accept: \"x\"\n  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth\n";
        let mut config = HermesConfig::parse(existing);
        assert!(config.remove_mcp_entry());
        let out = config.render();
        assert!(!out.contains("drun:"));
        assert!(out.contains(
            "mcp_servers:\n  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth\n"
        ));
    }

    #[test]
    fn remove_mcp_entry_is_a_no_op_when_absent() {
        let mut config = HermesConfig::parse("");
        assert!(!config.remove_mcp_entry());
    }

    #[test]
    fn merge_disabled_toolsets_creates_full_block_when_agent_key_missing() {
        let mut config = HermesConfig::parse("");
        assert!(config.merge_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
        let out = config.render();
        for t in Hermes::REQUIRED_TOOLSETS {
            assert!(out.contains(&format!("    - {t}")), "missing {t} in {out}");
        }
        assert!(out.starts_with("agent:\n  disabled_toolsets:\n"));
    }

    #[test]
    fn merge_disabled_toolsets_adds_list_when_agent_key_exists_without_it() {
        let existing = "agent:\n  max_iterations: 50\n";
        let mut config = HermesConfig::parse(existing);
        assert!(config.merge_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
        let out = config.render();
        assert!(out.contains("agent:\n  disabled_toolsets:\n"));
        assert!(out.contains("  max_iterations: 50"));
        for t in Hermes::REQUIRED_TOOLSETS {
            assert!(out.contains(&format!("    - {t}")));
        }
    }

    #[test]
    fn merge_disabled_toolsets_preserves_user_entries_and_dedupes() {
        let existing = "agent:\n  disabled_toolsets:\n    - memory\n    - terminal\n";
        let mut config = HermesConfig::parse(existing);
        assert!(config.merge_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
        let out = config.render();
        assert_eq!(out.matches("- terminal").count(), 1);
        assert!(out.contains("- memory"));
        for t in Hermes::REQUIRED_TOOLSETS {
            assert!(out.contains(&format!("- {t}")));
        }
    }

    #[test]
    fn merge_disabled_toolsets_is_idempotent() {
        let mut config = HermesConfig::parse("");
        assert!(config.merge_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
        let first = config.render();
        assert!(!config.merge_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
        assert_eq!(first, config.render());
    }

    #[test]
    fn remove_disabled_toolsets_drops_only_our_entries() {
        let existing = "agent:\n  disabled_toolsets:\n    - memory\n    - terminal\n    - file\n    - web\n    - search\n    - delegation\n";
        let mut config = HermesConfig::parse(existing);
        assert!(config.remove_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
        let out = config.render();
        assert!(out.contains("- memory"));
        for t in Hermes::REQUIRED_TOOLSETS {
            assert!(!out.contains(&format!("- {t}")));
        }
    }

    #[test]
    fn remove_disabled_toolsets_drops_the_key_when_list_becomes_empty() {
        let mut config = HermesConfig::parse("");
        config.merge_disabled_toolsets(Hermes::REQUIRED_TOOLSETS);
        assert!(config.remove_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
        let out = config.render();
        assert!(!out.contains("disabled_toolsets"));
        assert!(out.contains("agent:"));
    }

    #[test]
    fn remove_disabled_toolsets_is_a_no_op_when_absent() {
        let mut config = HermesConfig::parse("");
        assert!(!config.remove_disabled_toolsets(Hermes::REQUIRED_TOOLSETS));
    }

    #[test]
    fn has_any_disabled_toolset_detects_our_entries() {
        let mut config = HermesConfig::parse("");
        config.merge_disabled_toolsets(Hermes::REQUIRED_TOOLSETS);
        assert!(config.has_any_disabled_toolset(Hermes::REQUIRED_TOOLSETS));
        assert!(
            !HermesConfig::parse("agent:\n  disabled_toolsets:\n    - memory\n")
                .has_any_disabled_toolset(Hermes::REQUIRED_TOOLSETS)
        );
    }
}

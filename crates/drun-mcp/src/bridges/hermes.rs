use std::{
    io,
    path::PathBuf,
    process::{Command, Stdio},
};

// Hermes has no CLI for registering arbitrary (non-catalog) MCP servers, so
// this edits ~/.hermes/config.yaml directly. Editing is done as targeted line
// surgery rather than a full YAML parse/reserialize, so any comments or
// formatting the user already has in that file survive untouched.
// See: https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/mcp.md
//      https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/configuration.md#global-toolset-disable

// Toolsets that overlap with drun's sandboxed tools (terminal/file execution,
// web access, subagent delegation) — mirrors Claude Code's REQUIRED_DENY list
// in bridges::claude, translated to Hermes's toolset names. Disabling these is
// a machine-wide setting (agent.disabled_toolsets applies to every Hermes
// session, not just this project) since Hermes has no per-project scope.
const REQUIRED_TOOLSETS: &[&str] = &["terminal", "file", "web", "search", "delegation"];

/// [`super::Bridge`] impl — see that trait for the extensibility contract.
pub struct Hermes;

impl super::Bridge for Hermes {
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
        super::shared::write_project_instructions(
            &project_dir,
            "HERMES.md",
            &hermes_md_content(project_path),
        );
        super::shared::allow_mount_path(&crate::drun_home(), &project_dir);

        if !hermes_available() {
            let path = hermes_config_path();
            eprintln!(
                "drun: Hermes CLI not found. Add drun manually to {}:",
                path.display()
            );
            eprint!("{}", rendered_mcp_entry(&mcp_http_url()));
            return;
        }

        register_mcp();
        restrict_toolsets();
    }

    fn deregister(&self) {
        let path = hermes_config_path();
        let Ok(existing) = std::fs::read_to_string(&path) else {
            return;
        };

        let mut updated = existing.clone();
        let mut changed = false;

        if has_mcp_entry(&updated) {
            updated = remove_mcp_entry(&updated);
            changed = true;
        }

        if has_any_disabled_toolset(&updated, REQUIRED_TOOLSETS) {
            updated = remove_disabled_toolsets(&updated, REQUIRED_TOOLSETS);
            changed = true;
        }

        if !changed {
            return;
        }

        if std::fs::write(&path, updated).is_ok() {
            eprintln!("drun: removed from Hermes ({}).", path.display());
        }
    }
}

fn hermes_config_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".hermes/config.yaml")
}

fn mcp_http_url() -> String {
    format!("http://127.0.0.1:{}/mcp", crate::mcp_port())
}

fn hermes_available() -> bool {
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

fn rendered_mcp_entry(url: &str) -> String {
    format!(
        "mcp_servers:\n  drun:\n    url: \"{url}\"\n    headers:\n      Accept: \"application/json, text/event-stream\"\n"
    )
}

fn hermes_md_content(project_path: &str) -> String {
    format!(
        "# Agent instructions\n\n\
         This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.\n\
         Hermes's native `terminal`, `file`, `web`, `search`, and `delegation` toolsets are\n\
         disabled (machine-wide — see `~/.hermes/config.yaml`) so they don't bypass the\n\
         sandbox. Use the drun MCP tools for everything.\n\n{}",
        super::shared::drun_instructions_body(project_path)
    )
}

fn register_mcp() {
    let path = hermes_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    if has_mcp_entry(&existing) {
        eprintln!("drun: already registered in Hermes, skipping.");
        return;
    }

    let updated = merge_mcp_entry(&existing, &mcp_http_url());
    match std::fs::write(&path, updated) {
        Ok(()) => eprintln!(
            "drun: added to Hermes (HTTP → {}, {}).",
            mcp_http_url(),
            path.display()
        ),
        Err(e) => eprintln!("drun: could not write {} ({e})", path.display()),
    }
}

fn restrict_toolsets() {
    let path = hermes_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let updated = merge_disabled_toolsets(&existing, REQUIRED_TOOLSETS);
    if updated == existing {
        eprintln!("drun: Hermes toolset restriction already applied, skipping.");
        return;
    }

    match std::fs::write(&path, updated) {
        Ok(()) => eprintln!(
            "drun: disabled Hermes's native {} toolsets globally (agent.disabled_toolsets in \
             {}) — this applies to every Hermes session on this machine, not just this project.",
            REQUIRED_TOOLSETS.join("/"),
            path.display()
        ),
        Err(e) => eprintln!("drun: could not write {} ({e})", path.display()),
    }
}

fn has_mcp_entry(content: &str) -> bool {
    content.lines().any(|l| l.trim_end() == "  drun:")
}

/// Inserts a `drun:` entry under the top-level `mcp_servers:` key, creating
/// that key if it doesn't exist yet. Leaves every other line untouched.
fn merge_mcp_entry(existing: &str, url: &str) -> String {
    let entry = [
        "  drun:".to_string(),
        format!("    url: \"{url}\""),
        "    headers:".to_string(),
        "      Accept: \"application/json, text/event-stream\"".to_string(),
    ];

    let lines: Vec<&str> = existing.lines().collect();

    if let Some(pos) = lines.iter().position(|l| l.trim_end() == "mcp_servers:") {
        let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        for (i, e) in entry.iter().enumerate() {
            out.insert(pos + 1 + i, e.clone());
        }
        let mut rendered = out.join("\n");
        rendered.push('\n');
        rendered
    } else {
        let mut out = existing.to_string();
        if !out.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str("mcp_servers:\n");
        for e in &entry {
            out.push_str(e);
            out.push('\n');
        }
        out
    }
}

/// Removes the `drun:` entry (and everything indented under it) from
/// `mcp_servers:`, leaving sibling entries and everything else untouched.
fn remove_mcp_entry(existing: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;

    for line in existing.lines() {
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
        out.push_str(line);
        out.push('\n');
    }

    out
}

fn agent_block_range(lines: &[String]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.trim_end() == "agent:")?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| !l.trim().is_empty() && !l.starts_with("  "))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Some((start, end))
}

fn disabled_toolsets_list_range(lines: &[String], dt_pos: usize) -> (usize, usize) {
    let end = lines[dt_pos + 1..]
        .iter()
        .position(|l| !(l.starts_with("    - ") || l.trim().is_empty()))
        .map(|i| dt_pos + 1 + i)
        .unwrap_or(lines.len());
    (dt_pos + 1, end)
}

/// Adds any of `required` not already present under `agent.disabled_toolsets`,
/// creating `agent:` and/or `disabled_toolsets:` as needed. Never removes an
/// existing (possibly user-added) entry.
fn merge_disabled_toolsets(existing: &str, required: &[&str]) -> String {
    let mut lines: Vec<String> = existing.lines().map(|s| s.to_string()).collect();

    let Some((agent_start, agent_end)) = agent_block_range(&lines) else {
        let mut out = existing.to_string();
        if !out.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str("agent:\n  disabled_toolsets:\n");
        for t in required {
            out.push_str(&format!("    - {t}\n"));
        }
        return out;
    };

    let dt_pos = lines[agent_start + 1..agent_end]
        .iter()
        .position(|l| l.trim_end() == "  disabled_toolsets:")
        .map(|i| agent_start + 1 + i);

    match dt_pos {
        Some(dt_pos) => {
            let (list_start, list_end) = disabled_toolsets_list_range(&lines, dt_pos);
            let existing_items: Vec<String> = lines[list_start..list_end]
                .iter()
                .map(|l| l.trim_start().trim_start_matches("- ").to_string())
                .collect();

            let mut insert_at = list_end;
            for t in required {
                if !existing_items.iter().any(|i| i == t) {
                    lines.insert(insert_at, format!("    - {t}"));
                    insert_at += 1;
                }
            }
        }
        None => {
            let mut insert_at = agent_start + 1;
            lines.insert(insert_at, "  disabled_toolsets:".to_string());
            insert_at += 1;
            for t in required {
                lines.insert(insert_at, format!("    - {t}"));
                insert_at += 1;
            }
        }
    }

    let mut rendered = lines.join("\n");
    rendered.push('\n');
    rendered
}

fn has_any_disabled_toolset(existing: &str, names: &[&str]) -> bool {
    let lines: Vec<String> = existing.lines().map(|s| s.to_string()).collect();
    let Some((agent_start, agent_end)) = agent_block_range(&lines) else {
        return false;
    };
    let Some(dt_pos) = lines[agent_start + 1..agent_end]
        .iter()
        .position(|l| l.trim_end() == "  disabled_toolsets:")
        .map(|i| agent_start + 1 + i)
    else {
        return false;
    };
    let (list_start, list_end) = disabled_toolsets_list_range(&lines, dt_pos);
    lines[list_start..list_end]
        .iter()
        .any(|l| names.contains(&l.trim_start().trim_start_matches("- ")))
}

/// Removes exactly the entries in `names` from `agent.disabled_toolsets`,
/// leaving any other (e.g. user-added) entries alone. If the list becomes
/// empty, removes `disabled_toolsets:` too; leaves `agent:` itself even if
/// it ends up empty, since it may hold unrelated keys we can't see here.
fn remove_disabled_toolsets(existing: &str, names: &[&str]) -> String {
    let lines: Vec<String> = existing.lines().map(|s| s.to_string()).collect();
    let Some((agent_start, agent_end)) = agent_block_range(&lines) else {
        return existing.to_string();
    };
    let Some(dt_pos) = lines[agent_start + 1..agent_end]
        .iter()
        .position(|l| l.trim_end() == "  disabled_toolsets:")
        .map(|i| agent_start + 1 + i)
    else {
        return existing.to_string();
    };
    let (list_start, list_end) = disabled_toolsets_list_range(&lines, dt_pos);

    let remaining: Vec<String> = lines[list_start..list_end]
        .iter()
        .filter(|l| !names.contains(&l.trim_start().trim_start_matches("- ")))
        .cloned()
        .collect();

    let mut out: Vec<String> = Vec::new();
    out.extend_from_slice(&lines[..dt_pos]);
    if !remaining.is_empty() {
        out.push(lines[dt_pos].clone());
        out.extend(remaining);
    }
    out.extend_from_slice(&lines[list_end..]);

    let mut rendered = out.join("\n");
    if !rendered.is_empty() {
        rendered.push('\n');
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:7273/mcp";

    #[test]
    fn hermes_md_content_includes_the_project_path() {
        let content = hermes_md_content("/home/user/myproject");
        assert!(content.contains("/home/user/myproject"));
    }

    #[test]
    fn hermes_md_content_documents_the_core_tools() {
        let content = hermes_md_content("/tmp/project");
        assert!(content.contains("session_bash"));
        assert!(content.contains("session_mount"));
    }

    #[test]
    fn merge_mcp_entry_creates_key_in_empty_file() {
        let out = merge_mcp_entry("", URL);
        assert_eq!(
            out,
            "mcp_servers:\n  drun:\n    url: \"http://127.0.0.1:7273/mcp\"\n    headers:\n      Accept: \"application/json, text/event-stream\"\n"
        );
    }

    #[test]
    fn merge_mcp_entry_appends_key_when_missing_alongside_other_settings() {
        let existing = "model: \"hermes-4-70b\"\ntemperature: 0.7\n";
        let out = merge_mcp_entry(existing, URL);
        assert!(out.starts_with(existing));
        assert!(out.contains("mcp_servers:\n  drun:"));
    }

    #[test]
    fn merge_mcp_entry_inserts_under_existing_key_and_preserves_siblings() {
        let existing = "mcp_servers:\n  filesystem:\n    command: \"npx\"\n  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth\n";
        let out = merge_mcp_entry(existing, URL);
        assert!(out.contains("  drun:\n    url: \"http://127.0.0.1:7273/mcp\""));
        assert!(out.contains("  filesystem:\n    command: \"npx\""));
        assert!(
            out.contains("  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth")
        );
    }

    #[test]
    fn has_mcp_entry_detects_existing_registration() {
        let existing = merge_mcp_entry("", URL);
        assert!(has_mcp_entry(&existing));
        assert!(!has_mcp_entry("mcp_servers:\n  other:\n    url: \"x\"\n"));
    }

    #[test]
    fn remove_mcp_entry_strips_only_the_drun_block() {
        let existing = "mcp_servers:\n  drun:\n    url: \"http://127.0.0.1:7273/mcp\"\n    headers:\n      Accept: \"x\"\n  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth\n";
        let out = remove_mcp_entry(existing);
        assert!(!out.contains("drun:"));
        assert!(out.contains(
            "mcp_servers:\n  linear:\n    url: \"https://mcp.linear.app/mcp\"\n    auth: oauth\n"
        ));
    }

    #[test]
    fn merge_disabled_toolsets_creates_full_block_when_agent_key_missing() {
        let out = merge_disabled_toolsets("", REQUIRED_TOOLSETS);
        for t in REQUIRED_TOOLSETS {
            assert!(out.contains(&format!("    - {t}")), "missing {t} in {out}");
        }
        assert!(out.starts_with("agent:\n  disabled_toolsets:\n"));
    }

    #[test]
    fn merge_disabled_toolsets_adds_list_when_agent_key_exists_without_it() {
        let existing = "agent:\n  max_iterations: 50\n";
        let out = merge_disabled_toolsets(existing, REQUIRED_TOOLSETS);
        assert!(out.contains("agent:\n  disabled_toolsets:\n"));
        assert!(out.contains("  max_iterations: 50"));
        for t in REQUIRED_TOOLSETS {
            assert!(out.contains(&format!("    - {t}")));
        }
    }

    #[test]
    fn merge_disabled_toolsets_preserves_user_entries_and_dedupes() {
        let existing = "agent:\n  disabled_toolsets:\n    - memory\n    - terminal\n";
        let out = merge_disabled_toolsets(existing, REQUIRED_TOOLSETS);
        assert_eq!(out.matches("- terminal").count(), 1);
        assert!(out.contains("- memory"));
        for t in REQUIRED_TOOLSETS {
            assert!(out.contains(&format!("- {t}")));
        }
    }

    #[test]
    fn merge_disabled_toolsets_is_idempotent() {
        let first = merge_disabled_toolsets("", REQUIRED_TOOLSETS);
        let second = merge_disabled_toolsets(&first, REQUIRED_TOOLSETS);
        assert_eq!(first, second);
    }

    #[test]
    fn remove_disabled_toolsets_drops_only_our_entries() {
        let existing = "agent:\n  disabled_toolsets:\n    - memory\n    - terminal\n    - file\n    - web\n    - search\n    - delegation\n";
        let out = remove_disabled_toolsets(existing, REQUIRED_TOOLSETS);
        assert!(out.contains("- memory"));
        for t in REQUIRED_TOOLSETS {
            assert!(!out.contains(&format!("- {t}")));
        }
    }

    #[test]
    fn remove_disabled_toolsets_drops_the_key_when_list_becomes_empty() {
        let existing = merge_disabled_toolsets("", REQUIRED_TOOLSETS);
        let out = remove_disabled_toolsets(&existing, REQUIRED_TOOLSETS);
        assert!(!out.contains("disabled_toolsets"));
        assert!(out.contains("agent:"));
    }

    #[test]
    fn has_any_disabled_toolset_detects_our_entries() {
        let existing = merge_disabled_toolsets("", REQUIRED_TOOLSETS);
        assert!(has_any_disabled_toolset(&existing, REQUIRED_TOOLSETS));
        assert!(!has_any_disabled_toolset(
            "agent:\n  disabled_toolsets:\n    - memory\n",
            REQUIRED_TOOLSETS
        ));
    }
}

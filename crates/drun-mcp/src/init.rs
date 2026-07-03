use std::{
    io::Write,
    path::{Path, PathBuf},
};

const CLAUDE_SETTINGS: &str = r#"{
  "permissions": {
    "deny": [
      "Bash", "BashOutput", "KillBash",
      "Edit", "Write", "NotebookEdit",
      "Read", "Glob", "Grep",
      "WebFetch", "WebSearch",
      "Task"
    ],
    "allow": ["mcp__drun__*"]
  }
}"#;

pub fn run() {
    let cwd = std::env::current_dir().expect("cannot read current directory");
    let drun_home = drun_home();

    write_settings(&cwd);
    write_claude_md(&cwd);
    register_project(&drun_home, &cwd);

    eprintln!("drun: initialized for {}", cwd.display());
}

fn drun_home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".drun")
}

fn write_settings(project_dir: &Path) {
    let settings_dir = project_dir.join(".claude");
    let settings_file = settings_dir.join("settings.json");

    std::fs::create_dir_all(&settings_dir).expect("cannot create .claude/");

    if settings_file.exists() {
        eprintln!("drun: .claude/settings.json already exists, skipping");
        return;
    }

    std::fs::write(&settings_file, CLAUDE_SETTINGS).expect("cannot write settings.json");
    eprintln!("drun: created .claude/settings.json");
}

fn write_claude_md(project_dir: &Path) {
    let claude_md = project_dir.join("CLAUDE.md");

    if claude_md.exists() {
        eprintln!("drun: CLAUDE.md already exists, skipping");
        return;
    }

    let project_path = project_dir.to_str().expect("non-UTF-8 project path");
    std::fs::write(&claude_md, claude_md_content(project_path)).expect("cannot write CLAUDE.md");
    eprintln!("drun: created CLAUDE.md");
}

fn register_project(drun_home: &Path, project_dir: &Path) {
    std::fs::create_dir_all(drun_home).expect("cannot create ~/.drun");
    let registry = drun_home.join("projects");
    let project_path = project_dir.to_str().expect("non-UTF-8 project path");

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

fn claude_md_content(project_path: &str) -> String {
    format!(
        r#"# Agent instructions

This project uses [drun](https://github.com/dmosc/drun) as a sandboxed runtime.
Native file, shell, and network tools (`Bash`, `Edit`, `Write`, `NotebookEdit`,
`Read`, `Glob`, `Grep`, `WebFetch`, `WebSearch`, `Task`) are disabled for this
workspace — they would otherwise read or write the host directly, bypassing the
sandbox. Use the drun MCP tools (prefixed `mcp__drun__`) for everything.

## Getting started

1. Call `create_session` — sessions start with an empty workspace.
2. Call `session_mount` with path `{project_path}` to load this project's files
   into the session. Re-mount any other host paths you need the same way.
3. From there, work entirely through drun tools — there is no host file or shell
   access outside of them.

## Core tools

- **`session_bash`** — run shell commands in the sandboxed workspace (also
  covers listing/searching files — e.g. `ls`, `grep`, `find`)
- **`session_read_file`** / **`session_write_file`** / **`session_delete_file`**
  — read, write, and delete files in the session
- **`session_mount`** — load a host file or directory into the session
- **`session_fetch`** — make HTTP requests from the sandbox (subject to the
  server's domain_allowlist)
- **`session_export`** — write session files back out to the host
- **`session_diff`** / **`session_rollback`** / **`session_fork`** — inspect and
  navigate checkpoint history (session_rollback is destructive past the rollback
  point once you continue the session — use session_fork first if you need to
  keep that history)
"#
    )
}

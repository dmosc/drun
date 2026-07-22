use std::path::Path;

pub(crate) fn drun_instructions_body(project_path: &str) -> String {
    format!(
        r#"## Getting started

1. Call `create_session` — sessions start with an empty workspace.
2. Call `session_mount` with path `{project_path}` to load this project's files
   into the session (already allowlisted by drun's setup for this project).
   Re-mount any other host paths you need the same way.
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

## If a fetch or mount is denied

`session_fetch` and `session_mount` are restricted to an allowlist. If either
is denied for a domain or path you need, tell the user to run:

- `drun-mcp config add-domain <domain>` to allow a new domain for
  `session_fetch`
- `drun-mcp config add-path <path>` to allow a new host path for
  `session_mount`

Both commands edit `~/.drun/config.toml` directly — no restart needed, and
the change is visible on your very next tool call in this same session.
"#
    )
}

pub(crate) fn write_project_instructions(project_dir: &Path, filename: &str, content: &str) {
    let path = project_dir.join(filename);

    if path.exists() {
        eprintln!("drun: {filename} already exists, skipping");
        return;
    }

    match std::fs::write(&path, content) {
        Ok(()) => eprintln!("drun: created {filename}"),
        Err(e) => eprintln!("drun: could not write {filename} ({e})"),
    }
}

/// Adds `project_dir` to `mount_allowlist` in `drun_home`'s config.toml, if
/// that config already exists (i.e. drun-mcp itself has been installed) —
/// a no-op otherwise, since there's nothing to update yet.
pub(crate) fn allow_mount_path(drun_home: &Path, project_dir: &Path) {
    let config_path = drun_home.join("config.toml");
    if !config_path.exists() {
        return;
    }

    match crate::config_cmd::add_path_to(&config_path, project_dir) {
        Ok(true) => eprintln!("drun: added '{}' to mount_allowlist", project_dir.display()),
        Ok(false) => {}
        Err(e) => eprintln!(
            "drun: could not update mount_allowlist ({e}) — add it manually with \
             `drun-mcp config add-path {}`",
            project_dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drun_instructions_body_includes_the_project_path() {
        let body = drun_instructions_body("/home/user/myproject");
        assert!(body.contains("/home/user/myproject"));
    }

    #[test]
    fn drun_instructions_body_documents_the_core_tools() {
        let body = drun_instructions_body("/tmp/project");
        assert!(body.contains("session_bash"));
        assert!(body.contains("session_mount"));
    }

    #[test]
    fn write_project_instructions_creates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_project_instructions(dir.path(), "HERMES.md", "content");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("HERMES.md")).unwrap(),
            "content"
        );
    }

    #[test]
    fn write_project_instructions_does_not_overwrite_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("HERMES.md"), "custom").unwrap();

        write_project_instructions(dir.path(), "HERMES.md", "generated");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("HERMES.md")).unwrap(),
            "custom"
        );
    }

    #[test]
    fn allow_mount_path_adds_the_project_dir_to_an_existing_config() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            drun_home.path().join("config.toml"),
            "mount_allowlist = []\n",
        )
        .unwrap();

        allow_mount_path(drun_home.path(), project_dir.path());

        let config = drun_core::Config::load_from(Some(&drun_home.path().join("config.toml")));
        assert!(
            config
                .mount_allowlist
                .contains(&project_dir.path().to_path_buf())
        );
    }

    #[test]
    fn allow_mount_path_is_a_no_op_without_a_daemon_config() {
        let drun_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        allow_mount_path(drun_home.path(), project_dir.path());

        assert!(!drun_home.path().join("config.toml").exists());
    }
}

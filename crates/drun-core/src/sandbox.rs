//! Sandbox execution for shell commands. On macOS uses sandbox-exec with an
//! SBPL profile; on Linux uses bubblewrap (bwrap). Both strategies confine
//! the command to the session workspace with no network access.

use std::path::Path;
use std::process::Command;

#[cfg(target_os = "macos")]
pub(crate) fn sandboxed_sh(command: &str, workspace_dir: &Path) -> anyhow::Result<Command> {
    let workspace = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    // Apple Sandbox Profile Language (SBPL): a Scheme-like DSL interpreted by
    // the macOS kernel. "deny default" blocks everything not explicitly
    // allowed.
    //
    // File writes are limited to the workspace temp dir, /private/tmp, and
    // /dev/null. No network rules means all network is denied by default.
    let profile = format!(
        "(version 1)\n\
         (deny default)\n\
         (allow file-read*)\n\
         (allow file-write*\n\
             (subpath \"{}\")\n\
             (subpath \"/private/tmp\")\n\
             (literal \"/dev/null\"))\n\
         (allow process-exec*)\n\
         (allow process-fork)\n\
         (allow signal)\n\
         (allow mach-lookup)\n\
         (allow mach-priv-host-port)\n\
         (allow sysctl-read)\n",
        workspace.display()
    );

    let mut cmd = Command::new("sandbox-exec");
    cmd.arg("-p").arg(profile).arg("sh").arg("-c").arg(command);
    Ok(cmd)
}

#[cfg(target_os = "linux")]
pub(crate) fn sandboxed_sh(command: &str, workspace_dir: &Path) -> anyhow::Result<Command> {
    let workspace = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    let workspace_str = workspace.to_string_lossy().into_owned();
    which::which("bwrap").map_err(|_| {
        anyhow::anyhow!(
            "bwrap not found; install bubblewrap (e.g. `apt install bubblewrap`) \
             to enable session_bash"
        )
    })?;
    let mut cmd = Command::new("bwrap");
    cmd.args([
        "--ro-bind",
        "/",
        "/", // read-only view of host fs (PATH, libs, tools)
        "--dev",
        "/dev", // basic devices
        "--proc",
        "/proc", // procfs (required by many programs)
        "--bind",
        &workspace_str,
        &workspace_str, // writable workspace
        "--tmpfs",
        "/tmp",              // isolated /tmp
        "--unshare-net",     // no network access
        "--die-with-parent", // clean up if parent process exits
        "--",
        "sh",
        "-c",
        command,
    ]);
    Ok(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn sandboxed_sh(_command: &str, _workspace_dir: &Path) -> anyhow::Result<Command> {
    anyhow::bail!("session_bash is not supported on this platform")
}

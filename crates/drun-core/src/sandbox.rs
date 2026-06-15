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

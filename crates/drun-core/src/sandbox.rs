//! Sandbox execution for shell commands. On macOS uses sandbox-exec with an
//! SBPL profile; on Linux uses bubblewrap (bwrap). Both strategies confine
//! the command to the session workspace with no network access.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct Sandbox {
    workspace: PathBuf,
    scratch: PathBuf,
    read_paths: Vec<PathBuf>,
}

impl Sandbox {
    pub(crate) fn new(workspace_dir: &Path, scratch_dir: &Path, read_paths: Vec<PathBuf>) -> Self {
        let canonicalize = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        Self {
            workspace: canonicalize(workspace_dir),
            scratch: canonicalize(scratch_dir),
            read_paths,
        }
    }

    // Redirect scratch locations from host to an ephemeral directory inside the
    // sandbox. These map from well-known environment variables in the host that
    // common binaries rely on to create temporary files.
    const SCRATCH_ENV_VARS: &'static [(&'static str, &'static str)] = &[
        ("HOME", ""),
        ("TMPDIR", ""),
        // https://specifications.freedesktop.org/basedir/latest
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_DATA_HOME", ".local/share"),
        ("XDG_STATE_HOME", ".local/state"),
    ];

    fn apply_scratch_env(&self, cmd: &mut Command) {
        cmd.envs(
            Self::SCRATCH_ENV_VARS
                .iter()
                .map(|(key, subpath)| (*key, self.scratch.join(subpath))),
        );
    }

    #[cfg(target_os = "macos")]
    const SYSTEM_READ_PATHS: &'static [&'static str] = &[
        "/usr",
        "/bin",
        "/sbin",
        "/opt",
        "/System",
        "/Library",
        "/etc",
        "/dev",
        "/private/tmp",
    ];

    #[cfg(target_os = "linux")]
    const SYSTEM_READ_PATHS: &'static [&'static str] =
        &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/opt", "/etc"];

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn allowed_read_paths(&self) -> Vec<PathBuf> {
        let mut candidates: Vec<PathBuf> = vec![self.workspace.clone(), self.scratch.clone()];
        candidates.extend(self.read_paths.iter().cloned());
        candidates.extend(Self::SYSTEM_READ_PATHS.iter().map(PathBuf::from));
        if let Ok(path_var) = std::env::var("PATH") {
            candidates.extend(std::env::split_paths(&path_var));
        }
        candidates
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn command(&self, command: &str) -> anyhow::Result<Command> {
        // Apple Sandbox Profile Language (SBPL): a Scheme-like DSL interpreted
        // by the macOS kernel. "deny default" blocks everything not
        // explicitly allowed.
        //
        // File contents can only be read from the workspace, scratch dir, any
        // mounted overlays, and the fixed/PATH system directories in
        // `allowed_read_paths` — not the whole host filesystem. File writes
        // are limited to the workspace, scratch dir, /private/tmp, and
        // /dev/null.
        let read_subpaths: String = self
            .allowed_read_paths()
            .iter()
            .map(|p| format!("    (subpath \"{}\")\n", p.display()))
            .collect();
        let profile = format!(
            "(version 1)\n\
             (deny default)\n\
             (allow file-read-metadata)\n\
             (allow file-read* (literal \"/\")\n{})\n\
             (allow file-write*\n\
                 (subpath \"{}\")\n\
                 (subpath \"{}\")\n\
                 (subpath \"/private/tmp\")\n\
                 (subpath \"{}\")\n\
                 (literal \"/dev/null\"))\n\
             (allow process-exec*)\n\
             (allow process-fork)\n\
             (allow signal)\n\
             (allow mach-lookup)\n\
             (allow mach-priv-host-port)\n\
             (allow sysctl-read)\n",
            read_subpaths,
            self.workspace.display(),
            self.scratch.display(),
            std::env::temp_dir().display()
        );

        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-p").arg(profile).arg("sh").arg("-c").arg(command);
        self.apply_scratch_env(&mut cmd);
        // New process group set globally allows cleanup workflows to wipe out
        // all spawned processes and subprocesses, ensuring that none remains
        // alive.
        cmd.process_group(0);
        Ok(cmd)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn command(&self, command: &str) -> anyhow::Result<Command> {
        let workspace_str = self.workspace.to_string_lossy().into_owned();
        let scratch_str = self.scratch.to_string_lossy().into_owned();
        which::which("bwrap").map_err(|_| {
            anyhow::anyhow!(
                "bwrap not found; install bubblewrap (e.g. `apt install bubblewrap`) \
                 to enable session_bash"
            )
        })?;
        let mut cmd = Command::new("bwrap");
        cmd.args(["--dev", "/dev", "--proc", "/proc"]);
        // Read-only binds are limited to the workspace, scratch dir, any
        // mounted overlays, and the fixed/PATH system directories in
        // `allowed_read_paths` — not the whole host root like a blanket
        // `--ro-bind / /` would give.
        for path in self.allowed_read_paths() {
            if path == self.workspace || path == self.scratch {
                continue; // bound read-write below instead
            }
            let path_str = path.to_string_lossy().into_owned();
            cmd.arg("--ro-bind").arg(&path_str).arg(&path_str);
        }
        cmd.args([
            "--bind",
            &workspace_str,
            &workspace_str, // writable workspace
            "--bind",
            &scratch_str,
            &scratch_str, // writable scratch dir (see SCRATCH_ENV_VARS)
            "--tmpfs",
            "/tmp",              // isolated /tmp
            "--unshare-net",     // no network access
            "--die-with-parent", // clean up if parent process exits
            "--",
            "sh",
            "-c",
            command,
        ]);
        self.apply_scratch_env(&mut cmd);
        // New process group set globally allows cleanup workflows to wipe out
        // all spawned processes and subprocesses, ensuring that none remains
        // alive.
        cmd.process_group(0);
        Ok(cmd)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn command(&self, _command: &str) -> anyhow::Result<Command> {
        anyhow::bail!("session_bash is not supported on this platform")
    }
}

//! Sandbox execution for shell commands. On macOS uses sandbox-exec with an
//! SBPL profile; on Linux uses bubblewrap (bwrap). Both strategies confine
//! the command to the session workspace with no network access, except
//! `networked_command`, which trades network confinement for filesystem
//! confinement to whatever directory it's pointed at (on macOS, loopback is
//! still denied — see `sbpl_profile`; on Linux there's no such carve-out).

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
        "/usr", "/bin", "/sbin", "/opt", "/System", "/Library", "/etc", "/dev",
    ];

    #[cfg(target_os = "linux")]
    const SYSTEM_READ_PATHS: &'static [&'static str] =
        &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/opt", "/etc"];

    // Paths the sandboxed process may look at but never modify: fixed OS
    // directories, whatever's on PATH, and any host paths explicitly mounted
    // in for this session.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn read_only_paths(&self) -> Vec<PathBuf> {
        let mut candidates = self.read_paths.clone();
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

    // Paths the sandboxed process may both read and write.
    #[cfg(target_os = "macos")]
    fn read_write_paths(&self) -> Vec<PathBuf> {
        let canonicalize = |p: PathBuf| p.canonicalize().unwrap_or(p);
        vec![
            self.workspace.clone(),
            self.scratch.clone(),
            canonicalize(PathBuf::from("/tmp")),
        ]
    }

    #[cfg(target_os = "linux")]
    fn read_write_paths(&self) -> Vec<PathBuf> {
        vec![self.workspace.clone(), self.scratch.clone()]
    }

    // xcrun/clang keep a cache file under the real per-user Darwin temp dir
    // regardless of the TMPDIR override in `SCRATCH_ENV_VARS`.
    #[cfg(target_os = "macos")]
    fn xcrun_cache_rule(&self) -> String {
        let temp_dir = std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir());
        let escaped = temp_dir.display().to_string().replace('.', "\\.");
        format!("    (regex #\"^{escaped}/xcrun_db.*$\")\n")
    }

    #[cfg(target_os = "macos")]
    fn sbpl_subpaths<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> String {
        paths
            .map(|p| format!("    (subpath \"{}\")\n", p.display()))
            .collect()
    }

    // Apple Sandbox Profile Language (SBPL): a Scheme-like DSL interpreted by
    // the macOS kernel. "deny default" blocks everything not explicitly
    // allowed. `allow_network` is only ever set by `networked_command` —
    // `command` (session_bash) always omits it.
    #[cfg(target_os = "macos")]
    fn sbpl_profile(&self, allow_network: bool) -> String {
        let read_only = self.read_only_paths();
        let read_write = self.read_write_paths();
        let xcrun_rule = self.xcrun_cache_rule();
        let read_subpaths = Self::sbpl_subpaths(read_only.iter().chain(read_write.iter()));
        let write_subpaths = Self::sbpl_subpaths(read_write.iter());
        let network_rule = if allow_network {
            // Block local loopback interface but allow egress network.
            "(deny network-outbound (remote ip \"localhost:*\"))\n(allow network*)\n"
        } else {
            ""
        };
        format!(
            "(version 1)\n\
             (deny default)\n\
             (allow file-read-metadata)\n\
             (allow file-read* (literal \"/\")\n{read_subpaths}{xcrun_rule})\n\
             (allow file-write*\n{write_subpaths}{xcrun_rule}    (literal \"/dev/null\"))\n\
             {network_rule}\
             (allow process-exec*)\n\
             (allow process-fork)\n\
             (allow signal)\n\
             (allow mach-lookup)\n\
             (allow mach-priv-host-port)\n\
             (allow sysctl-read)\n"
        )
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn command(&self, command: &str) -> anyhow::Result<Command> {
        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-p")
            .arg(self.sbpl_profile(false))
            .arg("sh")
            .arg("-c")
            .arg(command);
        self.apply_scratch_env(&mut cmd);
        // New process group set globally allows cleanup workflows to wipe out
        // all spawned processes and subprocesses, ensuring that none remains
        // alive.
        cmd.process_group(0);
        Ok(cmd)
    }

    // Runs argv directly (no shell) with network allowed (except loopback,
    // see sbpl_profile), still confined to this Sandbox's workspace/scratch.
    // SBPL can't filter network access by hostname, so this is unrestricted
    // egress to anything non-local — callers keep the blast radius small by
    // pointing it at a disposable directory, never the real session
    // workspace.
    #[cfg(target_os = "macos")]
    pub(crate) fn networked_command(&self, argv: &[String]) -> anyhow::Result<Command> {
        let Some((program, args)) = argv.split_first() else {
            anyhow::bail!("networked_command requires a non-empty argv");
        };
        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-p")
            .arg(self.sbpl_profile(true))
            .arg(program)
            .args(args);
        self.apply_scratch_env(&mut cmd);
        cmd.process_group(0);
        Ok(cmd)
    }

    #[cfg(target_os = "linux")]
    fn bwrap_command(&self) -> anyhow::Result<Command> {
        which::which("bwrap").map_err(|_| {
            anyhow::anyhow!(
                "bwrap not found; install bubblewrap (e.g. `apt install bubblewrap`) \
                 to enable session_bash"
            )
        })?;
        let mut cmd = Command::new("bwrap");
        cmd.args(["--dev", "/dev", "--proc", "/proc"]);
        for path in self.read_only_paths() {
            let path_str = path.to_string_lossy().into_owned();
            cmd.arg("--ro-bind").arg(path_str.clone()).arg(path_str);
        }
        for path in self.read_write_paths() {
            let path_str = path.to_string_lossy().into_owned();
            cmd.arg("--bind").arg(path_str.clone()).arg(path_str);
        }
        cmd.arg("--tmpfs").arg("/tmp"); // isolated /tmp
        Ok(cmd)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn command(&self, command: &str) -> anyhow::Result<Command> {
        let mut cmd = self.bwrap_command()?;
        cmd.args([
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

    // Unlike macOS's sbpl_profile, this doesn't deny loopback: bwrap has no
    // per-destination network filter short of a real network namespace plus
    // nftables/a proxy, which is out of scope here. Shares the host's
    // network namespace outright (no --unshare-net), so on Linux this is
    // unrestricted egress including to other local services.
    #[cfg(target_os = "linux")]
    pub(crate) fn networked_command(&self, argv: &[String]) -> anyhow::Result<Command> {
        let mut cmd = self.bwrap_command()?;
        cmd.arg("--die-with-parent").arg("--").args(argv);
        self.apply_scratch_env(&mut cmd);
        cmd.process_group(0);
        Ok(cmd)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn command(&self, _command: &str) -> anyhow::Result<Command> {
        anyhow::bail!("session_bash is not supported on this platform")
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn networked_command(&self, _argv: &[String]) -> anyhow::Result<Command> {
        anyhow::bail!("session_package_install is not supported on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    // The security-critical property of this module: session_bash's sandbox
    // must never reach the network — including loopback — while
    // networked_command's must deny loopback specifically but not the wider
    // network. Runs against a real local listener rather than asserting on
    // profile text, so it fails if sandbox-exec's actual enforcement ever
    // drifts from what the generated SBPL claims.
    #[test]
    #[cfg(target_os = "macos")]
    fn plain_command_and_networked_command_both_deny_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // A denied connect() attempt never reaches this listener, so it
        // stays available to accept the one connection a false positive
        // would make.
        std::thread::spawn(move || {
            let _ = listener.accept();
            let _ = listener.accept();
        });

        let workspace = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(workspace.path(), scratch.path(), vec![]);
        let probe = format!("nc -z -w2 127.0.0.1 {port}");

        let denied = sandbox.command(&probe).unwrap().status().unwrap();
        assert!(
            !denied.success(),
            "session_bash's sandbox must not reach the network"
        );

        let denied = sandbox
            .networked_command(&[
                "nc".to_string(),
                "-z".to_string(),
                "-w2".to_string(),
                "127.0.0.1".to_string(),
                port.to_string(),
            ])
            .unwrap()
            .status()
            .unwrap();
        assert!(
            !denied.success(),
            "networked_command's sandbox must not reach loopback"
        );
    }

    // Needs real outbound internet access, so it's excluded from the default
    // run — verifies the loopback-deny rule above doesn't also block the
    // wider network egress networked_command exists for.
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn networked_command_still_reaches_the_real_internet() {
        let workspace = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(workspace.path(), scratch.path(), vec![]);

        let allowed = sandbox
            .networked_command(&[
                "nc".to_string(),
                "-z".to_string(),
                "-w5".to_string(),
                "1.1.1.1".to_string(),
                "443".to_string(),
            ])
            .unwrap()
            .status()
            .unwrap();
        assert!(
            allowed.success(),
            "networked_command's sandbox must still reach the real internet"
        );
    }
}

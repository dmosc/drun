//! Sandbox execution for shell commands. On macOS uses sandbox-exec with an
//! SBPL profile; on Linux uses bubblewrap (bwrap). Both strategies confine
//! the command to the session workspace with no network access, except
//! `networked_command`, which trades network confinement for filesystem
//! confinement to whatever directory it's pointed at (on macOS, loopback is
//! still denied — see `sbpl_profile`; on Linux there's no such carve-out).

use std::collections::BTreeSet;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub(crate) struct Sandbox {
    workspace_dir: PathBuf,
    scratch_dir: PathBuf,
    read_only_paths: Vec<PathBuf>,
}

impl Sandbox {
    pub(crate) fn new(
        workspace_dir: &Path,
        scratch_dir: &Path,
        read_only_paths: Vec<PathBuf>,
    ) -> Self {
        let canonicalize = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Self {
            workspace_dir: canonicalize(workspace_dir),
            scratch_dir: canonicalize(scratch_dir),
            read_only_paths,
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
                .map(|(key, subpath)| (*key, self.scratch_dir.join(subpath))),
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
    fn system_and_path_dirs() -> &'static [PathBuf] {
        static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
        DIRS.get_or_init(|| {
            let mut candidate_paths: Vec<PathBuf> =
                Self::SYSTEM_READ_PATHS.iter().map(PathBuf::from).collect();
            if let Ok(path_env_var) = std::env::var("PATH") {
                candidate_paths.extend(std::env::split_paths(&path_env_var));
            }
            candidate_paths
                .into_iter()
                .filter_map(|path| path.canonicalize().ok())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
    }

    pub(crate) fn resolve_read_only_paths(extra_read_only_paths: &[PathBuf]) -> Vec<PathBuf> {
        if extra_read_only_paths.is_empty() {
            return Self::system_and_path_dirs().to_vec();
        }
        extra_read_only_paths
            .iter()
            .filter_map(|path| path.canonicalize().ok())
            .chain(Self::system_and_path_dirs().iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn read_only_paths(&self) -> &[PathBuf] {
        &self.read_only_paths
    }

    // Paths the sandboxed process may both read and write.
    #[cfg(target_os = "macos")]
    fn read_write_paths(&self) -> Vec<PathBuf> {
        let canonicalize = |path: PathBuf| path.canonicalize().unwrap_or(path);
        vec![
            self.workspace_dir.clone(),
            self.scratch_dir.clone(),
            canonicalize(PathBuf::from("/tmp")),
        ]
    }

    #[cfg(target_os = "linux")]
    fn read_write_paths(&self) -> Vec<PathBuf> {
        vec![self.workspace_dir.clone(), self.scratch_dir.clone()]
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
            .map(|path| format!("    (subpath \"{}\")\n", path.display()))
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

    // Whether bwrap is on PATH never changes over the process's lifetime, so
    // only resolve it once instead of shelling out on every command.
    #[cfg(target_os = "linux")]
    fn bwrap_available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| which::which("bwrap").is_ok())
    }

    #[cfg(target_os = "linux")]
    fn bwrap_command(&self) -> anyhow::Result<Command> {
        if !Self::bwrap_available() {
            anyhow::bail!(
                "bwrap not found; install bubblewrap (e.g. `apt install bubblewrap`) \
                 to enable session_bash"
            );
        }
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
mod resolve_read_only_paths_tests {
    use super::*;

    #[test]
    fn resolve_read_only_paths_includes_a_canonicalized_extra_path() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = Sandbox::resolve_read_only_paths(&[dir.path().to_path_buf()]);
        assert!(resolved.contains(&dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn resolve_read_only_paths_dedupes_an_extra_path_already_on_path_or_system_dirs() {
        let before = Sandbox::resolve_read_only_paths(&[]).len();
        let with_duplicate =
            Sandbox::resolve_read_only_paths(&[PathBuf::from(Sandbox::SYSTEM_READ_PATHS[0])]);
        assert_eq!(with_duplicate.len(), before);
    }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
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
        let sandbox = Sandbox::new(
            workspace.path(),
            scratch.path(),
            Sandbox::resolve_read_only_paths(&[]),
        );
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
    fn networked_command_still_reaches_the_real_internet() {
        let workspace = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(
            workspace.path(),
            scratch.path(),
            Sandbox::resolve_read_only_paths(&[]),
        );

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

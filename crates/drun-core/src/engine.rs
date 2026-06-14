use std::path::PathBuf;
use std::process::Stdio;

pub const PYTHON_PACKAGE_HOSTS: &[&str] =
    &["cdn.jsdelivr.net", "files.pythonhosted.org", "pypi.org"];

pub struct DrunEngineConfig {
    pub max_workspace_bytes: Option<u64>,
    pub max_checkpoints: Option<usize>,
    pub mount_allowlist: Vec<PathBuf>,
}

impl Default for DrunEngineConfig {
    fn default() -> Self {
        Self {
            max_workspace_bytes: Some(512 * 1024 * 1024), // 512 MB
            max_checkpoints: Some(200),
            mount_allowlist: vec![],
        }
    }
}

#[derive(Clone)]
pub struct DrunEngine {
    pub(crate) deno_path: PathBuf,
    pub(crate) runner_path: PathBuf,
    pub max_workspace_bytes: Option<u64>,
    pub(crate) max_checkpoints: Option<usize>,
    pub(crate) mount_allowlist: Vec<PathBuf>,
}

impl DrunEngine {
    pub fn new(config: DrunEngineConfig) -> anyhow::Result<Self> {
        let deno_path = which::which("deno")
            .map_err(|_| anyhow::anyhow!("deno not found; install from https://deno.land"))?;
        // Use a per-process unique filename to avoid TOCTOU race conditions.
        let runner_path =
            std::env::temp_dir().join(format!("drun_runner_{}.ts", std::process::id()));
        std::fs::write(&runner_path, include_str!("assets/runner.ts"))?;
        Ok(Self {
            deno_path,
            runner_path,
            max_workspace_bytes: config.max_workspace_bytes,
            max_checkpoints: config.max_checkpoints,
            mount_allowlist: config.mount_allowlist,
        })
    }

    pub(crate) fn spawn_runner(
        &self,
        allowed_hosts: &[String],
    ) -> anyhow::Result<std::process::Child> {
        let mut args = vec!["run", "--allow-read", "--allow-write"];
        let net_flag: Option<String> = if allowed_hosts.iter().any(|h| h == "*") {
            Some("--allow-net".to_owned())
        } else if !allowed_hosts.is_empty() {
            Some(format!("--allow-net={}", allowed_hosts.join(",")))
        } else {
            None
        };
        if let Some(ref flag) = net_flag {
            args.push(flag);
        }
        let runner = self.runner_path.to_string_lossy().into_owned();
        args.push(&runner);
        Ok(std::process::Command::new(&self.deno_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?)
    }
}

impl Drop for DrunEngine {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.runner_path);
    }
}

//! DrunEngine: shared, cloneable handle to the Deno runtime. Writes the
//! bundled runner script to a temp file on construction and removes it on drop.

use crate::config::Config;
use std::path::PathBuf;
use std::process::Stdio;

#[derive(Clone)]
pub struct DrunEngine {
    pub(crate) deno_path: PathBuf,
    pub(crate) runner_path: PathBuf,
    pub config: Config,
}

impl DrunEngine {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let deno_path = which::which("deno")
            .map_err(|_| anyhow::anyhow!("deno not found; install from https://deno.land"))?;
        // Use a per-process unique filename to avoid TOCTOU race conditions.
        let runner_path =
            std::env::temp_dir().join(format!("drun_runner_{}.ts", std::process::id()));
        std::fs::write(&runner_path, include_str!("assets/runner.ts"))?;
        Ok(Self {
            deno_path,
            runner_path,
            config,
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

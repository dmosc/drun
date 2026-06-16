//! DrunEngine: shared, cloneable handle to the Deno runtime. Writes the
//! bundled runner script to a temp file on construction and removes it on drop.

use crate::config::Config;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

struct RunnerFile(PathBuf);

impl Drop for RunnerFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Clone)]
pub struct DrunEngine {
    pub(crate) deno_path: PathBuf,
    runner_file: Arc<RunnerFile>,
    pub config: Config,
}

impl DrunEngine {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let deno_path = which::which("deno")
            .map_err(|_| anyhow::anyhow!("deno not found; install from https://deno.land"))?;
        let runner_path =
            std::env::temp_dir().join(format!("drun_runner_{}.ts", std::process::id()));
        std::fs::write(&runner_path, include_str!("assets/runner.ts"))?;
        Ok(Self {
            deno_path,
            runner_file: Arc::new(RunnerFile(runner_path)),
            config,
        })
    }

    pub(crate) fn spawn_runner(
        &self,
        allowed_hosts: &[String],
    ) -> anyhow::Result<std::process::Child> {
        let runner = self.runner_file.0.to_string_lossy().into_owned();
        let deno_cache = self.find_deno_cache_dir().to_string_lossy().into_owned();
        let allow_read = format!("--allow-read={},{}", runner, deno_cache);
        let allow_write = format!("--allow-write={}", deno_cache);

        let mut args = vec!["run", &allow_read, &allow_write];
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
        args.push(&runner);
        Ok(std::process::Command::new(&self.deno_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?)
    }

    fn find_deno_cache_dir(&self) -> PathBuf {
        if let Ok(dir) = std::env::var("DENO_DIR") {
            return PathBuf::from(dir);
        }
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        if cfg!(target_os = "macos") {
            home.join("Library/Caches/deno")
        } else {
            std::env::var("XDG_CACHE_HOME")
                .map(|xdg| PathBuf::from(xdg).join("deno"))
                .unwrap_or_else(|_| home.join(".cache/deno"))
        }
    }
}

//! Engine initialization: locates the Deno binary, writes the runner script,
//! and spawns sandboxed subprocesses.

use crate::NetworkPolicy;
use std::process::Stdio;

#[derive(Clone)]
pub struct DrunEngine {
    pub(crate) deno_path: std::path::PathBuf,
    pub(crate) runner_path: std::path::PathBuf,
}

impl DrunEngine {
    pub fn new() -> anyhow::Result<Self> {
        let deno_path = which::which("deno")
            .map_err(|_| anyhow::anyhow!("deno not found; install from https://deno.land"))?;
        let runner_path = std::env::temp_dir().join("drun_runner.ts");
        std::fs::write(&runner_path, include_str!("assets/runner.ts"))?;
        Ok(Self {
            deno_path,
            runner_path,
        })
    }

    pub(crate) fn spawn_runner(
        &self,
        network: NetworkPolicy,
    ) -> anyhow::Result<std::process::Child> {
        let mut args = vec!["run", "--allow-read", "--allow-write"];
        let net;
        match network {
            NetworkPolicy::Packages => {
                net = "--allow-net=cdn.jsdelivr.net,files.pythonhosted.org,pypi.org";
                args.push(net);
            }
            NetworkPolicy::Full => args.push("--allow-net"),
            NetworkPolicy::None => {}
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

//! Engine initialization: locates the Deno binary, writes the runner script,
//! and spawns sandboxed subprocesses.

use crate::{NetworkPolicy, Session};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;

pub struct DrunEngine {
    pub(crate) deno_path: std::path::PathBuf,
    pub(crate) runner_path: std::path::PathBuf,
}

pub struct DrunOutput {
    pub stdout: String,
    pub files: HashMap<String, Vec<u8>>,
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

    pub fn run_python(&self, code: &str, mounts: Vec<String>) -> anyhow::Result<DrunOutput> {
        let files = self.read_mounts(mounts)?;
        let mut session = Session::with_files(self, files, NetworkPolicy::Packages)?;
        let checkpoint = session.execute(code)?;
        Ok(DrunOutput {
            stdout: checkpoint.stdout.clone(),
            files: checkpoint.files.clone(),
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

    fn read_mounts(&self, mounts: Vec<String>) -> anyhow::Result<HashMap<String, Vec<u8>>> {
        let mut files = HashMap::new();
        for source in mounts {
            let source_path = Path::new(&source);
            if !source_path.exists() {
                anyhow::bail!("mount source does not exist: {}", source);
            }
            if source_path.is_dir() {
                for entry in walkdir::WalkDir::new(source_path) {
                    let entry = entry?;
                    if entry.file_type().is_file() {
                        files.insert(
                            entry.path().to_string_lossy().into_owned(),
                            std::fs::read(entry.path())?,
                        );
                    }
                }
            } else {
                files.insert(source.clone(), std::fs::read(source_path)?);
            }
        }
        Ok(files)
    }
}

mod session;

pub use session::{Checkpoint, Session};

use fs_extra::dir::CopyOptions;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

pub struct DrunEngine {
    deno_path: std::path::PathBuf,
    runner_path: std::path::PathBuf,
}

pub struct DrunOutput {
    pub stdout: String,
    pub files: HashMap<String, Vec<u8>>,
}

#[derive(Deserialize)]
struct RunnerOutput {
    stdout: String,
    files: HashMap<String, Vec<u8>>,
}

impl DrunEngine {
    pub fn new() -> anyhow::Result<Self> {
        let deno_path = which::which("deno")
            .ok()
            .ok_or_else(|| anyhow::anyhow!("deno not found; install from https://deno.land"))?;
        let runner_path = std::env::temp_dir().join("drun_runner.ts");
        std::fs::write(&runner_path, include_str!("assets/runner.ts"))?;
        Ok(Self {
            deno_path,
            runner_path,
        })
    }

    pub fn run_python(&self, code: &str, mounts: Vec<String>) -> anyhow::Result<DrunOutput> {
        let workspace = tempfile::tempdir()?;
        self.mount_assets(&workspace, mounts)?;
        self.exec(code, workspace)
    }

    pub fn run_python_from_files(
        &self,
        code: &str,
        files: HashMap<String, Vec<u8>>,
    ) -> anyhow::Result<DrunOutput> {
        let workspace = tempfile::tempdir()?;
        for (path, bytes) in &files {
            let dest = workspace.path().join(path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, bytes)?;
        }
        self.exec(code, workspace)
    }

    fn exec(&self, code: &str, workspace: tempfile::TempDir) -> anyhow::Result<DrunOutput> {
        let mut code_file = tempfile::NamedTempFile::new()?;
        code_file.write_all(code.as_bytes())?;

        let output = std::process::Command::new(&self.deno_path)
            .args([
                "run",
                "--allow-read",
                "--allow-write",
                "--allow-net",
                &self.runner_path.to_string_lossy(),
                &workspace.path().to_string_lossy(),
                &code_file.path().to_string_lossy(),
            ])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr));
        }

        let result: RunnerOutput = serde_json::from_slice(&output.stdout)?;
        Ok(DrunOutput {
            stdout: result.stdout,
            files: result.files,
        })
    }

    fn mount_assets(
        &self,
        workspace: &tempfile::TempDir,
        mounts: Vec<String>,
    ) -> anyhow::Result<()> {
        let target_base = workspace.path();
        for source in mounts {
            let source_path = Path::new(&source);
            if !source_path.exists() {
                anyhow::bail!("Mount source does not exist: {}", source);
            }

            let destination = target_base.join(&source);
            if source_path.is_dir() {
                std::fs::create_dir_all(&destination)?;
                let options = CopyOptions::new().overwrite(true).content_only(true);
                fs_extra::dir::copy(source_path, &destination, &options)?;
            } else {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(source_path, &destination)?;
            }
        }

        Ok(())
    }
}

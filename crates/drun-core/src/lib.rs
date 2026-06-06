use fs_extra::dir::CopyOptions;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;

pub struct DrunEngine {
    runner_path: std::path::PathBuf,
}

pub struct DrunOutput {
    pub stdout: String,
    pub files: std::collections::HashMap<String, Vec<u8>>,
}

#[derive(Deserialize)]
struct RunnerOutput {
    stdout: String,
    files: std::collections::HashMap<String, Vec<u8>>,
}

impl DrunEngine {
    pub fn new() -> anyhow::Result<Self> {
        let runner_path = std::env::temp_dir().join("drun_runner.ts");
        std::fs::write(&runner_path, include_str!("assets/runner.ts"))?;
        Ok(Self { runner_path })
    }

    pub fn run_python(&self, code: &str, mounts: Vec<String>) -> anyhow::Result<DrunOutput> {
        let workspace = tempfile::tempdir()?;
        self.mount_assets(&workspace, mounts)?;

        let mut code_file = tempfile::NamedTempFile::new()?;
        code_file.write_all(code.as_bytes())?;

        let workspace_str = workspace.path().to_string_lossy().into_owned();
        let code_str = code_file.path().to_string_lossy().into_owned();
        let runner_str = self.runner_path.to_string_lossy().into_owned();

        let output = std::process::Command::new("deno")
            .args([
                "run",
                "--allow-read",
                "--allow-write",
                "--allow-net",
                &runner_str,
                &workspace_str,
                &code_str,
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

use crate::config::Config;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::Arc;

struct RunnerFile(PathBuf);

impl Drop for RunnerFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Clone)]
pub struct DrunEngine {
    python_path: PathBuf,
    runner_file: Arc<RunnerFile>,
    pub packages_dir: PathBuf,
    pub config: Config,
}

impl DrunEngine {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let python_path = which::which("python3")
            .map_err(|_| anyhow::anyhow!("python not found; install Python 3 to use drun"))?;
        let runner_path =
            std::env::temp_dir().join(format!("drun_runner_{}.py", std::process::id()));
        std::fs::write(&runner_path, include_str!("assets/runner.py"))?;
        let packages_dir = config
            .packages_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("drun-packages"));
        std::fs::create_dir_all(&packages_dir)?;
        Ok(Self {
            python_path,
            runner_file: Arc::new(RunnerFile(runner_path)),
            packages_dir,
            config,
        })
    }

    pub(crate) fn spawn_python_runner(&self) -> anyhow::Result<Child> {
        Ok(std::process::Command::new(&self.python_path)
            .arg(&self.runner_file.0)
            .arg(&self.packages_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?)
    }
}

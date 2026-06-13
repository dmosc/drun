use crate::{DrunEngine, FileMap};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const INSTALL_TIMEOUT_MS: u64 = 120_000;

#[derive(Serialize)]
struct ExecRequest<'a> {
    code: &'a str,
    files: &'a FileMap,
}

#[derive(Serialize)]
struct PackageInstallRequest<'a> {
    package: &'a str,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RunnerResponse {
    Ok {
        stdout: String,
        stderr: String,
        files: FileMap,
    },
    Err {
        error: String,
    },
}

#[derive(Deserialize)]
struct ProgressLine {
    progress: String,
}

pub(crate) struct ExecSuccess {
    pub stdout: String,
    pub stderr: String,
    pub files: FileMap,
}

/// Result of a Python execution. `Err` means the child process died and the
/// runner must be respawned. `Ok(Err(msg))` means Python raised an exception
/// but the child is still alive.
pub(crate) type ExecResult = anyhow::Result<Result<ExecSuccess, String>>;

pub(crate) struct Runner {
    child: Arc<Mutex<Child>>,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Runner {
    pub fn new(engine: &DrunEngine, allowed_hosts: &[String]) -> anyhow::Result<Self> {
        let mut child = engine.spawn_runner(allowed_hosts)?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
        })
    }

    /// Builds a fresh runner and silently re-installs packages into it.
    /// Used when a prior runner's child died after a timeout.
    pub fn new_from_timeout_recovery(
        engine: &DrunEngine,
        allowed_hosts: &[String],
        packages: &[String],
    ) -> anyhow::Result<Self> {
        let mut runner = Self::new(engine, allowed_hosts)?;
        for package in packages {
            let _ = runner.install(package);
        }
        Ok(runner)
    }

    pub fn execute(
        &mut self,
        code: &str,
        files: &FileMap,
        timeout_ms: u64,
        on_progress: &mut dyn FnMut(String),
    ) -> ExecResult {
        let request = serde_json::to_string(&ExecRequest { code, files })?;
        writeln!(self.stdin, "{}", request)?;
        self.stdin.flush()?;

        let child_handle = Arc::clone(&self.child);
        let timeout = Duration::from_millis(timeout_ms);
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if cancel_rx.recv_timeout(timeout).is_err() {
                let _ = child_handle.lock().unwrap().kill();
            }
        });

        loop {
            let mut line = String::new();
            match self.stdout.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = cancel_tx.send(());
                    anyhow::bail!("execution timed out after {}ms", timeout_ms);
                }
                Ok(_) => {
                    if let Ok(p) = serde_json::from_str::<ProgressLine>(line.trim()) {
                        on_progress(p.progress);
                        continue;
                    }
                    let _ = cancel_tx.send(());
                    return match serde_json::from_str::<RunnerResponse>(&line)? {
                        RunnerResponse::Ok {
                            stdout,
                            stderr,
                            files,
                        } => Ok(Ok(ExecSuccess {
                            stdout,
                            stderr,
                            files,
                        })),
                        RunnerResponse::Err { error } => Ok(Err(error)),
                    };
                }
            }
        }
    }

    pub fn install(&mut self, package: &str) -> anyhow::Result<()> {
        let request = serde_json::to_string(&PackageInstallRequest { package })?;
        writeln!(self.stdin, "{}", request)?;
        self.stdin.flush()?;

        let mut response_line = String::new();
        let child_handle = Arc::clone(&self.child);
        let timeout = Duration::from_millis(INSTALL_TIMEOUT_MS);
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if cancel_rx.recv_timeout(timeout).is_err() {
                let _ = child_handle.lock().unwrap().kill();
            }
        });
        let read_result = self.stdout.read_line(&mut response_line);
        let _ = cancel_tx.send(());

        match read_result {
            Ok(0) | Err(_) => {
                anyhow::bail!("package install timed out after {}ms", INSTALL_TIMEOUT_MS)
            }
            Ok(_) => {}
        }

        match serde_json::from_str::<RunnerResponse>(&response_line)? {
            RunnerResponse::Ok { .. } => Ok(()),
            RunnerResponse::Err { error } => anyhow::bail!(error),
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

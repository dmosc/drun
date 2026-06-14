use crate::error::RunnerError;
use crate::{DrunEngine, FileMap};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

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

pub(crate) struct Runner {
    child: Arc<Mutex<Child>>,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Runner {
    pub fn new(engine: &DrunEngine) -> anyhow::Result<Self> {
        let mut child = engine.spawn_runner(&engine.config.domain_allowlist)?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
        })
    }

    pub fn new_from_timeout_recovery(
        engine: &DrunEngine,
        packages: &[String],
    ) -> anyhow::Result<Self> {
        let mut runner = Self::new(engine)?;
        for package in packages {
            let _ = runner.install(package, engine.config.install_timeout_ms);
        }
        Ok(runner)
    }

    pub fn execute(
        &mut self,
        code: &str,
        files: &FileMap,
        timeout_ms: u64,
        on_progress: &mut dyn FnMut(String),
    ) -> anyhow::Result<ExecSuccess> {
        let request = serde_json::to_string(&ExecRequest { code, files })?;
        writeln!(self.stdin, "{}", request)?;
        self.stdin.flush()?;

        let timed_out = Arc::new(AtomicBool::new(false));
        let child_handle = Arc::clone(&self.child);
        let timed_out_flag = Arc::clone(&timed_out);
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if cancel_rx
                .recv_timeout(Duration::from_millis(timeout_ms))
                .is_err()
            {
                timed_out_flag.store(true, Ordering::Relaxed);
                let _ = child_handle.lock().unwrap().kill();
            }
        });

        loop {
            let mut line = String::new();
            match self.stdout.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = cancel_tx.send(());
                    return Err(self.classify_eof(timed_out.load(Ordering::Relaxed), timeout_ms));
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
                        } => Ok(ExecSuccess {
                            stdout,
                            stderr,
                            files,
                        }),
                        RunnerResponse::Err { error } => {
                            Err(anyhow::Error::from(RunnerError::Application(error)))
                        }
                    };
                }
            }
        }
    }

    pub fn install(&mut self, package: &str, timeout_ms: u64) -> anyhow::Result<()> {
        let request = serde_json::to_string(&PackageInstallRequest { package })?;
        writeln!(self.stdin, "{}", request)?;
        self.stdin.flush()?;

        let timed_out = Arc::new(AtomicBool::new(false));
        let child_handle = Arc::clone(&self.child);
        let timed_out_flag = Arc::clone(&timed_out);
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if cancel_rx
                .recv_timeout(Duration::from_millis(timeout_ms))
                .is_err()
            {
                timed_out_flag.store(true, Ordering::Relaxed);
                let _ = child_handle.lock().unwrap().kill();
            }
        });

        loop {
            let mut line = String::new();
            match self.stdout.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = cancel_tx.send(());
                    return Err(self.classify_eof(timed_out.load(Ordering::Relaxed), timeout_ms));
                }
                Ok(_) => {
                    if serde_json::from_str::<ProgressLine>(line.trim()).is_ok() {
                        continue;
                    }
                    let _ = cancel_tx.send(());
                    return match serde_json::from_str::<RunnerResponse>(&line)? {
                        RunnerResponse::Ok { .. } => Ok(()),
                        RunnerResponse::Err { error } => anyhow::bail!(error),
                    };
                }
            }
        }
    }

    fn classify_eof(&mut self, timed_out: bool, timeout_ms: u64) -> anyhow::Error {
        if timed_out {
            anyhow::Error::from(RunnerError::Timeout { timeout_ms })
        } else {
            let exit_code = self
                .child
                .lock()
                .unwrap()
                .try_wait()
                .ok()
                .flatten()
                .and_then(|s| s.code());
            anyhow::Error::from(RunnerError::Crash { exit_code })
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

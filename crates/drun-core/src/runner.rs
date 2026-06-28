use crate::error::RunnerError;
use crate::{DrunEngine, FileMap};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
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
    overlays: &'a HashMap<String, PathBuf>,
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

#[derive(Deserialize)]
struct ReadyLine {
    ready: bool,
}

pub(crate) struct ExecSuccess {
    pub stdout: String,
    pub stderr: String,
    pub files: FileMap,
}

pub(crate) struct Runner {
    engine: DrunEngine,
    child: Arc<Mutex<Child>>,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Runner {
    pub fn new(engine: &DrunEngine) -> anyhow::Result<Self> {
        let mut child = engine.spawn_python_runner()?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let child = Arc::new(Mutex::new(child));
        let child_for_timeout = Arc::clone(&child);
        let timed_out = Arc::new(AtomicBool::new(false));
        let timed_out_flag = Arc::clone(&timed_out);
        let install_timeout_ms = engine.config.install_timeout_ms;
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();

        std::thread::spawn(move || {
            if cancel_rx
                .recv_timeout(Duration::from_millis(install_timeout_ms))
                .is_err()
            {
                timed_out_flag.store(true, Ordering::Relaxed);
                let _ = child_for_timeout.lock().unwrap().kill();
            }
        });

        let mut line = String::new();
        let ready = match stdout.read_line(&mut line) {
            Ok(n) if n > 0 => {
                serde_json::from_str::<ReadyLine>(line.trim()).map_or(false, |r| r.ready)
            }
            _ => false,
        };
        let _ = cancel_tx.send(());

        if !ready {
            let exit_code = child
                .lock()
                .unwrap()
                .try_wait()
                .ok()
                .flatten()
                .and_then(|s| s.code());
            if timed_out.load(Ordering::Relaxed) {
                anyhow::bail!(
                    "Python runner startup timed out after {}ms",
                    engine.config.install_timeout_ms
                );
            }
            anyhow::bail!("Python runner exited during startup (exit code: {exit_code:?})");
        }

        Ok(Self {
            engine: engine.clone(),
            child,
            stdin,
            stdout,
        })
    }

    pub fn child_arc(&self) -> Arc<Mutex<Child>> {
        Arc::clone(&self.child)
    }

    pub fn execute_python(
        &mut self,
        code: &str,
        files: &FileMap,
        overlays: &HashMap<String, PathBuf>,
        on_progress: &mut dyn FnMut(String),
    ) -> anyhow::Result<ExecSuccess> {
        writeln!(
            self.stdin,
            "{}",
            serde_json::to_string(&ExecRequest {
                code,
                files,
                overlays
            })?
        )?;
        self.stdin.flush()?;
        match self.await_response(self.engine.config.exec_timeout_ms, on_progress)? {
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
        }
    }

    pub fn install(&mut self, package: &str) -> anyhow::Result<()> {
        writeln!(
            self.stdin,
            "{}",
            serde_json::to_string(&PackageInstallRequest { package })?
        )?;
        self.stdin.flush()?;
        match self.await_response(self.engine.config.install_timeout_ms, &mut |_| {})? {
            RunnerResponse::Ok { .. } => Ok(()),
            RunnerResponse::Err { error } => anyhow::bail!(error),
        }
    }

    fn await_response(
        &mut self,
        timeout_ms: u64,
        on_progress: &mut dyn FnMut(String),
    ) -> anyhow::Result<RunnerResponse> {
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
                    return serde_json::from_str::<RunnerResponse>(&line)
                        .map_err(anyhow::Error::from);
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

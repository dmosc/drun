use crate::{Checkpoint, CheckpointRef, DrunEngine, NetworkPolicy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Runner {
    child: Arc<Mutex<Child>>,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Runner {
    fn new(engine: &DrunEngine, network: NetworkPolicy) -> anyhow::Result<Self> {
        let mut child = engine.spawn_runner(network)?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
        })
    }
}

pub struct Session {
    runner: Runner,
    engine: DrunEngine,
    checkpoints: Vec<Checkpoint>,
    checkpoint_idx: usize,
    origins: HashMap<String, PathBuf>,
    packages: Vec<String>,
    network: NetworkPolicy,
    pub timeout_ms: u64,
    pub parent: Option<CheckpointRef>,
}

#[derive(Serialize)]
struct ExecRequest<'a> {
    code: &'a str,
    files: &'a HashMap<String, Vec<u8>>,
}

#[derive(Serialize)]
struct InstallRequest<'a> {
    package: &'a str,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RunnerResponse {
    Ok {
        stdout: String,
        stderr: String,
        files: HashMap<String, Vec<u8>>,
    },
    Err {
        error: String,
    },
}

impl Session {
    pub fn new(
        engine: &DrunEngine,
        network: NetworkPolicy,
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<Self> {
        let runner = Runner::new(engine, network)?;
        Ok(Self {
            runner,
            engine: engine.clone(),
            checkpoints: vec![Self::empty_checkpoint(0, HashMap::new())],
            checkpoint_idx: 0,
            origins: HashMap::new(),
            packages: Vec::new(),
            network,
            timeout_ms: timeout_ms.unwrap_or(10_000),
            parent: None,
        })
    }

    pub fn from_session(
        engine: &DrunEngine,
        source_session_id: &str,
        source: &Session,
        checkpoint_id: Option<usize>,
    ) -> anyhow::Result<Self> {
        let checkpoint_idx = checkpoint_id.unwrap_or(source.checkpoint_idx);
        if checkpoint_idx >= source.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", checkpoint_idx);
        }
        let files = source.checkpoints[checkpoint_idx].files.clone();
        let mut session = Self::new(engine, source.network, Some(source.timeout_ms))?;
        for package in &source.packages {
            session.install(package)?;
        }
        session.checkpoints[0].files = files;
        session.parent = Some(CheckpointRef {
            session_id: source_session_id.to_string(),
            checkpoint_id: checkpoint_idx,
        });
        Ok(session)
    }

    fn restart(&mut self) -> anyhow::Result<()> {
        let _ = self.runner.child.lock().unwrap().kill();
        self.runner = Runner::new(&self.engine, self.network)?;
        let packages = std::mem::take(&mut self.packages);
        for package in &packages {
            let request = serde_json::to_string(&InstallRequest { package })?;
            writeln!(self.runner.stdin, "{}", request)?;
            self.runner.stdin.flush()?;
            let mut line = String::new();
            let _ = self.runner.stdout.read_line(&mut line);
        }
        self.packages = packages;
        Ok(())
    }

    pub fn mount(&mut self, path: &Path) -> anyhow::Result<Vec<String>> {
        let abs = path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("path does not exist: {}", path.display()))?;

        let mut entries: Vec<(String, Vec<u8>, PathBuf)> = Vec::new();
        if abs.is_dir() {
            for entry in walkdir::WalkDir::new(&abs) {
                let entry = entry?;
                if entry.file_type().is_file() {
                    let key = entry
                        .path()
                        .strip_prefix(&abs)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    entries.push((
                        key,
                        std::fs::read(entry.path())?,
                        entry.path().to_path_buf(),
                    ));
                }
            }
        } else {
            let key = abs
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("path has no filename: {}", abs.display()))?
                .to_string_lossy()
                .into_owned();
            entries.push((key, std::fs::read(&abs)?, abs.clone()));
        }

        let keys: Vec<String> = entries.iter().map(|(k, _, _)| k.clone()).collect();
        let checkpoint = &mut self.checkpoints[self.checkpoint_idx];
        for (key, bytes, host_path) in entries {
            checkpoint.files.insert(key.clone(), bytes);
            self.origins.insert(key, host_path);
        }
        Ok(keys)
    }

    pub fn write_file(&mut self, path: &str, content: Vec<u8>) -> &Checkpoint {
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        files.insert(path.to_string(), content);
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let checkpoint_idx = self.checkpoints.len();
        self.checkpoints
            .push(Self::empty_checkpoint(checkpoint_idx, files));
        self.checkpoint_idx = checkpoint_idx;
        self.checkpoints.last().unwrap()
    }

    pub fn delete_file(&mut self, path: &str) -> anyhow::Result<&Checkpoint> {
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        if files.remove(path).is_none() {
            anyhow::bail!("'{}' not in current checkpoint", path);
        }
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let id = self.checkpoints.len();
        self.checkpoints.push(Self::empty_checkpoint(id, files));
        self.checkpoint_idx = id;
        Ok(self.checkpoints.last().unwrap())
    }

    pub fn install(&mut self, package: &str) -> anyhow::Result<()> {
        let request = serde_json::to_string(&InstallRequest { package })?;
        writeln!(self.runner.stdin, "{}", request)?;
        self.runner.stdin.flush()?;
        let mut line = String::new();
        self.runner.stdout.read_line(&mut line)?;
        match serde_json::from_str::<RunnerResponse>(&line)? {
            RunnerResponse::Ok { .. } => {
                self.packages.push(package.to_string());
                Ok(())
            }
            RunnerResponse::Err { error } => anyhow::bail!(error),
        }
    }

    pub fn execute(&mut self, code: &str) -> anyhow::Result<&Checkpoint> {
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let files = &self.checkpoints[self.checkpoint_idx].files;
        let request = serde_json::to_string(&ExecRequest { code, files })?;
        writeln!(self.runner.stdin, "{}", request)?;
        self.runner.stdin.flush()?;

        let mut line = String::new();

        let child = Arc::clone(&self.runner.child);
        let duration = Duration::from_millis(self.timeout_ms);
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if cancel_rx.recv_timeout(duration).is_err() {
                let _ = child.lock().unwrap().kill();
            }
        });
        let read_result = self.runner.stdout.read_line(&mut line);
        let _ = cancel_tx.send(());

        match read_result {
            Ok(0) | Err(_) => {
                self.restart()?;
                anyhow::bail!("execution timed out after {}ms", self.timeout_ms);
            }
            Ok(_) => {}
        }

        match serde_json::from_str::<RunnerResponse>(&line)? {
            RunnerResponse::Ok {
                stdout,
                stderr,
                files,
            } => {
                let id = self.checkpoints.len();
                self.checkpoints.push(Checkpoint {
                    id,
                    stdout,
                    stderr,
                    files,
                });
                self.checkpoint_idx = id;
                Ok(self.checkpoints.last().unwrap())
            }
            RunnerResponse::Err { error } => anyhow::bail!(error),
        }
    }

    pub fn rollback(&mut self, checkpoint_idx: usize) -> anyhow::Result<()> {
        if checkpoint_idx >= self.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", checkpoint_idx);
        }
        self.checkpoint_idx = checkpoint_idx;
        Ok(())
    }

    pub fn export(
        &self,
        output_dir: &Path,
        keys: Option<Vec<String>>,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let current = &self.current().files;
        let keys_to_export: Vec<&String> = match &keys {
            Some(ks) => ks.iter().collect(),
            None => current
                .keys()
                .filter(|k| !self.origins.contains_key(*k))
                .collect(),
        };
        let mut exported_files = Vec::new();
        for key in keys_to_export {
            let output_bytes = current
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("'{}' not in current checkpoint", key))?;
            let target_path = output_dir.join(key);
            if let Some(target_path_parent) = target_path.parent() {
                std::fs::create_dir_all(target_path_parent)?;
            }
            std::fs::write(&target_path, output_bytes)?;
            exported_files.push(target_path);
        }
        Ok(exported_files)
    }

    pub fn commit(&self, keys: Option<Vec<String>>) -> anyhow::Result<Vec<PathBuf>> {
        let mounted = &self.checkpoints[0].files;
        let current = &self.current().files;
        let keys_to_commit: Vec<&String> = match &keys {
            Some(ks) => ks.iter().collect(),
            None => self.origins.keys().collect(),
        };
        let mut committed = Vec::new();
        for key in keys_to_commit {
            let host_path = self
                .origins
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("'{}' was not mounted from host", key))?;
            let current_bytes = current
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("'{}' not in current checkpoint", key))?;
            let mounted_bytes = mounted.get(key).map(Vec::as_slice).unwrap_or(&[]);
            if current_bytes.as_slice() == mounted_bytes {
                continue;
            }
            std::fs::write(host_path, current_bytes)?;
            committed.push(host_path.clone());
        }
        Ok(committed)
    }

    pub fn diff(&self, from_id: usize, to_id: usize) -> anyhow::Result<String> {
        if from_id >= self.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", from_id);
        }
        if to_id >= self.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", to_id);
        }
        let from = &self.checkpoints[from_id].files;
        let to = &self.checkpoints[to_id].files;
        let keys: std::collections::BTreeSet<&String> = from.keys().chain(to.keys()).collect();
        let mut output = String::new();
        for key in keys {
            let from_bytes = from.get(key).map(Vec::as_slice).unwrap_or(&[]);
            let to_bytes = to.get(key).map(Vec::as_slice).unwrap_or(&[]);
            if from_bytes == to_bytes {
                continue;
            }
            match (
                std::str::from_utf8(from_bytes),
                std::str::from_utf8(to_bytes),
            ) {
                (Ok(a), Ok(b)) => output.push_str(
                    &similar::TextDiff::from_lines(a, b)
                        .unified_diff()
                        .header(&format!("a/{key}"), &format!("b/{key}"))
                        .to_string(),
                ),
                _ => output.push_str(&format!("Binary file {key} changed.\n")),
            }
        }
        Ok(output)
    }

    pub fn packages(&self) -> &[String] {
        &self.packages
    }

    pub fn current(&self) -> &Checkpoint {
        &self.checkpoints[self.checkpoint_idx]
    }

    pub fn history(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    fn empty_checkpoint(id: usize, files: HashMap<String, Vec<u8>>) -> Checkpoint {
        Checkpoint {
            id,
            stdout: String::new(),
            stderr: String::new(),
            files,
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.runner.child.lock().unwrap().kill();
    }
}

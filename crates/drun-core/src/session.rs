use crate::error::RunnerError;
use crate::runner::{ExecSuccess, Runner};
use crate::snapshot::{CheckpointSnapshot, SessionSnapshot};
use crate::{Checkpoint, CheckpointRef, DrunEngine, FileMap};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Session {
    runner: Runner,
    engine: DrunEngine,
    checkpoints: Vec<Checkpoint>,
    checkpoint_idx: usize,
    origins: HashMap<String, PathBuf>,
    packages: Vec<String>,
    allowed_hosts: Vec<String>,
    pub label: Option<String>,
    pub timeout_ms: u64,
    pub parent: Option<CheckpointRef>,
    pub created_at: Instant,
    pub last_activity: Instant,
}

impl Session {
    pub fn new(
        engine: &DrunEngine,
        allowed_hosts: Vec<String>,
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            runner: Runner::new(engine, &allowed_hosts)?,
            engine: engine.clone(),
            checkpoints: vec![Self::empty_checkpoint(0, HashMap::new())],
            checkpoint_idx: 0,
            origins: HashMap::new(),
            packages: Vec::new(),
            allowed_hosts,
            label: None,
            timeout_ms: timeout_ms.unwrap_or(60_000),
            parent: None,
            created_at: Instant::now(),
            last_activity: Instant::now(),
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
        let origins: HashMap<String, PathBuf> = source
            .origins
            .iter()
            .filter(|(k, _)| files.contains_key(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut session = Self::new(
            engine,
            source.allowed_hosts.clone(),
            Some(source.timeout_ms),
        )?;
        for package in &source.packages {
            session.install(package)?;
        }
        session.checkpoints[0].files = files;
        session.origins = origins;
        session.parent = Some(CheckpointRef {
            session_id: source_session_id.to_string(),
            checkpoint_id: checkpoint_idx,
        });
        Ok(session)
    }

    pub fn mount(&mut self, path: &Path) -> anyhow::Result<Vec<String>> {
        let abs = path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("path does not exist: {}", path.display()))?;
        if !self.engine.mount_allowlist.is_empty() {
            let allowed = self
                .engine
                .mount_allowlist
                .iter()
                .any(|prefix| abs.starts_with(prefix));
            if !allowed {
                anyhow::bail!(
                    "'{}' is not in the mount allowlist; permitted prefixes: {}",
                    abs.display(),
                    self.engine
                        .mount_allowlist
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        let entries = Self::read_host_entries(&abs)?;
        let keys: Vec<String> = entries.iter().map(|(key, _, _)| key.clone()).collect();
        let checkpoint = &mut self.checkpoints[self.checkpoint_idx];
        for (key, bytes, host_path) in entries {
            checkpoint.files.insert(key.clone(), bytes);
            self.origins.insert(key, host_path);
        }
        Ok(keys)
    }

    pub fn write_file(&mut self, path: &str, content: Vec<u8>) -> anyhow::Result<()> {
        self.validate_file_path(path)?;
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        files.insert(path.to_string(), content);
        self.check_workspace_size(&files)?;
        self.push_files_as_checkpoint(files)?;
        Ok(())
    }

    pub fn delete_file(&mut self, path: &str) -> anyhow::Result<&Checkpoint> {
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        if files.remove(path).is_none() {
            anyhow::bail!("'{}' not in current checkpoint", path);
        }
        self.push_files_as_checkpoint(files)
    }

    pub fn install(&mut self, package: &str) -> anyhow::Result<()> {
        let result = self.runner.install(package);
        if result.is_err() {
            self.runner = Runner::new_from_timeout_recovery(
                &self.engine,
                &self.allowed_hosts,
                &self.packages,
            )?;
        }
        result?;
        self.packages.push(package.to_string());
        Ok(())
    }

    pub fn execute(
        &mut self,
        code: &str,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<&Checkpoint> {
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let files = &self.checkpoints[self.checkpoint_idx].files;
        match self.runner.execute(code, files, self.timeout_ms, on_stdout) {
            Ok(ExecSuccess {
                stdout,
                stderr,
                files,
            }) => {
                self.check_workspace_size(&files)?;
                self.check_checkpoint_limit()?;
                let id = self.checkpoints.len();
                self.checkpoints.push(Checkpoint {
                    id,
                    stdout,
                    stderr,
                    files,
                    label: None,
                });
                self.checkpoint_idx = id;
                Ok(self.checkpoints.last().unwrap())
            }
            Err(e) => {
                let runner_died = e
                    .downcast_ref::<RunnerError>()
                    .map_or(false, |r| !matches!(r, RunnerError::Application(_)));
                if runner_died {
                    self.rebuild_runner_after_timeout()?;
                }
                Err(e)
            }
        }
    }

    pub fn rollback(&mut self, checkpoint_idx: usize) -> anyhow::Result<()> {
        if checkpoint_idx >= self.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", checkpoint_idx);
        }
        self.checkpoint_idx = checkpoint_idx;
        Ok(())
    }

    pub fn set_label(&mut self, label: String) {
        self.label = if label.is_empty() { None } else { Some(label) };
    }

    pub fn set_checkpoint_label(
        &mut self,
        checkpoint_id: usize,
        label: String,
    ) -> anyhow::Result<()> {
        let cp = self
            .checkpoints
            .get_mut(checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("checkpoint {} does not exist", checkpoint_id))?;
        cp.label = if label.is_empty() { None } else { Some(label) };
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
            self.validate_file_path(key)?;
            let bytes = current
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("'{}' not in current checkpoint", key))?;
            let dest_path = output_dir.join(key);
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest_path, bytes)?;
            exported_files.push(dest_path);
        }
        Ok(exported_files)
    }

    pub fn commit(&self, keys: Option<Vec<String>>) -> anyhow::Result<Vec<PathBuf>> {
        let mounted_files = &self.checkpoints[0].files;
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
            let mounted_bytes = mounted_files.get(key).map(Vec::as_slice).unwrap_or(&[]);
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
        let all_keys: std::collections::BTreeSet<&String> = from.keys().chain(to.keys()).collect();
        let mut output = String::new();
        for key in all_keys {
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

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            allowed_hosts: self.allowed_hosts.clone(),
            timeout_ms: self.timeout_ms,
            max_workspace_bytes: self.engine.max_workspace_bytes,
            checkpoint_idx: self.checkpoint_idx,
            packages: self.packages.clone(),
            parent: self.parent.clone(),
            label: self.label.clone(),
            checkpoints: self
                .checkpoints
                .iter()
                .map(|c| CheckpointSnapshot {
                    id: c.id,
                    stdout: c.stdout.clone(),
                    stderr: c.stderr.clone(),
                    files: c.files.clone(),
                    label: c.label.clone(),
                })
                .collect(),
        }
    }

    pub fn from_snapshot(engine: &DrunEngine, snapshot: SessionSnapshot) -> anyhow::Result<Self> {
        let packages_to_install = snapshot.packages.clone();
        let mut session = Self {
            runner: Runner::new(engine, &snapshot.allowed_hosts)?,
            engine: engine.clone(),
            checkpoints: snapshot
                .checkpoints
                .into_iter()
                .map(|s| Checkpoint {
                    id: s.id,
                    stdout: s.stdout,
                    stderr: s.stderr,
                    files: s.files,
                    label: s.label,
                })
                .collect(),
            checkpoint_idx: snapshot.checkpoint_idx,
            origins: HashMap::new(),
            packages: Vec::new(),
            allowed_hosts: snapshot.allowed_hosts,
            label: snapshot.label,
            timeout_ms: snapshot.timeout_ms,
            parent: snapshot.parent,
            created_at: Instant::now(),
            last_activity: Instant::now(),
        };
        for package in &packages_to_install {
            session
                .install(package)
                .map_err(|e| anyhow::anyhow!("failed to reinstall '{package}': {e}"))?;
        }
        Ok(session)
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

    fn read_host_entries(abs: &Path) -> anyhow::Result<Vec<(String, Vec<u8>, PathBuf)>> {
        if abs.is_dir() {
            walkdir::WalkDir::new(abs)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|entry| {
                    let key = entry
                        .path()
                        .strip_prefix(abs)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    let bytes = std::fs::read(entry.path())?;
                    Ok((key, bytes, entry.path().to_path_buf()))
                })
                .collect()
        } else {
            let key = abs
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("path has no filename: {}", abs.display()))?
                .to_string_lossy()
                .into_owned();
            Ok(vec![(key, std::fs::read(abs)?, abs.to_path_buf())])
        }
    }

    fn push_files_as_checkpoint(&mut self, files: FileMap) -> anyhow::Result<&Checkpoint> {
        self.check_checkpoint_limit()?;
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let id = self.checkpoints.len();
        self.checkpoints.push(Self::empty_checkpoint(id, files));
        self.checkpoint_idx = id;
        Ok(self.checkpoints.last().unwrap())
    }

    fn check_checkpoint_limit(&self) -> anyhow::Result<()> {
        if let Some(max) = self.engine.max_checkpoints {
            if self.checkpoints.len() >= max {
                anyhow::bail!(
                    "checkpoint limit of {} reached; close or snapshot this session and start a new one",
                    max
                );
            }
        }
        Ok(())
    }

    fn check_workspace_size(&self, files: &FileMap) -> anyhow::Result<()> {
        if let Some(limit) = self.engine.max_workspace_bytes {
            let total: u64 = files.values().map(|v| v.len() as u64).sum();
            if total > limit {
                anyhow::bail!(
                    "workspace size {} bytes exceeds limit of {} bytes",
                    total,
                    limit
                );
            }
        }
        Ok(())
    }

    fn rebuild_runner_after_timeout(&mut self) -> anyhow::Result<()> {
        self.runner =
            Runner::new_from_timeout_recovery(&self.engine, &self.allowed_hosts, &self.packages)?;
        Ok(())
    }

    fn empty_checkpoint(id: usize, files: FileMap) -> Checkpoint {
        Checkpoint {
            id,
            stdout: String::new(),
            stderr: String::new(),
            files,
            label: None,
        }
    }

    fn validate_file_path(&self, key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            anyhow::bail!("workspace key must not be empty");
        }
        use std::path::Component;
        for component in std::path::Path::new(key).components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir => {
                    anyhow::bail!("workspace key must not contain '..': '{key}'");
                }
                Component::RootDir | Component::Prefix(_) => {
                    anyhow::bail!("workspace key must be a relative path: '{key}'");
                }
            }
        }
        Ok(())
    }
}

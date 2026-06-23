use crate::error::RunnerError;
use crate::runner::{ExecSuccess, Runner};
use crate::snapshot::{CheckpointRecord, SessionSnapshot};
use crate::{Checkpoint, CheckpointRef, DrunEngine, FileMap, sandbox, workspace};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

pub struct Session {
    runner: Runner,
    engine: DrunEngine,
    checkpoints: Vec<Checkpoint>,
    checkpoint_idx: usize,
    origins: HashMap<String, PathBuf>,
    packages: Vec<String>,
    intern_table: HashMap<u64, Weak<Vec<u8>>>,
    pub label: Option<String>,
    pub parent: Option<CheckpointRef>,
    pub created_at: Instant,
    pub last_activity: Instant,
}

impl Session {
    pub fn new(engine: &DrunEngine) -> anyhow::Result<Self> {
        Ok(Self {
            runner: Runner::new(engine)?,
            engine: engine.clone(),
            checkpoints: vec![empty_checkpoint(0, HashMap::new())],
            checkpoint_idx: 0,
            origins: HashMap::new(),
            packages: Vec::new(),
            intern_table: HashMap::new(),
            label: None,
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
        let source_checkpoint_idx = checkpoint_id.unwrap_or(source.checkpoint_idx);
        if source_checkpoint_idx >= source.checkpoints.len() {
            anyhow::bail!("checkpoint {} does not exist", source_checkpoint_idx);
        }
        let forked_files = source.checkpoints[source_checkpoint_idx].files.clone();
        let inherited_origins: HashMap<String, PathBuf> = source
            .origins
            .iter()
            .filter(|(k, _)| forked_files.contains_key(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let mut session = Self::new(engine)?;
        for package in &source.packages {
            session.install(package)?;
        }
        for arc in forked_files.values() {
            let hash = file_content_hash(arc);
            session
                .intern_table
                .entry(hash)
                .or_insert_with(|| Arc::downgrade(arc));
        }
        session.checkpoints[0].files = forked_files;
        session.origins = inherited_origins;
        session.parent = Some(CheckpointRef {
            session_id: source_session_id.to_string(),
            checkpoint_id: source_checkpoint_idx,
        });
        Ok(session)
    }

    pub fn mount(&mut self, path: &Path) -> anyhow::Result<Vec<String>> {
        let abs = path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("path does not exist: {}", path.display()))?;
        if !self.engine.config.mount_allowlist.is_empty() {
            let allowed = self
                .engine
                .config
                .mount_allowlist
                .iter()
                .any(|prefix| abs.starts_with(prefix));
            if !allowed {
                anyhow::bail!(
                    "'{}' is not in the mount allowlist; permitted prefixes: {}",
                    abs.display(),
                    self.engine
                        .config
                        .mount_allowlist
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        let host_entries = read_host_entries(&abs)?;
        let mounted_keys: Vec<String> =
            host_entries.iter().map(|(key, _, _)| key.clone()).collect();
        let interned_entries: Vec<(String, Arc<Vec<u8>>, PathBuf)> = host_entries
            .into_iter()
            .map(|(key, bytes, host_path)| (key, self.intern_bytes(bytes), host_path))
            .collect();
        let checkpoint = &mut self.checkpoints[self.checkpoint_idx];
        for (key, arc, host_path) in interned_entries {
            checkpoint.files.insert(key.clone(), arc);
            self.origins.insert(key, host_path);
        }
        Ok(mounted_keys)
    }

    pub fn write_file(&mut self, path: &str, content: Vec<u8>) -> anyhow::Result<()> {
        self.validate_file_path(path)?;
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        let arc = self.intern_bytes(content);
        files.insert(path.to_string(), arc);
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
        match self.runner.install(package) {
            Ok(()) => {
                self.packages.push(package.to_string());
                Ok(())
            }
            Err(e) => {
                if e.downcast_ref::<RunnerError>().is_some() {
                    self.runner = Runner::new(&self.engine)?;
                }
                Err(e)
            }
        }
    }

    pub fn execution_handle(&self) -> Arc<Mutex<Child>> {
        self.runner.child_arc()
    }

    pub fn execute_python(
        &mut self,
        code: &str,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<&Checkpoint> {
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let current_files = &self.checkpoints[self.checkpoint_idx].files;
        match self.runner.execute_python(code, current_files, on_stdout) {
            Ok(ExecSuccess {
                stdout,
                stderr,
                files: result_files,
            }) => {
                let interned_files = self.intern_file_map(result_files);
                self.check_workspace_size(&interned_files)?;
                self.check_checkpoint_limit()?;
                let id = self.checkpoints.len();
                self.checkpoints.push(Checkpoint {
                    id,
                    stdout,
                    stderr,
                    files: interned_files,
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
                    self.rebuild_runner_after_crash()?;
                }
                Err(e)
            }
        }
    }

    pub fn execute_bash(
        &mut self,
        command: &str,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<&Checkpoint> {
        self.check_command_policy(command)?;
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let workspace_dir = tempfile::TempDir::new()?;
        workspace::materialize(
            &self.checkpoints[self.checkpoint_idx].files,
            workspace_dir.path(),
        )?;
        let child = sandbox::sandboxed_sh(command, workspace_dir.path())?
            .current_dir(workspace_dir.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let BashOutput { stdout, stderr } = self.run_sandboxed_bash_child(child, on_stdout)?;
        let collected_files = workspace::collect(workspace_dir.path())?;
        let interned_files = self.intern_file_map(collected_files);
        self.check_workspace_size(&interned_files)?;
        self.check_checkpoint_limit()?;
        let id = self.checkpoints.len();
        self.checkpoints.push(Checkpoint {
            id,
            stdout,
            stderr,
            files: interned_files,
            label: None,
        });
        self.checkpoint_idx = id;
        Ok(self.checkpoints.last().unwrap())
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
            std::fs::write(&dest_path, bytes.as_slice())?;
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
            let mounted_bytes = mounted_files.get(key).map(|a| a.as_slice()).unwrap_or(&[]);
            if current_bytes.as_slice() == mounted_bytes {
                continue;
            }
            std::fs::write(host_path, current_bytes.as_slice())?;
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
            let from_bytes = from.get(key).map(|a| a.as_slice()).unwrap_or(&[]);
            let to_bytes = to.get(key).map(|a| a.as_slice()).unwrap_or(&[]);
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
        let mut blob_ptr_to_index: HashMap<usize, usize> = HashMap::new();
        let mut blobs: Vec<Vec<u8>> = Vec::new();

        let checkpoint_records = self
            .checkpoints
            .iter()
            .map(|cp| {
                let files = cp
                    .files
                    .iter()
                    .map(|(key, arc)| {
                        let ptr = Arc::as_ptr(arc) as usize;
                        let blob_index = *blob_ptr_to_index.entry(ptr).or_insert_with(|| {
                            let idx = blobs.len();
                            blobs.push((**arc).clone());
                            idx
                        });
                        (key.clone(), blob_index)
                    })
                    .collect();
                CheckpointRecord {
                    id: cp.id,
                    stdout: cp.stdout.clone(),
                    stderr: cp.stderr.clone(),
                    label: cp.label.clone(),
                    files,
                }
            })
            .collect();

        SessionSnapshot {
            checkpoint_idx: self.checkpoint_idx,
            packages: self.packages.clone(),
            parent: self.parent.clone(),
            label: self.label.clone(),
            origins: self.origins.clone(),
            blobs,
            checkpoints: checkpoint_records,
        }
    }

    pub fn from_snapshot(engine: &DrunEngine, snapshot: SessionSnapshot) -> anyhow::Result<Self> {
        let packages_to_install = snapshot.packages.clone();

        let blob_arcs: Vec<Arc<Vec<u8>>> = snapshot.blobs.into_iter().map(Arc::new).collect();
        let checkpoints: Vec<Checkpoint> = snapshot
            .checkpoints
            .into_iter()
            .map(|record| {
                let files: FileMap = record
                    .files
                    .into_iter()
                    .map(|(key, blob_index)| (key, Arc::clone(&blob_arcs[blob_index])))
                    .collect();
                Checkpoint {
                    id: record.id,
                    stdout: record.stdout,
                    stderr: record.stderr,
                    label: record.label,
                    files,
                }
            })
            .collect();

        let mut intern_table: HashMap<u64, Weak<Vec<u8>>> = HashMap::new();
        for cp in &checkpoints {
            for arc in cp.files.values() {
                let hash = file_content_hash(arc);
                intern_table
                    .entry(hash)
                    .or_insert_with(|| Arc::downgrade(arc));
            }
        }

        let origins = snapshot
            .origins
            .into_iter()
            .filter(|(_, path)| path.exists())
            .collect();

        let mut session = Self {
            runner: Runner::new(engine)?,
            engine: engine.clone(),
            checkpoints,
            checkpoint_idx: snapshot.checkpoint_idx,
            origins,
            packages: Vec::new(),
            intern_table,
            label: snapshot.label,
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

    fn intern_bytes(&mut self, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        let hash = file_content_hash(&bytes);
        if let Some(weak) = self.intern_table.get(&hash) {
            if let Some(existing_arc) = weak.upgrade() {
                if existing_arc.as_slice() == bytes.as_slice() {
                    return existing_arc;
                }
            }
        }
        let arc = Arc::new(bytes);
        self.intern_table.insert(hash, Arc::downgrade(&arc));
        arc
    }

    fn intern_file_map(&mut self, file_map: FileMap) -> FileMap {
        let mut result = FileMap::with_capacity(file_map.len());
        for (key, arc) in file_map {
            let hash = file_content_hash(&arc);
            let interned_arc = if let Some(weak) = self.intern_table.get(&hash) {
                if let Some(existing_arc) = weak.upgrade() {
                    if Arc::ptr_eq(&existing_arc, &arc) || existing_arc.as_slice() == arc.as_slice()
                    {
                        existing_arc
                    } else {
                        self.intern_table.insert(hash, Arc::downgrade(&arc));
                        arc
                    }
                } else {
                    self.intern_table.insert(hash, Arc::downgrade(&arc));
                    arc
                }
            } else {
                self.intern_table.insert(hash, Arc::downgrade(&arc));
                arc
            };
            result.insert(key, interned_arc);
        }
        result
    }

    fn push_files_as_checkpoint(&mut self, files: FileMap) -> anyhow::Result<&Checkpoint> {
        self.check_checkpoint_limit()?;
        self.checkpoints.truncate(self.checkpoint_idx + 1);
        let id = self.checkpoints.len();
        self.checkpoints.push(empty_checkpoint(id, files));
        self.checkpoint_idx = id;
        Ok(self.checkpoints.last().unwrap())
    }

    fn check_checkpoint_limit(&self) -> anyhow::Result<()> {
        if let Some(max) = self.engine.config.max_checkpoints {
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
        if let Some(limit) = self
            .engine
            .config
            .max_workspace_mb
            .map(|mb| mb * 1024 * 1024)
        {
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

    fn rebuild_runner_after_crash(&mut self) -> anyhow::Result<()> {
        self.runner = Runner::new(&self.engine)?;
        Ok(())
    }

    fn check_command_policy(&self, command: &str) -> anyhow::Result<()> {
        for denied in &self.engine.config.bash_command_denylist {
            if command.contains(denied.as_str()) {
                return Err(anyhow::Error::from(RunnerError::CommandDenied(format!(
                    "command denied: matches denylist pattern '{denied}'"
                ))));
            }
        }
        if !self.engine.config.bash_command_allowlist.is_empty()
            && !self
                .engine
                .config
                .bash_command_allowlist
                .iter()
                .any(|a| command.contains(a.as_str()))
        {
            return Err(anyhow::Error::from(RunnerError::CommandDenied(format!(
                "command denied: not matched by any allowlist pattern; permitted: {}",
                self.engine.config.bash_command_allowlist.join(", ")
            ))));
        }
        Ok(())
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

    fn run_sandboxed_bash_child(
        &self,
        mut child: std::process::Child,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<BashOutput> {
        let bash_timeout_ms = self.engine.config.bash_timeout_ms;
        let child_stderr = child.stderr.take().unwrap();
        let child_stdout = child.stdout.take().unwrap();
        let child = Arc::new(Mutex::new(child));
        let child_for_timeout = Arc::clone(&child);
        let timed_out = Arc::new(AtomicBool::new(false));
        let timed_out_flag = Arc::clone(&timed_out);
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if cancel_rx
                .recv_timeout(Duration::from_millis(bash_timeout_ms))
                .is_err()
            {
                timed_out_flag.store(true, Ordering::Relaxed);
                let _ = child_for_timeout.lock().unwrap().kill();
            }
        });
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = BufReader::new(child_stderr).read_to_string(&mut buf);
            buf
        });
        let mut stdout = String::new();
        let mut stdout_reader = BufReader::new(child_stdout);
        loop {
            let mut line = String::new();
            match stdout_reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    on_stdout(line.trim_end_matches('\n').to_string());
                    stdout.push_str(&line);
                }
            }
        }
        let _ = cancel_tx.send(());
        let stderr = stderr_thread.join().unwrap_or_default();
        let _ = child.lock().unwrap().wait();
        if timed_out.load(Ordering::Relaxed) {
            return Err(anyhow::Error::from(RunnerError::Timeout {
                timeout_ms: self.engine.config.bash_timeout_ms,
            }));
        }
        Ok(BashOutput { stdout, stderr })
    }
}

struct BashOutput {
    stdout: String,
    stderr: String,
}

fn file_content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
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

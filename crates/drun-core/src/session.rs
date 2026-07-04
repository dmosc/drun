use crate::config::Config;
use crate::error::RunnerError;
use crate::snapshot::{CheckpointRecord, SessionSnapshot};
use crate::{Checkpoint, CheckpointRef, FileMap, sandbox, workspace};
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
    config: Config,
    checkpoints: Vec<Checkpoint>,
    checkpoint_idx: usize,
    origins: HashMap<String, PathBuf>,
    overlays: HashMap<String, PathBuf>,
    intern_table: HashMap<u64, Weak<Vec<u8>>>,
    pub label: Option<String>,
    pub parent: Option<CheckpointRef>,
    pub created_at: Instant,
    pub last_activity: Instant,
}

impl Session {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            config: config.clone(),
            checkpoints: vec![Checkpoint::empty(0, HashMap::new())],
            checkpoint_idx: 0,
            origins: HashMap::new(),
            overlays: HashMap::new(),
            intern_table: HashMap::new(),
            label: None,
            parent: None,
            created_at: Instant::now(),
            last_activity: Instant::now(),
        })
    }

    pub fn from_session(
        config: &Config,
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

        let mut session = Self::new(config)?;
        for arc in forked_files.values() {
            let hash = Self::file_content_hash(arc);
            session
                .intern_table
                .entry(hash)
                .or_insert_with(|| Arc::downgrade(arc));
        }
        session.checkpoints[0].files = forked_files;
        session.origins = inherited_origins;
        session.overlays = source.overlays.clone();
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
        if !self.config.mount_allowlist.is_empty() {
            let allowed = self
                .config
                .mount_allowlist
                .iter()
                .any(|prefix| abs.starts_with(prefix));
            if !allowed {
                anyhow::bail!(
                    "'{}' is not in the mount allowlist; permitted prefixes: {}",
                    abs.display(),
                    self.config
                        .mount_allowlist
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }

        let (file_entries, overlay_entries) = if abs.is_dir() {
            Self::scan_mount_path(&abs, "", &self.config.mount_overlay_paths)?
        } else {
            let key = abs
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("path has no filename: {}", abs.display()))?
                .to_string_lossy()
                .into_owned();
            (vec![(key, std::fs::read(&abs)?, abs.clone())], vec![])
        };

        let interned_file_entries: Vec<(String, Arc<Vec<u8>>, PathBuf)> = file_entries
            .into_iter()
            .map(|(key, bytes, host_path)| (key, self.intern_bytes(bytes), host_path))
            .collect();

        let mut mounted_keys: Vec<String> = Vec::new();
        let checkpoint = &mut self.checkpoints[self.checkpoint_idx];
        for (key, arc, host_path) in interned_file_entries {
            checkpoint.files.insert(key.clone(), arc);
            self.origins.insert(key.clone(), host_path);
            mounted_keys.push(key);
        }
        for (key, host_path) in overlay_entries {
            self.overlays.insert(key.clone(), host_path);
            mounted_keys.push(key);
        }
        Ok(mounted_keys)
    }

    pub fn write_file(&mut self, path: &str, content: Vec<u8>) -> anyhow::Result<()> {
        self.validate_file_path(path)?;
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        let arc = self.intern_bytes(content);
        files.insert(path.to_string(), arc);
        self.check_workspace_size(&files)?;
        self.push_checkpoint(files, String::new(), String::new())?;
        Ok(())
    }

    pub fn delete_file(&mut self, path: &str) -> anyhow::Result<&Checkpoint> {
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        if files.remove(path).is_none() {
            anyhow::bail!("'{}' not in current checkpoint", path);
        }
        self.push_checkpoint(files, String::new(), String::new())
    }

    pub fn execute_bash(
        &mut self,
        command: &str,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<&Checkpoint> {
        self.check_command_policy(command)?;
        let workspace_dir = tempfile::TempDir::new()?;
        workspace::materialize(
            &self.checkpoints[self.checkpoint_idx].files,
            workspace_dir.path(),
        )?;
        for (key, host_path) in &self.overlays {
            let dest = workspace_dir.path().join(key);
            if !dest.exists() {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::os::unix::fs::symlink(host_path, &dest)?;
            }
        }
        let child = sandbox::sandboxed_sh(command, workspace_dir.path())?
            .current_dir(workspace_dir.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let BashOutput { stdout, stderr } = self.run_sandboxed_bash_child(child, on_stdout)?;
        let collected_files = workspace::collect(workspace_dir.path())?;
        let interned_files = self.intern_file_map(collected_files);
        self.check_workspace_size(&interned_files)?;
        self.push_checkpoint(interned_files, stdout, stderr)
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

    pub fn checkpoint_by_label(&self, label: &str) -> Option<usize> {
        self.checkpoints
            .iter()
            .find(|c| c.label.as_deref() == Some(label))
            .map(|c| c.id)
    }

    pub fn resolve_checkpoint(
        &self,
        id: Option<u64>,
        label: Option<&str>,
    ) -> anyhow::Result<Option<usize>> {
        match (id, label) {
            (_, Some(lbl)) => self
                .checkpoint_by_label(lbl)
                .map(Some)
                .ok_or_else(|| anyhow::anyhow!("no checkpoint with label '{lbl}'")),
            (Some(id), None) => Ok(Some(id as usize)),
            (None, None) => Ok(None),
        }
    }

    pub fn squash_checkpoints(
        &mut self,
        from_id: usize,
        to_id: usize,
        label: Option<String>,
    ) -> anyhow::Result<&Checkpoint> {
        anyhow::ensure!(
            from_id >= 1,
            "checkpoint 0 is the mounted baseline and cannot be squashed; start the range at checkpoint 1 or later"
        );
        anyhow::ensure!(
            from_id <= to_id,
            "from_id {} must be <= to_id {}",
            from_id,
            to_id
        );
        anyhow::ensure!(
            to_id < self.checkpoints.len(),
            "checkpoint {} does not exist",
            to_id
        );
        let combined_stdout = self.checkpoints[from_id..=to_id]
            .iter()
            .map(|c| c.stdout.as_str())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let combined_stderr = self.checkpoints[from_id..=to_id]
            .iter()
            .map(|c| c.stderr.as_str())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let terminal_files = self.checkpoints[to_id].files.clone();
        let squashed = Checkpoint {
            id: from_id,
            stdout: combined_stdout,
            stderr: combined_stderr,
            files: terminal_files,
            label,
        };
        let removed_count = to_id - from_id;
        self.checkpoints
            .splice(from_id..=to_id, std::iter::once(squashed));
        for (i, cp) in self.checkpoints.iter_mut().enumerate() {
            cp.id = i;
        }
        if self.checkpoint_idx >= from_id && self.checkpoint_idx <= to_id {
            self.checkpoint_idx = from_id;
        } else if self.checkpoint_idx > to_id {
            self.checkpoint_idx -= removed_count;
        }
        Ok(&self.checkpoints[self.checkpoint_idx])
    }

    pub fn drop_checkpoints(&mut self, from_id: usize, to_id: usize) -> anyhow::Result<()> {
        anyhow::ensure!(
            from_id >= 1,
            "checkpoint 0 is the mounted baseline and cannot be dropped; start the range at checkpoint 1 or later"
        );
        anyhow::ensure!(
            from_id <= to_id,
            "from_id {} must be <= to_id {}",
            from_id,
            to_id
        );
        anyhow::ensure!(
            to_id < self.checkpoints.len(),
            "checkpoint {} does not exist",
            to_id
        );
        anyhow::ensure!(
            self.checkpoint_idx < from_id || self.checkpoint_idx > to_id,
            "cannot drop the current checkpoint ({})",
            self.checkpoint_idx
        );
        let removed_count = to_id - from_id + 1;
        self.checkpoints.drain(from_id..=to_id);
        for (i, cp) in self.checkpoints.iter_mut().enumerate() {
            cp.id = i;
        }
        if self.checkpoint_idx > to_id {
            self.checkpoint_idx -= removed_count;
        }
        Ok(())
    }

    pub fn merge_from(
        &mut self,
        source: &Session,
        checkpoint_id: Option<usize>,
        keys: Option<Vec<String>>,
    ) -> anyhow::Result<&Checkpoint> {
        let source_checkpoint_id = checkpoint_id.unwrap_or(source.checkpoint_idx);
        let source_files = &source
            .checkpoints
            .get(source_checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("checkpoint {} does not exist", source_checkpoint_id))?
            .files;
        let mut merged = self.checkpoints[self.checkpoint_idx].files.clone();
        match keys {
            Some(ks) => {
                for key in &ks {
                    match source_files.get(key) {
                        Some(blob) => {
                            merged.insert(key.clone(), Arc::clone(blob));
                        }
                        None => anyhow::bail!("file '{}' not found in source checkpoint", key),
                    }
                }
            }
            None => {
                for (key, blob) in source_files {
                    merged.insert(key.clone(), Arc::clone(blob));
                }
            }
        }
        self.check_workspace_size(&merged)?;
        self.push_checkpoint(merged, String::new(), String::new())
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
            parent: self.parent.clone(),
            label: self.label.clone(),
            origins: self.origins.clone(),
            overlays: self.overlays.clone(),
            blobs,
            checkpoints: checkpoint_records,
        }
    }

    pub fn from_snapshot(config: &Config, snapshot: SessionSnapshot) -> anyhow::Result<Self> {
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
                let hash = Self::file_content_hash(arc);
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

        let overlays = snapshot
            .overlays
            .into_iter()
            .filter(|(_, path)| path.exists())
            .collect();

        Ok(Self {
            config: config.clone(),
            checkpoints,
            checkpoint_idx: snapshot.checkpoint_idx,
            origins,
            overlays,
            intern_table,
            label: snapshot.label,
            parent: snapshot.parent,
            created_at: Instant::now(),
            last_activity: Instant::now(),
        })
    }

    pub fn current(&self) -> &Checkpoint {
        &self.checkpoints[self.checkpoint_idx]
    }

    pub fn history(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    fn intern_bytes(&mut self, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        let hash = Self::file_content_hash(&bytes);
        if let Some(weak) = self.intern_table.get(&hash)
            && let Some(existing_arc) = weak.upgrade()
            && existing_arc.as_slice() == bytes.as_slice()
        {
            return existing_arc;
        }
        let arc = Arc::new(bytes);
        self.intern_table.insert(hash, Arc::downgrade(&arc));
        arc
    }

    fn intern_file_map(&mut self, file_map: FileMap) -> FileMap {
        let mut result = FileMap::with_capacity(file_map.len());
        for (key, arc) in file_map {
            let hash = Self::file_content_hash(&arc);
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

    fn push_checkpoint(
        &mut self,
        files: FileMap,
        stdout: String,
        stderr: String,
    ) -> anyhow::Result<&Checkpoint> {
        self.check_checkpoint_limit()?;
        self.checkpoints.truncate(self.checkpoint_idx + 1);
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

    fn check_checkpoint_limit(&self) -> anyhow::Result<()> {
        if let Some(max) = self.config.max_checkpoints
            && self.checkpoints.len() >= max
        {
            anyhow::bail!(
                "checkpoint limit of {} reached; close or snapshot this session and start a new one",
                max
            );
        }
        Ok(())
    }

    fn check_workspace_size(&self, files: &FileMap) -> anyhow::Result<()> {
        if let Some(limit) = self.config.max_workspace_mb.map(|mb| mb * 1024 * 1024) {
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

    fn check_command_policy(&self, command: &str) -> anyhow::Result<()> {
        for denied in &self.config.bash_command_denylist {
            if command.contains(denied.as_str()) {
                return Err(anyhow::Error::from(RunnerError::CommandDenied(format!(
                    "command denied: matches denylist pattern '{denied}'"
                ))));
            }
        }
        if !self.config.bash_command_allowlist.is_empty()
            && !self
                .config
                .bash_command_allowlist
                .iter()
                .any(|a| command.contains(a.as_str()))
        {
            return Err(anyhow::Error::from(RunnerError::CommandDenied(format!(
                "command denied: not matched by any allowlist pattern; permitted: {}",
                self.config.bash_command_allowlist.join(", ")
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
        mut child: Child,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<BashOutput> {
        let bash_timeout_ms = self.config.bash_timeout_ms;
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
                timeout_ms: self.config.bash_timeout_ms,
            }));
        }
        Ok(BashOutput { stdout, stderr })
    }

    fn file_content_hash(bytes: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        hasher.finish()
    }

    fn scan_mount_path(
        dir: &Path,
        key_prefix: &str,
        overlay_patterns: &[String],
    ) -> anyhow::Result<(ScannedFiles, ScannedOverlays)> {
        let mut file_entries = vec![];
        let mut overlay_entries = vec![];
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = if key_prefix.is_empty() {
                name.clone()
            } else {
                format!("{key_prefix}/{name}")
            };
            let path = entry.path();
            if path.is_dir() {
                if overlay_patterns.iter().any(|p| p == &name) {
                    overlay_entries.push((key, path));
                } else {
                    let (sub_files, sub_overlays) =
                        Self::scan_mount_path(&path, &key, overlay_patterns)?;
                    file_entries.extend(sub_files);
                    overlay_entries.extend(sub_overlays);
                }
            } else if path.is_file() {
                file_entries.push((key, std::fs::read(&path)?, path));
            }
        }
        Ok((file_entries, overlay_entries))
    }
}

struct BashOutput {
    stdout: String,
    stderr: String,
}

/// (workspace key, file bytes, host path) for regular files discovered under a
/// mount.
type ScannedFiles = Vec<(String, Vec<u8>, PathBuf)>;
/// (workspace key, host path) for directories matching `mount_overlay_paths`.
type ScannedOverlays = Vec<(String, PathBuf)>;

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new(&Config::default()).unwrap()
    }

    fn file_map(pairs: &[(&str, &[u8])]) -> FileMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Arc::new(v.to_vec())))
            .collect()
    }

    #[test]
    fn failed_bash_after_rollback_does_not_discard_forward_checkpoints() {
        let config = Config {
            max_workspace_mb: Some(1),
            ..Config::default()
        };
        let mut s = Session::new(&config).unwrap();
        s.write_file("a.txt", b"1".to_vec()).unwrap(); // checkpoint 1
        s.write_file("a.txt", b"2".to_vec()).unwrap(); // checkpoint 2
        assert_eq!(s.history().len(), 3);
        s.rollback(1).unwrap();

        // Writes a 2 MB file — execution succeeds, but the post-run
        // workspace-size check must fail. Before the fix, execute_bash
        // truncated forward history *before* running the command, so this
        // failure would have permanently discarded checkpoint 2 even though
        // nothing new was ever committed.
        let result = s.execute_bash("head -c 2000000 /dev/zero > big.bin", &mut |_| {});
        assert!(result.is_err());
        assert_eq!(
            s.history().len(),
            3,
            "checkpoint 2 must survive a failed run"
        );
        assert_eq!(s.current().id, 1, "head must stay put on failure");
    }

    #[test]
    fn merge_after_rollback_discards_forward_checkpoints_like_other_mutators() {
        let mut s = session();
        s.write_file("a.txt", b"1".to_vec()).unwrap(); // checkpoint 1
        s.write_file("a.txt", b"2".to_vec()).unwrap(); // checkpoint 2
        assert_eq!(s.history().len(), 3);
        s.rollback(1).unwrap();

        let mut source = session();
        source.write_file("b.txt", b"src".to_vec()).unwrap();

        s.merge_from(&source, None, None).unwrap();

        // Checkpoint 2 (a.txt = "2") must be gone, not left dangling past a
        // new head at id 2 — merge now truncates forward history exactly
        // like session_bash / session_write_file / session_delete_file.
        assert_eq!(s.history().len(), 3);
        assert_eq!(s.current().id, 2);
        assert_eq!(s.current().files.get("a.txt").unwrap().as_slice(), b"1");
        assert_eq!(s.current().files.get("b.txt").unwrap().as_slice(), b"src");
    }

    #[test]
    fn squash_cannot_include_checkpoint_zero() {
        let mut s = session();
        s.write_file("a.txt", b"1".to_vec()).unwrap();
        s.write_file("a.txt", b"2".to_vec()).unwrap();
        let err = s.squash_checkpoints(0, 1, None).unwrap_err();
        assert!(err.to_string().contains("checkpoint 0"));
        // The mounted baseline must still be squashable-range-adjacent but
        // untouched: a range starting at 1 is fine.
        assert!(s.squash_checkpoints(1, 2, None).is_ok());
    }

    #[test]
    fn drop_cannot_include_checkpoint_zero() {
        let mut s = session();
        s.write_file("a.txt", b"1".to_vec()).unwrap();
        s.write_file("a.txt", b"2".to_vec()).unwrap();
        let err = s.drop_checkpoints(0, 0).unwrap_err();
        assert!(err.to_string().contains("checkpoint 0"));
    }

    #[test]
    fn commit_diffs_against_the_true_mounted_baseline_after_squash() {
        // Regression guard for the checkpoint-0 protection above: commit()
        // and diff() both read checkpoints[0] as "what was on disk before
        // the sandbox touched it". If a squash/drop were ever allowed to
        // consume checkpoint 0, this baseline would silently become
        // whatever the squash's terminal state was instead.
        let dir = tempfile::tempdir().unwrap();
        let host_path = dir.path().join("mounted.txt");
        std::fs::write(&host_path, b"original").unwrap();

        let mut s = session();
        s.mount(&host_path).unwrap();
        s.write_file("mounted.txt", b"changed".to_vec()).unwrap();
        s.write_file("mounted.txt", b"changed again".to_vec())
            .unwrap();

        // Squashing checkpoints 1..=2 is allowed and must not touch checkpoint 0.
        s.squash_checkpoints(1, 2, None).unwrap();
        assert!(s.diff(0, s.current().id).unwrap().contains("original"));

        let committed = s.commit(None).unwrap();
        assert_eq!(committed, vec![host_path.canonicalize().unwrap()]);
        assert_eq!(std::fs::read(&host_path).unwrap(), b"changed again");
    }

    #[test]
    fn file_content_hash_is_deterministic() {
        assert_eq!(
            Session::file_content_hash(b"hello"),
            Session::file_content_hash(b"hello")
        );
    }

    #[test]
    fn file_content_hash_differs_for_different_content() {
        assert_ne!(
            Session::file_content_hash(b"hello"),
            Session::file_content_hash(b"world")
        );
    }

    #[test]
    fn file_content_hash_handles_empty_bytes() {
        assert_eq!(
            Session::file_content_hash(b""),
            Session::file_content_hash(b"")
        );
    }

    #[test]
    fn checkpoint_empty_has_given_id_and_files_with_empty_streams() {
        let files = file_map(&[("a.txt", b"hi")]);
        let checkpoint = Checkpoint::empty(3, files.clone());
        assert_eq!(
            checkpoint,
            Checkpoint {
                id: 3,
                stdout: String::new(),
                stderr: String::new(),
                files,
                label: None,
            }
        );
    }

    #[test]
    fn scan_mount_path_reads_a_flat_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let (files, overlays) = Session::scan_mount_path(dir.path(), "", &[]).unwrap();
        assert_eq!(files.len(), 1);
        let (key, bytes, host_path) = &files[0];
        assert_eq!(key.as_str(), "a.txt");
        assert_eq!(bytes.as_slice(), b"hello");
        assert_eq!(*host_path, dir.path().join("a.txt"));
        assert!(overlays.is_empty());
    }

    #[test]
    fn scan_mount_path_builds_slash_joined_keys_for_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), b"nested").unwrap();

        let (files, _) = Session::scan_mount_path(dir.path(), "", &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0.as_str(), "sub/b.txt");
    }

    #[test]
    fn scan_mount_path_treats_matching_directories_as_overlays_not_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules/pkg.js"), b"ignored").unwrap();
        std::fs::write(dir.path().join("real.txt"), b"kept").unwrap();

        let overlay_patterns = vec!["node_modules".to_string()];
        let (files, overlays) =
            Session::scan_mount_path(dir.path(), "", &overlay_patterns).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0.as_str(), "real.txt");
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].0.as_str(), "node_modules");
        assert_eq!(overlays[0].1, dir.path().join("node_modules"));
    }

    #[test]
    fn scan_mount_path_respects_key_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();

        let (files, _) = Session::scan_mount_path(dir.path(), "prefix", &[]).unwrap();
        assert_eq!(files[0].0.as_str(), "prefix/a.txt");
    }
}

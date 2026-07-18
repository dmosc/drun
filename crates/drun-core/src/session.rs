use crate::config::ConfigHandle;
use crate::error::RunnerError;
use crate::snapshot::{CheckpointRecord, SessionSnapshot};
use crate::{Checkpoint, CheckpointRef, FileMap, sandbox, workspace};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

pub struct Session {
    config: ConfigHandle,
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

static RUNNING_CHILD_PGIDS: OnceLock<Mutex<HashSet<i32>>> = OnceLock::new();

struct SessionChildGuard(i32);

impl SessionChildGuard {
    fn new(pgid: i32) -> Self {
        Session::running_child_pgids().lock().unwrap().insert(pgid);
        Self(pgid)
    }
}

impl Drop for SessionChildGuard {
    fn drop(&mut self) {
        Session::running_child_pgids()
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}

impl Session {
    pub fn new(config: ConfigHandle) -> anyhow::Result<Self> {
        Ok(Self {
            config,
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

    /// Process groups of in-flight sandboxed children, keyed by pgid.
    fn running_child_pgids() -> &'static Mutex<HashSet<i32>> {
        RUNNING_CHILD_PGIDS.get_or_init(|| Mutex::new(HashSet::new()))
    }

    pub fn from_session(
        config: ConfigHandle,
        source_session_id: &str,
        source: &Session,
        checkpoint_id: Option<usize>,
    ) -> anyhow::Result<Self> {
        let source_checkpoint_idx = checkpoint_id.unwrap_or(source.checkpoint_idx);
        if source_checkpoint_idx >= source.checkpoints.len() {
            return Err(RunnerError::checkpoint_not_found(source_checkpoint_idx).into());
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
        let abs = path.canonicalize().map_err(|_| {
            RunnerError::invalid_workspace_path(format!("path does not exist: {}", path.display()))
        })?;
        let config = self.config.get();
        if !config.mount_allowlist.is_empty() {
            let allowed = config
                .mount_allowlist
                .iter()
                .any(|prefix| abs.starts_with(prefix));
            if !allowed {
                return Err(RunnerError::mount_denied(format!(
                    "'{}' is not in the mount allowlist; permitted prefixes: {}",
                    abs.display(),
                    config
                        .mount_allowlist
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .into());
            }
        }

        let (file_entries, overlay_entries) = if abs.is_dir() {
            Self::scan_mount_path(&abs, "", &config.mount_overlay_paths)?
        } else {
            let key = abs
                .file_name()
                .ok_or_else(|| {
                    RunnerError::invalid_workspace_path(format!(
                        "path has no filename: {}",
                        abs.display()
                    ))
                })?
                .to_string_lossy()
                .into_owned();
            (vec![(key, std::fs::read(&abs)?, abs.clone())], vec![])
        };

        let interned_file_entries: Vec<(String, Arc<Vec<u8>>, PathBuf)> = file_entries
            .into_iter()
            .map(|(key, bytes, host_path)| (key, self.intern_bytes(bytes), host_path))
            .collect();

        let mut prospective_files = self.checkpoints[self.checkpoint_idx].files.clone();
        for (key, arc, _) in &interned_file_entries {
            prospective_files.insert(key.clone(), Arc::clone(arc));
        }
        self.check_workspace_size(&prospective_files)?;

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
        self.push_checkpoint(files, String::new(), String::new(), None)?;
        Ok(())
    }

    pub fn delete_file(&mut self, path: &str) -> anyhow::Result<&Checkpoint> {
        let mut files = self.checkpoints[self.checkpoint_idx].files.clone();
        if files.remove(path).is_none() {
            return Err(RunnerError::file_not_found_in_current(path).into());
        }
        self.push_checkpoint(files, String::new(), String::new(), None)
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
        let mut read_paths: Vec<PathBuf> = self.overlays.values().cloned().collect();
        read_paths.extend(self.config.get().mount_allowlist);
        let child = sandbox::Sandbox::new(workspace_dir.path(), read_paths)
            .command(command)?
            .current_dir(workspace_dir.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let BashOutput { stdout, stderr } = self.run_sandboxed_bash_child(child, on_stdout)?;
        let collected_files = workspace::collect(workspace_dir.path())?;
        self.record_bash_checkpoint(command, collected_files, stdout, stderr)
    }

    fn record_bash_checkpoint(
        &mut self,
        command: &str,
        files: FileMap,
        stdout: String,
        stderr: String,
    ) -> anyhow::Result<&Checkpoint> {
        let interned_files = self.intern_file_map(files);
        self.check_workspace_size(&interned_files)?;
        self.push_checkpoint(interned_files, stdout, stderr, Some(command.to_string()))
    }

    pub fn rollback(&mut self, checkpoint_idx: usize) -> anyhow::Result<()> {
        if checkpoint_idx >= self.checkpoints.len() {
            return Err(RunnerError::checkpoint_not_found(checkpoint_idx).into());
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
            .ok_or_else(|| RunnerError::checkpoint_not_found(checkpoint_id))?;
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
                .ok_or_else(|| RunnerError::checkpoint_label_not_found(lbl).into()),
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
        if to_id >= self.checkpoints.len() {
            return Err(RunnerError::checkpoint_not_found(to_id).into());
        }
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
        let combined_command = self.checkpoints[from_id..=to_id]
            .iter()
            .filter_map(|c| c.command.as_deref())
            .collect::<Vec<_>>()
            .join(" && ");
        let terminal_files = self.checkpoints[to_id].files.clone();
        let squashed = Checkpoint {
            id: from_id,
            stdout: combined_stdout,
            stderr: combined_stderr,
            files: terminal_files,
            label,
            command: (!combined_command.is_empty()).then_some(combined_command),
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
        self.prune_intern_table();
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
        if to_id >= self.checkpoints.len() {
            return Err(RunnerError::checkpoint_not_found(to_id).into());
        }
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
        self.prune_intern_table();
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
            .ok_or_else(|| RunnerError::checkpoint_not_found(source_checkpoint_id))?
            .files;
        let mut merged = self.checkpoints[self.checkpoint_idx].files.clone();
        match keys {
            Some(ks) => {
                for key in &ks {
                    match source_files.get(key) {
                        Some(blob) => {
                            merged.insert(key.clone(), Arc::clone(blob));
                        }
                        None => {
                            return Err(RunnerError::file_not_found_in_source(key).into());
                        }
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
        self.push_checkpoint(merged, String::new(), String::new(), None)
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
                .ok_or_else(|| RunnerError::file_not_found_in_current(key))?;
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
                .ok_or_else(|| RunnerError::file_not_found_in_current(key))?;
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
            return Err(RunnerError::checkpoint_not_found(from_id).into());
        }
        if to_id >= self.checkpoints.len() {
            return Err(RunnerError::checkpoint_not_found(to_id).into());
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
                    command: cp.command.clone(),
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

    pub fn from_snapshot(config: ConfigHandle, snapshot: SessionSnapshot) -> anyhow::Result<Self> {
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
                    command: record.command,
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
            config,
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

    fn prune_intern_table(&mut self) {
        let mut live = HashMap::with_capacity(self.intern_table.len());
        for checkpoint in &self.checkpoints {
            for arc in checkpoint.files.values() {
                let hash = Self::file_content_hash(arc);
                live.entry(hash).or_insert_with(|| Arc::downgrade(arc));
            }
        }
        self.intern_table = live;
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
        command: Option<String>,
    ) -> anyhow::Result<&Checkpoint> {
        self.check_checkpoint_limit()?;
        let discarding_forward_history = self.checkpoints.len() > self.checkpoint_idx + 1;
        if discarding_forward_history {
            self.checkpoints.truncate(self.checkpoint_idx + 1);
        }
        let id = self.checkpoints.len();
        self.checkpoints.push(Checkpoint {
            id,
            stdout,
            stderr,
            files,
            label: None,
            command,
        });
        self.checkpoint_idx = id;
        if discarding_forward_history {
            self.prune_intern_table();
        }
        Ok(self.checkpoints.last().unwrap())
    }

    fn check_checkpoint_limit(&self) -> anyhow::Result<()> {
        if let Some(max) = self.config.get().max_checkpoints
            && self.checkpoints.len() >= max
        {
            return Err(RunnerError::checkpoint_limit_reached(max).into());
        }
        Ok(())
    }

    fn check_workspace_size(&self, files: &FileMap) -> anyhow::Result<()> {
        if let Some(limit_bytes) = self
            .config
            .get()
            .max_workspace_mb
            .map(|mb| mb * 1024 * 1024)
        {
            let actual_bytes: u64 = files.values().map(|v| v.len() as u64).sum();
            if actual_bytes > limit_bytes {
                return Err(RunnerError::workspace_size_exceeded(actual_bytes, limit_bytes).into());
            }
        }
        Ok(())
    }

    fn check_command_policy(&self, command: &str) -> anyhow::Result<()> {
        let config = self.config.get();
        for denied in &config.bash_command_denylist {
            if command.contains(denied.as_str()) {
                return Err(RunnerError::command_denied(format!(
                    "command denied: matches denylist pattern '{denied}'"
                ))
                .into());
            }
        }
        if !config.bash_command_allowlist.is_empty()
            && !config
                .bash_command_allowlist
                .iter()
                .any(|a| command.contains(a.as_str()))
        {
            return Err(RunnerError::command_denied(format!(
                "command denied: not matched by any allowlist pattern; permitted: {}",
                config.bash_command_allowlist.join(", ")
            ))
            .into());
        }
        Ok(())
    }

    fn validate_file_path(&self, key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(
                RunnerError::invalid_workspace_path("workspace key must not be empty").into(),
            );
        }
        use std::path::Component;
        for component in std::path::Path::new(key).components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir => {
                    return Err(RunnerError::invalid_workspace_path(format!(
                        "workspace key must not contain '..': '{key}'"
                    ))
                    .into());
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(RunnerError::invalid_workspace_path(format!(
                        "workspace key must be a relative path: '{key}'"
                    ))
                    .into());
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
        let bash_timeout_ms = self.config.get().bash_timeout_ms;
        let pgid = child.id() as i32;
        let _pgid_guard = SessionChildGuard::new(pgid);
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
                Self::kill_process_tree(pgid);
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
        Self::kill_process_tree(pgid);
        if timed_out.load(Ordering::Relaxed) {
            return Err(RunnerError::timeout(bash_timeout_ms).into());
        }
        Ok(BashOutput { stdout, stderr })
    }

    /// Kills every sandboxed child (and its descendants) currently tracked
    /// as running. Intended for the daemon's shutdown handler, so an
    /// in-flight `session_bash` call doesn't outlive the daemon process.
    pub fn kill_all_running_children() {
        let pgids: Vec<i32> = Self::running_child_pgids()
            .lock()
            .unwrap()
            .iter()
            .copied()
            .collect();
        for pgid in pgids {
            Self::kill_process_tree(pgid);
        }
    }

    #[cfg(unix)]
    fn kill_process_tree(pid: i32) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        for descendant_pid in Self::descendant_pids(pid) {
            unsafe {
                libc::kill(descendant_pid, libc::SIGKILL);
            }
        }
    }

    #[cfg(not(unix))]
    fn kill_process_tree(_pid: i32) {}

    #[cfg(unix)]
    fn descendant_pids(root_pid: i32) -> Vec<i32> {
        let Ok(output) = std::process::Command::new("ps")
            .args(["-Ao", "pid=,ppid="])
            .output()
        else {
            return Vec::new();
        };
        let parent_of: Vec<(i32, i32)> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid: i32 = fields.next()?.parse().ok()?;
                let ppid: i32 = fields.next()?.parse().ok()?;
                Some((pid, ppid))
            })
            .collect();

        let mut descendants = std::collections::HashSet::new();
        let mut frontier = vec![root_pid];
        while let Some(parent_pid) = frontier.pop() {
            for &(pid, ppid) in &parent_of {
                if ppid == parent_pid && descendants.insert(pid) {
                    frontier.push(pid);
                }
            }
        }
        descendants.into_iter().collect()
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
            let file_type = entry.file_type()?;
            // Skip symlinks to avoid cyclical recursion.
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = if key_prefix.is_empty() {
                name.clone()
            } else {
                format!("{key_prefix}/{name}")
            };
            let path = entry.path();
            if file_type.is_dir() {
                if overlay_patterns.iter().any(|p| p == &name) {
                    overlay_entries.push((key, path));
                } else {
                    let (sub_files, sub_overlays) =
                        Self::scan_mount_path(&path, &key, overlay_patterns)?;
                    file_entries.extend(sub_files);
                    overlay_entries.extend(sub_overlays);
                }
            } else if file_type.is_file() {
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
    use crate::config::Config;

    fn session() -> Session {
        Session::new(Config::default().into()).unwrap()
    }

    fn file_map(pairs: &[(&str, &[u8])]) -> FileMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Arc::new(v.to_vec())))
            .collect()
    }

    #[test]
    fn execute_bash_rejects_a_denylisted_command_without_running_it() {
        let config = Config {
            bash_command_denylist: vec!["rm -rf".to_string()],
            ..Config::default()
        };
        let mut s = Session::new(config.into()).unwrap();
        let err = s
            .execute_bash("rm -rf /tmp/whatever", &mut |_| {})
            .unwrap_err();
        assert!(err.to_string().contains("command denied"));
        assert_eq!(s.history().len(), 1, "denied command must not checkpoint");
    }

    #[test]
    fn execute_bash_records_the_command_on_the_new_checkpoint() {
        let mut s = session();
        let cp = s
            .record_bash_checkpoint("echo hi", FileMap::new(), "hi\n".to_string(), String::new())
            .unwrap();
        assert_eq!(cp.command.as_deref(), Some("echo hi"));
    }

    #[test]
    fn write_file_and_delete_file_leave_the_checkpoints_command_unset() {
        let mut s = session();
        s.write_file("a.txt", b"hi".to_vec()).unwrap();
        assert_eq!(s.current().command, None);
        s.delete_file("a.txt").unwrap();
        assert_eq!(s.current().command, None);
    }

    #[test]
    fn squash_checkpoints_joins_the_absorbed_commands() {
        let mut s = session();
        s.record_bash_checkpoint("echo one", FileMap::new(), String::new(), String::new())
            .unwrap();
        s.record_bash_checkpoint("echo two", FileMap::new(), String::new(), String::new())
            .unwrap();

        let squashed = s.squash_checkpoints(1, 2, None).unwrap();

        assert_eq!(squashed.command.as_deref(), Some("echo one && echo two"));
    }

    // These two exercise the real sandbox-exec profile end to end, so they
    // (like descendant_pids_finds_a_grandchild_process below) only pass when
    // `cargo test` itself runs unsandboxed. macOS refuses to apply a second
    // sandbox-exec profile inside an already-sandboxed process
    // ("sandbox_apply: Operation not permitted"), so running the suite from
    // inside another sandbox (e.g. a drun session) fails them both — not a
    // sign the restriction logic is wrong, just that this nests one sandbox
    // too deep to self-test.
    #[test]
    #[cfg(target_os = "macos")]
    fn execute_bash_can_still_read_and_write_within_the_workspace() {
        let mut s = session();
        s.write_file("greeting.txt", b"hello".to_vec()).unwrap();
        let cp = s.execute_bash("cat greeting.txt", &mut |_| {}).unwrap();
        assert_eq!(cp.stdout.trim(), "hello");
        assert_eq!(cp.stderr, "");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn execute_bash_cannot_read_a_host_path_outside_the_sandbox_allowlist() {
        // A tempdir the session never mounted or overlaid — distinct from
        // the workspace's own tempdir even though both live under the same
        // OS temp root, since only the workspace's exact subpath is allowed.
        let secret_dir = tempfile::tempdir().unwrap();
        let secret_path = secret_dir.path().join("secret.txt");
        std::fs::write(&secret_path, b"do-not-leak").unwrap();

        let mut s = session();
        let cp = s
            .execute_bash(&format!("cat {}", secret_path.display()), &mut |_| {})
            .unwrap();

        assert!(!cp.stdout.contains("do-not-leak"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn execute_bash_read_access_reflects_a_mount_allowlist_edit_without_recreating_the_session() {
        // Same read-path denial as the test above, but the fix is a
        // mount_allowlist config edit — no session recreation and no daemon
        // restart — mirroring
        // mount_reflects_a_config_file_edit_without_recreating_the_session,
        // just for session_bash's sandbox instead of session_mount.
        let extra_dir = tempfile::tempdir().unwrap();
        let extra_path = extra_dir.path().join("readable.txt");
        std::fs::write(&extra_path, b"now-readable").unwrap();

        let config_dir = tempfile::tempdir().unwrap();
        let config_path = config_dir.path().join("config.toml");
        std::fs::write(&config_path, "mount_allowlist = []\n").unwrap();

        let mut s = Session::new(ConfigHandle::new(
            Config::load_from(Some(&config_path)),
            Some(config_path.clone()),
        ))
        .unwrap();

        let cat_extra = format!("cat {}", extra_path.display());
        let cp = s.execute_bash(&cat_extra, &mut |_| {}).unwrap();
        assert!(!cp.stdout.contains("now-readable"));

        std::fs::write(
            &config_path,
            format!(
                "mount_allowlist = [{:?}]\n",
                extra_dir.path().to_str().unwrap()
            ),
        )
        .unwrap();

        let cp = s.execute_bash(&cat_extra, &mut |_| {}).unwrap();
        assert!(cp.stdout.contains("now-readable"));
    }

    #[test]
    fn descendant_pids_finds_a_grandchild_process() {
        // A plain "sleep 5" tail-call-execs into sh's own pid, so force a
        // genuine subshell fork to get a real two-level descendant chain.
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("(sleep 5 & wait) & wait")
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let descendants = Session::descendant_pids(child.id() as i32);
        assert_eq!(descendants.len(), 2, "expected a subshell and its sleep");

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn kill_all_running_children_kills_a_registered_process_group() {
        use std::os::unix::process::CommandExt;
        // Sandboxed children are spawned as their own process-group leader
        // (see sandbox.rs) so kill_process_tree's `-pgid` reaches them.
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .process_group(0)
            .spawn()
            .unwrap();
        let pgid = child.id() as i32;
        let guard = SessionChildGuard::new(pgid);

        Session::kill_all_running_children();

        let status = child.wait().unwrap();
        assert!(
            !status.success(),
            "child should have been killed, not exited cleanly"
        );

        drop(guard);
        assert!(
            !Session::running_child_pgids()
                .lock()
                .unwrap()
                .contains(&pgid),
            "dropping the guard should unregister the pgid"
        );
    }

    #[test]
    fn mount_reflects_a_config_file_edit_without_recreating_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let allowed_dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, b"hello").unwrap();

        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                "mount_allowlist = [{:?}]\n",
                allowed_dir.path().to_str().unwrap()
            ),
        )
        .unwrap();

        let mut s = Session::new(ConfigHandle::new(
            Config::load_from(Some(&config_path)),
            Some(config_path.clone()),
        ))
        .unwrap();

        assert!(s.mount(&file_path).is_err());

        // Editing the file on disk — no restart, no signal, no shared lock —
        // must be visible on the very next call against the same session.
        std::fs::write(
            &config_path,
            format!(
                "mount_allowlist = [{:?}, {:?}]\n",
                allowed_dir.path().to_str().unwrap(),
                dir.path().canonicalize().unwrap().to_str().unwrap()
            ),
        )
        .unwrap();

        assert!(s.mount(&file_path).is_ok());
    }

    #[test]
    fn mount_rejects_a_directory_that_exceeds_max_workspace_mb() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.bin"), vec![0u8; 1024]).unwrap();

        let config = Config {
            max_workspace_mb: Some(0),
            ..Config::default()
        };
        let mut s = Session::new(config.into()).unwrap();
        let err = s.mount(dir.path()).unwrap_err();
        assert!(err.to_string().contains("exceeds limit"));
        assert!(
            s.current().files.is_empty(),
            "a rejected mount must not partially populate the workspace"
        );
    }

    #[test]
    fn mount_ignores_a_symlink_cycle_instead_of_recursing_forever() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();

        let mut s = session();
        let mounted_keys = s.mount(dir.path()).unwrap();

        assert_eq!(mounted_keys, vec!["a.txt".to_string()]);
        assert!(!s.current().files.contains_key("loop"));
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
    fn rollback_then_write_prunes_intern_table_of_discarded_content() {
        let mut s = session();
        s.write_file("a.txt", b"one".to_vec()).unwrap(); // checkpoint 1
        s.write_file("a.txt", b"two".to_vec()).unwrap(); // checkpoint 2
        assert_eq!(s.intern_table.len(), 2, "one and two are both live");

        s.rollback(0).unwrap();
        s.write_file("a.txt", b"three".to_vec()).unwrap();

        assert_eq!(s.intern_table.len(), 1, "only three is still live");
    }

    #[test]
    fn squash_checkpoints_prunes_intern_table_of_the_absorbed_intermediate_content() {
        let mut s = session();
        s.write_file("a.txt", b"one".to_vec()).unwrap(); // checkpoint 1
        s.write_file("a.txt", b"two".to_vec()).unwrap(); // checkpoint 2
        assert_eq!(s.intern_table.len(), 2);

        s.squash_checkpoints(1, 2, None).unwrap();

        assert_eq!(s.intern_table.len(), 1, "only two survives the squash");
    }

    #[test]
    fn drop_checkpoints_prunes_intern_table_of_the_dropped_content() {
        let mut s = session();
        s.write_file("a.txt", b"one".to_vec()).unwrap(); // checkpoint 1
        s.write_file("a.txt", b"two".to_vec()).unwrap(); // checkpoint 2
        assert_eq!(s.intern_table.len(), 2);

        s.drop_checkpoints(1, 1).unwrap();

        assert_eq!(s.intern_table.len(), 1, "only two survives the drop");
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
                command: None,
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

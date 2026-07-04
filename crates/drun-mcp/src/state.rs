//! Serializable view types for session and checkpoint state. All functions
//! return JSON strings consumed directly by MCP tool responses.

use drun_core::{FileMap, Session, SnapshotMeta};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Serialize)]
struct SessionSummary {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_id: usize,
    checkpoint_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_checkpoint_id: Option<usize>,
}

#[derive(Serialize)]
struct CheckpointTreeNode {
    checkpoint_id: usize,
    is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    file_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    forks: Vec<SessionTreeNode>,
}

#[derive(Serialize)]
struct SnapshotEntry {
    path: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_count: usize,
}

#[derive(Serialize)]
struct SessionTreeNode {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    parent_session_id: Option<String>,
    parent_checkpoint_id: Option<usize>,
    checkpoints: Vec<CheckpointTreeNode>,
}

#[derive(Serialize)]
struct SessionState {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_checkpoint_id: Option<usize>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    workspace_file_count: usize,
    files_added_count: usize,
    files_modified_count: usize,
    files_removed_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    committed_files: Vec<String>,
}

#[derive(Serialize)]
struct CheckpointSummary {
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    file_count: usize,
    files_added_count: usize,
    files_modified_count: usize,
    files_removed_count: usize,
}

#[derive(Debug, PartialEq)]
struct FileDelta {
    added: Vec<String>,
    modified: Vec<String>,
    removed: Vec<String>,
}

impl FileDelta {
    fn compute(previous_files: Option<&FileMap>, current_files: &FileMap) -> FileDelta {
        let Some(previous) = previous_files else {
            return FileDelta {
                added: vec![],
                modified: vec![],
                removed: vec![],
            };
        };
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut removed = Vec::new();
        for key in current_files.keys() {
            if !previous.contains_key(key) {
                added.push(key.clone());
            } else {
                let cur = &current_files[key];
                let prev = &previous[key];
                if !Arc::ptr_eq(cur, prev) && cur != prev {
                    modified.push(key.clone());
                }
            }
        }
        for key in previous.keys() {
            if !current_files.contains_key(key) {
                removed.push(key.clone());
            }
        }
        added.sort();
        modified.sort();
        removed.sort();
        FileDelta {
            added,
            modified,
            removed,
        }
    }
}

pub(crate) fn build_session_list(sessions: &HashMap<String, Arc<Mutex<Session>>>) -> String {
    let summaries: Vec<SessionSummary> = sessions
        .iter()
        .map(|(id, arc)| {
            let session = arc.lock().unwrap();
            let (parent_session_id, parent_checkpoint_id) = match &session.parent {
                Some(r) => (Some(r.session_id.clone()), Some(r.checkpoint_id)),
                None => (None, None),
            };
            SessionSummary {
                session_id: id.clone(),
                label: session.label.clone(),
                checkpoint_id: session.current().id,
                checkpoint_count: session.history().len(),
                parent_session_id,
                parent_checkpoint_id,
            }
        })
        .collect();
    serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".into())
}

pub(crate) fn build_session_state(
    session_id: &str,
    session: &Session,
    previous_files: Option<&FileMap>,
    committed_files: Vec<String>,
) -> String {
    let current = session.current();
    let delta = FileDelta::compute(previous_files, &current.files);
    let (parent_session_id, parent_checkpoint_id) = match &session.parent {
        Some(r) => (Some(r.session_id.clone()), Some(r.checkpoint_id)),
        None => (None, None),
    };
    serde_json::to_string(&SessionState {
        session_id: session_id.to_string(),
        label: session.label.clone(),
        checkpoint_id: current.id,
        parent_session_id,
        parent_checkpoint_id,
        stdout_bytes: current.stdout.len(),
        stderr_bytes: current.stderr.len(),
        workspace_file_count: current.files.len(),
        files_added_count: delta.added.len(),
        files_modified_count: delta.modified.len(),
        files_removed_count: delta.removed.len(),
        committed_files,
    })
    .unwrap_or_else(|_| "{}".into())
}

pub(crate) fn build_checkpoint_history(session: &Session) -> String {
    let history = session.history();
    let summaries: Vec<CheckpointSummary> = history
        .iter()
        .enumerate()
        .map(|(index, checkpoint)| {
            let previous_files = if index > 0 {
                Some(&history[index - 1].files)
            } else {
                None
            };
            let delta = FileDelta::compute(previous_files, &checkpoint.files);
            CheckpointSummary {
                checkpoint_id: checkpoint.id,
                label: checkpoint.label.clone(),
                stdout_bytes: checkpoint.stdout.len(),
                stderr_bytes: checkpoint.stderr.len(),
                file_count: checkpoint.files.len(),
                files_added_count: delta.added.len(),
                files_modified_count: delta.modified.len(),
                files_removed_count: delta.removed.len(),
            }
        })
        .collect();
    serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".into())
}

pub(crate) fn build_snapshot_catalog(snapshots_dir: &Path) -> String {
    let Ok(entries) = std::fs::read_dir(snapshots_dir) else {
        return "[]".into();
    };
    let mut catalog: Vec<SnapshotEntry> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            p.extension().and_then(|x| x.to_str()) == Some("meta")
                && Path::new(p.file_stem().unwrap_or_default())
                    .extension()
                    .and_then(|x| x.to_str())
                    == Some("drun")
        })
        .filter_map(|e| {
            let meta_path = e.path();
            let drun_path = meta_path.with_extension("");
            let size_bytes = std::fs::metadata(&drun_path).ok()?.len();
            let meta = SnapshotMeta::decode(&std::fs::read(&meta_path).ok()?).ok()?;
            Some(SnapshotEntry {
                path: drun_path.to_string_lossy().into_owned(),
                size_bytes,
                label: meta.label,
                checkpoint_count: meta.checkpoint_count,
            })
        })
        .collect();
    catalog.sort_by(|a, b| a.path.cmp(&b.path));
    serde_json::to_string(&catalog).unwrap_or_else(|_| "[]".into())
}

fn build_session_node(
    session_id: &str,
    session: &Session,
    children: &HashMap<(String, usize), Vec<(String, &Session)>>,
) -> SessionTreeNode {
    let current_id = session.current().id;
    let checkpoints = session
        .history()
        .iter()
        .map(|cp| {
            let forks = children
                .get(&(session_id.to_string(), cp.id))
                .map(|kids| {
                    kids.iter()
                        .map(|(id, s)| build_session_node(id, s, children))
                        .collect()
                })
                .unwrap_or_default();
            CheckpointTreeNode {
                checkpoint_id: cp.id,
                is_current: cp.id == current_id,
                label: cp.label.clone(),
                stdout_bytes: cp.stdout.len(),
                stderr_bytes: cp.stderr.len(),
                file_count: cp.files.len(),
                forks,
            }
        })
        .collect();
    let (parent_session_id, parent_checkpoint_id) = match &session.parent {
        Some(r) => (Some(r.session_id.clone()), Some(r.checkpoint_id)),
        None => (None, None),
    };
    SessionTreeNode {
        session_id: session_id.to_string(),
        label: session.label.clone(),
        parent_session_id,
        parent_checkpoint_id,
        checkpoints,
    }
}

pub(crate) fn build_session_tree(sessions: &HashMap<String, Arc<Mutex<Session>>>) -> String {
    let locked_sessions: Vec<(String, std::sync::MutexGuard<Session>)> = sessions
        .iter()
        .map(|(id, arc)| (id.clone(), arc.lock().unwrap()))
        .collect();
    let mut children: HashMap<(String, usize), Vec<(String, &Session)>> = HashMap::new();
    let mut roots: Vec<(&str, &Session)> = Vec::new();

    for (id, session) in &locked_sessions {
        let session: &Session = session;
        let parent_exists = session
            .parent
            .as_ref()
            .is_some_and(|r| sessions.contains_key(&r.session_id));
        if parent_exists {
            let r = session.parent.as_ref().unwrap();
            children
                .entry((r.session_id.clone(), r.checkpoint_id))
                .or_default()
                .push((id.clone(), session));
        } else {
            roots.push((id.as_str(), session));
        }
    }

    let tree: Vec<SessionTreeNode> = roots
        .into_iter()
        .map(|(id, session)| build_session_node(id, session, &children))
        .collect();

    serde_json::to_string(&tree).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_map(pairs: &[(&str, &[u8])]) -> FileMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Arc::new(v.to_vec())))
            .collect()
    }

    #[test]
    fn compute_with_no_previous_reports_nothing() {
        let current = file_map(&[("a.txt", b"hello")]);
        let delta = FileDelta::compute(None, &current);
        assert_eq!(
            delta,
            FileDelta {
                added: vec![],
                modified: vec![],
                removed: vec![]
            }
        );
    }

    #[test]
    fn compute_detects_added_file() {
        let previous = file_map(&[]);
        let current = file_map(&[("new.txt", b"hello")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.added, vec!["new.txt".to_string()]);
        assert!(delta.modified.is_empty());
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn compute_detects_removed_file() {
        let previous = file_map(&[("gone.txt", b"hello")]);
        let current = file_map(&[]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.removed, vec!["gone.txt".to_string()]);
        assert!(delta.added.is_empty());
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_detects_modified_file_by_content() {
        let previous = file_map(&[("a.txt", b"old")]);
        let current = file_map(&[("a.txt", b"new")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.modified, vec!["a.txt".to_string()]);
    }

    #[test]
    fn compute_ignores_unchanged_file_even_with_different_arc_allocation() {
        // Same bytes, but two separate Arc allocations (not Arc::ptr_eq) — content
        // equality must win over pointer inequality here.
        let previous = file_map(&[("a.txt", b"same")]);
        let current = file_map(&[("a.txt", b"same")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_ignores_unchanged_file_sharing_the_same_arc() {
        let shared = Arc::new(b"same".to_vec());
        let mut previous = FileMap::new();
        previous.insert("a.txt".to_string(), Arc::clone(&shared));
        let mut current = FileMap::new();
        current.insert("a.txt".to_string(), shared);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_sorts_each_category() {
        let previous = file_map(&[("z_remove.txt", b"1"), ("m_remove.txt", b"1")]);
        let current = file_map(&[("z_add.txt", b"1"), ("a_add.txt", b"1")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(
            delta.added,
            vec!["a_add.txt".to_string(), "z_add.txt".to_string()]
        );
        assert_eq!(
            delta.removed,
            vec!["m_remove.txt".to_string(), "z_remove.txt".to_string()]
        );
    }
}

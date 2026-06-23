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
    packages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_checkpoint_id: Option<usize>,
}

const TREE_PREVIEW_LEN: usize = 200;

#[derive(Serialize)]
struct CheckpointTreeNode {
    checkpoint_id: usize,
    is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    stdout_preview: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    stderr_preview: String,
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
    packages: Vec<String>,
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
    #[serde(skip_serializing_if = "String::is_empty")]
    stdout: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    stderr: String,
    workspace: Vec<String>,
    packages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_modified: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_removed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    committed_files: Vec<String>,
}

#[derive(Serialize)]
struct CheckpointSummary {
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    stdout: String,
    file_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_modified: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_removed: Vec<String>,
}

struct FileDelta {
    added: Vec<String>,
    modified: Vec<String>,
    removed: Vec<String>,
}

fn file_delta(previous_files: Option<&FileMap>, current_files: &FileMap) -> FileDelta {
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
                packages: session.packages().to_vec(),
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
    let mut workspace: Vec<String> = current.files.keys().cloned().collect();
    workspace.sort();
    let delta = file_delta(previous_files, &current.files);
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
        stdout: current.stdout.clone(),
        stderr: current.stderr.clone(),
        workspace,
        packages: session.packages().to_vec(),
        files_added: delta.added,
        files_modified: delta.modified,
        files_removed: delta.removed,
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
            let delta = file_delta(previous_files, &checkpoint.files);
            CheckpointSummary {
                checkpoint_id: checkpoint.id,
                label: checkpoint.label.clone(),
                stdout: checkpoint.stdout.clone(),
                file_count: checkpoint.files.len(),
                files_added: delta.added,
                files_modified: delta.modified,
                files_removed: delta.removed,
            }
        })
        .collect();
    serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".into())
}

fn truncate_preview(s: &str) -> String {
    if s.len() <= TREE_PREVIEW_LEN {
        s.to_string()
    } else {
        format!("{}…", &s[..TREE_PREVIEW_LEN])
    }
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
                packages: meta.packages,
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
    orphan_parent: Option<(String, usize)>,
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
                        .map(|(id, s)| build_session_node(id, s, children, None))
                        .collect()
                })
                .unwrap_or_default();
            CheckpointTreeNode {
                checkpoint_id: cp.id,
                is_current: cp.id == current_id,
                label: cp.label.clone(),
                stdout_preview: truncate_preview(&cp.stdout),
                stderr_preview: truncate_preview(&cp.stderr),
                file_count: cp.files.len(),
                forks,
            }
        })
        .collect();
    let (parent_session_id, parent_checkpoint_id) = match orphan_parent {
        Some((sid, cid)) => (Some(sid), Some(cid)),
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
        let session: &Session = &*session;
        let parent_exists = session
            .parent
            .as_ref()
            .map_or(false, |r| sessions.contains_key(&r.session_id));
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
        .map(|(id, session)| {
            let orphan_parent = session.parent.as_ref().and_then(|r| {
                if !sessions.contains_key(&r.session_id) {
                    Some((r.session_id.clone(), r.checkpoint_id))
                } else {
                    None
                }
            });
            build_session_node(id, session, &children, orphan_parent)
        })
        .collect();

    serde_json::to_string(&tree).unwrap_or_else(|_| "[]".into())
}

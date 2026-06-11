use drun_core::{FileMap, Session};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Serialize)]
struct SessionSummary {
    session_id: String,
    checkpoint_id: usize,
    checkpoint_count: usize,
    packages: Vec<String>,
    timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_checkpoint_id: Option<usize>,
}

#[derive(Serialize)]
struct CheckpointTreeNode {
    checkpoint_id: usize,
    is_current: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    stdout: String,
    file_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    forks: Vec<SessionTreeNode>,
}

#[derive(Serialize)]
struct SessionTreeNode {
    session_id: String,
    checkpoints: Vec<CheckpointTreeNode>,
}

#[derive(Serialize)]
pub(crate) struct SessionState {
    pub session_id: String,
    pub checkpoint_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_checkpoint_id: Option<usize>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stderr: String,
    pub workspace: Vec<String>,
    pub packages: Vec<String>,
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_modified: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_removed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub committed_files: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct CheckpointSummary {
    pub checkpoint_id: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    pub file_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_modified: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_removed: Vec<String>,
}

fn file_delta(
    previous_files: Option<&FileMap>,
    current_files: &FileMap,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let Some(previous) = previous_files else {
        return (vec![], vec![], vec![]);
    };
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut removed = Vec::new();
    for key in current_files.keys() {
        if !previous.contains_key(key) {
            added.push(key.clone());
        } else if current_files[key] != previous[key] {
            modified.push(key.clone());
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
    (added, modified, removed)
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
                checkpoint_id: session.current().id,
                checkpoint_count: session.history().len(),
                packages: session.packages().to_vec(),
                timeout_ms: session.timeout_ms,
                parent_session_id,
                parent_checkpoint_id,
            }
        })
        .collect();
    serde_json::to_string(&summaries).unwrap()
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
    let (files_added, files_modified, files_removed) = file_delta(previous_files, &current.files);
    let (parent_session_id, parent_checkpoint_id) = match &session.parent {
        Some(r) => (Some(r.session_id.clone()), Some(r.checkpoint_id)),
        None => (None, None),
    };
    serde_json::to_string(&SessionState {
        session_id: session_id.to_string(),
        checkpoint_id: current.id,
        parent_session_id,
        parent_checkpoint_id,
        stdout: current.stdout.clone(),
        stderr: current.stderr.clone(),
        workspace,
        packages: session.packages().to_vec(),
        timeout_ms: session.timeout_ms,
        files_added,
        files_modified,
        files_removed,
        committed_files,
    })
    .unwrap()
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
            let (files_added, files_modified, files_removed) =
                file_delta(previous_files, &checkpoint.files);
            CheckpointSummary {
                checkpoint_id: checkpoint.id,
                stdout: checkpoint.stdout.clone(),
                file_count: checkpoint.files.len(),
                files_added,
                files_modified,
                files_removed,
            }
        })
        .collect();
    serde_json::to_string(&summaries).unwrap()
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
                stdout: cp.stdout.clone(),
                file_count: cp.files.len(),
                forks,
            }
        })
        .collect();
    SessionTreeNode {
        session_id: session_id.to_string(),
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
        match &session.parent {
            Some(r) => children
                .entry((r.session_id.clone(), r.checkpoint_id))
                .or_default()
                .push((id.clone(), session)),
            None => roots.push((id.as_str(), session)),
        }
    }

    let tree: Vec<SessionTreeNode> = roots
        .into_iter()
        .map(|(id, session)| build_session_node(id, session, &children))
        .collect();

    serde_json::to_string(&tree).unwrap()
}

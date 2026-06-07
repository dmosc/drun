//! Shared state serialization helpers used by all tool handlers.
//!
//! Every mutating tool returns a `SessionState` JSON blob so agents always
//! have a consistent picture of the session after each operation. Read-only
//! tools like `session_history` return their own purpose-built summaries, but
//! still use the shared `file_delta` primitive for computing what changed
//! between checkpoints.

use drun_core::Session;
use serde::Serialize;
use std::collections::HashMap;

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

/// Computes which files were added, modified, or removed between two workspace
/// snapshots. Returns sorted lists so the output is stable across calls.
fn file_delta(
    previous_files: Option<&HashMap<String, Vec<u8>>>,
    current_files: &HashMap<String, Vec<u8>>,
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

/// Builds the standard `SessionState` JSON returned after every mutating tool
/// call. `previous_files` is used to populate the file-delta fields; pass
/// `None` when no meaningful delta exists (e.g. `create_session`).
pub(crate) fn build_session_state(
    session_id: &str,
    session: &Session,
    previous_files: Option<&HashMap<String, Vec<u8>>>,
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

/// Builds a summary of every checkpoint in the session, each annotated with
/// the file delta relative to its predecessor. Useful for agents deciding
/// which checkpoint to roll back to.
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

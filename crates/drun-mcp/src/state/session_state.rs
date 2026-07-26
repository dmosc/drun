use crate::state::file_delta::FileDelta;
use drun_core::{CheckpointRef, FileMap, Session};
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SessionState {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    workspace_file_count: usize,
    files_added_count: usize,
    files_modified_count: usize,
    files_removed_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    committed_files: Vec<String>,
}

impl SessionState {
    pub(crate) fn compute(
        session_id: &str,
        session: &Session,
        previous_files: Option<&FileMap>,
        committed_files: Vec<String>,
    ) -> SessionState {
        let current = session.current();
        let delta = FileDelta::compute(previous_files, &current.files);
        let (parent_session_id, parent_checkpoint_id) = CheckpointRef::split(&session.parent);
        SessionState {
            session_id: session_id.to_string(),
            label: session.label.clone(),
            checkpoint_id: current.id,
            parent_session_id,
            parent_checkpoint_id,
            stdout_bytes: current.stdout.len(),
            stderr_bytes: current.stderr.len(),
            exit_code: current.exit_code,
            workspace_file_count: current.files.len(),
            files_added_count: delta.added.len(),
            files_modified_count: delta.modified.len(),
            files_removed_count: delta.removed.len(),
            committed_files,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drun_core::Config;

    fn new_session() -> Session {
        Session::new(Config::default().into()).unwrap()
    }

    #[test]
    fn session_state_compute_reports_zero_deltas_with_no_previous_files() {
        let session = new_session();
        let state = SessionState::compute("s1", &session, None, vec![]);
        assert_eq!(state.files_added_count, 0);
        assert_eq!(state.files_modified_count, 0);
        assert_eq!(state.files_removed_count, 0);
        assert!(state.committed_files.is_empty());
    }

    #[test]
    fn session_state_compute_counts_added_files_against_previous_snapshot() {
        let mut session = new_session();
        let previous_files = session.current().files.clone();
        session.write_file("a.txt", b"hi".to_vec(), None).unwrap();

        let state = SessionState::compute("s1", &session, Some(&previous_files), vec![]);
        assert_eq!(state.files_added_count, 1);
        assert_eq!(state.workspace_file_count, 1);
    }

    #[test]
    fn session_state_compute_passes_through_committed_files() {
        let session = new_session();
        let state = SessionState::compute(
            "s1",
            &session,
            None,
            vec!["a.txt".to_string(), "b.txt".to_string()],
        );
        assert_eq!(
            state.committed_files,
            vec!["a.txt".to_string(), "b.txt".to_string()]
        );
    }
}

use crate::state::file_delta::FileDelta;
use drun_core::Session;
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct CheckpointSummary {
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    file_count: usize,
    files_added_count: usize,
    files_modified_count: usize,
    files_removed_count: usize,
}

impl CheckpointSummary {
    pub(crate) fn history(session: &Session) -> Vec<CheckpointSummary> {
        let history = session.history();
        history
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
                    command: checkpoint.command.clone(),
                    exit_code: checkpoint.exit_code,
                    tool: checkpoint.tool.clone(),
                    description: checkpoint.description.clone(),
                    stdout_bytes: checkpoint.stdout.len(),
                    stderr_bytes: checkpoint.stderr.len(),
                    file_count: checkpoint.files.len(),
                    files_added_count: delta.added.len(),
                    files_modified_count: delta.modified.len(),
                    files_removed_count: delta.removed.len(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drun_core::{CheckpointRecord, Config, SessionSnapshot};
    use std::collections::HashMap;

    fn new_session() -> Session {
        Session::new(Config::default().into()).unwrap()
    }

    fn session_with_command(command: Option<String>) -> Session {
        let snapshot = SessionSnapshot {
            checkpoint_idx: 1,
            parent: None,
            label: None,
            overlays: HashMap::new(),
            blobs: vec![],
            checkpoints: vec![
                CheckpointRecord {
                    id: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    label: None,
                    command: None,
                    exit_code: None,
                    tool: None,
                    description: None,
                    files: HashMap::new(),
                },
                CheckpointRecord {
                    id: 1,
                    stdout: String::new(),
                    stderr: String::new(),
                    label: None,
                    command,
                    exit_code: None,
                    tool: None,
                    description: None,
                    files: HashMap::new(),
                },
            ],
        };
        Session::from_snapshot(Config::default().into(), snapshot).unwrap()
    }

    #[test]
    fn checkpoint_summary_history_reports_the_executed_command() {
        let session = session_with_command(Some("echo hi".to_string()));

        let history = CheckpointSummary::history(&session);
        assert_eq!(history[0].command, None);
        assert_eq!(history[1].command.as_deref(), Some("echo hi"));
    }

    #[test]
    fn checkpoint_summary_history_treats_first_checkpoint_as_having_no_delta() {
        let session = new_session();
        let history = CheckpointSummary::history(&session);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].files_added_count, 0);
    }

    #[test]
    fn checkpoint_summary_history_diffs_against_the_prior_checkpoint() {
        let mut session = new_session();
        session
            .write_files(
                vec![("a.txt".to_string(), b"hi".to_vec())],
                "session_write_file",
                None,
            )
            .unwrap();
        session
            .write_files(
                vec![("b.txt".to_string(), b"hi".to_vec())],
                "session_write_file",
                None,
            )
            .unwrap();

        let history = CheckpointSummary::history(&session);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].files_added_count, 0);
        assert_eq!(history[1].files_added_count, 1); // a.txt
        assert_eq!(history[2].files_added_count, 1); // b.txt, not a.txt again
    }
}

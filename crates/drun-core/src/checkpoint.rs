//! Core value types: FileMap, Checkpoint, and CheckpointRef.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub type FileMap = HashMap<String, Arc<Vec<u8>>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: usize,
    pub stdout: String,
    pub stderr: String,
    pub files: FileMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The caller's one-sentence account of why this operation happened
    /// (e.g. "installing pytest to run the test suite"). `None` for the
    /// initial checkpoint and for operations that don't take one (squash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Checkpoint {
    pub(crate) fn empty(id: usize, files: FileMap) -> Checkpoint {
        Checkpoint {
            id,
            stdout: String::new(),
            stderr: String::new(),
            files,
            label: None,
            command: None,
            exit_code: None,
            tool: None,
            description: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub session_id: String,
    pub checkpoint_id: usize,
}

impl CheckpointRef {
    /// Splits an optional parent reference into the (session_id, checkpoint_id)
    /// pair shape serializable views represent it as.
    pub fn split(parent: &Option<CheckpointRef>) -> (Option<String>, Option<usize>) {
        match parent {
            Some(r) => (Some(r.session_id.clone()), Some(r.checkpoint_id)),
            None => (None, None),
        }
    }
}

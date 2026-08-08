//! Core value types: FileMap, Checkpoint, and CheckpointRef.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub type FileMap = HashMap<String, Arc<Vec<u8>>>;

/// Tool call steps within a checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub tool: String,
    pub description: String,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
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
            steps: Vec::new(),
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

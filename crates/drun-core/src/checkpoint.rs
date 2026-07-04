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
}

impl Checkpoint {
    pub(crate) fn empty(id: usize, files: FileMap) -> Checkpoint {
        Checkpoint {
            id,
            stdout: String::new(),
            stderr: String::new(),
            files,
            label: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
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

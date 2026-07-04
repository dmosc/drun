//! Core value types: FileMap, Checkpoint, and CheckpointRef.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub type FileMap = HashMap<String, Arc<Vec<u8>>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: usize,
    pub stdout: String,
    pub stderr: String,
    pub files: FileMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub session_id: String,
    pub checkpoint_id: usize,
}

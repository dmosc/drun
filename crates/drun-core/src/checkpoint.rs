use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type FileMap = HashMap<String, Vec<u8>>;

pub struct Checkpoint {
    pub id: usize,
    pub stdout: String,
    pub stderr: String,
    pub files: FileMap,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub session_id: String,
    pub checkpoint_id: usize,
}

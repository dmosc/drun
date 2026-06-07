//! Snapshot of sandbox state after a single execution step, holding stdout and
//! all workspace files.

use std::collections::HashMap;

pub struct Checkpoint {
    pub id: usize,
    pub stdout: String,
    pub stderr: String,
    pub files: HashMap<String, Vec<u8>>,
}

pub struct CheckpointRef {
    pub session_id: String,
    pub checkpoint_id: usize,
}

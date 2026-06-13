use crate::{CheckpointRef, FileMap};
use anyhow::Result;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 4] = b"DRUN";
const COMPRESSION_LEVEL: i32 = 3;

#[derive(Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    pub id: usize,
    pub stdout: String,
    pub stderr: String,
    pub files: FileMap,
}

#[derive(Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub allowed_hosts: Vec<String>,
    pub timeout_ms: u64,
    pub max_workspace_bytes: Option<u64>,
    pub checkpoint_idx: usize,
    pub packages: Vec<String>,
    pub parent: Option<CheckpointRef>,
    pub checkpoints: Vec<CheckpointSnapshot>,
}

impl SessionSnapshot {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;
        let compressed = zstd::encode_all(json.as_slice(), COMPRESSION_LEVEL)?;
        let mut out = Vec::with_capacity(MAGIC.len() + compressed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&compressed);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        anyhow::ensure!(
            bytes.starts_with(MAGIC),
            "not a valid .drun snapshot: missing magic bytes"
        );
        let decompressed = zstd::decode_all(&bytes[MAGIC.len()..])?;
        Ok(serde_json::from_slice(&decompressed)?)
    }
}

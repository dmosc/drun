//! Session persistence: encode/decode the full checkpoint history to/from a
//! zstd-compressed .drun file, plus a lightweight .drun.meta sidecar.

use crate::CheckpointRef;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"DRUN";
const COMPRESSION_LEVEL: i32 = 3;

#[derive(Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub id: usize,
    pub stdout: String,
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub files: HashMap<String, usize>,
}

#[derive(Serialize, Deserialize)]
pub struct SnapshotMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub packages: Vec<String>,
    pub checkpoint_count: usize,
}

impl SnapshotMeta {
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

#[derive(Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub checkpoint_idx: usize,
    pub packages: Vec<String>,
    pub parent: Option<CheckpointRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub origins: HashMap<String, PathBuf>,
    #[serde(default)]
    pub overlays: HashMap<String, PathBuf>,
    pub blobs: Vec<Vec<u8>>,
    pub checkpoints: Vec<CheckpointRecord>,
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

    pub fn write(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.encode()?)?;
        let meta = SnapshotMeta {
            label: self.label.clone(),
            packages: self.packages.clone(),
            checkpoint_count: self.checkpoints.len(),
        };
        std::fs::write(path.with_extension("drun.meta"), meta.encode()?)?;
        Ok(())
    }
}

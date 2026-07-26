//! Session persistence: encode/decode the full checkpoint history to/from a
//! zstd-compressed .drun file, plus a lightweight .drun.meta sidecar.

use crate::interner::Interner;
use crate::mounts::MountTable;
use crate::session::Session;
use crate::{Checkpoint, CheckpointRef, ConfigHandle, FileMap};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"DRUN";
const COMPRESSION_LEVEL: i32 = 3;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub id: usize,
    pub stdout: String,
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub files: HashMap<String, usize>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub checkpoint_idx: usize,
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
            checkpoint_count: self.checkpoints.len(),
        };
        std::fs::write(path.with_extension("drun.meta"), meta.encode()?)?;
        Ok(())
    }

    /// Encodes `session`'s full checkpoint history, deduplicating identical
    /// blobs by pointer identity so shared content is stored once.
    pub(crate) fn from_session(session: &Session) -> Self {
        let mut blob_ptr_to_index: HashMap<usize, usize> = HashMap::new();
        let mut blobs: Vec<Vec<u8>> = Vec::new();

        let checkpoints = session
            .history()
            .iter()
            .map(|checkpoint| {
                let files = checkpoint
                    .files
                    .iter()
                    .map(|(key, arc)| {
                        let ptr = Arc::as_ptr(arc) as usize;
                        let blob_index = *blob_ptr_to_index.entry(ptr).or_insert_with(|| {
                            let idx = blobs.len();
                            blobs.push((**arc).clone());
                            idx
                        });
                        (key.clone(), blob_index)
                    })
                    .collect();
                CheckpointRecord {
                    id: checkpoint.id,
                    stdout: checkpoint.stdout.clone(),
                    stderr: checkpoint.stderr.clone(),
                    label: checkpoint.label.clone(),
                    command: checkpoint.command.clone(),
                    exit_code: checkpoint.exit_code,
                    tool: checkpoint.tool.clone(),
                    files,
                }
            })
            .collect();

        Self {
            checkpoint_idx: session.checkpoint_idx(),
            parent: session.parent.clone(),
            label: session.label.clone(),
            origins: session.mounts().origins().clone(),
            overlays: session.mounts().overlays().clone(),
            blobs,
            checkpoints,
        }
    }

    /// Rebuilds a `Session` from a decoded snapshot, re-seeding the interner
    /// so already-shared blobs stay deduplicated, and dropping any origin or
    /// overlay whose host path has since disappeared.
    pub(crate) fn restore(self, config: ConfigHandle) -> Result<Session> {
        let blob_arcs: Vec<Arc<Vec<u8>>> = self.blobs.into_iter().map(Arc::new).collect();
        let checkpoints: Vec<Checkpoint> = self
            .checkpoints
            .into_iter()
            .map(|record| {
                let files: FileMap = record
                    .files
                    .into_iter()
                    .map(|(key, blob_index)| (key, Arc::clone(&blob_arcs[blob_index])))
                    .collect();
                Checkpoint {
                    id: record.id,
                    stdout: record.stdout,
                    stderr: record.stderr,
                    label: record.label,
                    command: record.command,
                    exit_code: record.exit_code,
                    tool: record.tool,
                    files,
                }
            })
            .collect();

        let mut interner = Interner::default();
        for checkpoint in &checkpoints {
            for arc in checkpoint.files.values() {
                interner.seed(arc);
            }
        }

        let mounts = MountTable::from_raw(self.origins, self.overlays).prune_missing();

        Ok(Session::from_parts(
            config,
            checkpoints,
            self.checkpoint_idx,
            mounts,
            interner,
            self.label,
            self.parent,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> SessionSnapshot {
        SessionSnapshot {
            checkpoint_idx: 1,
            parent: None,
            label: Some("my-session".to_string()),
            origins: HashMap::new(),
            overlays: HashMap::new(),
            blobs: vec![b"hello".to_vec(), b"world".to_vec()],
            checkpoints: vec![
                CheckpointRecord {
                    id: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    label: None,
                    command: None,
                    exit_code: None,
                    tool: None,
                    files: HashMap::new(),
                },
                CheckpointRecord {
                    id: 1,
                    stdout: "ok".to_string(),
                    stderr: String::new(),
                    label: Some("cp1".to_string()),
                    command: Some("echo ok".to_string()),
                    exit_code: Some(0),
                    tool: Some("session_bash".to_string()),
                    files: [("a.txt".to_string(), 0)].into_iter().collect(),
                },
            ],
        }
    }

    #[test]
    fn session_snapshot_round_trips_through_encode_decode() {
        let snapshot = sample_snapshot();
        let decoded = SessionSnapshot::decode(&snapshot.encode().unwrap()).unwrap();
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn session_snapshot_decode_rejects_missing_magic_bytes() {
        let result = SessionSnapshot::decode(b"not a real snapshot");
        assert!(result.is_err());
    }

    #[test]
    fn session_snapshot_decode_rejects_truncated_data() {
        let snapshot = sample_snapshot();
        let mut encoded = snapshot.encode().unwrap();
        encoded.truncate(6); // keeps the magic bytes, drops most of the compressed payload
        assert!(SessionSnapshot::decode(&encoded).is_err());
    }

    #[test]
    fn snapshot_meta_round_trips_through_encode_decode() {
        let meta = SnapshotMeta {
            label: Some("checkpoint-1".to_string()),
            checkpoint_count: 4,
        };
        let decoded = SnapshotMeta::decode(&meta.encode().unwrap()).unwrap();
        assert_eq!(decoded, meta);
    }

    #[test]
    fn snapshot_meta_omits_label_field_when_none() {
        let meta = SnapshotMeta {
            label: None,
            checkpoint_count: 1,
        };
        let json = String::from_utf8(meta.encode().unwrap()).unwrap();
        assert!(!json.contains("label"));
    }

    #[test]
    fn write_produces_both_the_snapshot_and_meta_sidecar_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.drun");
        let snapshot = sample_snapshot();
        snapshot.write(&path).unwrap();

        assert!(path.exists());
        let meta_path = path.with_extension("drun.meta");
        assert!(meta_path.exists());

        let meta = SnapshotMeta::decode(&std::fs::read(&meta_path).unwrap()).unwrap();
        assert_eq!(meta.checkpoint_count, snapshot.checkpoints.len());
        assert_eq!(meta.label, snapshot.label);
    }
}

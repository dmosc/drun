use drun_core::SnapshotMeta;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SnapshotEntry {
    path: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_count: usize,
}

impl SnapshotEntry {
    pub(crate) fn catalog(snapshots_dir: &Path) -> Vec<SnapshotEntry> {
        let Ok(entries) = std::fs::read_dir(snapshots_dir) else {
            return vec![];
        };
        let mut catalog: Vec<SnapshotEntry> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let p = e.path();
                p.extension().and_then(|x| x.to_str()) == Some("meta")
                    && Path::new(p.file_stem().unwrap_or_default())
                        .extension()
                        .and_then(|x| x.to_str())
                        == Some("drun")
            })
            .filter_map(|e| {
                let meta_path = e.path();
                let drun_path = meta_path.with_extension("");
                let size_bytes = std::fs::metadata(&drun_path).ok()?.len();
                let meta = SnapshotMeta::decode(&std::fs::read(&meta_path).ok()?).ok()?;
                Some(SnapshotEntry {
                    path: drun_path.to_string_lossy().into_owned(),
                    size_bytes,
                    label: meta.label,
                    checkpoint_count: meta.checkpoint_count,
                })
            })
            .collect();
        catalog.sort_by(|a, b| a.path.cmp(&b.path));
        catalog
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drun_core::{Config, Session};

    fn new_session() -> Session {
        Session::new(Config::default().into()).unwrap()
    }

    #[test]
    fn snapshot_entry_catalog_returns_empty_for_missing_directory() {
        let catalog = SnapshotEntry::catalog(Path::new("/nonexistent/drun-snapshots-test"));
        assert!(catalog.is_empty());
    }

    #[test]
    fn snapshot_entry_catalog_returns_empty_for_directory_with_no_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"hello").unwrap();
        let catalog = SnapshotEntry::catalog(dir.path());
        assert!(catalog.is_empty());
    }

    #[test]
    fn snapshot_entry_catalog_finds_a_real_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = new_session();
        session.set_label("checkpoint-1".to_string());
        session
            .write_files(
                vec![("a.txt".to_string(), b"hi".to_vec())],
                "session_write_file",
                None,
            )
            .unwrap();
        let snapshot_path = dir.path().join("session.drun");
        session.snapshot().write(&snapshot_path).unwrap();

        let catalog = SnapshotEntry::catalog(dir.path());
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].checkpoint_count, 2);
    }
}

//! Tracks host directories overlaid read-only into the sandbox (`overlays`)
//! — e.g. `node_modules`, `.venv`, `target` — so `session_bash` can read
//! them without copying their contents into the workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// (workspace key, file bytes) for regular files discovered under a mount.
pub(crate) type ScannedFiles = Vec<(String, Vec<u8>)>;
/// (workspace key, host path) for directories matching `mount_overlay_paths`.
pub(crate) type ScannedOverlays = Vec<(String, PathBuf)>;

#[derive(Default, Clone)]
pub(crate) struct MountTable {
    overlays: HashMap<String, PathBuf>,
}

impl MountTable {
    pub(crate) fn from_raw(overlays: HashMap<String, PathBuf>) -> Self {
        Self { overlays }
    }

    pub(crate) fn record_overlay(&mut self, key: String, host_path: PathBuf) {
        self.overlays.insert(key, host_path);
    }

    pub(crate) fn overlays(&self) -> &HashMap<String, PathBuf> {
        &self.overlays
    }

    /// Drops overlays whose host path no longer exists — used when
    /// restoring from a snapshot, since they may have disappeared since.
    pub(crate) fn prune_missing(self) -> Self {
        Self {
            overlays: self
                .overlays
                .into_iter()
                .filter(|(_, p)| p.exists())
                .collect(),
        }
    }

    /// Recursively walks `dir`, splitting entries into plain files versus
    /// directories matching `overlay_patterns` (which get overlaid, not
    /// copied). Skips symlinks to avoid cyclical recursion.
    pub(crate) fn scan(
        dir: &Path,
        key_prefix: &str,
        overlay_patterns: &[String],
    ) -> anyhow::Result<(ScannedFiles, ScannedOverlays)> {
        let mut file_entries = vec![];
        let mut overlay_entries = vec![];
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = if key_prefix.is_empty() {
                name.clone()
            } else {
                format!("{key_prefix}/{name}")
            };
            let path = entry.path();
            if file_type.is_dir() {
                if overlay_patterns.iter().any(|p| p == &name) {
                    overlay_entries.push((key, path));
                } else {
                    let (sub_files, sub_overlays) = Self::scan(&path, &key, overlay_patterns)?;
                    file_entries.extend(sub_files);
                    overlay_entries.extend(sub_overlays);
                }
            } else if file_type.is_file() {
                file_entries.push((key, std::fs::read(&path)?));
            }
        }
        Ok((file_entries, overlay_entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_reads_a_flat_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let (files, overlays) = MountTable::scan(dir.path(), "", &[]).unwrap();
        assert_eq!(files, vec![("a.txt".to_string(), b"hello".to_vec())]);
        assert!(overlays.is_empty());
    }

    #[test]
    fn scan_builds_slash_joined_keys_for_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), b"nested").unwrap();

        let (files, _) = MountTable::scan(dir.path(), "", &[]).unwrap();
        assert_eq!(files[0].0.as_str(), "sub/b.txt");
    }

    #[test]
    fn scan_treats_matching_directories_as_overlays_not_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules/pkg.js"), b"ignored").unwrap();
        std::fs::write(dir.path().join("real.txt"), b"kept").unwrap();

        let overlay_patterns = vec!["node_modules".to_string()];
        let (files, overlays) = MountTable::scan(dir.path(), "", &overlay_patterns).unwrap();

        assert_eq!(files, vec![("real.txt".to_string(), b"kept".to_vec())]);
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].0.as_str(), "node_modules");
        assert_eq!(overlays[0].1, dir.path().join("node_modules"));
    }

    #[test]
    fn scan_respects_key_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();

        let (files, _) = MountTable::scan(dir.path(), "prefix", &[]).unwrap();
        assert_eq!(files[0].0.as_str(), "prefix/a.txt");
    }

    #[test]
    fn prune_missing_drops_an_overlay_whose_host_path_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut mounts = MountTable::default();
        mounts.record_overlay("present".to_string(), dir.path().to_path_buf());
        mounts.record_overlay("gone".to_string(), dir.path().join("does-not-exist"));

        let pruned = mounts.prune_missing();
        assert_eq!(pruned.overlays().len(), 1);
        assert!(pruned.overlays().contains_key("present"));
    }
}

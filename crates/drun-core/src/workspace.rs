//! Mirrors a session's FileMap onto disk so a sandboxed command can operate
//! on real files, then reports back what changed.

use crate::FileMap;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

struct SyncedFile {
    content: Arc<Vec<u8>>,
    mtime: SystemTime,
}

pub(crate) struct Workspace {
    dir: tempfile::TempDir,
    synced: HashMap<String, SyncedFile>,
}

impl Workspace {
    pub(crate) fn new() -> anyhow::Result<Self> {
        Ok(Self {
            dir: tempfile::TempDir::new()?,
            synced: HashMap::new(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    pub(crate) fn sync_to(&mut self, files: &FileMap) -> anyhow::Result<()> {
        let removed: Vec<String> = self
            .synced
            .keys()
            .filter(|key| !files.contains_key(*key))
            .cloned()
            .collect();
        for key in removed {
            std::fs::remove_file(self.dir.path().join(&key))?;
            self.synced.remove(&key);
        }

        for (key, content) in files {
            if self
                .synced
                .get(key)
                .is_some_and(|synced| Arc::ptr_eq(&synced.content, content))
            {
                continue;
            }
            let dest = self.dir.path().join(key);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, content.as_slice())?;
            let mtime = std::fs::metadata(&dest)?.modified()?;
            self.synced.insert(
                key.clone(),
                SyncedFile {
                    content: Arc::clone(content),
                    mtime,
                },
            );
        }
        Ok(())
    }

    pub(crate) fn collect_changes(&mut self) -> anyhow::Result<FileMap> {
        let mut files = FileMap::new();
        for entry in walkdir::WalkDir::new(self.dir.path()) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let key = entry
                .path()
                .strip_prefix(self.dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let metadata = entry.metadata()?;
            let mtime = metadata.modified()?;
            let content = match self.synced.get(&key) {
                Some(synced)
                    if synced.mtime == mtime && synced.content.len() as u64 == metadata.len() =>
                {
                    Arc::clone(&synced.content)
                }
                _ => Arc::new(std::fs::read(entry.path())?),
            };
            self.synced.insert(
                key.clone(),
                SyncedFile {
                    content: Arc::clone(&content),
                    mtime,
                },
            );
            files.insert(key, content);
        }
        self.synced.retain(|key, _| files.contains_key(key));
        Ok(files)
    }

    pub(crate) fn collect(dir: &Path) -> anyhow::Result<FileMap> {
        let mut files = FileMap::new();
        for entry in walkdir::WalkDir::new(dir) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let key = entry
                .path()
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            files.insert(key, Arc::new(std::fs::read(entry.path())?));
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_map(pairs: &[(&str, &[u8])]) -> FileMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Arc::new(v.to_vec())))
            .collect()
    }

    #[test]
    fn collect_reads_flat_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let collected = Workspace::collect(dir.path()).unwrap();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected["a.txt"].as_slice(), b"hello");
    }

    #[test]
    fn collect_uses_forward_slash_relative_keys_for_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), b"fn main() {}").unwrap();
        let collected = Workspace::collect(dir.path()).unwrap();
        assert!(collected.contains_key("src/main.rs"));
    }

    #[test]
    fn collect_ignores_empty_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("empty_dir")).unwrap();
        let collected = Workspace::collect(dir.path()).unwrap();
        assert!(collected.is_empty());
    }

    #[test]
    fn collect_on_empty_directory_returns_empty_filemap() {
        let dir = tempfile::tempdir().unwrap();
        let collected = Workspace::collect(dir.path()).unwrap();
        assert!(collected.is_empty());
    }

    #[test]
    fn sync_to_writes_new_files() {
        let mut workspace = Workspace::new().unwrap();
        workspace
            .sync_to(&file_map(&[("a.txt", b"hello")]))
            .unwrap();
        assert_eq!(
            std::fs::read(workspace.path().join("a.txt")).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn sync_to_skips_rewriting_a_file_whose_arc_is_unchanged() {
        let mut workspace = Workspace::new().unwrap();
        let files = file_map(&[("a.txt", b"original")]);
        workspace.sync_to(&files).unwrap();

        std::fs::write(workspace.path().join("a.txt"), b"changed-by-command").unwrap();
        workspace.sync_to(&files).unwrap();

        assert_eq!(
            std::fs::read(workspace.path().join("a.txt")).unwrap(),
            b"changed-by-command"
        );
    }

    #[test]
    fn sync_to_rewrites_a_file_whose_content_changed() {
        let mut workspace = Workspace::new().unwrap();
        workspace.sync_to(&file_map(&[("a.txt", b"one")])).unwrap();
        workspace.sync_to(&file_map(&[("a.txt", b"two")])).unwrap();
        assert_eq!(
            std::fs::read(workspace.path().join("a.txt")).unwrap(),
            b"two"
        );
    }

    #[test]
    fn sync_to_removes_a_file_no_longer_in_the_target_map() {
        let mut workspace = Workspace::new().unwrap();
        workspace
            .sync_to(&file_map(&[("a.txt", b"hello"), ("b.txt", b"world")]))
            .unwrap();
        workspace
            .sync_to(&file_map(&[("a.txt", b"hello")]))
            .unwrap();
        assert!(!workspace.path().join("b.txt").exists());
    }

    #[test]
    fn collect_changes_reuses_the_arc_for_an_untouched_file() {
        let mut workspace = Workspace::new().unwrap();
        let files = file_map(&[("a.txt", b"hello")]);
        workspace.sync_to(&files).unwrap();

        let collected = workspace.collect_changes().unwrap();
        assert!(Arc::ptr_eq(&collected["a.txt"], &files["a.txt"]));
    }

    #[test]
    fn collect_changes_reads_fresh_content_for_a_modified_file() {
        let mut workspace = Workspace::new().unwrap();
        workspace
            .sync_to(&file_map(&[("a.txt", b"short")]))
            .unwrap();

        std::fs::write(workspace.path().join("a.txt"), b"a much longer value").unwrap();

        let collected = workspace.collect_changes().unwrap();
        assert_eq!(collected["a.txt"].as_slice(), b"a much longer value");
    }

    #[test]
    fn collect_changes_omits_a_file_the_command_deleted() {
        let mut workspace = Workspace::new().unwrap();
        workspace
            .sync_to(&file_map(&[("a.txt", b"hello"), ("b.txt", b"world")]))
            .unwrap();

        std::fs::remove_file(workspace.path().join("b.txt")).unwrap();

        let collected = workspace.collect_changes().unwrap();
        assert!(!collected.contains_key("b.txt"));
    }

    #[test]
    fn collect_changes_picks_up_a_file_the_command_created() {
        let mut workspace = Workspace::new().unwrap();
        workspace
            .sync_to(&file_map(&[("a.txt", b"hello")]))
            .unwrap();

        std::fs::write(workspace.path().join("new.txt"), b"created").unwrap();

        let collected = workspace.collect_changes().unwrap();
        assert_eq!(collected["new.txt"].as_slice(), b"created");
    }
}

//! Workspace helpers: materialize a FileMap onto the filesystem and collect it
//! back after execution.

use crate::FileMap;
use std::path::Path;
use std::sync::Arc;

pub(crate) fn materialize(files: &FileMap, dir: &Path) -> anyhow::Result<()> {
    for (key, bytes) in files {
        let dest = dir.join(key);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, bytes.as_slice())?;
    }
    Ok(())
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
    fn materialize_writes_flat_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let files = file_map(&[("a.txt", b"hello")]);
        materialize(&files, dir.path()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"hello");
    }

    #[test]
    fn materialize_creates_nested_directories() {
        let dir = tempfile::tempdir().unwrap();
        let files = file_map(&[("src/main.rs", b"fn main() {}")]);
        materialize(&files, dir.path()).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("src/main.rs")).unwrap(),
            b"fn main() {}"
        );
    }

    #[test]
    fn materialize_with_empty_filemap_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        materialize(&FileMap::new(), dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn collect_reads_flat_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let collected = collect(dir.path()).unwrap();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected["a.txt"].as_slice(), b"hello");
    }

    #[test]
    fn collect_uses_forward_slash_relative_keys_for_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), b"fn main() {}").unwrap();
        let collected = collect(dir.path()).unwrap();
        assert!(collected.contains_key("src/main.rs"));
    }

    #[test]
    fn collect_ignores_empty_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("empty_dir")).unwrap();
        let collected = collect(dir.path()).unwrap();
        assert!(collected.is_empty());
    }

    #[test]
    fn collect_on_empty_directory_returns_empty_filemap() {
        let dir = tempfile::tempdir().unwrap();
        let collected = collect(dir.path()).unwrap();
        assert!(collected.is_empty());
    }

    #[test]
    fn materialize_then_collect_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let files = file_map(&[("a.txt", b"hello"), ("src/main.rs", b"fn main() {}")]);
        materialize(&files, dir.path()).unwrap();
        let collected = collect(dir.path()).unwrap();
        assert_eq!(collected, files);
    }
}

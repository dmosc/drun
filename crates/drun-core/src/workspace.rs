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

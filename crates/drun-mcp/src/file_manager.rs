//! Safe file writes: temp file + rename, so a reader never observes a
//! half-written file.

use std::path::Path;

pub(crate) struct FileManager;

impl FileManager {
    pub(crate) fn write(path: &Path, contents: &str) -> Result<(), String> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, contents)
            .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

//! Python-facing data types exposed via PyO3.

use pyo3::prelude::*;
use std::collections::HashMap;

#[pyclass]
pub struct DrunCheckpoint {
    #[pyo3(get)]
    pub id: usize,
    #[pyo3(get)]
    pub stdout: String,
    #[pyo3(get)]
    pub stderr: String,
    #[pyo3(get)]
    pub files: HashMap<String, Vec<u8>>,
}

pub fn checkpoint_to_py(c: &drun_core::Checkpoint) -> DrunCheckpoint {
    DrunCheckpoint {
        id: c.id,
        stdout: c.stdout.clone(),
        stderr: c.stderr.clone(),
        files: c
            .files
            .iter()
            .map(|(k, arc)| (k.clone(), (**arc).clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn checkpoint_to_py_maps_each_field_without_swapping_them() {
        let mut files = HashMap::new();
        files.insert("a.txt".to_string(), Arc::new(b"contents".to_vec()));
        let checkpoint = drun_core::Checkpoint {
            id: 3,
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            files,
            label: Some("milestone".to_string()),
        };

        let py_checkpoint = checkpoint_to_py(&checkpoint);

        assert_eq!(py_checkpoint.id, 3);
        assert_eq!(py_checkpoint.stdout, "out");
        assert_eq!(py_checkpoint.stderr, "err");
        assert_eq!(
            py_checkpoint.files.get("a.txt"),
            Some(&b"contents".to_vec())
        );
    }
}

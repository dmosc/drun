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

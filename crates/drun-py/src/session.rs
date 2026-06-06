//! `DrunSession` Python class: wraps a core `Session` behind a `Mutex` and
//! exposes execute, rollback, and history.

use crate::types::{DrunCheckpoint, checkpoint_to_py};
use drun_core::{DrunEngine, NetworkPolicy, Session, read_host_path};
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::sync::Mutex;

#[pyclass]
pub struct DrunSession {
    inner: Mutex<Session>,
}

#[pymethods]
impl DrunSession {
    #[new]
    #[pyo3(signature = (network=None))]
    pub fn new(network: Option<String>) -> PyResult<Self> {
        let engine = DrunEngine::new().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let policy = match network.as_deref() {
            Some("full") => NetworkPolicy::Full,
            Some("none") => NetworkPolicy::None,
            _ => NetworkPolicy::Packages,
        };
        let session =
            Session::new(&engine, policy).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(Self {
            inner: Mutex::new(session),
        })
    }

    pub fn mount(&self, path: String) -> PyResult<()> {
        let files = read_host_path(std::path::Path::new(&path))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        self.inner.lock().unwrap().mount(files);
        Ok(())
    }

    pub fn execute(&self, code: String) -> PyResult<DrunCheckpoint> {
        self.inner
            .lock()
            .unwrap()
            .execute(&code)
            .map(checkpoint_to_py)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    pub fn rollback(&self, id: usize) -> PyResult<()> {
        self.inner
            .lock()
            .unwrap()
            .rollback(id)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    #[getter]
    pub fn current(&self) -> DrunCheckpoint {
        checkpoint_to_py(self.inner.lock().unwrap().current())
    }

    #[getter]
    pub fn history(&self) -> Vec<DrunCheckpoint> {
        self.inner
            .lock()
            .unwrap()
            .history()
            .iter()
            .map(checkpoint_to_py)
            .collect()
    }
}

//! `DrunSession` Python class: wraps a core `Session` and exposes all session
//! operations to Python.

use crate::types::{DrunCheckpoint, checkpoint_to_py};
use drun_core::{DrunEngine, NetworkPolicy, Session};
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

    pub fn mount(&self, path: String) -> PyResult<Vec<String>> {
        self.inner
            .lock()
            .unwrap()
            .mount(std::path::Path::new(&path))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    pub fn commit(&self, keys: Option<Vec<String>>) -> PyResult<Vec<String>> {
        self.inner
            .lock()
            .unwrap()
            .commit(keys)
            .map(|paths| {
                paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            })
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    #[pyo3(signature = (from_id=0, to_id=None))]
    pub fn diff(&self, from_id: usize, to_id: Option<usize>) -> PyResult<String> {
        let inner = self.inner.lock().unwrap();
        let to = to_id.unwrap_or_else(|| inner.current().id);
        inner
            .diff(from_id, to)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
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

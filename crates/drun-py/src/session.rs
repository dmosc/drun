//! `DrunSession` Python class: wraps a core `Session` behind a `Mutex` and
//! exposes execute, rollback, and history.

use crate::types::{DrunCheckpoint, checkpoint_to_py};
use drun_core::{DrunEngine, NetworkPolicy, Session};
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::collections::HashMap;
use std::sync::Mutex;

#[pyclass]
pub struct DrunSession {
    inner: Mutex<Session>,
}

#[pymethods]
impl DrunSession {
    #[new]
    #[pyo3(signature = (files=None, network=None))]
    pub fn new(files: Option<HashMap<String, Vec<u8>>>, network: Option<String>) -> PyResult<Self> {
        let engine = DrunEngine::new().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let policy = match network.as_deref() {
            Some("full") => NetworkPolicy::Full,
            Some("none") => NetworkPolicy::None,
            _ => NetworkPolicy::Packages,
        };
        let session = match files {
            Some(f) => Session::with_files(&engine, f, policy),
            None => Session::new(&engine, policy),
        }
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(Self {
            inner: Mutex::new(session),
        })
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

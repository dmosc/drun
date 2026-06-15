//! DrunSession: PyO3 wrapper around the core Session, exposing the execution
//! and checkpoint API to Python callers.

use crate::types::{DrunCheckpoint, checkpoint_to_py};
use drun_core::{DrunEngine, Session};
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::sync::Mutex;

#[pyclass]
pub struct DrunSession {
    inner: Mutex<Session>,
}

#[pymethods]
impl DrunSession {
    #[new]
    pub fn new() -> PyResult<Self> {
        let engine = DrunEngine::new(drun_core::Config::load())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let session = Session::new(&engine).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
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

    pub fn install(&self, package: String) -> PyResult<()> {
        self.inner
            .lock()
            .unwrap()
            .install(&package)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    pub fn execute_python(&self, code: String) -> PyResult<DrunCheckpoint> {
        self.inner
            .lock()
            .unwrap()
            .execute_python(&code, &mut |_| {})
            .map(checkpoint_to_py)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    pub fn execute_bash(&self, command: String) -> PyResult<DrunCheckpoint> {
        self.inner
            .lock()
            .unwrap()
            .execute_bash(&command, &mut |_| {})
            .map(checkpoint_to_py)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    pub fn write_file(&self, path: String, content: Vec<u8>) -> PyResult<()> {
        self.inner
            .lock()
            .unwrap()
            .write_file(&path, content)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    pub fn delete_file(&self, path: String) -> PyResult<DrunCheckpoint> {
        self.inner
            .lock()
            .unwrap()
            .delete_file(&path)
            .map(checkpoint_to_py)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    #[pyo3(signature = (output_dir, keys=None))]
    pub fn export(&self, output_dir: String, keys: Option<Vec<String>>) -> PyResult<Vec<String>> {
        self.inner
            .lock()
            .unwrap()
            .export(std::path::Path::new(&output_dir), keys)
            .map(|paths| {
                paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            })
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    pub fn set_label(&self, label: String) {
        self.inner.lock().unwrap().set_label(label);
    }

    pub fn set_checkpoint_label(&self, checkpoint_id: usize, label: String) -> PyResult<()> {
        self.inner
            .lock()
            .unwrap()
            .set_checkpoint_label(checkpoint_id, label)
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

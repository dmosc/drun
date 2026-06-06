use drun_core::{DrunEngine, NetworkPolicy, Session};
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static ENGINE: OnceLock<DrunEngine> = OnceLock::new();

fn engine() -> &'static DrunEngine {
    ENGINE.get_or_init(|| DrunEngine::new().expect("Failed to initialize DrunEngine."))
}

#[pyclass]
pub struct DrunOutput {
    #[pyo3(get)]
    pub stdout: String,
    #[pyo3(get)]
    pub files: HashMap<String, Vec<u8>>,
}

#[pyclass]
pub struct DrunCheckpoint {
    #[pyo3(get)]
    pub id: usize,
    #[pyo3(get)]
    pub stdout: String,
    #[pyo3(get)]
    pub files: HashMap<String, Vec<u8>>,
}

#[pyclass]
pub struct DrunSession {
    inner: Mutex<Session>,
}

#[pymethods]
impl DrunSession {
    #[new]
    #[pyo3(signature = (files=None, network=None))]
    fn new(files: Option<HashMap<String, Vec<u8>>>, network: Option<String>) -> PyResult<Self> {
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

    fn execute(&self, code: String) -> PyResult<DrunCheckpoint> {
        self.inner
            .lock()
            .unwrap()
            .execute(&code)
            .map(checkpoint_to_py)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    fn rollback(&self, id: usize) -> PyResult<()> {
        self.inner
            .lock()
            .unwrap()
            .rollback(id)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    #[getter]
    fn current(&self) -> DrunCheckpoint {
        checkpoint_to_py(self.inner.lock().unwrap().current())
    }

    #[getter]
    fn history(&self) -> Vec<DrunCheckpoint> {
        self.inner
            .lock()
            .unwrap()
            .history()
            .iter()
            .map(checkpoint_to_py)
            .collect()
    }
}

fn checkpoint_to_py(c: &drun_core::Checkpoint) -> DrunCheckpoint {
    DrunCheckpoint {
        id: c.id,
        stdout: c.stdout.clone(),
        files: c.files.clone(),
    }
}

#[pyfunction]
#[pyo3(signature = (code, mounts=None))]
fn execute(code: String, mounts: Option<Vec<String>>) -> PyResult<DrunOutput> {
    engine()
        .run_python(&code, mounts.unwrap_or_default())
        .map(|o| DrunOutput {
            stdout: o.stdout,
            files: o.files,
        })
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pymodule]
fn drun_internal(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<DrunOutput>()?;
    m.add_class::<DrunCheckpoint>()?;
    m.add_class::<DrunSession>()?;
    m.add_function(wrap_pyfunction!(execute, m)?)?;
    Ok(())
}

//! PyO3 module root. Holds the shared engine singleton, the one-shot `execute`
//! function, and module registration.

mod session;
mod types;

use drun_core::DrunEngine;
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use session::DrunSession;
use std::sync::OnceLock;
use types::{DrunCheckpoint, DrunOutput};

static ENGINE: OnceLock<DrunEngine> = OnceLock::new();

fn engine() -> &'static DrunEngine {
    ENGINE.get_or_init(|| DrunEngine::new().expect("Failed to initialize DrunEngine."))
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

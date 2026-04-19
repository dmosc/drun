use drun_core::DrunEngine;
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::sync::OnceLock;

static ENGINE: OnceLock<DrunEngine> = OnceLock::new();

#[pyfunction]
#[pyo3(signature = (code, mounts=None))]
fn execute(code: String, mounts: Option<Vec<String>>) -> PyResult<String> {
    let engine =
        ENGINE.get_or_init(|| DrunEngine::new().expect("Failed to initialize DrunEngine."));

    engine
        .run_python(&code, mounts.unwrap_or_default())
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pymodule]
fn drun(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(execute, m)?)?;

    Ok(())
}

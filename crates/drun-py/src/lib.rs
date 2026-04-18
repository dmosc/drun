use pyo3::prelude::*;

#[pyfunction]
fn get_version() -> String {
    drun_core::version().to_string()
}

#[pymodule]
fn drun(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(get_version, m)?)?;
    Ok(())
}

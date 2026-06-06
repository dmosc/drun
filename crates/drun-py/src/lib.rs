//! PyO3 module root. Registers the session class and checkpoint type.

mod session;
mod types;

use pyo3::prelude::*;
use session::DrunSession;
use types::DrunCheckpoint;

#[pymodule]
fn drun_internal(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<DrunCheckpoint>()?;
    m.add_class::<DrunSession>()?;
    Ok(())
}

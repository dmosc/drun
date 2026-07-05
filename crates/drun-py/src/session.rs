use crate::types::{DrunCheckpoint, checkpoint_to_py};
use drun_core::Session;
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
        let config = drun_core::Config::load();
        let session = Session::new(&config).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> DrunSession {
        let config = drun_core::Config::default();
        DrunSession {
            inner: Mutex::new(Session::new(&config).unwrap()),
        }
    }

    #[test]
    fn new_starts_with_an_empty_checkpoint_zero() {
        let session = DrunSession::new().unwrap();
        let current = session.current();
        assert_eq!(current.id, 0);
        assert!(current.files.is_empty());
    }

    #[test]
    fn mount_loads_a_host_file_into_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();

        let session = test_session();
        let mounted = session
            .mount(dir.path().to_string_lossy().into_owned())
            .unwrap();

        assert_eq!(mounted, vec!["a.txt".to_string()]);
        assert_eq!(session.current().files.len(), 1);
    }

    #[test]
    fn mount_surfaces_session_errors_as_a_py_runtime_error() {
        let session = test_session();
        let err = session
            .mount("/definitely/does/not/exist".to_string())
            .unwrap_err();
        assert!(err.to_string().contains("path does not exist"));
    }

    #[test]
    fn write_file_then_delete_file_round_trips_through_checkpoints() {
        let session = test_session();
        session
            .write_file("a.txt".to_string(), b"hi".to_vec())
            .unwrap();
        assert_eq!(session.current().files.len(), 1);

        let after_delete = session.delete_file("a.txt".to_string()).unwrap();
        assert!(after_delete.files.is_empty());
    }

    #[test]
    fn commit_writes_back_a_changed_mounted_file_to_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let host_file = dir.path().join("a.txt");
        std::fs::write(&host_file, b"original").unwrap();

        let session = test_session();
        session
            .mount(dir.path().to_string_lossy().into_owned())
            .unwrap();
        session
            .write_file("a.txt".to_string(), b"changed".to_vec())
            .unwrap();

        let committed = session.commit(None).unwrap();
        assert_eq!(committed.len(), 1);
        assert_eq!(std::fs::read(&host_file).unwrap(), b"changed");
    }

    #[test]
    fn diff_reports_no_changes_between_identical_checkpoints() {
        let session = test_session();
        assert_eq!(session.diff(0, None).unwrap(), "");
    }

    #[test]
    fn diff_reports_changes_since_a_write() {
        let session = test_session();
        session
            .write_file("a.txt".to_string(), b"hi".to_vec())
            .unwrap();
        let diff = session.diff(0, None).unwrap();
        assert!(diff.contains("a.txt"));
    }

    #[test]
    fn set_label_and_set_checkpoint_label_apply_to_the_session_and_checkpoint() {
        let session = test_session();
        session.set_label("my-session".to_string());
        session
            .set_checkpoint_label(0, "start".to_string())
            .unwrap();
        assert_eq!(session.current().id, 0);
    }

    #[test]
    fn checkpoint_label_set_from_python_can_be_read_back_from_python() {
        let session = test_session();
        session
            .set_checkpoint_label(0, "start".to_string())
            .unwrap();
        assert_eq!(session.current().label.as_deref(), Some("start"));
    }

    #[test]
    fn rollback_moves_the_current_checkpoint_back() {
        let session = test_session();
        session
            .write_file("a.txt".to_string(), b"hi".to_vec())
            .unwrap();
        assert_eq!(session.current().id, 1);

        session.rollback(0).unwrap();
        assert_eq!(session.current().id, 0);
    }

    #[test]
    fn history_grows_with_each_checkpoint_and_stays_in_order() {
        let session = test_session();
        session
            .write_file("a.txt".to_string(), b"hi".to_vec())
            .unwrap();
        session
            .write_file("b.txt".to_string(), b"bye".to_vec())
            .unwrap();

        let history = session.history();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].id, 0);
        assert_eq!(history[1].id, 1);
        assert_eq!(history[2].id, 2);
    }

    #[test]
    fn export_writes_session_files_under_the_given_directory() {
        let dir = tempfile::tempdir().unwrap();
        let session = test_session();
        session
            .write_file("a.txt".to_string(), b"hi".to_vec())
            .unwrap();

        let exported = session
            .export(dir.path().to_string_lossy().into_owned(), None)
            .unwrap();

        assert_eq!(exported.len(), 1);
        assert!(dir.path().join("a.txt").exists());
    }

    #[test]
    fn execute_bash_surfaces_a_denylisted_command_as_a_py_runtime_error() {
        let config = drun_core::Config {
            bash_command_denylist: vec!["rm -rf".to_string()],
            ..drun_core::Config::default()
        };
        let session = DrunSession {
            inner: Mutex::new(Session::new(&config).unwrap()),
        };
        let err = session
            .execute_bash("rm -rf /tmp/whatever".to_string())
            .err()
            .unwrap();
        assert!(err.to_string().contains("command denied"));
    }
}

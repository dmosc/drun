use crate::config::Config;
use crate::reaper::{self, SessionMap};
use crate::response::err;
use drun_core::{DrunEngine, DrunEngineConfig, PYTHON_PACKAGE_HOSTS, Session};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct DrunHandler {
    pub(crate) engine: DrunEngine,
    pub(crate) sessions: SessionMap,
    pub(crate) fetch_allowlist: Vec<String>,
    pub(crate) fetch_timeout_ms: Option<u64>,
    pub(crate) export_root: Option<PathBuf>,
    pub(crate) snapshots_dir: Option<PathBuf>,
    pub(crate) session_idle_timeout_secs: Option<u64>,
    pub(crate) max_sessions: Option<usize>,
    pub(crate) auto_snapshot: bool,
    pub(crate) env_allowlist: Vec<String>,
    pub(crate) allowed_packages: Vec<String>,
}

impl DrunHandler {
    pub fn new(config: Config) -> Self {
        Self {
            engine: DrunEngine::new(DrunEngineConfig {
                max_workspace_bytes: config.session.max_workspace_mb.map(|mb| mb * 1024 * 1024),
                max_checkpoints: config.session.max_checkpoints,
                mount_allowlist: config
                    .session
                    .mount_allowlist
                    .iter()
                    .map(PathBuf::from)
                    .collect(),
            })
            .expect("failed to initialize drun engine"),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            fetch_allowlist: config.fetch.allowlist,
            fetch_timeout_ms: config.fetch.timeout_ms,
            export_root: config.session.export_root.map(PathBuf::from),
            snapshots_dir: config.session.snapshots_dir.map(PathBuf::from),
            session_idle_timeout_secs: config.session.session_idle_timeout_secs,
            max_sessions: config.session.max_sessions,
            auto_snapshot: config.session.auto_snapshot,
            env_allowlist: config.session.env_allowlist,
            allowed_packages: config.session.allowed_packages,
        }
    }

    pub fn start_idle_reaper(&self) {
        if let Some(timeout_secs) = self.session_idle_timeout_secs {
            reaper::spawn(Arc::clone(&self.sessions), timeout_secs);
        }
    }

    pub(crate) fn build_allowed_hosts(&self, requested: Option<Vec<String>>) -> Vec<String> {
        if let Some(hosts) = requested {
            return hosts;
        }
        if self.fetch_allowlist.iter().any(|h| h == "*") {
            return vec!["*".to_string()];
        }
        let mut hosts: Vec<String> = PYTHON_PACKAGE_HOSTS.iter().map(|s| s.to_string()).collect();
        for host in &self.fetch_allowlist {
            if !hosts.contains(host) {
                hosts.push(host.clone());
            }
        }
        hosts
    }

    pub(crate) fn with_session(
        &self,
        session_id: &str,
        f: impl FnOnce(&Session) -> Result<CallToolResult, CallToolError>,
    ) -> Result<CallToolResult, CallToolError> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .ok_or_else(|| err(format!("session '{}' not found", session_id)))?
            .clone();
        let guard = session.lock().unwrap();
        self.check_idle(session_id, &guard)?;
        f(&guard)
    }

    pub(crate) fn with_session_mut(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut Session) -> Result<CallToolResult, CallToolError>,
    ) -> Result<CallToolResult, CallToolError> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .ok_or_else(|| err(format!("session '{}' not found", session_id)))?
            .clone();
        let mut guard = session.lock().unwrap();
        self.check_idle(session_id, &guard)?;
        guard.last_activity = std::time::Instant::now();
        f(&mut guard)
    }

    fn check_idle(&self, session_id: &str, session: &Session) -> Result<(), CallToolError> {
        if let Some(timeout) = self.session_idle_timeout_secs {
            if session.last_activity.elapsed().as_secs() > timeout {
                return Err(err(format!(
                    "session '{}' has been idle for over {}s; close it and start a new one",
                    session_id, timeout
                )));
            }
        }
        Ok(())
    }
}

use crate::config::Config;
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
    pub(crate) sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
    pub(crate) fetch_allowlist: Vec<String>,
    pub(crate) fetch_timeout_ms: Option<u64>,
    pub(crate) export_root: Option<PathBuf>,
}

impl DrunHandler {
    pub fn new(config: Config) -> Self {
        Self {
            engine: DrunEngine::new(DrunEngineConfig {
                max_workspace_bytes: config.session.max_workspace_mb.map(|mb| mb * 1024 * 1024),
                mount_allowlist: config.session.mount_allowlist.iter().map(PathBuf::from).collect(),
            })
            .expect("failed to initialize drun engine"),
            sessions: Mutex::new(HashMap::new()),
            fetch_allowlist: config.fetch.allowlist,
            fetch_timeout_ms: config.fetch.timeout_ms,
            export_root: config.session.export_root.map(PathBuf::from),
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
        f(&session.lock().unwrap())
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
        f(&mut session.lock().unwrap())
    }
}

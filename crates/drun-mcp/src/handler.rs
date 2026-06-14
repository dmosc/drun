use crate::config::{Config, FetchConfig, SessionConfig};
use crate::errors::DrunError;
use crate::reaper::{self, SessionMap};
use drun_core::{DrunEngine, PYTHON_PACKAGE_HOSTS, Session};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct DrunHandler {
    pub(crate) engine: DrunEngine,
    pub(crate) sessions: SessionMap,
    pub(crate) fetch: FetchConfig,
    pub(crate) session: SessionConfig,
}

impl DrunHandler {
    pub fn new(config: Config) -> Self {
        let engine = DrunEngine::new(config.session.engine_config())
            .expect("failed to initialize drun engine");
        Self {
            engine,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            fetch: config.fetch,
            session: config.session,
        }
    }

    pub fn start_idle_reaper(&self) {
        if let Some(timeout_secs) = self.session.session_idle_timeout_secs {
            reaper::spawn(Arc::clone(&self.sessions), timeout_secs);
        }
    }

    pub(crate) fn get_domain_allowlist(&self) -> Vec<String> {
        if self.fetch.domain_allowlist.iter().any(|h| h == "*") {
            return vec!["*".to_string()];
        }
        let mut allowed_domains: Vec<String> =
            PYTHON_PACKAGE_HOSTS.iter().map(|s| s.to_string()).collect();
        for domain in &self.fetch.domain_allowlist {
            if !allowed_domains.contains(domain) {
                allowed_domains.push(domain.clone());
            }
        }
        allowed_domains
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
            .ok_or_else(|| CallToolError::from(DrunError::session_not_found(session_id)))?
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
            .ok_or_else(|| CallToolError::from(DrunError::session_not_found(session_id)))?
            .clone();
        match session.try_lock() {
            Ok(mut guard) => {
                self.check_idle(session_id, &guard)?;
                guard.last_activity = std::time::Instant::now();
                f(&mut guard)
            }
            Err(_) => Err(DrunError::session_busy(session_id).into_tool_err()),
        }
    }

    fn check_idle(&self, session_id: &str, session: &Session) -> Result<(), CallToolError> {
        if let Some(limit_secs) = self.session.session_idle_timeout_secs {
            let idle_secs = session.last_activity.elapsed().as_secs();
            if idle_secs > limit_secs {
                return Err(
                    DrunError::session_idle(session_id, idle_secs, limit_secs).into_tool_err()
                );
            }
        }
        Ok(())
    }
}

use crate::errors::DrunError;
use crate::reaper::{self, SessionMap};
use drun_core::{Config, Session};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct DrunHandler {
    pub(crate) config: Config,
    pub(crate) sessions: SessionMap,
}

impl DrunHandler {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_idle_reaper(&self) {
        if let Some(timeout_secs) = self.config.session_idle_timeout_secs {
            reaper::spawn(Arc::clone(&self.sessions), timeout_secs);
        }
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
        if let Some(limit_secs) = self.config.session_idle_timeout_secs {
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

use crate::errors::DrunError;
use crate::reaper::{self, SessionMap};
#[cfg(test)]
use drun_core::Config;
use drun_core::{ConfigHandle, Session};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct DrunHandler {
    pub(crate) config: ConfigHandle,
    pub(crate) sessions: SessionMap,
}

impl DrunHandler {
    #[cfg(test)]
    pub fn new(config: Config) -> Self {
        Self {
            config: config.into(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn new_live() -> Self {
        Self {
            config: ConfigHandle::from_env(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_idle_reaper(&self) {
        if let Some(timeout_secs) = self.config.get().session_idle_timeout_secs {
            reaper::spawn(Arc::clone(&self.sessions), timeout_secs);
        }
    }

    pub(crate) fn insert_session(
        &self,
        session_id: String,
        session: Session,
    ) -> Result<(), CallToolError> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(max) = self.config.get().max_sessions
            && sessions.len() >= max
        {
            return Err(DrunError::session_limit_reached(max).into_tool_err());
        }
        sessions.insert(session_id, Arc::new(Mutex::new(session)));
        Ok(())
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
        match session.try_lock() {
            Ok(guard) => f(&guard),
            Err(std::sync::TryLockError::WouldBlock) => {
                Err(DrunError::session_busy(session_id).into_tool_err())
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                eprintln!(
                    "drun: session '{session_id}' recovered from a poisoned lock (a prior call panicked)"
                );
                f(&poisoned.into_inner())
            }
        }
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
        let mut guard = match session.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(DrunError::session_busy(session_id).into_tool_err());
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                eprintln!(
                    "drun: session '{session_id}' recovered from a poisoned lock (a prior call panicked)"
                );
                poisoned.into_inner()
            }
        };
        self.check_idle(session_id, &guard)?;
        guard.last_activity = std::time::Instant::now();
        f(&mut guard)
    }

    fn check_idle(&self, session_id: &str, session: &Session) -> Result<(), CallToolError> {
        if let Some(limit_secs) = self.config.get().session_idle_timeout_secs {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::text;
    use std::time::{Duration, Instant};

    fn handler_with_session() -> (DrunHandler, String) {
        let handler = DrunHandler::new(Config::default());
        let session_id = "s1".to_string();
        handler.sessions.lock().unwrap().insert(
            session_id.clone(),
            Arc::new(Mutex::new(Session::new(Config::default().into()).unwrap())),
        );
        (handler, session_id)
    }

    #[test]
    fn with_session_returns_session_not_found_for_unknown_id() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .with_session("missing", |_session| Ok(text("ok")))
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn with_session_runs_closure_and_returns_its_result() {
        let (handler, session_id) = handler_with_session();
        let result = handler.with_session(&session_id, |_session| Ok(text("hello")));
        assert!(result.is_ok());
    }

    #[test]
    fn with_session_returns_session_busy_when_session_is_already_locked() {
        let (handler, session_id) = handler_with_session();
        let session_arc = handler
            .sessions
            .lock()
            .unwrap()
            .get(&session_id)
            .unwrap()
            .clone();
        let _guard = session_arc.lock().unwrap(); // simulate an in-flight call

        let err = handler
            .with_session(&session_id, |_session| Ok(text("ok")))
            .unwrap_err();
        assert!(err.to_string().contains("session_busy"));
    }

    #[test]
    fn with_session_recovers_from_a_poisoned_lock_instead_of_staying_busy_forever() {
        let (handler, session_id) = handler_with_session();
        let session_arc = handler
            .sessions
            .lock()
            .unwrap()
            .get(&session_id)
            .unwrap()
            .clone();

        // Poison the mutex the same way a real bug would: panic while
        // holding its guard.
        let arc_for_panic = session_arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(session_arc.is_poisoned());

        // Repeated calls must keep recovering and succeeding, not
        // permanently report session_busy.
        for _ in 0..2 {
            let result = handler.with_session(&session_id, |_session| Ok(text("ok")));
            assert!(
                result.is_ok(),
                "a poisoned session must recover, not stay busy forever"
            );
        }
    }

    #[test]
    fn with_session_mut_recovers_from_a_poisoned_lock_instead_of_staying_busy_forever() {
        let (handler, session_id) = handler_with_session();
        let session_arc = handler
            .sessions
            .lock()
            .unwrap()
            .get(&session_id)
            .unwrap()
            .clone();

        let arc_for_panic = session_arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(session_arc.is_poisoned());

        for _ in 0..2 {
            let result = handler.with_session_mut(&session_id, |_session| Ok(text("ok")));
            assert!(
                result.is_ok(),
                "a poisoned session must recover, not stay busy forever"
            );
        }
    }

    #[test]
    fn with_session_mut_returns_session_not_found_for_unknown_id() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .with_session_mut("missing", |_session| Ok(text("ok")))
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn with_session_mut_returns_session_busy_when_session_is_already_locked() {
        let (handler, session_id) = handler_with_session();
        let session_arc = handler
            .sessions
            .lock()
            .unwrap()
            .get(&session_id)
            .unwrap()
            .clone();
        let _guard = session_arc.lock().unwrap();

        let err = handler
            .with_session_mut(&session_id, |_session| Ok(text("ok")))
            .unwrap_err();
        assert!(err.to_string().contains("session_busy"));
    }

    #[test]
    fn with_session_mut_updates_last_activity() {
        let (handler, session_id) = handler_with_session();
        {
            let session_arc = handler
                .sessions
                .lock()
                .unwrap()
                .get(&session_id)
                .unwrap()
                .clone();
            session_arc.lock().unwrap().last_activity = Instant::now() - Duration::from_secs(120);
        }

        handler
            .with_session_mut(&session_id, |_session| Ok(text("ok")))
            .unwrap();

        let session_arc = handler
            .sessions
            .lock()
            .unwrap()
            .get(&session_id)
            .unwrap()
            .clone();
        let idle_secs = session_arc
            .lock()
            .unwrap()
            .last_activity
            .elapsed()
            .as_secs();
        assert!(idle_secs < 5, "last_activity should have been refreshed");
    }

    #[test]
    fn with_session_mut_rejects_sessions_past_the_configured_idle_timeout() {
        let config = Config {
            session_idle_timeout_secs: Some(60),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let session_id = "s1".to_string();
        let mut session = Session::new(Config::default().into()).unwrap();
        session.last_activity = Instant::now() - Duration::from_secs(120);
        handler
            .sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), Arc::new(Mutex::new(session)));

        let err = handler
            .with_session_mut(&session_id, |_session| Ok(text("ok")))
            .unwrap_err();
        assert!(err.to_string().contains("session_idle"));
    }

    #[test]
    fn with_session_mut_allows_sessions_within_the_idle_window() {
        let config = Config {
            session_idle_timeout_secs: Some(60),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let session_id = "s1".to_string();
        let session = Session::new(Config::default().into()).unwrap();
        handler
            .sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), Arc::new(Mutex::new(session)));

        let result = handler.with_session_mut(&session_id, |_session| Ok(text("ok")));
        assert!(result.is_ok());
    }

    #[test]
    fn with_session_mut_ignores_idle_state_when_timeout_is_disabled() {
        let config = Config {
            session_idle_timeout_secs: None,
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let session_id = "s1".to_string();
        let mut session = Session::new(Config::default().into()).unwrap();
        session.last_activity = Instant::now() - Duration::from_secs(999_999);
        handler
            .sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), Arc::new(Mutex::new(session)));

        let result = handler.with_session_mut(&session_id, |_session| Ok(text("ok")));
        assert!(result.is_ok());
    }
}

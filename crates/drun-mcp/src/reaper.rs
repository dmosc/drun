//! Idle session reaper: periodically evicts sessions that have exceeded the
//! configured idle timeout, freeing their Python subprocess and session state.

use drun_core::Session;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) type SessionMap = Arc<Mutex<HashMap<String, Arc<Mutex<Session>>>>>;

pub(crate) fn spawn(sessions: SessionMap, idle_timeout_secs: u64) {
    let check_every = Duration::from_secs((idle_timeout_secs / 2).max(30));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(check_every);
        ticker.tick().await; // skip the immediate first tick
        loop {
            ticker.tick().await;
            evict_idle(&sessions, idle_timeout_secs);
        }
    });
}

fn evict_idle(sessions: &SessionMap, timeout_secs: u64) {
    sessions.lock().unwrap().retain(|_, arc| {
        arc.try_lock()
            .map(|session| session.last_activity.elapsed().as_secs() <= timeout_secs)
            .unwrap_or(true) // session is in use — keep it
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use drun_core::Config;
    use std::time::Instant;

    fn session_map(entries: Vec<(&str, Session)>) -> SessionMap {
        let mut map = HashMap::new();
        for (id, session) in entries {
            map.insert(id.to_string(), Arc::new(Mutex::new(session)));
        }
        Arc::new(Mutex::new(map))
    }

    fn session_idle_for(secs: u64) -> Session {
        let mut session = Session::new(&Config::default()).unwrap();
        session.last_activity = Instant::now() - Duration::from_secs(secs);
        session
    }

    #[test]
    fn evict_idle_removes_sessions_past_the_timeout() {
        let sessions = session_map(vec![("stale", session_idle_for(120))]);
        evict_idle(&sessions, 60);
        assert!(sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn evict_idle_keeps_sessions_within_the_idle_window() {
        let sessions = session_map(vec![("fresh", session_idle_for(10))]);
        evict_idle(&sessions, 60);
        assert!(sessions.lock().unwrap().contains_key("fresh"));
    }

    #[test]
    fn evict_idle_keeps_a_session_that_is_currently_locked_even_if_stale() {
        let sessions = session_map(vec![("busy", session_idle_for(120))]);
        let session_arc = sessions.lock().unwrap().get("busy").unwrap().clone();
        let _guard = session_arc.lock().unwrap(); // simulate an in-flight call

        evict_idle(&sessions, 60);
        assert!(sessions.lock().unwrap().contains_key("busy"));
    }

    #[test]
    fn evict_idle_only_removes_the_stale_sessions_from_a_mixed_map() {
        let sessions = session_map(vec![
            ("stale", session_idle_for(120)),
            ("fresh", session_idle_for(10)),
        ]);
        evict_idle(&sessions, 60);
        let remaining = sessions.lock().unwrap();
        assert!(!remaining.contains_key("stale"));
        assert!(remaining.contains_key("fresh"));
    }
}

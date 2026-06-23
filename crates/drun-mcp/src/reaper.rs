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

//! Tracks the output accumulated so far for an in-flight `session_bash`
//! call, so the web UI can show a session as running and stream its output
//! before the checkpoint it produces exists. Mirrors the `SessionChildGuard`
//! pattern in `drun_core::Session`: starting a call returns a guard whose
//! `Drop` clears the entry, so it's removed on every exit path (success,
//! error, or panic) with no explicit cleanup call required.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LiveEntry {
    pub(crate) command: String,
    pub(crate) output: String,
}

#[derive(Clone, Default)]
pub(crate) struct LiveOutputRegistry {
    entries: Arc<Mutex<HashMap<String, LiveEntry>>>,
}

impl LiveOutputRegistry {
    pub(crate) fn start(&self, session_id: &str, command: &str) -> LiveOutputGuard {
        self.entries.lock().unwrap().insert(
            session_id.to_string(),
            LiveEntry {
                command: command.to_string(),
                output: String::new(),
            },
        );
        LiveOutputGuard {
            registry: self.clone(),
            session_id: session_id.to_string(),
        }
    }

    pub(crate) fn is_running(&self, session_id: &str) -> bool {
        self.entries.lock().unwrap().contains_key(session_id)
    }

    pub(crate) fn snapshot(&self, session_id: &str) -> Option<LiveEntry> {
        self.entries.lock().unwrap().get(session_id).cloned()
    }

    fn append(&self, session_id: &str, chunk: &str) {
        if let Some(entry) = self.entries.lock().unwrap().get_mut(session_id) {
            entry.output.push_str(chunk);
            entry.output.push('\n');
        }
    }

    fn finish(&self, session_id: &str) {
        self.entries.lock().unwrap().remove(session_id);
    }
}

pub(crate) struct LiveOutputGuard {
    registry: LiveOutputRegistry,
    session_id: String,
}

impl LiveOutputGuard {
    pub(crate) fn append(&self, chunk: &str) {
        self.registry.append(&self.session_id, chunk);
    }
}

impl Drop for LiveOutputGuard {
    fn drop(&mut self) {
        self.registry.finish(&self.session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_running_is_false_until_started() {
        let registry = LiveOutputRegistry::default();
        assert!(!registry.is_running("s1"));
    }

    #[test]
    fn start_makes_the_session_report_as_running_with_the_given_command() {
        let registry = LiveOutputRegistry::default();
        let guard = registry.start("s1", "echo hi");

        assert!(registry.is_running("s1"));
        assert_eq!(
            registry.snapshot("s1"),
            Some(LiveEntry {
                command: "echo hi".to_string(),
                output: String::new()
            })
        );
        drop(guard);
    }

    #[test]
    fn append_accumulates_chunks_as_newline_separated_lines() {
        let registry = LiveOutputRegistry::default();
        let guard = registry.start("s1", "echo hi");

        guard.append("line one");
        guard.append("line two");

        assert_eq!(
            registry.snapshot("s1").unwrap().output,
            "line one\nline two\n"
        );
    }

    #[test]
    fn dropping_the_guard_clears_the_entry() {
        let registry = LiveOutputRegistry::default();
        let guard = registry.start("s1", "echo hi");

        drop(guard);

        assert!(!registry.is_running("s1"));
        assert_eq!(registry.snapshot("s1"), None);
    }

    #[test]
    fn append_after_finish_is_a_harmless_no_op() {
        let registry = LiveOutputRegistry::default();
        let guard = registry.start("s1", "echo hi");
        registry.finish("s1");

        guard.append("too late");

        assert_eq!(registry.snapshot("s1"), None);
    }

    #[test]
    fn entries_for_different_sessions_are_independent() {
        let registry = LiveOutputRegistry::default();
        let guard1 = registry.start("s1", "echo one");
        let guard2 = registry.start("s2", "echo two");

        guard1.append("from s1");
        guard2.append("from s2");

        assert_eq!(registry.snapshot("s1").unwrap().output, "from s1\n");
        assert_eq!(registry.snapshot("s2").unwrap().output, "from s2\n");
    }
}

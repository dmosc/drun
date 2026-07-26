use crate::handler::DrunHandler;
use drun_core::{CheckpointRef, Session};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SessionSummary {
    session_id: String,
    is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_id: usize,
    checkpoint_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_checkpoint_id: Option<usize>,
}

impl SessionSummary {
    pub(crate) fn all(
        sessions: &HashMap<String, Arc<Mutex<Session>>>,
        current_id: Option<&str>,
    ) -> Vec<SessionSummary> {
        sessions
            .iter()
            .map(|(id, arc)| {
                let session = DrunHandler::lock_recovering(id, arc);
                let (parent_session_id, parent_checkpoint_id) =
                    CheckpointRef::split(&session.parent);
                SessionSummary {
                    session_id: id.clone(),
                    is_current: current_id == Some(id.as_str()),
                    label: session.label.clone(),
                    checkpoint_id: session.current().id,
                    checkpoint_count: session.history().len(),
                    parent_session_id,
                    parent_checkpoint_id,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_session() -> Session {
        Session::new(drun_core::Config::default().into()).unwrap()
    }

    #[test]
    fn session_summary_all_reflects_current_checkpoint_and_history_length() {
        let mut session = new_session();
        session.write_file("a.txt", b"hi".to_vec()).unwrap();
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(session)));

        let summaries = SessionSummary::all(&sessions, Some("s1"));
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, "s1");
        assert_eq!(summaries[0].checkpoint_id, 1);
        assert_eq!(summaries[0].checkpoint_count, 2);
        assert_eq!(summaries[0].parent_session_id, None);
        assert!(summaries[0].is_current);
    }

    #[test]
    fn session_summary_all_marks_non_matching_sessions_as_not_current() {
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(new_session())));

        let summaries = SessionSummary::all(&sessions, Some("other"));
        assert!(!summaries[0].is_current);

        let summaries = SessionSummary::all(&sessions, None);
        assert!(!summaries[0].is_current);
    }

    #[test]
    fn session_summary_all_reports_parent_reference() {
        let mut session = new_session();
        session.parent = Some(CheckpointRef {
            session_id: "parent-session".to_string(),
            checkpoint_id: 3,
        });
        let mut sessions = HashMap::new();
        sessions.insert("child".to_string(), Arc::new(Mutex::new(session)));

        let summaries = SessionSummary::all(&sessions, None);
        assert_eq!(
            summaries[0].parent_session_id,
            Some("parent-session".to_string())
        );
        assert_eq!(summaries[0].parent_checkpoint_id, Some(3));
    }

    #[test]
    fn session_summary_all_recovers_from_a_poisoned_lock_instead_of_panicking() {
        let arc = Arc::new(Mutex::new(new_session()));
        let arc_for_panic = arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(arc.is_poisoned());

        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), arc);

        let summaries = SessionSummary::all(&sessions, None);
        assert_eq!(summaries.len(), 1);
    }
}

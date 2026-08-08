use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::{self, DrunHandler};
use crate::state::{SessionState, SessionSummary, SessionTreeNode};
use crate::tools::{
    GetSessionState, SessionChatRecord, SessionFork, SessionLabel, SessionMerge, SessionSwitch,
};
use drun_core::Session;
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};

impl DrunHandler {
    pub(super) fn handle_create_session(
        &self,
        connection_id: &str,
    ) -> Result<CallToolResult, CallToolError> {
        let session = Session::new(self.config.clone())
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let (session_id, arc) = self
            .insert_session(session)
            .map_err(|max| DrunError::session_limit_reached(max).into_tool_err())?;
        let state =
            SessionState::compute(&session_id, &Self::lock_recovering(&session_id, &arc), None);
        self.current_sessions.set(connection_id, session_id);
        Ok(ResponseBuilder::json(&state))
    }

    pub(super) fn handle_session_switch(
        &self,
        connection_id: &str,
        t: SessionSwitch,
    ) -> Result<CallToolResult, CallToolError> {
        let arc = self.resolve_session(&t.session_id)?;
        self.current_sessions
            .set(connection_id, t.session_id.clone());
        let session = Self::lock_recovering(&t.session_id, &arc);
        Ok(ResponseBuilder::json(&SessionState::compute(
            &t.session_id,
            &session,
            None,
        )))
    }

    pub(super) fn handle_session_fork(
        &self,
        connection_id: &str,
        t: SessionFork,
    ) -> Result<CallToolResult, CallToolError> {
        let source_id = self.current_sessions.resolve(connection_id)?;
        let source_arc = self.resolve_session(&source_id)?;
        let forked_session = {
            let source = DrunHandler::lock_recovering(&source_id, &source_arc);
            let checkpoint_id = source
                .resolve_checkpoint(t.checkpoint_id, t.checkpoint_label.as_deref())
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Session::from_session(self.config.clone(), &source_id, &source, checkpoint_id)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?
        };
        let (fork_id, arc) = self
            .insert_session(forked_session)
            .map_err(|max| DrunError::session_limit_reached(max).into_tool_err())?;
        let state = SessionState::compute(&fork_id, &Self::lock_recovering(&fork_id, &arc), None);
        self.current_sessions.set(connection_id, fork_id);
        Ok(ResponseBuilder::json(&state))
    }

    pub(super) fn handle_session_list(
        &self,
        connection_id: &str,
    ) -> Result<CallToolResult, CallToolError> {
        let sessions = self.sessions.lock().unwrap().clone();
        let current_id = self.current_sessions.get(connection_id);
        Ok(ResponseBuilder::json(&SessionSummary::all(
            &sessions,
            current_id.as_deref(),
        )))
    }

    pub(super) fn handle_session_close(
        &self,
        connection_id: &str,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        self.close_session(&session_id).map_err(|e| match e {
            handler::CloseSessionError::NotFound => {
                DrunError::session_not_found(&session_id).into_tool_err()
            }
            handler::CloseSessionError::Io(err) => DrunError::internal(err).into_tool_err(),
        })?;
        Ok(ResponseBuilder::text(format!("closed {session_id}")))
    }

    pub(super) fn handle_get_session_state(
        &self,
        connection_id: &str,
        t: GetSessionState,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |session_id, session| {
            session.record_step(None, "get_session_state", &t.description);
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id, session, None,
            )))
        })
    }

    pub(super) fn handle_session_chat_record(
        &self,
        connection_id: &str,
        t: SessionChatRecord,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |session_id, session| {
            session.record_chat_turn(t.prompt.clone(), t.response);
            session.record_step(None, "session_chat_record", &t.prompt);
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id, session, None,
            )))
        })
    }

    pub(super) fn handle_session_tree(&self) -> Result<CallToolResult, CallToolError> {
        let sessions = self.sessions.lock().unwrap().clone();
        Ok(ResponseBuilder::json(&SessionTreeNode::forest(
            &sessions,
            &self.live_output,
        )))
    }

    pub(super) fn handle_session_label(
        &self,
        connection_id: &str,
        t: SessionLabel,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |session_id, session| {
            session.set_label(t.label);
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id, session, None,
            )))
        })
    }

    pub(super) fn handle_session_merge(
        &self,
        connection_id: &str,
        t: SessionMerge,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        if session_id == t.source_session_id {
            return Err(DrunError::internal("cannot merge a session with itself").into_tool_err());
        }
        let source_arc = self.resolve_session(&t.source_session_id)?;
        let source = DrunHandler::lock_recovering(&t.source_session_id, &source_arc);
        let source_checkpoint_id = source
            .resolve_checkpoint(t.source_checkpoint_id, t.source_checkpoint_label.as_deref())
            .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
        self.with_session_mut(&session_id, |session| {
            session
                .merge_from(&source, source_checkpoint_id, t.keys, Some(&t.description))
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&SessionState::compute(
                &session_id,
                session,
                None,
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use drun_core::Config;

    #[test]
    fn create_session_succeeds_and_registers_the_session() {
        let handler = DrunHandler::new(Config::default());
        let result = handler.handle_create_session(CLIENT).unwrap();
        assert_eq!(handler.sessions.lock().unwrap().len(), 1);
        assert!(result_text(&result).contains("checkpoint_id"));
    }

    #[test]
    fn create_session_makes_the_new_session_current() {
        let handler = DrunHandler::new(Config::default());
        let result = handler.handle_create_session(CLIENT).unwrap();
        let session_id = result_json(&result)["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(handler.current_sessions.get(CLIENT), Some(session_id));
    }

    #[test]
    fn create_session_rejects_once_max_sessions_is_reached() {
        let config = Config {
            max_sessions: Some(1),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        handler.handle_create_session(CLIENT).unwrap();
        let err = handler.handle_create_session(CLIENT).unwrap_err();
        assert!(err.to_string().contains("session_limit_reached"));
    }

    #[test]
    fn session_fork_rejects_once_max_sessions_is_reached() {
        let config = Config {
            max_sessions: Some(1),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "source");

        let err = handler
            .handle_session_fork(
                CLIENT,
                SessionFork {
                    checkpoint_id: None,
                    checkpoint_label: None,
                },
            )
            .unwrap_err();

        assert!(err.to_string().contains("session_limit_reached"));
        assert_eq!(handler.sessions.lock().unwrap().len(), 1);
    }

    #[test]
    fn session_fork_returns_no_active_session_without_a_current_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_fork(
                CLIENT,
                SessionFork {
                    checkpoint_id: None,
                    checkpoint_label: None,
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("no_active_session"));
    }

    #[test]
    fn session_fork_inherits_files_from_the_source_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "source");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("source")
                .unwrap()
                .lock()
                .unwrap()
                .write_files(
                    vec![("a.txt".to_string(), b"hi".to_vec())],
                    "session_write_file",
                    None,
                )
                .unwrap();
        }

        let result = handler
            .handle_session_fork(
                CLIENT,
                SessionFork {
                    checkpoint_id: None,
                    checkpoint_label: None,
                },
            )
            .unwrap();
        assert_eq!(handler.sessions.lock().unwrap().len(), 2);
        let json = result_json(&result);
        assert_eq!(json["workspace_file_count"], 1);
    }

    #[test]
    fn session_fork_switches_current_to_the_fork() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "source");

        let result = handler
            .handle_session_fork(
                CLIENT,
                SessionFork {
                    checkpoint_id: None,
                    checkpoint_label: None,
                },
            )
            .unwrap();
        let fork_id = result_json(&result)["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(handler.current_sessions.get(CLIENT), Some(fork_id));
    }

    #[test]
    fn session_fork_recovers_from_a_poisoned_source_lock_instead_of_panicking() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "source");
        let source_arc = handler
            .sessions
            .lock()
            .unwrap()
            .get("source")
            .unwrap()
            .clone();
        let arc_for_panic = source_arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(source_arc.is_poisoned());

        let result = handler
            .handle_session_fork(
                CLIENT,
                SessionFork {
                    checkpoint_id: None,
                    checkpoint_label: None,
                },
            )
            .unwrap();
        assert!(result_text(&result).contains("checkpoint_id"));
    }

    #[test]
    fn session_close_removes_the_session_from_the_map() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        handler.handle_session_close(CLIENT).unwrap();
        assert!(!handler.sessions.lock().unwrap().contains_key("s1"));
    }

    #[test]
    fn session_close_clears_current_for_this_connection() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        handler.handle_session_close(CLIENT).unwrap();
        assert_eq!(handler.current_sessions.get(CLIENT), None);
    }

    #[test]
    fn session_close_returns_no_active_session_without_a_current_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler.handle_session_close(CLIENT).unwrap_err();
        assert!(err.to_string().contains("no_active_session"));
    }

    #[test]
    fn session_close_writes_a_snapshot_when_snapshot_on_close_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshot_on_close: true,
            snapshots_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");

        handler.handle_session_close(CLIENT).unwrap();

        assert!(dir.path().join("s1.drun").exists());
    }

    #[test]
    fn session_close_surfaces_an_error_when_the_snapshot_directory_cannot_be_created() {
        let dir = tempfile::tempdir().unwrap();
        let blocking_file = dir.path().join("blocked");
        std::fs::write(&blocking_file, b"not a directory").unwrap();
        let config = Config {
            snapshot_on_close: true,
            snapshots_dir: blocking_file.join("nested"),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");

        let err = handler.handle_session_close(CLIENT).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn session_close_recovers_from_a_poisoned_lock_when_snapshotting() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshot_on_close: true,
            snapshots_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");
        let session_arc = handler.sessions.lock().unwrap().get("s1").unwrap().clone();
        let arc_for_panic = session_arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(session_arc.is_poisoned());

        handler.handle_session_close(CLIENT).unwrap();

        assert!(dir.path().join("s1.drun").exists());
    }

    #[test]
    fn session_list_reports_every_active_session() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        insert_session(&handler, "s2");
        let result = handler.handle_session_list(CLIENT).unwrap();
        let json = result_json(&result);
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[test]
    fn session_list_marks_the_active_session_as_current() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        insert_session(&handler, "s2");
        let result = handler.handle_session_list(CLIENT).unwrap();
        let json = result_json(&result);
        let entries = json.as_array().unwrap();
        let s1 = entries.iter().find(|e| e["session_id"] == "s1").unwrap();
        let s2 = entries.iter().find(|e| e["session_id"] == "s2").unwrap();
        assert_eq!(s1["is_current"], true);
        assert_eq!(s2["is_current"], false);
    }

    #[test]
    fn get_session_state_returns_no_active_session_without_a_current_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_get_session_state(
                CLIENT,
                GetSessionState {
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("no_active_session"));
    }

    #[test]
    fn get_session_state_reports_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_get_session_state(
                CLIENT,
                GetSessionState {
                    description: "test".to_string(),
                },
            )
            .unwrap();
        assert_eq!(result_json(&result)["checkpoint_id"], 0);
    }

    #[test]
    fn session_chat_record_returns_no_active_session_without_a_current_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_chat_record(
                CLIENT,
                SessionChatRecord {
                    prompt: "hi".to_string(),
                    response: "hello".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("no_active_session"));
    }

    #[test]
    fn session_chat_record_appends_to_the_session_chat_log() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");

        handler
            .handle_session_chat_record(
                CLIENT,
                SessionChatRecord {
                    prompt: "list the files".to_string(),
                    response: "a.txt is present".to_string(),
                },
            )
            .unwrap();

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        let chat_log = session.chat_log();
        assert_eq!(chat_log.len(), 1);
        assert_eq!(chat_log[0].prompt, "list the files");
        assert_eq!(chat_log[0].response, "a.txt is present");
    }

    #[test]
    fn session_chat_record_survives_a_snapshot_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        handler
            .handle_session_chat_record(
                CLIENT,
                SessionChatRecord {
                    prompt: "list the files".to_string(),
                    response: "a.txt is present".to_string(),
                },
            )
            .unwrap();
        let snapshot_path = dir.path().join("s1.drun");
        {
            let sessions = handler.sessions.lock().unwrap();
            let session = sessions.get("s1").unwrap().lock().unwrap();
            session.snapshot().write(&snapshot_path).unwrap();
        }

        let bytes = std::fs::read(&snapshot_path).unwrap();
        let snapshot = drun_core::SessionSnapshot::decode(&bytes).unwrap();
        assert_eq!(snapshot.chat_log.len(), 1);
        assert_eq!(snapshot.chat_log[0].prompt, "list the files");
    }

    #[test]
    fn session_tree_reflects_the_current_sessions() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let result = handler.handle_session_tree().unwrap();
        assert!(result_text(&result).contains("s1"));
    }

    #[test]
    fn session_switch_changes_the_active_session() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        insert_session(&handler, "s2");

        handler
            .handle_session_switch(
                CLIENT,
                SessionSwitch {
                    session_id: "s2".to_string(),
                },
            )
            .unwrap();

        assert_eq!(handler.current_sessions.get(CLIENT), Some("s2".to_string()));
    }

    #[test]
    fn session_switch_returns_session_not_found_for_an_unknown_id() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_switch(
                CLIENT,
                SessionSwitch {
                    session_id: "missing".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn session_label_sets_the_session_label() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_session_label(
                CLIENT,
                SessionLabel {
                    label: "milestone".to_string(),
                },
            )
            .unwrap();
        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(session.label.as_deref(), Some("milestone"));
        drop(session);
        let _ = result;
    }

    #[test]
    fn session_merge_rejects_merging_a_session_with_itself() {
        let handler = DrunHandler::new(Config::default());
        set_current(&handler, "s1");
        let err = handler
            .handle_session_merge(
                CLIENT,
                SessionMerge {
                    source_session_id: "s1".to_string(),
                    source_checkpoint_id: None,
                    source_checkpoint_label: None,
                    keys: None,
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("cannot merge a session with itself")
        );
    }

    #[test]
    fn session_merge_returns_session_not_found_for_missing_source() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "target");
        let err = handler
            .handle_session_merge(
                CLIENT,
                SessionMerge {
                    source_session_id: "missing-source".to_string(),
                    source_checkpoint_id: None,
                    source_checkpoint_label: None,
                    keys: None,
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn session_merge_overlays_files_from_the_source_session() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "target");
        insert_session(&handler, "source");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("source")
                .unwrap()
                .lock()
                .unwrap()
                .write_files(
                    vec![("shared.txt".to_string(), b"from source".to_vec())],
                    "session_write_file",
                    None,
                )
                .unwrap();
        }

        let result = handler
            .handle_session_merge(
                CLIENT,
                SessionMerge {
                    source_session_id: "source".to_string(),
                    source_checkpoint_id: None,
                    source_checkpoint_label: None,
                    keys: None,
                    description: "test".to_string(),
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["workspace_file_count"], 1);
    }

    #[test]
    fn session_merge_recovers_from_a_poisoned_source_lock_instead_of_panicking() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "target");
        insert_session(&handler, "source");
        let source_arc = handler
            .sessions
            .lock()
            .unwrap()
            .get("source")
            .unwrap()
            .clone();
        let arc_for_panic = source_arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(source_arc.is_poisoned());

        let result = handler
            .handle_session_merge(
                CLIENT,
                SessionMerge {
                    source_session_id: "source".to_string(),
                    source_checkpoint_id: None,
                    source_checkpoint_label: None,
                    keys: None,
                    description: "test".to_string(),
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["checkpoint_id"], 1);
    }
}

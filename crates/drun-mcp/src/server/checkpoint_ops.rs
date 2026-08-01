use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::state::{CheckpointSummary, SessionState};
use crate::tools::{
    CheckpointReadStdstreams, SessionCheckpointDrop, SessionCheckpointLabel,
    SessionCheckpointSquash, SessionDiff, SessionRollback,
};
use drun_core::TextParserUtilities;
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};

impl DrunHandler {
    pub(super) fn handle_session_history(
        &self,
        connection_id: &str,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session(connection_id, |_session_id, session| {
            Ok(ResponseBuilder::json(&CheckpointSummary::history(session)))
        })
    }

    pub(super) fn handle_session_rollback(
        &self,
        connection_id: &str,
        t: SessionRollback,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |session_id, session| {
            let checkpoint_id = session
                .resolve_checkpoint(t.checkpoint_id, t.checkpoint_label.as_deref())
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?
                .ok_or_else(|| {
                    DrunError::internal("provide checkpoint_id or checkpoint_label").into_tool_err()
                })?;
            let previous_files = session.current().files.clone();
            session
                .rollback(checkpoint_id)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    pub(super) fn handle_session_diff(
        &self,
        connection_id: &str,
        t: SessionDiff,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session(connection_id, |_session_id, session| {
            let from = session
                .resolve_checkpoint(t.from_checkpoint_id, t.from_checkpoint_label.as_deref())
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?
                .unwrap_or(0);
            let to = session
                .resolve_checkpoint(t.to_checkpoint_id, t.to_checkpoint_label.as_deref())
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?
                .unwrap_or_else(|| session.current().id);
            let diff = session
                .diff(from, to)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::text(if diff.is_empty() {
                "no changes".into()
            } else {
                diff
            }))
        })
    }

    pub(super) fn handle_session_checkpoint_label(
        &self,
        connection_id: &str,
        t: SessionCheckpointLabel,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |_session_id, session| {
            let checkpoint_id = t
                .checkpoint_id
                .map(|id| id as usize)
                .unwrap_or_else(|| session.current().id);
            session
                .set_checkpoint_label(checkpoint_id, t.label)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&CheckpointSummary::history(session)))
        })
    }

    pub(super) fn handle_session_checkpoint_squash(
        &self,
        connection_id: &str,
        t: SessionCheckpointSquash,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |_session_id, session| {
            session
                .squash_checkpoints(
                    t.from_checkpoint_id as usize,
                    t.to_checkpoint_id as usize,
                    t.label,
                )
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&CheckpointSummary::history(session)))
        })
    }

    pub(super) fn handle_session_checkpoint_drop(
        &self,
        connection_id: &str,
        t: SessionCheckpointDrop,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |_session_id, session| {
            session
                .drop_checkpoints(t.from_checkpoint_id as usize, t.to_checkpoint_id as usize)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&CheckpointSummary::history(session)))
        })
    }

    pub(super) fn handle_checkpoint_read_stdstreams(
        &self,
        connection_id: &str,
        t: CheckpointReadStdstreams,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session(connection_id, |_session_id, session| {
            let checkpoint_id = t
                .checkpoint_id
                .map(|id| id as usize)
                .unwrap_or_else(|| session.current().id);
            let checkpoint = session.history().get(checkpoint_id).ok_or_else(|| {
                DrunError::checkpoint_not_found(format!(
                    "checkpoint {checkpoint_id} does not exist"
                ))
                .into_tool_err()
            })?;
            let stream = t.stream.as_deref().unwrap_or("stdout");
            let content = match stream {
                "stdout" => &checkpoint.stdout,
                "stderr" => &checkpoint.stderr,
                _ => {
                    return Err(DrunError::internal(format!(
                        "unknown stream '{}'; use 'stdout' or 'stderr'",
                        stream
                    ))
                    .into_tool_err());
                }
            };
            let total = content.len();
            let start = (t.offset.unwrap_or(0) as usize).min(total);
            let end = t
                .limit
                .map(|l| start.saturating_add(l as usize).min(total))
                .unwrap_or(total);

            if let Some(pattern) = &t.pattern {
                let slice = &content.as_bytes()[start..end];
                let grep = TextParserUtilities::grep(slice, pattern)
                    .map_err(|e| DrunError::from_exec(e.into()).into_tool_err())?;
                return Ok(ResponseBuilder::text(
                    serde_json::json!({
                        "stream": stream,
                        "checkpoint_id": checkpoint_id,
                        "total_matches": grep.total_matches,
                        "matches": grep.matches,
                    })
                    .to_string(),
                ));
            }

            Ok(ResponseBuilder::text(
                serde_json::json!({
                    "stream": stream,
                    "checkpoint_id": checkpoint_id,
                    "exit_code": checkpoint.exit_code,
                    "offset": start,
                    "length": end - start,
                    "total_bytes": total,
                    "has_more": end < total,
                    "content": &content[start..end],
                })
                .to_string(),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use drun_core::Config;

    #[test]
    fn session_history_returns_the_checkpoint_list_for_a_session() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let result = handler.handle_session_history(CLIENT).unwrap();
        assert!(result_text(&result).contains("checkpoint_id"));
    }

    #[test]
    fn session_rollback_requires_a_checkpoint_id_or_label() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_rollback(
                CLIENT,
                SessionRollback {
                    checkpoint_id: None,
                    checkpoint_label: None,
                },
            )
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("provide checkpoint_id or checkpoint_label")
        );
    }

    #[test]
    fn session_rollback_moves_the_head_to_the_given_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"1".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_rollback(
                CLIENT,
                SessionRollback {
                    checkpoint_id: Some(0),
                    checkpoint_label: None,
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["checkpoint_id"], 0);
    }

    #[test]
    fn session_rollback_returns_checkpoint_not_found_for_an_unknown_checkpoint_id() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_rollback(
                CLIENT,
                SessionRollback {
                    checkpoint_id: Some(99),
                    checkpoint_label: None,
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("checkpoint_not_found"));
    }

    #[test]
    fn session_diff_defaults_from_checkpoint_zero_to_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hi".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_diff(
                CLIENT,
                SessionDiff {
                    from_checkpoint_id: None,
                    from_checkpoint_label: None,
                    to_checkpoint_id: None,
                    to_checkpoint_label: None,
                },
            )
            .unwrap();
        assert!(result_text(&result).contains("a.txt"));
    }

    #[test]
    fn session_diff_reports_no_changes_between_identical_checkpoints() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");

        let result = handler
            .handle_session_diff(
                CLIENT,
                SessionDiff {
                    from_checkpoint_id: Some(0),
                    from_checkpoint_label: None,
                    to_checkpoint_id: Some(0),
                    to_checkpoint_label: None,
                },
            )
            .unwrap();
        assert_eq!(result_text(&result), "no changes");
    }

    #[test]
    fn session_checkpoint_label_defaults_to_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hi".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_checkpoint_label(
                CLIENT,
                SessionCheckpointLabel {
                    checkpoint_id: None,
                    label: "milestone".to_string(),
                },
            )
            .unwrap();
        let json = result_json(&result);
        let entry = json
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["checkpoint_id"] == 1)
            .unwrap();
        assert_eq!(entry["label"], "milestone");
    }

    #[test]
    fn session_checkpoint_squash_merges_a_checkpoint_range() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.write_file("a.txt", b"one".to_vec(), None).unwrap();
            session.write_file("a.txt", b"two".to_vec(), None).unwrap();
        }

        let result = handler
            .handle_session_checkpoint_squash(
                CLIENT,
                SessionCheckpointSquash {
                    from_checkpoint_id: 1,
                    to_checkpoint_id: 2,
                    label: Some("squashed".to_string()),
                },
            )
            .unwrap();
        assert!(result_text(&result).contains("squashed"));
    }

    #[test]
    fn session_checkpoint_drop_removes_a_checkpoint_range() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.write_file("a.txt", b"one".to_vec(), None).unwrap();
            session.write_file("a.txt", b"two".to_vec(), None).unwrap();
        }

        let result = handler
            .handle_session_checkpoint_drop(
                CLIENT,
                SessionCheckpointDrop {
                    from_checkpoint_id: 1,
                    to_checkpoint_id: 1,
                },
            )
            .unwrap();
        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(session.history().len(), 2);
        let _ = result;
    }

    #[test]
    fn checkpoint_read_stdstreams_rejects_an_unknown_stream_name() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_checkpoint_read_stdstreams(
                CLIENT,
                CheckpointReadStdstreams {
                    checkpoint_id: None,
                    stream: Some("stdxyz".to_string()),
                    offset: None,
                    limit: None,
                    pattern: None,
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("unknown stream"));
    }

    #[test]
    fn checkpoint_read_stdstreams_defaults_to_stdout_of_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_checkpoint_read_stdstreams(
                CLIENT,
                CheckpointReadStdstreams {
                    checkpoint_id: None,
                    stream: None,
                    offset: None,
                    limit: None,
                    pattern: None,
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["stream"], "stdout");
        assert_eq!(json["checkpoint_id"], 0);
        assert_eq!(json["total_bytes"], 0);
    }

    #[test]
    fn checkpoint_read_stdstreams_returns_checkpoint_does_not_exist_for_bad_id() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_checkpoint_read_stdstreams(
                CLIENT,
                CheckpointReadStdstreams {
                    checkpoint_id: Some(99),
                    stream: None,
                    offset: None,
                    limit: None,
                    pattern: None,
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn checkpoint_read_stdstreams_clamps_a_limit_that_would_overflow_past_total_bytes() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");

        let result = handler
            .handle_checkpoint_read_stdstreams(
                CLIENT,
                CheckpointReadStdstreams {
                    checkpoint_id: Some(0),
                    stream: None,
                    offset: Some(0),
                    limit: Some(u64::MAX),
                    pattern: None,
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["content"], "");
    }

    #[test]
    fn checkpoint_read_stdstreams_with_pattern_returns_zero_matches_for_empty_stdout() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");

        let result = handler
            .handle_checkpoint_read_stdstreams(
                CLIENT,
                CheckpointReadStdstreams {
                    checkpoint_id: None,
                    stream: None,
                    offset: None,
                    limit: None,
                    pattern: Some("ERROR".to_string()),
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["total_matches"], 0);
        assert_eq!(json["matches"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn checkpoint_read_stdstreams_with_pattern_returns_invalid_pattern_for_bad_regex() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_checkpoint_read_stdstreams(
                CLIENT,
                CheckpointReadStdstreams {
                    checkpoint_id: None,
                    stream: None,
                    offset: None,
                    limit: None,
                    pattern: Some("(unclosed".to_string()),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("invalid_pattern"));
    }
}

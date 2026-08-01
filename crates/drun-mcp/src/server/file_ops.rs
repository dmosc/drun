use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::state::SessionState;
use crate::tools::{
    SessionCommit, SessionDeleteFile, SessionExport, SessionExtractText, SessionMount,
    SessionReadFile, SessionWriteFile,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use drun_core::TextParserUtilities;
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::path::PathBuf;

impl DrunHandler {
    pub(super) fn handle_session_read_file(
        &self,
        connection_id: &str,
        t: SessionReadFile,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session(connection_id, |_session_id, session| {
            let all_bytes = session
                .current()
                .files
                .get(&t.path)
                .ok_or_else(|| DrunError::file_not_found(&t.path).into_tool_err())?;
            let total = all_bytes.len();
            let start = (t.offset.unwrap_or(0) as usize).min(total);
            let end = t
                .limit
                .map(|l| start.saturating_add(l as usize).min(total))
                .unwrap_or(total);
            let slice = &all_bytes[start..end];

            if let Some(pattern) = &t.pattern {
                let grep = TextParserUtilities::grep(slice, pattern)
                    .map_err(|e| DrunError::from_exec(e.into()).into_tool_err())?;
                return Ok(ResponseBuilder::text(
                    serde_json::json!({
                        "path": t.path,
                        "total_matches": grep.total_matches,
                        "matches": grep.matches,
                    })
                    .to_string(),
                ));
            }

            if t.offset.is_none() && t.limit.is_none() {
                return Ok(ResponseBuilder::file_content(&t.path, all_bytes.as_slice()));
            }
            let (content, encoding) = match std::str::from_utf8(slice) {
                Ok(s) => (s.to_string(), "text"),
                Err(_) => (BASE64.encode(slice), "base64"),
            };
            Ok(ResponseBuilder::text(
                serde_json::json!({
                    "offset": start,
                    "length": slice.len(),
                    "total_bytes": total,
                    "has_more": end < total,
                    "encoding": encoding,
                    "content": content,
                })
                .to_string(),
            ))
        })
    }

    pub(super) fn handle_session_write_file(
        &self,
        connection_id: &str,
        t: SessionWriteFile,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |session_id, session| {
            let bytes = if t.is_base64.unwrap_or(false) {
                BASE64.decode(&t.content).map_err(|e| {
                    DrunError::internal(format!("base64 decode error: {e}")).into_tool_err()
                })?
            } else {
                t.content.into_bytes()
            };
            let previous_files = session.current().files.clone();
            session
                .write_file(&t.path, bytes, Some(&t.description))
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    pub(super) fn handle_session_delete_file(
        &self,
        connection_id: &str,
        t: SessionDeleteFile,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |session_id, session| {
            let previous_files = session.current().files.clone();
            session
                .delete_file(&t.path, Some(&t.description))
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    pub(super) fn handle_session_mount(
        &self,
        connection_id: &str,
        t: SessionMount,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |session_id, session| {
            let previous_files = session.current().files.clone();
            session
                .mount(std::path::Path::new(&t.path))
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    pub(super) fn handle_session_extract_text(
        &self,
        connection_id: &str,
        t: SessionExtractText,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session_mut(connection_id, |_session_id, session| {
            let saved_to = session
                .extract_text(&t.path, t.save_to.as_deref(), Some(&t.description))
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::text(
                serde_json::json!({
                    "saved_to": saved_to,
                    "checkpoint_id": session.current().id,
                })
                .to_string(),
            ))
        })
    }

    pub(super) fn handle_session_export(
        &self,
        connection_id: &str,
        t: SessionExport,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        let output_dir = PathBuf::from(&t.output_dir);
        self.config
            .get()
            .check_mount_path(&output_dir)
            .map_err(|e| DrunError::from_exec(e.into()).into_tool_err())?;
        self.with_session(&session_id, |session| {
            let exported = session
                .export(&output_dir, t.keys)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            let paths: Vec<String> = exported
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            Ok(ResponseBuilder::text(
                serde_json::json!({
                    "output_dir": output_dir.to_string_lossy(),
                    "exported_files": paths,
                })
                .to_string(),
            ))
        })
    }

    pub(super) fn handle_session_commit(
        &self,
        connection_id: &str,
        t: SessionCommit,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_current_session(connection_id, |session_id, session| {
            let paths = session
                .commit(t.keys)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            let committed_files = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            Ok(ResponseBuilder::json(&SessionState::compute(
                session_id,
                session,
                None,
                committed_files,
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
    fn session_read_file_returns_full_utf8_content_without_offset_or_limit() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hello world".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "a.txt".to_string(),
                    offset: None,
                    limit: None,
                    pattern: None,
                },
            )
            .unwrap();
        assert_eq!(result_text(&result), "hello world");
    }

    #[test]
    fn session_read_file_pages_through_content_with_offset_and_limit() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hello world".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "a.txt".to_string(),
                    offset: Some(6),
                    limit: Some(5),
                    pattern: None,
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["content"], "world");
        assert_eq!(json["has_more"], false);
        assert_eq!(json["total_bytes"], 11);
    }

    #[test]
    fn session_read_file_clamps_a_limit_that_would_overflow_past_total_bytes() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hello world".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "a.txt".to_string(),
                    offset: Some(6),
                    limit: Some(u64::MAX),
                    pattern: None,
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["content"], "world");
        assert_eq!(json["has_more"], false);
    }

    #[test]
    fn session_read_file_base64_encodes_non_utf8_content_when_paginated() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let bytes = vec![0xff, 0xfe, 0xfd];
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("bin.dat", bytes, None)
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "bin.dat".to_string(),
                    offset: Some(0),
                    limit: Some(3),
                    pattern: None,
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["encoding"], "base64");
    }

    #[test]
    fn session_read_file_returns_file_not_found_for_missing_path() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "missing.txt".to_string(),
                    offset: None,
                    limit: None,
                    pattern: None,
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("file_not_found"));
    }

    #[test]
    fn session_read_file_with_pattern_returns_only_matching_lines() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"one\nERROR: boom\nthree\n".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "a.txt".to_string(),
                    offset: None,
                    limit: None,
                    pattern: Some("^ERROR".to_string()),
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["total_matches"], 1);
        assert_eq!(json["matches"][0]["line_number"], 2);
        assert_eq!(json["matches"][0]["line"], "ERROR: boom");
        assert_eq!(json["matches"][0]["byte_offset"], "one\n".len());
    }

    #[test]
    fn session_read_file_with_pattern_searches_within_the_offset_limit_range() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"match\nmatch\nmatch\n".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "a.txt".to_string(),
                    offset: Some(0),
                    limit: Some(6),
                    pattern: Some("match".to_string()),
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["total_matches"], 1);
    }

    #[test]
    fn session_read_file_with_pattern_returns_invalid_pattern_for_bad_regex() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hi\n".to_vec(), None)
                .unwrap();
        }

        let err = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "a.txt".to_string(),
                    offset: None,
                    limit: None,
                    pattern: Some("(unclosed".to_string()),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("invalid_pattern"));
    }

    #[test]
    fn session_read_file_with_pattern_returns_binary_content_for_non_utf8_files() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("bin.dat", vec![0xff, 0xfe, 0xfd], None)
                .unwrap();
        }

        let err = handler
            .handle_session_read_file(
                CLIENT,
                SessionReadFile {
                    path: "bin.dat".to_string(),
                    offset: None,
                    limit: None,
                    pattern: Some("anything".to_string()),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("binary_content"));
    }

    #[test]
    fn session_write_file_decodes_base64_content() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let encoded = BASE64.encode(b"hello");

        handler
            .handle_session_write_file(
                CLIENT,
                SessionWriteFile {
                    path: "a.txt".to_string(),
                    content: encoded,
                    is_base64: Some(true),
                    description: "test".to_string(),
                },
            )
            .unwrap();

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(session.current().files["a.txt"].as_slice(), b"hello");
    }

    #[test]
    fn session_write_file_rejects_invalid_base64() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_write_file(
                CLIENT,
                SessionWriteFile {
                    path: "a.txt".to_string(),
                    content: "not valid base64!!".to_string(),
                    is_base64: Some(true),
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("base64 decode error"));
    }

    #[test]
    fn session_write_file_returns_invalid_workspace_path_for_a_path_escaping_the_workspace() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_write_file(
                CLIENT,
                SessionWriteFile {
                    path: "../escape.txt".to_string(),
                    content: "hi".to_string(),
                    is_base64: Some(false),
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("invalid_workspace_path"));
    }

    #[test]
    fn session_write_file_returns_workspace_size_exceeded_over_the_configured_limit() {
        let config = Config {
            max_workspace_mb: Some(0),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_write_file(
                CLIENT,
                SessionWriteFile {
                    path: "a.txt".to_string(),
                    content: "hi".to_string(),
                    is_base64: Some(false),
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("workspace_size_exceeded"));
    }

    #[test]
    fn session_delete_file_removes_the_file_and_creates_a_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.write_file("a.txt", b"hi".to_vec(), None).unwrap();
        }

        let result = handler
            .handle_session_delete_file(
                CLIENT,
                SessionDeleteFile {
                    path: "a.txt".to_string(),
                    description: "test".to_string(),
                },
            )
            .unwrap();
        assert_eq!(result_json(&result)["workspace_file_count"], 0);
    }

    #[test]
    fn session_delete_file_returns_file_not_found_for_a_missing_path() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_delete_file(
                CLIENT,
                SessionDeleteFile {
                    path: "missing.txt".to_string(),
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("file_not_found"));
    }

    #[test]
    fn session_mount_loads_a_host_directory_into_the_workspace() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("a.txt"), b"hi").unwrap();

        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_session_mount(
                CLIENT,
                SessionMount {
                    path: source.path().to_string_lossy().into_owned(),
                },
            )
            .unwrap();
        assert_eq!(result_json(&result)["workspace_file_count"], 1);
    }

    #[test]
    fn session_mount_returns_mount_denied_for_a_path_outside_the_allowlist() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("a.txt"), b"hi").unwrap();
        let allowed = tempfile::tempdir().unwrap();

        let config = Config {
            mount_allowlist: vec![allowed.path().to_path_buf()],
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_mount(
                CLIENT,
                SessionMount {
                    path: source.path().to_string_lossy().into_owned(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("mount_denied"));
    }

    #[test]
    fn session_extract_text_returns_file_not_found_for_a_missing_source() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_extract_text(
                CLIENT,
                SessionExtractText {
                    path: "missing.pdf".to_string(),
                    save_to: None,
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("file_not_found"));
    }

    #[test]
    fn session_extract_text_returns_unsupported_extraction_format_for_a_non_pdf() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("notes.docx", b"whatever".to_vec(), None)
                .unwrap();
        }
        let err = handler
            .handle_session_extract_text(
                CLIENT,
                SessionExtractText {
                    path: "notes.docx".to_string(),
                    save_to: None,
                    description: "test".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("unsupported_extraction_format"));
    }

    #[test]
    fn session_commit_writes_back_a_changed_mounted_file_to_the_host() {
        let source = tempfile::tempdir().unwrap();
        let host_file = source.path().join("a.txt");
        std::fs::write(&host_file, b"original").unwrap();

        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.mount(&host_file).unwrap();
            session
                .write_file("a.txt", b"changed".to_vec(), None)
                .unwrap();
        }

        let result = handler
            .handle_session_commit(CLIENT, SessionCommit { keys: None })
            .unwrap();
        assert_eq!(
            result_json(&result)["committed_files"][0],
            host_file
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(std::fs::read(&host_file).unwrap(), b"changed");
    }

    #[test]
    fn session_export_rejects_a_path_containing_dotdot() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_export(
                CLIENT,
                SessionExport {
                    output_dir: "../escape".to_string(),
                    keys: None,
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("mount_denied"));
    }

    #[test]
    fn session_export_rejects_a_directory_outside_the_mount_allowlist() {
        let config = Config {
            mount_allowlist: vec![PathBuf::from("/allowed")],
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_export(
                CLIENT,
                SessionExport {
                    output_dir: "/tmp/somewhere-else".to_string(),
                    keys: None,
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("mount_denied"));
    }

    #[test]
    fn session_export_writes_files_under_an_allowed_mount_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            mount_allowlist: vec![dir.path().to_path_buf()],
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("out.txt", b"data".to_vec(), None)
                .unwrap();
        }

        handler
            .handle_session_export(
                CLIENT,
                SessionExport {
                    output_dir: dir.path().join("sub").to_string_lossy().into_owned(),
                    keys: Some(vec!["out.txt".to_string()]),
                },
            )
            .unwrap();
        assert!(dir.path().join("sub/out.txt").exists());
    }

    #[test]
    fn session_export_permits_any_path_when_the_mount_allowlist_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("out.txt", b"data".to_vec(), None)
                .unwrap();
        }

        handler
            .handle_session_export(
                CLIENT,
                SessionExport {
                    output_dir: dir.path().to_string_lossy().into_owned(),
                    keys: Some(vec!["out.txt".to_string()]),
                },
            )
            .unwrap();
        assert!(dir.path().join("out.txt").exists());
    }
}

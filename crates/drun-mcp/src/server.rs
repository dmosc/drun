//! MCP tool dispatch: implements ServerHandler to route each tool call to the
//! appropriate Session method, wrapping results as MCP CallToolResult responses.

use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::response::{file_content, text};
use crate::state::{
    build_checkpoint_history, build_session_list, build_session_state, build_session_tree,
    build_snapshot_catalog,
};
use crate::tools::{
    CheckpointReadStdstreamsTool, DrunTools, GetSessionStateTool, SessionBashTool,
    SessionCheckpointDropTool, SessionCheckpointLabelTool, SessionCheckpointSquashTool,
    SessionCloseTool, SessionCommitTool, SessionDeleteFileTool, SessionDiffTool, SessionExportTool,
    SessionFetchTool, SessionForkTool, SessionGetEnvTool, SessionHistoryTool, SessionLabelTool,
    SessionMergeTool, SessionMountTool, SessionReadFileTool, SessionRestoreTool,
    SessionRollbackTool, SessionSnapshotTool, SessionWriteFileTool,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use drun_core::{Session, SessionSnapshot};
use rust_mcp_sdk::{
    McpServer,
    mcp_server::ServerHandler,
    schema::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
        ProgressNotificationParams, ProgressToken, RpcError, schema_utils::CallToolError,
    },
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

#[async_trait]
impl ServerHandler for DrunHandler {
    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: DrunTools::tools(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let progress_token = params.meta.as_ref().and_then(|m| m.progress_token.clone());
        let tool = DrunTools::try_from(params)?;
        match tool {
            DrunTools::CreateSessionTool(_) => self.handle_create_session(),
            DrunTools::SessionForkTool(t) => self.handle_session_fork(t),
            DrunTools::SessionListTool(_) => self.handle_session_list(),
            DrunTools::SessionCloseTool(t) => self.handle_session_close(t),
            DrunTools::SessionHistoryTool(t) => self.handle_session_history(t),
            DrunTools::GetSessionStateTool(t) => self.handle_get_session_state(t),
            DrunTools::SessionBashTool(t) => self.handle_session_bash(t, runtime, progress_token),
            DrunTools::SessionRollbackTool(t) => self.handle_session_rollback(t),
            DrunTools::SessionReadFileTool(t) => self.handle_session_read_file(t),
            DrunTools::SessionWriteFileTool(t) => self.handle_session_write_file(t),
            DrunTools::SessionDeleteFileTool(t) => self.handle_session_delete_file(t),
            DrunTools::SessionMountTool(t) => self.handle_session_mount(t),
            DrunTools::SessionDiffTool(t) => self.handle_session_diff(t),
            DrunTools::SessionCommitTool(t) => self.handle_session_commit(t),
            DrunTools::SessionTreeTool(_) => self.handle_session_tree(),
            DrunTools::ListSnapshotsTool(_) => self.handle_list_snapshots(),
            DrunTools::SessionExportTool(t) => self.handle_session_export(t),
            DrunTools::SessionFetchTool(t) => self.handle_session_fetch(t).await,
            DrunTools::GetFetchAllowlistTool(_) => self.handle_get_fetch_allowlist(),
            DrunTools::SessionSnapshotTool(t) => self.handle_session_snapshot(t),
            DrunTools::SessionGetEnvTool(t) => self.handle_session_get_env(t),
            DrunTools::SessionRestoreTool(t) => self.handle_session_restore(t),
            DrunTools::SessionLabelTool(t) => self.handle_session_label(t),
            DrunTools::SessionCheckpointLabelTool(t) => self.handle_session_checkpoint_label(t),
            DrunTools::SessionCheckpointSquashTool(t) => self.handle_session_checkpoint_squash(t),
            DrunTools::SessionMergeTool(t) => self.handle_session_merge(t),
            DrunTools::SessionCheckpointDropTool(t) => self.handle_session_checkpoint_drop(t),
            DrunTools::CheckpointReadStdstreamsTool(t) => self.handle_checkpoint_read_stdstreams(t),
        }
    }
}

impl DrunHandler {
    fn handle_create_session(&self) -> Result<CallToolResult, CallToolError> {
        if let Some(max) = self.config.max_sessions
            && self.sessions.lock().unwrap().len() >= max
        {
            return Err(DrunError::session_limit_reached(max).into_tool_err());
        }
        let session_id = Uuid::new_v4().to_string();
        let session =
            Session::new(&self.config).map_err(|e| DrunError::internal(e).into_tool_err())?;
        let state = build_session_state(&session_id, &session, None, vec![]);
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id, Arc::new(Mutex::new(session)));
        Ok(text(state))
    }

    fn handle_session_fork(&self, t: SessionForkTool) -> Result<CallToolResult, CallToolError> {
        let source_arc = self
            .sessions
            .lock()
            .unwrap()
            .get(&t.session_id)
            .ok_or_else(|| DrunError::session_not_found(&t.session_id).into_tool_err())?
            .clone();
        let forked_session = {
            let source = source_arc.lock().unwrap();
            let checkpoint_id = source
                .resolve_checkpoint(t.checkpoint_id, t.checkpoint_label.as_deref())
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Session::from_session(&self.config, &t.session_id, &source, checkpoint_id)
                .map_err(|e| DrunError::internal(e).into_tool_err())?
        };
        let fork_id = Uuid::new_v4().to_string();
        let state = build_session_state(&fork_id, &forked_session, None, vec![]);
        self.sessions
            .lock()
            .unwrap()
            .insert(fork_id, Arc::new(Mutex::new(forked_session)));
        Ok(text(state))
    }

    fn handle_session_list(&self) -> Result<CallToolResult, CallToolError> {
        let sessions = self.sessions.lock().unwrap().clone();
        Ok(text(build_session_list(&sessions)))
    }

    fn handle_session_close(&self, t: SessionCloseTool) -> Result<CallToolResult, CallToolError> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .remove(&t.session_id)
            .ok_or_else(|| DrunError::session_not_found(&t.session_id).into_tool_err())?;
        if self.config.snapshot_on_close {
            let output_path = self
                .config
                .snapshots_dir
                .join(format!("{}.drun", t.session_id));
            if let Some(parent_dir) = output_path.parent() {
                let _ = std::fs::create_dir_all(parent_dir);
            }
            let _ = session.lock().unwrap().snapshot().write(&output_path);
        }
        Ok(text(format!("closed {}", t.session_id)))
    }

    fn handle_session_history(
        &self,
        t: SessionHistoryTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            Ok(text(build_checkpoint_history(session)))
        })
    }

    fn handle_get_session_state(
        &self,
        t: GetSessionStateTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            Ok(text(build_session_state(
                &t.session_id,
                session,
                None,
                vec![],
            )))
        })
    }

    fn handle_session_bash(
        &self,
        t: SessionBashTool,
        runtime: Arc<dyn McpServer>,
        progress_token: Option<ProgressToken>,
    ) -> Result<CallToolResult, CallToolError> {
        let progress_tx = Self::spawn_progress_forwarder(runtime, progress_token);
        self.with_session_mut(&t.session_id, |session| {
            let previous_files = session.current().files.clone();
            session
                .execute_bash(&t.command, &mut |chunk| {
                    let _ = progress_tx.send(chunk);
                })
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(text(build_session_state(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_rollback(
        &self,
        t: SessionRollbackTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let checkpoint_id = session
                .resolve_checkpoint(t.checkpoint_id, t.checkpoint_label.as_deref())
                .map_err(|e| DrunError::internal(e).into_tool_err())?
                .ok_or_else(|| {
                    DrunError::internal("provide checkpoint_id or checkpoint_label").into_tool_err()
                })?;
            let previous_files = session.current().files.clone();
            session
                .rollback(checkpoint_id)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_session_state(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_read_file(
        &self,
        t: SessionReadFileTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            let all_bytes = session
                .current()
                .files
                .get(&t.path)
                .ok_or_else(|| DrunError::file_not_found(&t.path).into_tool_err())?;
            if t.offset.is_none() && t.limit.is_none() {
                return Ok(file_content(&t.path, all_bytes.as_slice()));
            }
            let total = all_bytes.len();
            let start = (t.offset.unwrap_or(0) as usize).min(total);
            let end = t
                .limit
                .map(|l| (start + l as usize).min(total))
                .unwrap_or(total);
            let slice = &all_bytes[start..end];
            let (content, encoding) = match std::str::from_utf8(slice) {
                Ok(s) => (s.to_string(), "text"),
                Err(_) => (BASE64.encode(slice), "base64"),
            };
            Ok(text(
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

    fn handle_session_write_file(
        &self,
        t: SessionWriteFileTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let bytes = if t.is_base64.unwrap_or(false) {
                BASE64.decode(&t.content).map_err(|e| {
                    DrunError::internal(format!("base64 decode error: {e}")).into_tool_err()
                })?
            } else {
                t.content.into_bytes()
            };
            let previous_files = session.current().files.clone();
            session
                .write_file(&t.path, bytes)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_session_state(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_delete_file(
        &self,
        t: SessionDeleteFileTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let previous_files = session.current().files.clone();
            session
                .delete_file(&t.path)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_session_state(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_mount(&self, t: SessionMountTool) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let previous_files = session.current().files.clone();
            session
                .mount(std::path::Path::new(&t.path))
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_session_state(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_diff(&self, t: SessionDiffTool) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            let from = session
                .resolve_checkpoint(t.from_checkpoint_id, t.from_checkpoint_label.as_deref())
                .map_err(|e| DrunError::internal(e).into_tool_err())?
                .unwrap_or(0);
            let to = session
                .resolve_checkpoint(t.to_checkpoint_id, t.to_checkpoint_label.as_deref())
                .map_err(|e| DrunError::internal(e).into_tool_err())?
                .unwrap_or_else(|| session.current().id);
            let diff = session
                .diff(from, to)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(if diff.is_empty() {
                "no changes".into()
            } else {
                diff
            }))
        })
    }

    fn handle_session_commit(&self, t: SessionCommitTool) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            let paths = session
                .commit(t.keys)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            let committed_files = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            Ok(text(build_session_state(
                &t.session_id,
                session,
                None,
                committed_files,
            )))
        })
    }

    fn handle_session_tree(&self) -> Result<CallToolResult, CallToolError> {
        let sessions = self.sessions.lock().unwrap().clone();
        Ok(text(build_session_tree(&sessions)))
    }

    fn handle_list_snapshots(&self) -> Result<CallToolResult, CallToolError> {
        Ok(text(build_snapshot_catalog(&self.config.snapshots_dir)))
    }

    fn handle_session_export(&self, t: SessionExportTool) -> Result<CallToolResult, CallToolError> {
        let export_root = &self.config.export_root;
        let output_dir = match &t.output_dir {
            Some(dir) => {
                let p = PathBuf::from(dir);
                if p.components().any(|c| c == std::path::Component::ParentDir) {
                    return Err(DrunError::export_denied(
                        &p.display().to_string(),
                        "path must not contain '..'",
                    )
                    .into());
                }
                if !p.starts_with(export_root) {
                    return Err(DrunError::export_denied(
                        &p.display().to_string(),
                        &export_root.display().to_string(),
                    )
                    .into());
                }
                p
            }
            None => export_root.join(&t.session_id),
        };
        self.with_session(&t.session_id, |session| {
            let exported = session
                .export(&output_dir, t.keys)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            let paths: Vec<String> = exported
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            Ok(text(
                serde_json::json!({
                    "output_dir": output_dir.to_string_lossy(),
                    "exported_files": paths,
                })
                .to_string(),
            ))
        })
    }

    async fn handle_session_fetch(
        &self,
        t: SessionFetchTool,
    ) -> Result<CallToolResult, CallToolError> {
        if !self.sessions.lock().unwrap().contains_key(&t.session_id) {
            return Err(DrunError::session_not_found(&t.session_id).into_tool_err());
        }
        let url_is_allowed =
            Self::host_from_url(&t.url).is_some_and(|h| self.config.domain_allowed(&h));
        if !url_is_allowed {
            return Err(DrunError::fetch_denied(&t.url).into_tool_err());
        }

        let method = t.method.as_deref().unwrap_or("GET").to_uppercase();
        let parsed_method = method.parse::<reqwest::Method>().map_err(|_| {
            DrunError::internal(format!("invalid HTTP method: {method}")).into_tool_err()
        })?;

        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(self.config.connect_timeout_ms))
            .timeout(Duration::from_millis(self.config.fetch_timeout_ms));
        let client = builder
            .build()
            .map_err(|e| DrunError::internal(e).into_tool_err())?;

        let mut req = client.request(parsed_method, &t.url);
        if let Some(headers) = t.headers {
            for header in headers {
                req = req.header(header.name, header.value);
            }
        }
        if let Some(body) = t.body {
            req = req.body(body);
        }

        let mut response = req
            .send()
            .await
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let max_body = self
            .config
            .max_workspace_mb
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(256 * 1024 * 1024);
        let mut body_bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| DrunError::internal(e).into_tool_err())?
        {
            body_bytes.extend_from_slice(&chunk);
            if body_bytes.len() as u64 > max_body {
                return Err(DrunError::internal(format!(
                    "response body exceeds the {} MB limit; use a smaller download \
                     or raise max_workspace_mb in server config",
                    max_body / 1024 / 1024
                ))
                .into_tool_err());
            }
        }

        let save_path = t
            .save_to
            .unwrap_or_else(|| Self::download_path_from_url(&t.url));
        let bytes_len = body_bytes.len();
        self.with_session_mut(&t.session_id, |session| {
            session
                .write_file(&save_path, body_bytes.to_vec())
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(
                serde_json::json!({
                    "status": status,
                    "bytes": bytes_len,
                    "content_type": content_type,
                    "saved_to": save_path,
                })
                .to_string(),
            ))
        })
    }

    fn handle_get_fetch_allowlist(&self) -> Result<CallToolResult, CallToolError> {
        Ok(text(
            serde_json::to_string(&self.config.domain_allowlist).unwrap(),
        ))
    }

    fn handle_session_snapshot(
        &self,
        t: SessionSnapshotTool,
    ) -> Result<CallToolResult, CallToolError> {
        let snapshots_dir = &self.config.snapshots_dir;
        let output_path = match t.path {
            Some(p) => {
                let p = PathBuf::from(p);
                if p.components().any(|c| c == std::path::Component::ParentDir) {
                    return Err(DrunError::snapshot_denied(
                        &p.display().to_string(),
                        "path must not contain '..'",
                    )
                    .into_tool_err());
                }
                if !p.starts_with(snapshots_dir) {
                    return Err(DrunError::snapshot_denied(
                        &p.display().to_string(),
                        &snapshots_dir.display().to_string(),
                    )
                    .into_tool_err());
                }
                p
            }
            None => snapshots_dir.join(format!("{}.drun", t.session_id)),
        };
        if let Some(parent_dir) = output_path.parent() {
            std::fs::create_dir_all(parent_dir)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
        }
        self.with_session(&t.session_id, |session| {
            session
                .snapshot()
                .write(&output_path)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(
                serde_json::json!({
                    "snapshot_path": output_path.to_string_lossy(),
                })
                .to_string(),
            ))
        })
    }

    fn handle_session_get_env(
        &self,
        t: SessionGetEnvTool,
    ) -> Result<CallToolResult, CallToolError> {
        if !self.sessions.lock().unwrap().contains_key(&t.session_id) {
            return Err(DrunError::session_not_found(&t.session_id).into_tool_err());
        }
        if !self.config.env_allowlist.contains(&t.name) {
            return Err(DrunError::env_var_denied(&t.name).into_tool_err());
        }
        let value = std::env::var(&t.name).unwrap_or_default();
        Ok(text(
            serde_json::json!({ "name": t.name, "value": value }).to_string(),
        ))
    }

    fn handle_session_restore(
        &self,
        t: SessionRestoreTool,
    ) -> Result<CallToolResult, CallToolError> {
        let bytes = std::fs::read(&t.path).map_err(|e| DrunError::internal(e).into_tool_err())?;
        let snapshot =
            SessionSnapshot::decode(&bytes).map_err(|e| DrunError::internal(e).into_tool_err())?;
        let restored = Session::from_snapshot(&self.config, snapshot)
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let session_id = Uuid::new_v4().to_string();
        let state = build_session_state(&session_id, &restored, None, vec![]);
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id, Arc::new(Mutex::new(restored)));
        Ok(text(state))
    }

    fn handle_session_label(&self, t: SessionLabelTool) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            session.set_label(t.label);
            Ok(text(build_session_state(
                &t.session_id,
                session,
                None,
                vec![],
            )))
        })
    }

    fn handle_session_checkpoint_label(
        &self,
        t: SessionCheckpointLabelTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let checkpoint_id = t
                .checkpoint_id
                .map(|id| id as usize)
                .unwrap_or_else(|| session.current().id);
            session
                .set_checkpoint_label(checkpoint_id, t.label)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_checkpoint_history(session)))
        })
    }

    fn handle_session_checkpoint_squash(
        &self,
        t: SessionCheckpointSquashTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            session
                .squash_checkpoints(
                    t.from_checkpoint_id as usize,
                    t.to_checkpoint_id as usize,
                    t.label,
                )
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_checkpoint_history(session)))
        })
    }

    fn handle_session_merge(&self, t: SessionMergeTool) -> Result<CallToolResult, CallToolError> {
        if t.session_id == t.source_session_id {
            return Err(DrunError::internal("cannot merge a session with itself").into_tool_err());
        }
        let source_arc = self
            .sessions
            .lock()
            .unwrap()
            .get(&t.source_session_id)
            .ok_or_else(|| DrunError::session_not_found(&t.source_session_id).into_tool_err())?
            .clone();
        let source = source_arc.lock().unwrap();
        let source_checkpoint_id = source
            .resolve_checkpoint(t.source_checkpoint_id, t.source_checkpoint_label.as_deref())
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        self.with_session_mut(&t.session_id, |session| {
            session
                .merge_from(&source, source_checkpoint_id, t.keys)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_session_state(
                &t.session_id,
                session,
                None,
                vec![],
            )))
        })
    }

    fn handle_session_checkpoint_drop(
        &self,
        t: SessionCheckpointDropTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            session
                .drop_checkpoints(t.from_checkpoint_id as usize, t.to_checkpoint_id as usize)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(text(build_checkpoint_history(session)))
        })
    }

    fn handle_checkpoint_read_stdstreams(
        &self,
        t: CheckpointReadStdstreamsTool,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            let checkpoint_id = t
                .checkpoint_id
                .map(|id| id as usize)
                .unwrap_or_else(|| session.current().id);
            let checkpoint = session.history().get(checkpoint_id).ok_or_else(|| {
                DrunError::internal(format!("checkpoint {} does not exist", checkpoint_id))
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
                .map(|l| (start + l as usize).min(total))
                .unwrap_or(total);
            Ok(text(
                serde_json::json!({
                    "stream": stream,
                    "checkpoint_id": checkpoint_id,
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

    fn spawn_progress_forwarder(
        mcp_server: Arc<dyn McpServer>,
        progress_token: Option<ProgressToken>,
    ) -> tokio::sync::mpsc::UnboundedSender<String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        if let Some(token) = progress_token {
            tokio::spawn(async move {
                while let Some(chunk) = rx.recv().await {
                    let _ = mcp_server
                        .notify_progress(ProgressNotificationParams {
                            progress: 0.0,
                            progress_token: token.clone(),
                            message: Some(chunk),
                            total: None,
                            meta: None,
                        })
                        .await;
                }
            });
        }
        tx
    }

    fn host_from_url(url: &str) -> Option<String> {
        let s = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let authority = s.split('/').next().filter(|h| !h.is_empty())?;
        let host = if authority.starts_with('[') {
            // IPv6: "[::1]" or "[::1]:port" — extract up to and including ']'
            let end = authority
                .find(']')
                .map(|i| i + 1)
                .unwrap_or(authority.len());
            authority[..end].to_string()
        } else {
            authority.split(':').next()?.to_string()
        };
        Some(host)
    }

    fn download_path_from_url(url: &str) -> String {
        let without_query = url.split('?').next().unwrap_or(url).trim_end_matches('/');
        let name = without_query
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("fetch");
        format!("downloads/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_from_url_extracts_https_host() {
        assert_eq!(
            DrunHandler::host_from_url("https://pypi.org/simple/requests/"),
            Some("pypi.org".to_string())
        );
    }

    #[test]
    fn host_from_url_extracts_http_host() {
        assert_eq!(
            DrunHandler::host_from_url("http://example.com"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn host_from_url_strips_port() {
        assert_eq!(
            DrunHandler::host_from_url("https://example.com:8080/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn host_from_url_rejects_missing_scheme() {
        assert_eq!(DrunHandler::host_from_url("example.com/path"), None);
    }

    #[test]
    fn host_from_url_rejects_unsupported_scheme() {
        assert_eq!(DrunHandler::host_from_url("ftp://example.com/foo"), None);
    }

    #[test]
    fn host_from_url_rejects_empty_authority() {
        assert_eq!(DrunHandler::host_from_url("https:///path"), None);
    }

    #[test]
    fn host_from_url_handles_ipv6_with_port() {
        assert_eq!(
            DrunHandler::host_from_url("https://[::1]:8080/path"),
            Some("[::1]".to_string())
        );
    }

    #[test]
    fn host_from_url_handles_ipv6_without_port() {
        assert_eq!(
            DrunHandler::host_from_url("https://[::1]/path"),
            Some("[::1]".to_string())
        );
    }

    #[test]
    fn download_path_from_url_uses_last_path_segment() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/path/to/file.tar.gz"),
            "downloads/file.tar.gz"
        );
    }

    #[test]
    fn download_path_from_url_strips_query_string() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/file.zip?token=abc"),
            "downloads/file.zip"
        );
    }

    #[test]
    fn download_path_from_url_strips_trailing_slash() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/dir/"),
            "downloads/dir"
        );
    }

    #[test]
    fn download_path_from_url_falls_back_to_fetch_for_empty_path() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/"),
            "downloads/example.com"
        );
        assert_eq!(DrunHandler::download_path_from_url(""), "downloads/fetch");
    }
}
